use chrono::{Duration, Local};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
    time::{Duration as Timeout, Instant},
};
use tomlook::{
    ai,
    calendar::Calendar,
    calendar_sync::Window,
    storage::Store,
    worker::{Command, Options, Worker},
};

fn receive(commands: &mut tokio::sync::mpsc::Receiver<ai::Command>) -> ai::Command {
    let deadline = Instant::now() + Timeout::from_secs(5);
    loop {
        match commands.try_recv() {
            Ok(ai::Command::Prompts(prompts)) => prompts.validate().unwrap(),
            Ok(command) => return command,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                assert!(
                    Instant::now() < deadline,
                    "Background worker did not dispatch"
                );
                std::thread::sleep(Timeout::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

#[test]
fn hidden_worker_backfills_empty_days_persists_them_and_ignores_stale_results() {
    let root = tempfile::tempdir().unwrap();
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().into(),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, mut commands) = tokio::sync::mpsc::channel(32);
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(false)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: Arc::new(AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::new()),
            workiq_ready: Arc::new(AtomicBool::new(true)),
        })
        .unwrap();
    assert!(matches!(
        receive(&mut commands),
        ai::Command::DailyAiLimit(200)
    ));
    assert!(matches!(receive(&mut commands), ai::Command::AttachWake(_)));
    assert!(matches!(
        receive(&mut commands),
        ai::Command::ContextSources(_)
    ));
    let window = Window::new(Local::now().date_naive()).unwrap();
    assert!(matches!(receive(&mut commands), ai::Command::Connect));
    let mut observed = BTreeSet::new();
    for _ in 0..7 {
        let ai::Command::SyncWorkiq { id, range } = receive(&mut commands) else {
            panic!("Calendar synchronization must not require AI or UI");
        };
        if observed.is_empty() {
            assert_eq!(range.first, window.today);
        }
        assert!((range.last - range.first).num_days() < 42);
        let days = (0..=(range.last - range.first).num_days())
            .map(|offset| range.first + Duration::days(offset))
            .collect::<BTreeSet<_>>();
        observed.extend(days.iter().copied());
        let snapshot = Arc::new(Calendar::build(Vec::new(), days, Vec::new()).unwrap());
        worker
            .commands
            .send(Command::CalendarSyncResult {
                id: id + 1000,
                range,
                result: Err("Obsolete response".into()),
            })
            .unwrap();
        worker
            .commands
            .send(Command::CalendarSyncResult {
                id,
                range,
                result: Ok((snapshot, "fixture@microsoft.com".into())),
            })
            .unwrap();
        if observed.len() == (window.last - window.first).num_days() as usize + 1 {
            break;
        }
    }
    let deadline = Instant::now() + Timeout::from_secs(5);
    while worker.activity.lock().unwrap().sync.fresh != observed.len() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Timeout::from_millis(5));
    }
    assert_eq!(observed.first(), Some(&window.first));
    assert_eq!(observed.last(), Some(&window.last));
    assert!(commands.try_recv().is_err(), "Fresh periods must not loop");
    worker.commands.send(Command::Exit).unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(Timeout::from_secs(5))
        .unwrap()
        .unwrap();
    let store = Store::open(root.path()).unwrap();
    assert_eq!(store.calendar_sync_days().unwrap().len(), observed.len());
    assert_eq!(store.calendar().unwrap().coverage, observed);
    assert!(store.calendar().unwrap().events.is_empty());
    drop(store);
    let mut restarted = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().into(),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let deadline = Instant::now() + Timeout::from_secs(5);
    while restarted.activity.lock().unwrap().sync.fresh != observed.len() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Timeout::from_millis(5));
    }
    restarted.commands.send(Command::Exit).unwrap();
    restarted
        .done
        .take()
        .unwrap()
        .recv_timeout(Timeout::from_secs(5))
        .unwrap()
        .unwrap();
}

#[test]
fn verified_calendar_result_is_saved_when_readiness_changes_before_delivery() {
    let root = tempfile::tempdir().unwrap();
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().into(),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, mut commands) = tokio::sync::mpsc::channel(32);
    let ready = Arc::new(AtomicBool::new(true));
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(false)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: Arc::new(AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::new()),
            workiq_ready: ready.clone(),
        })
        .unwrap();
    assert!(matches!(
        receive(&mut commands),
        ai::Command::DailyAiLimit(200)
    ));
    assert!(matches!(receive(&mut commands), ai::Command::AttachWake(_)));
    assert!(matches!(
        receive(&mut commands),
        ai::Command::ContextSources(_)
    ));
    assert!(matches!(receive(&mut commands), ai::Command::Connect));
    let ai::Command::SyncWorkiq { id, range } = receive(&mut commands) else {
        panic!("Expected calendar read");
    };
    ready.store(false, std::sync::atomic::Ordering::Release);
    worker.commands.send(Command::Wake).unwrap();
    let days = (0..=(range.last - range.first).num_days())
        .map(|offset| range.first + Duration::days(offset))
        .collect::<BTreeSet<_>>();
    worker
        .commands
        .send(Command::CalendarSyncResult {
            id,
            range,
            result: Ok((
                Arc::new(Calendar::build(Vec::new(), days.clone(), Vec::new()).unwrap()),
                "fixture@microsoft.com".into(),
            )),
        })
        .unwrap();
    worker.commands.send(Command::Exit).unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(Timeout::from_secs(5))
        .unwrap()
        .unwrap();
    let store = Store::open(root.path()).unwrap();
    assert_eq!(store.calendar_sync_days().unwrap().len(), days.len());
    assert_eq!(store.calendar().unwrap().coverage, days);
}
#[test]
fn future_preparation_is_bounded_and_one_failure_does_not_block_other_meetings() {
    let now = Local::now().fixed_offset();
    let mut events = tomlook::calendar::demo(Local::now(), 2);
    for (index, event) in events.iter_mut().enumerate() {
        event.start = now + Duration::days(30 + index as i64);
        event.end = event.start + Duration::minutes(30);
    }
    let coverage = events
        .iter()
        .map(|event| event.start.with_timezone(&Local).date_naive())
        .collect();
    let calendar = Calendar::build(events, coverage, Vec::new()).unwrap();
    let mut queue = tomlook::scheduler::Queue::default();
    queue.reconcile(&calendar, now);
    let (id, first) = queue.dispatch().unwrap();
    queue.background_failed(id, "Temporary provider failure".into());
    assert!(queue.failure.is_none());
    let job = &queue.jobs[&first.event_id];
    assert_eq!(job.retries, 1);
    assert!(job.retry_at.unwrap() > now);
    let (id, second) = queue.dispatch().unwrap();
    assert_ne!(first.event_id, second.event_id);
    queue.finish(id, Ok(()));
    assert!(
        queue.dispatch().is_none(),
        "Backoff must prevent immediate resubmission"
    );
    let mut waiting: tomlook::scheduler::Queue =
        serde_json::from_str(&serde_json::to_string(&queue).unwrap()).unwrap();
    waiting.recover().unwrap();
    waiting.reconcile(&calendar, now);
    assert!(
        waiting.dispatch().is_none(),
        "Restart must preserve retry delay"
    );
    for attempt in 2..=4 {
        queue.jobs.get_mut(&first.event_id).unwrap().retry_at = Some(now - Duration::seconds(1));
        queue.reconcile(&calendar, now);
        let (id, _) = queue.dispatch().unwrap();
        queue.background_failed(id, "Still unavailable".into());
        if attempt < 4 {
            assert_eq!(queue.jobs[&first.event_id].retries, attempt);
        }
    }
    assert_eq!(
        queue.jobs[&first.event_id].state,
        tomlook::scheduler::State::Failed
    );
    assert!(queue.failure.is_none());
    let json = serde_json::to_string(&queue).unwrap();
    let mut restored: tomlook::scheduler::Queue = serde_json::from_str(&json).unwrap();
    restored.recover().unwrap();
    assert_eq!(restored.jobs[&first.event_id].retries, 3);
}

#[test]
fn upcoming_briefings_refresh_daily_but_distant_results_are_reused() {
    let now = Local::now().fixed_offset();
    let mut events = tomlook::calendar::demo(Local::now(), 2);
    for (index, event) in events.iter_mut().enumerate() {
        event.start = now + Duration::days(if index == 0 { 2 } else { 90 });
        event.end = event.start + Duration::minutes(30);
    }
    let coverage = events
        .iter()
        .map(|event| event.start.with_timezone(&Local).date_naive())
        .collect();
    let calendar = Calendar::build(events, coverage, Vec::new()).unwrap();
    let mut queue = tomlook::scheduler::Queue::default();
    queue.reconcile(&calendar, now);
    for _ in 0..2 {
        let (id, _) = queue.dispatch().unwrap();
        queue.finish(id, Ok(()));
    }
    for job in queue.jobs.values_mut() {
        job.prepared_at = Some(now - Duration::hours(25));
    }
    queue.reconcile(&calendar, now);
    assert_eq!(
        queue.jobs[&calendar.events[0].id].state,
        tomlook::scheduler::State::Queued
    );
    assert_eq!(
        queue.jobs[&calendar.events[1].id].state,
        tomlook::scheduler::State::Completed
    );
    queue.request_now(&calendar.events[0], true).unwrap();
    let (id, _) = queue.dispatch_for(false).unwrap();
    queue.finish(id, Ok(()));
    queue
        .jobs
        .get_mut(&calendar.events[0].id)
        .unwrap()
        .prepared_at = Some(now - Duration::hours(25));
    queue.reconcile(&calendar, now);
    assert!(
        queue.dispatch_for(false).is_none(),
        "A one-off manual request must not repeat while automatic preparation is off"
    );
    let upcoming = queue.jobs.get_mut(&calendar.events[0].id).unwrap();
    upcoming.manual = false;
    upcoming.prepared_at = None;
    queue
        .jobs
        .get_mut(&calendar.events[1].id)
        .unwrap()
        .prepared_at = None;
    queue.reconcile(&calendar, now);
    assert_eq!(
        queue.jobs[&calendar.events[0].id].state,
        tomlook::scheduler::State::Queued,
        "Legacy upcoming completions without an age must refresh once"
    );
    assert_eq!(
        queue.jobs[&calendar.events[1].id].state,
        tomlook::scheduler::State::Completed
    );
}

#[test]
fn invalid_background_coverage_preserves_cache_and_backs_off() {
    let root = tempfile::tempdir().unwrap();
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().into(),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, mut commands) = tokio::sync::mpsc::channel(32);
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(false)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: Arc::new(AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::new()),
            workiq_ready: Arc::new(AtomicBool::new(true)),
        })
        .unwrap();
    assert!(matches!(
        receive(&mut commands),
        ai::Command::DailyAiLimit(200)
    ));
    assert!(matches!(receive(&mut commands), ai::Command::AttachWake(_)));
    assert!(matches!(
        receive(&mut commands),
        ai::Command::ContextSources(_)
    ));
    assert!(matches!(receive(&mut commands), ai::Command::Connect));
    let ai::Command::SyncWorkiq { id, range } = receive(&mut commands) else {
        panic!("No background read");
    };
    let partial = Calendar::build(Vec::new(), BTreeSet::from([range.first]), Vec::new()).unwrap();
    worker
        .commands
        .send(Command::CalendarSyncResult {
            id,
            range,
            result: Ok((Arc::new(partial), "fixture@microsoft.com".into())),
        })
        .unwrap();
    let deadline = Instant::now() + Timeout::from_secs(5);
    while worker.activity.lock().unwrap().sync.error.is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Timeout::from_millis(5));
    }
    assert_eq!(worker.activity.lock().unwrap().sync.fresh, 0);
    assert!(
        worker.activity.lock().unwrap().error.is_none(),
        "Calendar failure must not stop the AI queue"
    );
    assert!(
        commands.try_recv().is_err(),
        "Calendar retries must back off"
    );
    worker.commands.send(Command::Exit).unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(Timeout::from_secs(5))
        .unwrap()
        .unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(store.calendar_sync_days().unwrap().is_empty());
    assert!(store.calendar_owner().unwrap().is_none());
}
