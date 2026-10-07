use chrono::{DateTime, Duration, Local};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
    time::Instant,
};
use tomlook::{
    ai,
    calendar::{self, Calendar},
    scheduler::{Queue, State},
    storage::{Preferences, Store},
    worker::{Command, Options, Worker, validate_prepared},
};

fn clock() -> DateTime<chrono::FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-08T10:00:00Z").unwrap()
}
fn fixture(count: usize) -> Calendar {
    let now = clock();
    let mut events = calendar::demo(Local::now(), count);
    for (index, event) in events.iter_mut().enumerate() {
        event.id = format!("fixture-{index}");
        event.start = now + Duration::minutes(index as i64 + 1);
        event.end = event.start + Duration::minutes(30);
        event.status = "confirmed".into();
        event.is_all_day = false;
    }
    let coverage = events
        .iter()
        .map(|event| event.start.with_timezone(&Local).date_naive())
        .collect();
    Calendar::build(events, coverage, Vec::new()).unwrap()
}

#[test]
fn burst_is_bounded_nearest_first_and_overflow_is_not_lost() {
    let calendar = fixture(5000);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    assert_eq!(queue.jobs.len(), 5000);
    assert_eq!(
        queue
            .jobs
            .values()
            .filter(|job| job.state == State::Queued)
            .count(),
        64
    );
    assert_eq!(
        queue
            .jobs
            .values()
            .filter(|job| job.state == State::Deferred)
            .count(),
        4936
    );
    let (_, first) = queue.dispatch().unwrap();
    assert_eq!(first.event_id, "fixture-0");
    assert!(
        queue.dispatch().is_none(),
        "second background job must not occupy foreground capacity"
    );
    assert!(queue.promote("fixture-4999"));
    assert_eq!(queue.jobs["fixture-4999"].state, State::Queued);
    let (_, promoted) = queue.dispatch().unwrap();
    assert_eq!(promoted.event_id, "fixture-4999");
    assert!(promoted.foreground);
    assert_eq!(queue.flights.len(), 2);
    assert!(queue.dispatch().is_none());
    assert_eq!(
        queue
            .jobs
            .values()
            .filter(|job| job.state == State::Queued)
            .count(),
        64
    );
}

#[test]
fn eligibility_includes_ongoing_but_never_past_cancelled_or_unverified_work() {
    let mut calendar = fixture(5);
    calendar.events[0].start = clock() - Duration::minutes(20);
    calendar.events[0].end = clock() + Duration::minutes(10);
    calendar.events[1].end = clock();
    calendar.events[1].start = clock() - Duration::hours(1);
    calendar.events[2].status = "cancelled".into();
    calendar.events[3].start = clock() + Duration::days(7);
    calendar.events[3].end = calendar.events[3].start + Duration::minutes(30);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    assert_eq!(queue.jobs.len(), 2);
    assert_eq!(queue.dispatch().unwrap().1.event_id, "fixture-0");
    let mut unknown = Queue::default();
    calendar.coverage = BTreeSet::new();
    unknown.reconcile(&calendar, clock());
    assert!(unknown.promote("fixture-4"));
    assert!(unknown.dispatch().is_none());
    assert!(
        unknown
            .jobs
            .values()
            .all(|job| job.state == State::Deferred)
    );
}

#[test]
fn revisions_cancel_old_work_and_cannot_execute_two_copies_of_an_identity() {
    let mut calendar = fixture(2);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    let (id, flight) = queue.dispatch().unwrap();
    calendar.events[0].title = "Changed meeting".into();
    assert_eq!(queue.reconcile(&calendar, clock()), vec![id]);
    assert!(queue.current(id).is_none());
    queue.promote(&flight.event_id);
    assert!(queue.dispatch().is_none());
    queue.finish(id, Ok(()));
    assert_ne!(queue.jobs[&flight.event_id].state, State::Completed);
    let (next_id, next) = queue.dispatch().unwrap();
    assert_ne!(id, next_id);
    assert_ne!(flight.fingerprint, next.fingerprint);
    calendar.events[0]
        .extra
        .insert("isCancelled".into(), true.into());
    assert_eq!(queue.reconcile(&calendar, clock()), vec![next_id]);
    queue.finish(next_id, Ok(()));
    assert_eq!(queue.jobs["fixture-0"].state, State::Cancelled);
}

#[test]
fn restart_requires_explicit_retry_and_saved_completion_is_atomic() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path()).unwrap();
    let calendar = fixture(1);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    let (_, flight) = queue.dispatch().unwrap();
    store.save_queue(&queue).unwrap();
    let mut recovered = store.queue().unwrap();
    recovered.reconcile(&calendar, clock());
    assert_eq!(recovered.jobs["fixture-0"].state, State::Interrupted);
    assert!(recovered.dispatch().is_none());
    assert!(recovered.retry("fixture-0"));
    let (request, _) = recovered.dispatch().unwrap();
    let payload = validate_prepared(r#"{"summary":"Fixture summary","sources":[],"gaps":["Synthetic fixture; no live evidence"]}"#, &flight.fingerprint).unwrap();
    recovered.finish(request, Ok(()));
    store
        .save_prepared(&recovered, "fixture-0", &payload)
        .unwrap();
    drop(store);
    let store = Store::open(root.path()).unwrap();
    assert_eq!(
        store.queue().unwrap().jobs["fixture-0"].state,
        State::Completed
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&store.briefing("fixture-0").unwrap().unwrap())
            .unwrap(),
        payload
    );
    assert!(validate_prepared(r#"{"summary":"Bad","sources":[{"title":"Invalid","url":"javascript:alert(1)"}],"gaps":[]}"#, &flight.fingerprint).is_err());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&store.briefing("fixture-0").unwrap().unwrap())
            .unwrap(),
        payload
    );
}

#[test]
fn storage_finishes_preparation_without_any_ui_notice_drain() {
    let root = tempfile::tempdir().unwrap();
    let now = Local::now().fixed_offset();
    let mut calendar = fixture(2);
    for event in &mut calendar.events {
        event.start = now + Duration::minutes(10);
        event.end = now + Duration::minutes(40);
    }

    calendar.coverage = BTreeSet::from([now.with_timezone(&Local).date_naive()]);
    {
        let mut store = Store::open(root.path()).unwrap();
        store.save_calendar(&calendar).unwrap();
        store
            .save_preferences(&Preferences {
                prepare_enabled: true,
                ..Default::default()
            })
            .unwrap();
    }
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().into(),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, mut commands) = tokio::sync::mpsc::channel(64);
    let terminated = Arc::new(AtomicBool::new(false));
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(true)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: terminated.clone(),
        })
        .unwrap();
    assert!(matches!(
        commands.blocking_recv(),
        Some(ai::Command::AttachWake(_))
    ));
    let Some(ai::Command::Prepare(request)) = commands.blocking_recv() else {
        panic!("preparation not dispatched");
    };
    request
        .reply
        .blocking_send(ai::Prepared {
            id: request.id,
            result: ai::PreparationResult::Finished(Ok(Arc::new(
                r#"{"summary":"Finished with no UI","sources":[],"gaps":["Synthetic fixture"]}"#
                    .into(),
            ))),
        })
        .unwrap();
    request.wake.send(Command::Wake).unwrap();
    let start = Instant::now();
    while worker
        .activity
        .lock()
        .unwrap()
        .queue
        .jobs
        .get(&request.event.id)
        .is_none_or(|job| job.state != State::Completed)
    {
        assert!(start.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let Some(ai::Command::Prepare(second)) = commands.blocking_recv() else {
        panic!("second preparation not dispatched");
    };
    terminated.store(true, std::sync::atomic::Ordering::Release);
    second.wake.send(Command::Wake).unwrap();
    let start = Instant::now();
    while worker.activity.lock().unwrap().queue.jobs[&second.event.id].state != State::Interrupted {
        assert!(start.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(worker.activity.lock().unwrap().error.is_some());
    worker.commands.send(Command::Exit).unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap()
        .unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(
        store
            .briefing(&request.event.id)
            .unwrap()
            .unwrap()
            .contains("Finished with no UI")
    );
}

#[test]
fn latest_foreground_demand_precedes_older_promotions() {
    let calendar = fixture(5);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    queue.dispatch().unwrap();
    queue.promote("fixture-1");
    queue.promote("fixture-4");
    assert_eq!(queue.dispatch().unwrap().1.event_id, "fixture-4");
}

#[test]
fn coverage_revocation_and_elapsed_results_cannot_publish() {
    let mut calendar = fixture(1);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    let (id, _) = queue.dispatch().unwrap();
    calendar.coverage.clear();
    assert_eq!(queue.reconcile(&calendar, clock()), vec![id]);
    assert!(queue.current(id).is_none());
    queue.finish(id, Ok(()));
    assert_eq!(queue.jobs["fixture-0"].state, State::Deferred);
    calendar
        .coverage
        .insert(calendar.events[0].start.with_timezone(&Local).date_naive());
    queue.reconcile(&calendar, clock());
    let (id, _) = queue.dispatch().unwrap();
    queue.reconcile(&calendar, clock() + Duration::hours(1));
    queue.finish(id, Ok(()));
    assert_eq!(queue.jobs["fixture-0"].state, State::Cancelled);
}

#[test]
fn malformed_ledgers_and_request_exhaustion_stop_visibly() {
    let calendar = fixture(1);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    let mut corrupt = queue.clone();
    corrupt.jobs.get_mut("fixture-0").unwrap().event_id = "wrong".into();
    assert!(corrupt.recover().is_err());
    let (id, _) = queue.dispatch().unwrap();
    let mut corrupt = queue.clone();
    corrupt.flights.remove(&id);
    assert!(corrupt.recover().is_err());
    let mut corrupt = queue.clone();
    corrupt.flights.insert(id + 1, corrupt.flights[&id].clone());
    corrupt.sequence += 1;
    assert!(corrupt.recover().is_err());
    queue.finish(id, Err("Unknown provider outcome".into()));
    assert!(queue.failure.is_some());
    queue.recover().unwrap();
    assert!(queue.failure.is_some());
    queue.retry("fixture-0");
    assert!(queue.failure.is_none());
    queue.sequence = (1 << 63) - 1;
    assert!(queue.dispatch().is_none());
    assert!(queue.failure.as_ref().unwrap().contains("identity limit"));
}

#[test]
fn hidden_notice_backpressure_and_exit_do_not_lose_a_ready_completion() {
    let root = tempfile::tempdir().unwrap();
    let now = Local::now().fixed_offset();
    let mut calendar = fixture(1);
    calendar.events[0].start = now + Duration::minutes(10);
    calendar.events[0].end = now + Duration::minutes(40);
    calendar.coverage = BTreeSet::from([now.with_timezone(&Local).date_naive()]);
    {
        let mut store = Store::open(root.path()).unwrap();
        store.save_calendar(&calendar).unwrap();
        store
            .save_preferences(&Preferences {
                prepare_enabled: true,
                ..Default::default()
            })
            .unwrap();
    }
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().into(),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, mut commands) = tokio::sync::mpsc::channel(64);
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(true)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: Arc::new(AtomicBool::new(false)),
        })
        .unwrap();
    assert!(matches!(
        commands.blocking_recv(),
        Some(ai::Command::AttachWake(_))
    ));
    let Some(ai::Command::Prepare(request)) = commands.blocking_recv() else {
        panic!("missing dispatch");
    };
    // More than the former 64-slot notification capacity, with no UI consumer.
    for revision in 0..80 {
        worker
            .commands
            .send(Command::Detail {
                revision,
                id: request.event.id.clone(),
            })
            .unwrap();
    }
    request
        .reply
        .blocking_send(ai::Prepared {
            id: request.id,
            result: ai::PreparationResult::Finished(Ok(Arc::new(
                r#"{"summary":"Saved during exit","sources":[],"gaps":["Fixture"]}"#.into(),
            ))),
        })
        .unwrap();
    worker
        .stop
        .store(true, std::sync::atomic::Ordering::Release);
    let _ = worker.commands.try_send(Command::Wake);
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    let store = Store::open(root.path()).unwrap();
    assert_eq!(
        store.queue().unwrap().jobs[&request.event.id].state,
        State::Completed
    );
    assert!(
        store
            .briefing(&request.event.id)
            .unwrap()
            .unwrap()
            .contains("Saved during exit")
    );
}

#[test]
#[cfg(windows)]
fn profile_ownership_prevents_parallel_workers_and_releases_on_exit() {
    let root = tempfile::tempdir().unwrap();
    let ownership = tomlook::instance::acquire(root.path()).unwrap();
    assert!(
        tomlook::instance::acquire(root.path())
            .unwrap_err()
            .contains("already open")
    );
    drop(ownership);
    tomlook::instance::acquire(root.path()).unwrap();
}

#[test]
fn appearance_updates_cannot_overwrite_newer_preparation_controls() {
    let root = tempfile::tempdir().unwrap();
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().into(),
            demo: Some(200),
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    worker
        .commands
        .send(Command::EnablePreparation(true))
        .unwrap();
    worker
        .commands
        .send(Command::PausePreparation(true))
        .unwrap();
    worker
        .commands
        .send(Command::Preferences(Preferences {
            dark: false,
            ..Default::default()
        }))
        .unwrap();
    worker
        .stop
        .store(true, std::sync::atomic::Ordering::Release);
    let _ = worker.commands.try_send(Command::Wake);
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    let preferences = Store::open(root.path()).unwrap().preferences().unwrap();
    assert!(preferences.prepare_enabled);
    assert!(preferences.paused);
    assert!(!preferences.dark);
}

#[test]
fn shutdown_acknowledgement_reports_failed_final_persistence() {
    let root = tempfile::tempdir().unwrap();
    drop(Store::open(root.path()).unwrap());
    let connection = rusqlite::Connection::open(root.path().join("tomlook.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER refuse_queue BEFORE INSERT ON settings
         WHEN NEW.key='preparation-v1'
         BEGIN SELECT RAISE(FAIL, 'Fixture persistence failure'); END;",
        )
        .unwrap();
    drop(connection);
    let mut worker = Worker::start(
        Options {
            state: root.path().to_owned(),
            legacy_root: root.path().to_owned(),
            demo: Some(200),
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    worker
        .stop
        .store(true, std::sync::atomic::Ordering::Release);
    let _ = worker.commands.try_send(Command::Wake);
    let error = worker
        .done
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap_err();
    assert!(error.contains("Fixture persistence failure"), "{error}");
}
