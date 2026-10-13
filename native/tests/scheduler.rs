use chrono::{DateTime, Duration, Local};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    worker::{
        ActivityFilter, ActivityIndex, Command, Options, Worker, validate_prepared,
        validate_researched,
    },
};

#[test]
fn connector_identity_changes_rebind_worker_jobs_and_empty_identity_restores_base_profile() {
    use std::sync::atomic::Ordering;
    use std::time::Duration as Wait;
    let root = tempfile::tempdir().unwrap();
    let calendar = Calendar::build(
        calendar::fixture(200),
        calendar::fixture_coverage(),
        Vec::new(),
    )
    .unwrap();
    let preferences = Preferences {
        paused: true,
        ..Default::default()
    };
    {
        let mut store = Store::open(root.path()).unwrap();
        store.save_calendar(&calendar).unwrap();
        store.save_preferences(&preferences).unwrap();
    }

    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().join("no-legacy"),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, _commands) = tokio::sync::mpsc::channel(64);
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(true)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: Arc::new(AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::from("a".repeat(64))),
            workiq_ready: Arc::new(AtomicBool::new(false)),
        })
        .unwrap();
    let wait_for = |expected: &str| {
        let deadline = Instant::now() + Wait::from_secs(5);
        loop {
            let snapshot = worker.activity.lock().unwrap().clone();
            if snapshot.queue.analysis_revision == expected && !snapshot.queue.jobs.is_empty() {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "Connector identity did not publish: expected {expected}, actual {}, jobs={}",
                snapshot.queue.analysis_revision,
                snapshot.queue.jobs.len()
            );
            std::thread::sleep(Wait::from_millis(2));
        }
    };
    use sha2::{Digest, Sha256};
    let profile = "a".repeat(64);
    let owner: Option<String> = None;
    let base = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(&profile, &owner, preferences.sources())).unwrap())
    );
    let first = wait_for(&base);
    let connector = "b".repeat(64);
    worker
        .commands
        .send(Command::ConnectorRevision(connector.clone()))
        .unwrap();
    let changed = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&profile, &owner, preferences.sources(), &connector)).unwrap()
        )
    );
    let second = wait_for(&changed);
    for (id, old) in &first.queue.jobs {
        assert_ne!(old.fingerprint, second.queue.jobs[id].fingerprint);
    }
    worker
        .commands
        .send(Command::ConnectorRevision(String::new()))
        .unwrap();
    let restored = wait_for(&base);
    for (id, old) in &first.queue.jobs {
        assert_eq!(old.fingerprint, restored.queue.jobs[id].fingerprint);
    }
    assert!(!worker.stop.load(Ordering::Acquire));
    worker.commands.send(Command::Exit).unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(Wait::from_secs(3))
        .unwrap()
        .unwrap();
    assert_eq!(
        Store::open(root.path())
            .unwrap()
            .queue()
            .unwrap()
            .analysis_revision,
        base
    );
}

#[test]
fn offline_preview_details_are_synthetic_and_never_import_the_legacy_calendar() {
    use tomlook::worker::Notice;
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join("legacy");
    std::fs::create_dir(&legacy).unwrap();
    std::fs::write(legacy.join("do-not-read.txt"), "private legacy fixture").unwrap();
    let mut worker = Worker::start(
        Options {
            state: root.path().join("demo"),
            legacy_root: legacy,
            demo: Some(200),
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    let event_id = loop {
        if let Ok(Notice::Loaded {
            calendar,
            calendar_owner,
            ..
        }) = worker.notices.try_recv()
        {
            assert!(calendar_owner.is_none());
            assert_eq!(calendar.events.len(), 200);
            assert!(
                calendar
                    .events
                    .iter()
                    .all(|event| event.extra["synthetic"] == true)
            );
            break calendar
                .events
                .iter()
                .find(|event| event.title == "Northstar / Architecture workshop")
                .unwrap()
                .id
                .clone();
        }
        assert!(Instant::now() < deadline, "Preview did not load");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    worker
        .commands
        .send(Command::Detail {
            revision: 7,
            id: event_id,
        })
        .unwrap();
    loop {
        if let Ok(Notice::Detail { revision, payload }) = worker.notices.try_recv() {
            assert_eq!(revision, 7);
            let payload = payload.unwrap();
            assert_eq!(payload["synthetic"], true);
            assert!(
                payload["summary"]
                    .as_str()
                    .unwrap()
                    .starts_with("You lead the workshop.")
            );
            assert!(payload["sources"].as_array().unwrap().is_empty());
            break;
        }
        assert!(Instant::now() < deadline, "Sample Insights did not load");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
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
    let store = Store::open(&root.path().join("demo")).unwrap();
    assert!(store.calendar().unwrap().events.is_empty());
    assert!(store.calendar_owner().unwrap().is_none());
}

#[test]
fn detailed_insights_schema_is_bounded_and_preserved_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let raw = serde_json::json!({
        "summary": "A short evidence-led overview",
        "preparation": ["Open the retrieved demo"],
        "talking_points": ["Confirm the next milestone"],
        "open_questions": ["Who owns the follow-up?"],
        "risks": ["No previous decision found"],
        "gaps": ["Teams history unavailable in this fixture"],
        "cs": {
            "summary": "Stručný přehled doložených souvislostí",
            "preparation": ["Otevřít dohledané demo"],
            "talking_points": ["Potvrdit další milník"],
            "open_questions": ["Kdo zajistí další krok?"],
            "risks": ["Předchozí rozhodnutí nebylo nalezeno"],
            "gaps": ["Historie Teams není v testovacích podkladech dostupná"]
        },
        "sources": [{"title": "Original message", "url": "https://outlook.office.com/mail/fixture"}]
    });
    let payload = validate_prepared(&raw.to_string(), "fixture-fingerprint").unwrap();
    let mut store = Store::open(root.path()).unwrap();
    store
        .save_prepared(&Queue::default(), "fixture-insights", &payload)
        .unwrap();
    drop(store);
    let store = Store::open(root.path()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&store.briefing("fixture-insights").unwrap().unwrap()).unwrap();
    assert_eq!(saved["talking_points"], raw["talking_points"]);
    assert_eq!(saved["open_questions"], raw["open_questions"]);
    assert_eq!(saved["cs"], raw["cs"]);
    let mut oversized = raw.clone();
    oversized["risks"] = serde_json::json!(["x".repeat(2001)]);
    assert!(validate_prepared(&oversized.to_string(), "fixture").is_err());
    let mut invalid = raw;
    invalid["unrecognized_field"] = true.into();
    assert!(validate_prepared(&invalid.to_string(), "fixture").is_err());
}

#[test]
fn czech_backfill_preserves_every_original_source_and_rejects_invalid_translations() {
    let previous = serde_json::json!({
        "summary": "Unverified summary", "sources": [{
            "title": "Original", "url": "https://example.com/original", "retrieved_records": [2]
        }],
        "preparation": ["Review: Check the original"], "gaps": ["Original message not read"],
        "evidence_status": "Claims remain unverified",
        "retrieved_evidence": {"records": [{"ordinal": 2, "observed_at": "2026-10-01T09:00:00Z"}]},
        "tomlook_fingerprint": "previous-fingerprint"
    });
    let cs = serde_json::json!({
        "summary": "Neověřený souhrn", "preparation": ["Kontrola: Ověřit původní zdroj"],
        "talking_points": [], "open_questions": [], "risks": [],
        "gaps": ["Původní zpráva nebyla přečtena"]
    });
    let translated =
        tomlook::worker::validate_translation(&previous, &cs.to_string(), "new-fingerprint")
            .unwrap();
    for key in [
        "summary",
        "sources",
        "retrieved_evidence",
        "evidence_status",
        "gaps",
        "preparation",
    ] {
        assert_eq!(translated[key], previous[key], "Backfill changed {key}");
    }
    assert_eq!(translated["cs"], cs);
    assert_eq!(translated["translation_only"], true);
    let prompt = tomlook::worker::translation_prompt(&previous).unwrap();
    assert!(
        !prompt.contains("https://example.com/original"),
        "Translation sends narrative, not sources or captured data"
    );
    assert!(!prompt.contains("observed_at"));
    let mut invalid = cs.clone();
    invalid["sources"] = serde_json::json!([]);
    assert!(tomlook::worker::validate_translation(&previous, &invalid.to_string(), "new").is_err());
    invalid = cs.clone();
    invalid["gaps"] = serde_json::json!([]);
    assert!(tomlook::worker::validate_translation(&previous, &invalid.to_string(), "new").is_err());
    invalid = cs;
    invalid["summary"] = "".into();
    assert!(tomlook::worker::validate_translation(&previous, &invalid.to_string(), "new").is_err());
    assert!(tomlook::worker::validate_translation(&previous, "null", "new").is_err());
    let root = tempfile::tempdir().unwrap();
    Store::open(root.path())
        .unwrap()
        .save_prepared(&Queue::default(), "fixture", &translated)
        .unwrap();
    let saved: serde_json::Value = serde_json::from_str(
        &Store::open(root.path())
            .unwrap()
            .briefing("fixture")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved, translated);
}

#[test]
fn researched_citations_keep_matching_records_gaps_and_provenance_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let recorder =
        tomlook::evidence::Recorder::new(Arc::new(tomlook::evidence::Redactor::default()));
    recorder.record(
        tomlook::workiq::connection::CONTEXT_TOOL,
        &serde_json::json!({
            "answer": "[Message](https://outlook.office.com/mail/actual)"
        }),
        true,
    );
    recorder.record_workiq(
        "/me/messages/actual?$select=body,webLink",
        &serde_json::json!({
            "body": "x".repeat(8000),
            "webLink": "https://outlook.office.com/mail/actual"
        }),
    );
    recorder.record(
        tomlook::workiq::connection::SDK_TOOL,
        &serde_json::json!({
            "webLink": "https://outlook.office.com/mail/failed"
        }),
        false,
    );
    let raw = serde_json::json!({
        "summary": "Synthetic source linkage, not a factual verification",
        "gaps": ["Teams original content unavailable"],
        "cs": {
            "summary": "Testovací vazby na zdroje, nikoli ověření tvrzení",
            "preparation": [], "talking_points": [], "open_questions": [], "risks": [],
            "gaps": ["Původní obsah Teams není dostupný"]
        },
        "sources": [
            {"title": "Read message", "url": "https://outlook.office.com/mail/actual"},
            {"title": "Failed read", "url": "https://outlook.office.com/mail/failed"},
            {"title": "Uncaptured model citation", "url": "https://example.com/uncaptured"}
        ]
    });
    let payload = validate_researched(
        ai::Answer::new(
            raw.to_string(),
            recorder.snapshot().unwrap(),
            Arc::new(Vec::new()),
        ),
        "fixture-fingerprint",
    )
    .unwrap();
    assert_eq!(
        payload["sources"][0]["retrieved_records"],
        serde_json::json!([1, 2])
    );
    assert_eq!(
        payload["sources"][1]["retrieved_records"],
        serde_json::json!([])
    );
    assert_eq!(
        payload["sources"][2]["retrieved_records"],
        serde_json::json!([])
    );
    assert_eq!(payload["sources"].as_array().unwrap().len(), 3);
    assert_eq!(payload["gaps"], raw["gaps"]);
    assert_eq!(
        payload["evidence_status"],
        "1/3 cited links in captured results. Claims remain unverified."
    );
    let mut store = Store::open(root.path()).unwrap();
    store
        .save_prepared(&Queue::default(), "fixture-citations", &payload)
        .unwrap();
    drop(store);
    let store = Store::open(root.path()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&store.briefing("fixture-citations").unwrap().unwrap()).unwrap();
    assert_eq!(saved, payload);
    assert_eq!(
        saved["retrieved_evidence"]["records"][1]["resource"],
        "/me/messages/actual?$select=body,webLink"
    );
    assert_eq!(saved["retrieved_evidence"]["records"][1]["truncated"], true);
    let mut forged = raw;
    forged["sources"][0]["retrieved_records"] = serde_json::json!([1]);
    assert!(validate_prepared(&forged.to_string(), "fixture").is_err());
}

#[test]
fn credential_bearing_citations_are_rejected_not_rendered_as_original_sources() {
    for url in [
        "https://example.com/source?access_token=fixture",
        "https://example.com/source?sig=fixture",
        "https://example.com/source?API%5FKEY=fixture",
    ] {
        let raw = serde_json::json!({
            "summary": "Fixture",
            "gaps": [],
            "sources": [{"title": "Unsafe citation", "url": url}]
        });
        assert!(validate_prepared(&raw.to_string(), "fixture").is_err());
    }
}

#[test]
fn manual_insights_can_queue_offline_and_survive_restart_while_automatic_work_is_paused() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path()).unwrap();
    store.save_calendar(&fixture(1)).unwrap();
    store
        .save_preferences(&Preferences {
            paused: true,
            prepare_enabled: false,
            ..Default::default()
        })
        .unwrap();
    drop(store);
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().into(),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, _commands) = tokio::sync::mpsc::channel(64);
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(false)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: Arc::new(AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::new()),
            workiq_ready: Arc::new(AtomicBool::new(false)),
        })
        .unwrap();
    worker
        .commands
        .send(Command::PrepareNow("fixture-0".into()))
        .unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if worker
            .activity
            .lock()
            .unwrap()
            .queue
            .jobs
            .get("fixture-0")
            .is_some_and(|job| job.manual)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Offline manual request did not reach the durable queue"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(worker.activity.lock().unwrap().queue.flights.is_empty());
    worker.commands.send(Command::Exit).unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    let store = Store::open(root.path()).unwrap();
    let queue = store.queue().unwrap();
    assert!(queue.jobs["fixture-0"].manual);
    assert_eq!(queue.jobs["fixture-0"].state, State::Queued);
    assert!(store.preferences().unwrap().paused);
    assert!(!store.preferences().unwrap().prepare_enabled);
}

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
fn activity_snapshot_indexes_preserve_every_state_order_and_original_job() {
    let calendar = fixture(5000);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    for (index, job) in queue.jobs.values_mut().enumerate() {
        job.state = [
            State::Deferred,
            State::Queued,
            State::Running,
            State::Completed,
            State::Failed,
            State::Interrupted,
            State::Cancelled,
        ][index % 7];
        job.start = clock() + Duration::minutes(index as i64 % 3);
    }
    let index = ActivityIndex::new(&queue);
    for filter in [
        ActivityFilter::All,
        ActivityFilter::Working,
        ActivityFilter::Waiting,
        ActivityFilter::Problems,
        ActivityFilter::Ready,
    ] {
        let mut expected = queue
            .jobs
            .values()
            .filter(|job| filter.matches(job.state))
            .collect::<Vec<_>>();
        expected.sort_by_key(|job| {
            (
                match job.state {
                    State::Running => 0,
                    State::Queued => 1,
                    State::Failed | State::Interrupted => 2,
                    State::Completed => 3,
                    State::Deferred => 4,
                    State::Cancelled => 5,
                },
                job.start,
            )
        });
        assert_eq!(index.len(filter), expected.len());
        for (row, job) in expected.into_iter().enumerate() {
            assert!(std::ptr::eq(index.job(&queue, filter, row), job));
        }
    }
    assert_eq!(index.len(ActivityFilter::All), 5000);
    assert_eq!(
        ActivityIndex::new(&Queue::default()).len(ActivityFilter::All),
        0
    );
}

#[test]
fn activity_snapshot_indexes_are_published_off_thread_and_reused_without_queue_changes() {
    let root = tempfile::tempdir().unwrap();
    let now = Local::now().fixed_offset();
    let mut events = calendar::fixture(5000);
    for (index, event) in events.iter_mut().enumerate() {
        event.start = now + Duration::minutes(10 + index as i64);
        event.end = event.start + Duration::minutes(30);
    }
    let coverage = events
        .iter()
        .map(|event| event.start.with_timezone(&Local).date_naive())
        .collect();
    let calendar = Calendar::build(events, coverage, Vec::new()).unwrap();
    let mut store = Store::open(root.path()).unwrap();
    store.save_calendar(&calendar).unwrap();
    store
        .save_preferences(&Preferences {
            paused: false,
            ..Default::default()
        })
        .unwrap();
    drop(store);
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().join("no-legacy"),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let snapshot_after = |revision| {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = worker.activity.lock().unwrap().clone();
            if snapshot.revision > revision {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "Activity snapshot was not published"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    let initial = snapshot_after(0);
    assert_eq!(initial.index.len(ActivityFilter::All), 5000);
    assert_eq!(initial.index.len(ActivityFilter::Waiting), 5000);
    assert_eq!(initial.index.len(ActivityFilter::Problems), 0);
    worker
        .commands
        .send(Command::PausePreparation(true))
        .unwrap();
    let paused = snapshot_after(initial.revision);
    assert!(paused.paused);
    assert_eq!(paused.queue, initial.queue);
    assert!(Arc::ptr_eq(&paused.index, &initial.index));
    let selected = calendar.events[4999].id.clone();
    worker
        .commands
        .send(Command::Promote(selected.clone()))
        .unwrap();
    let promoted = snapshot_after(paused.revision);
    assert!(promoted.queue.jobs[&selected].foreground);
    assert!(!Arc::ptr_eq(&promoted.index, &paused.index));
    assert!(!paused.queue.jobs[&selected].foreground);
    assert_eq!(promoted.index.len(ActivityFilter::All), 5000);
    assert_eq!(promoted.index.len(ActivityFilter::Waiting), 5000);
    assert!(promoted.queue.flights.is_empty());
    worker.commands.send(Command::Exit).unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
}

#[test]
fn manual_preparation_survives_the_past_horizon_and_automatic_pause() {
    let mut calendar = fixture(4);
    calendar.events[0].start = clock() - Duration::days(1);
    calendar.events[0].end = calendar.events[0].start + Duration::minutes(30);
    calendar
        .coverage
        .insert(calendar.events[0].start.with_timezone(&Local).date_naive());
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    queue.request_now(&calendar.events[0], true).unwrap();
    queue.reconcile(&calendar, clock());
    let (id, flight) = queue.dispatch_for(false).unwrap();
    assert_eq!(flight.event_id, calendar.events[0].id);
    assert!(flight.foreground);
    queue.finish(id, Ok(()));
    assert!(queue.dispatch_for(false).is_none());
}

#[test]
fn manual_preparation_yields_automatic_work_and_ignores_its_old_result() {
    let calendar = fixture(4);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    let (background, _) = queue.dispatch().unwrap();
    queue.promote("fixture-1");
    queue.dispatch().unwrap();
    queue.request_now(&calendar.events[3], true).unwrap();
    assert_eq!(queue.yield_for_manual(), Some(background));
    let (_, flight) = queue.dispatch_for(false).unwrap();
    assert_eq!(flight.event_id, "fixture-3");
    queue.finish(background, Err("Cancelled after yielding".into()));
    assert!(queue.failure.is_none());
    assert_eq!(queue.jobs["fixture-0"].state, State::Queued);
}

#[test]
fn exhausted_manual_admission_does_not_mutate_the_ledger() {
    let calendar = fixture(2);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    queue.sequence = (1 << 63) - 1;
    let before = queue.clone();
    assert!(queue.request_now(&calendar.events[0], true).is_err());
    assert_eq!(before, queue);
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
    calendar.events[3].start = clock() + Duration::days(200);
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
fn configuration_revisions_invalidate_completion_and_flights_without_replaying_unknown_outcomes() {
    let calendar = fixture(3);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    let (completed, _) = queue.dispatch().unwrap();
    queue.finish(completed, Ok(()));
    let (failed, _) = queue.dispatch().unwrap();
    queue.finish(failed, Err("Unknown fixture outcome".into()));
    let (running, _) = queue.dispatch().unwrap();
    let mut interrupted = queue.clone();
    interrupted.recover().unwrap();
    interrupted.set_analysis_revision(&"b".repeat(64)).unwrap();
    interrupted.reconcile(&calendar, clock());
    assert_eq!(interrupted.jobs["fixture-2"].state, State::Interrupted);
    assert!(interrupted.failure.is_some());
    let previous = queue.jobs["fixture-0"].fingerprint.clone();
    assert!(queue.set_analysis_revision(&"a".repeat(64)).unwrap());
    assert_eq!(queue.reconcile(&calendar, clock()), vec![running]);
    assert_ne!(previous, queue.jobs["fixture-0"].fingerprint);
    assert_eq!(queue.jobs["fixture-0"].state, State::Queued);
    assert_eq!(queue.jobs["fixture-1"].state, State::Failed);
    assert!(queue.failure.is_some());
    assert!(queue.current(running).is_none());
    queue.finish(running, Ok(()));
    assert_ne!(queue.jobs["fixture-2"].state, State::Completed);
    assert!(!queue.set_analysis_revision(&"a".repeat(64)).unwrap());
    queue.reconcile(&calendar, clock());
    assert_eq!(queue.jobs["fixture-1"].state, State::Failed);
    let mut legacy = serde_json::to_value(Queue::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("analysis_revision");
    let mut legacy: Queue = serde_json::from_value(legacy).unwrap();
    legacy.recover().unwrap();
    legacy.reconcile(&calendar, clock());
    assert_eq!(legacy.jobs["fixture-0"].fingerprint, previous);
    assert!(legacy.set_analysis_revision("bad").is_err());
    legacy.analysis_revision = "bad".into();
    assert!(legacy.recover().is_err());
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    store.save_queue(&queue).unwrap();
    let recovered = store.queue().unwrap();
    assert_eq!(recovered.analysis_revision, "a".repeat(64));
    assert_eq!(recovered.jobs["fixture-1"].state, State::Failed);
    assert_eq!(
        recovered.jobs["fixture-0"].fingerprint,
        queue.jobs["fixture-0"].fingerprint
    );
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
fn cancellation_keeps_manual_review_local_and_preserves_unrelated_failures() {
    let calendar = fixture(3);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    queue.request_now(&calendar.events[0], true).unwrap();
    let (manual, _) = queue.dispatch().unwrap();
    queue.cancelled(manual, "Requested shutdown before submission".into());
    assert_eq!(queue.jobs["fixture-0"].state, State::Interrupted);
    assert!(queue.jobs["fixture-0"].manual);
    assert!(queue.failure.is_none());
    let (automatic, flight) = queue.dispatch().unwrap();
    queue.failure = Some("Storage fixture error".into());
    queue.cancelled(automatic, "Automatic work paused".into());
    assert_eq!(queue.jobs[&flight.event_id].state, State::Deferred);
    assert_eq!(queue.jobs[&flight.event_id].retries, 0);
    assert_eq!(queue.failure.as_deref(), Some("Storage fixture error"));
    assert!(
        queue.jobs[&flight.event_id]
            .message
            .contains("Previously saved")
    );
    queue.recover().unwrap();
    assert_eq!(queue.jobs["fixture-0"].state, State::Interrupted);
    assert_eq!(queue.failure.as_deref(), Some("Storage fixture error"));
}

#[test]
fn legacy_presend_shutdown_failure_repair_does_not_retry_manual_or_clear_real_errors() {
    let calendar = fixture(2);
    let mut queue = Queue::default();
    queue.reconcile(&calendar, clock());
    queue.request_now(&calendar.events[0], true).unwrap();
    let (id, _) = queue.dispatch().unwrap();
    queue.finish(id, Err("Historical failure".into()));
    queue.failure = Some("Preparation stopped after a failed operation: AI is stopping; the previous result was discarded; Cancelled before the question was sent. Inspect saved evidence and explicitly retry.".into());
    queue.recover().unwrap();
    assert!(queue.failure.is_none());
    assert_eq!(queue.jobs["fixture-0"].state, State::Failed);
    assert!(queue.jobs["fixture-0"].manual);
    assert_eq!(queue.dispatch().unwrap().1.event_id, "fixture-1");
    queue.failure =
        Some("Preparation stopped after a failed operation: provider rejected request".into());
    queue.flights.clear();
    queue.jobs.get_mut("fixture-1").unwrap().state = State::Failed;
    queue.recover().unwrap();
    assert!(
        queue
            .failure
            .as_ref()
            .unwrap()
            .contains("provider rejected")
    );
}

#[test]
fn worker_persists_expected_manual_cancellation_without_global_failure_or_result_loss() {
    let root = tempfile::tempdir().unwrap();
    let mut calendar = fixture(2);
    let now = Local::now().fixed_offset();
    for event in &mut calendar.events {
        event.start = now + Duration::minutes(10);
        event.end = now + Duration::minutes(40);
    }
    calendar.coverage = BTreeSet::from([calendar.events[0].calendar_dates().0]);
    {
        let mut store = Store::open(root.path()).unwrap();
        store.save_calendar(&calendar).unwrap();
        store
            .save_preferences(&Preferences {
                paused: true,
                prepare_enabled: true,
                ..Default::default()
            })
            .unwrap();
        store
            .save_prepared(
                &Queue::default(),
                "fixture-0",
                &serde_json::json!({"summary":"Previous saved result"}),
            )
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
    let (commands, mut input) = tokio::sync::mpsc::channel(64);
    worker
        .commands
        .send(Command::AttachAi {
            commands,
            ready: Arc::new(AtomicBool::new(true)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: Arc::new(AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::from("a".repeat(64))),
            workiq_ready: Arc::new(AtomicBool::new(true)),
        })
        .unwrap();
    worker
        .commands
        .send(Command::PrepareNow("fixture-0".into()))
        .unwrap();
    let request = loop {
        match input.blocking_recv() {
            Some(ai::Command::Prepare(request)) => break request,
            Some(
                ai::Command::AttachWake(_)
                | ai::Command::DailyAiLimit(_)
                | ai::Command::ContextSources(_)
                | ai::Command::SyncWorkiq { .. },
            ) => {}
            _ => panic!("Manual request not admitted"),
        }
    };
    request
        .reply
        .blocking_send(ai::Prepared {
            id: request.id,
            result: ai::PreparationResult::Cancelled(
                "Requested shutdown; cancelled before sending".into(),
            ),
        })
        .unwrap();
    request.wake.send(Command::Wake).unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let activity = worker.activity.lock().unwrap().clone();
        if activity.queue.jobs["fixture-0"].state == State::Interrupted {
            assert!(activity.error.is_none());
            assert!(activity.queue.failure.is_none());
            break;
        }
        assert!(Instant::now() < deadline, "Cancellation was not persisted");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    worker.commands.send(Command::Exit).unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap()
        .unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(store.queue().unwrap().failure.is_none());
    assert_eq!(
        store.queue().unwrap().jobs["fixture-0"].state,
        State::Interrupted
    );
    assert!(
        store
            .briefing("fixture-0")
            .unwrap()
            .unwrap()
            .contains("Previous saved result")
    );
}

#[test]
fn storage_waits_for_configuration_and_finishes_without_completion_notice_drain() {
    let root = tempfile::tempdir().unwrap();
    let now = Local::now().fixed_offset();
    let mut calendar = fixture(2);
    for event in &mut calendar.events {
        event.start = now + Duration::minutes(10);
        event.end = now + Duration::minutes(40);
    }

    calendar.coverage =
        BTreeSet::from([calendar.events[0].start.with_timezone(&Local).date_naive()]);
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
    let analysis_revision = Arc::new(std::sync::OnceLock::new());
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(true)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: terminated.clone(),
            analysis_revision: analysis_revision.clone(),
            workiq_ready: Arc::new(AtomicBool::new(true)),
        })
        .unwrap();
    assert!(matches!(
        commands.blocking_recv(),
        Some(ai::Command::DailyAiLimit(200))
    ));
    assert!(matches!(
        commands.blocking_recv(),
        Some(ai::Command::AttachWake(_))
    ));
    assert!(matches!(
        commands.blocking_recv(),
        Some(ai::Command::ContextSources(_))
    ));
    worker
        .commands
        .send(Command::Detail {
            revision: 77,
            id: "configuration-barrier".into(),
        })
        .unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if matches!(
            worker.notices.try_recv(),
            Ok(tomlook::worker::Notice::Detail { revision: 77, .. })
        ) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Storage did not process the configuration barrier"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    while let Ok(command) = commands.try_recv() {
        assert!(
            matches!(command, ai::Command::SyncWorkiq { .. }),
            "Ready alone must not admit preparation without an active configuration identity"
        );
    }
    analysis_revision.set("a".repeat(64)).unwrap();
    worker.commands.send(Command::Wake).unwrap();
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
    assert_eq!(store.queue().unwrap().analysis_revision.len(), 64);
    assert_ne!(store.queue().unwrap().analysis_revision, "a".repeat(64));
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
fn worker_completes_offsite_travel_drafts_without_dispatching_a_calendar_write() {
    for skip_arrival in [false, true] {
        use chrono::TimeZone;
        use tomlook::recommendations::{Kind, State as RecommendationState};
        let root = tempfile::tempdir().unwrap();
        let now = Local::now().fixed_offset();
        let monday = calendar::monday(now.date_naive()) + Duration::days(7);
        let mut event = fixture(1).events.remove(0);
        event.title = "Client architecture workshop".into();
        event.location = "Client office".into();
        event.start = Local
            .from_local_datetime(&monday.and_hms_opt(14, 0, 0).unwrap())
            .single()
            .unwrap()
            .fixed_offset();
        event.end = event.start + Duration::hours(1);
        event.category.clear();
        event
            .extra
            .insert("categories".into(), serde_json::json!([]));
        let mut categories = Vec::new();
        for (index, category) in ["office", "mimo office"].into_iter().enumerate() {
            let mut reference = event.clone();
            reference.id = format!("category-reference-{index}");
            reference.status = "cancelled".into();
            reference.category = category.into();
            reference
                .extra
                .insert("categories".into(), serde_json::json!([category]));
            categories.push(reference);
        }
        categories.push(event.clone());
        let calendar = Calendar::build(categories, [monday].into(), vec![]).unwrap();
        {
            let mut store = Store::open(root.path()).unwrap();
            store
                .save_workiq_calendar(&calendar, "fixture-owner")
                .unwrap();
            store
                .save_preferences(&Preferences {
                    prepare_enabled: true,
                    ..Default::default()
                })
                .unwrap();
            if skip_arrival {
                let mut older_event = event.clone();
                older_event.title = "Previous title of this same meeting".into();
                let mut skipped = tomlook::recommendations::Recommendation::from_insights(
                    &older_event,
                    "fixture-owner",
                    tomlook::recommendations::Suggestion {
                        kind: Kind::Travel,
                        reason: "Earlier arrival estimate.".into(),
                        reason_cs: "Drivejsi odhad prijezdu.".into(),
                        category: None,
                        minutes: Some(60),
                    },
                    now - Duration::days(40),
                )
                .unwrap();
                skipped.state = RecommendationState::Dismissed;
                store
                    .save_recommendations(&BTreeMap::from([(skipped.id.clone(), skipped)]))
                    .unwrap();
                store.save_recommendations(&BTreeMap::new()).unwrap();
            }
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
                analysis_revision: Arc::new(std::sync::OnceLock::from("a".repeat(64))),
                workiq_ready: Arc::new(AtomicBool::new(true)),
            })
            .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let request = loop {
            match commands.try_recv() {
                Ok(ai::Command::Prepare(request)) => break request,
                Ok(ai::Command::CalendarChange { .. }) => {
                    panic!("Research dispatched a calendar write")
                }
                Ok(_) | Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                Err(error) => panic!("Executor disconnected: {error}"),
            }
            assert!(Instant::now() < deadline, "No preparation dispatch");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(
            request.event.extra["tomlook_category_meanings"]["office"]
                .as_str()
                .unwrap()
                .contains("employer's premises")
        );
        assert_eq!(request.event.id, event.id);
        request.reply.blocking_send(ai::Prepared {
        id: request.id,
        result: ai::PreparationResult::Finished(Ok(Arc::new(serde_json::json!({
            "summary":"Client visit.", "sources":[], "gaps":[],
            "recommendations":[{"kind":"category","reason":"The invitation names client premises.",
                "reason_cs":"Pozvanka uvadi kancelar zakaznika.", "category":"mimo office","minutes":null}]
        }).to_string()))),
    }).unwrap();
        request.wake.send(Command::Wake).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if Store::open(root.path()).unwrap().queue().unwrap().jobs[&event.id].state
                == State::Completed
            {
                break;
            }
            assert!(Instant::now() < deadline, "Preparation was not persisted");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        worker.commands.send(Command::Exit).unwrap();
        worker
            .done
            .take()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        while let Ok(command) = commands.try_recv() {
            assert!(!matches!(command, ai::Command::CalendarChange { .. }));
        }
        let store = Store::open(root.path()).unwrap();
        let inbox = store.recommendations().unwrap();
        assert_eq!(inbox.len(), if skip_arrival { 2 } else { 3 });
        for kind in [Kind::Category, Kind::Travel, Kind::TravelAfter] {
            assert_eq!(
                inbox.values().any(|record| record.suggestion.kind == kind
                    && record.state == RecommendationState::Pending),
                !skip_arrival || kind != Kind::Travel
            );
        }
        let briefing: serde_json::Value =
            serde_json::from_str(&store.briefing(&event.id).unwrap().unwrap()).unwrap();
        assert_eq!(briefing["recommendations"].as_array().unwrap().len(), 3);
    }
}

#[test]
fn hidden_notice_backpressure_and_exit_do_not_lose_a_ready_completion() {
    let root = tempfile::tempdir().unwrap();
    let now = Local::now().fixed_offset();
    let mut calendar = fixture(1);
    calendar.events[0].start = now + Duration::minutes(10);
    calendar.events[0].end = now + Duration::minutes(40);
    calendar.coverage =
        BTreeSet::from([calendar.events[0].start.with_timezone(&Local).date_naive()]);
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
            analysis_revision: Arc::new(std::sync::OnceLock::from("a".repeat(64))),
            workiq_ready: Arc::new(AtomicBool::new(true)),
        })
        .unwrap();
    assert!(matches!(
        commands.blocking_recv(),
        Some(ai::Command::DailyAiLimit(200))
    ));
    assert!(matches!(
        commands.blocking_recv(),
        Some(ai::Command::AttachWake(_))
    ));
    assert!(matches!(
        commands.blocking_recv(),
        Some(ai::Command::ContextSources(_))
    ));
    let request = loop {
        match commands.blocking_recv() {
            Some(ai::Command::SyncWorkiq { .. }) => {}
            Some(ai::Command::Prepare(request)) => break request,
            _ => panic!("missing dispatch"),
        }
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
fn saturated_worker_waits_for_cancellation_and_persists_only_the_current_manual_result() {
    use std::sync::atomic::Ordering;
    use std::time::Duration as Wait;

    let root = tempfile::tempdir().unwrap();
    let now = Local::now().fixed_offset();
    let mut calendar = fixture(5000);
    for (index, event) in calendar.events.iter_mut().enumerate() {
        event.start = now + Duration::minutes(10 + index as i64);
        event.end = event.start + Duration::minutes(30);
    }
    calendar.coverage = calendar
        .events
        .iter()
        .map(|event| event.start.with_timezone(&Local).date_naive())
        .collect();
    Store::open(root.path())
        .unwrap()
        .save_calendar(&calendar)
        .unwrap();
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().join("no-legacy"),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, mut commands) = tokio::sync::mpsc::channel(64);
    let occupied = Arc::new(AtomicUsize::new(0));
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(true)),
            occupied: occupied.clone(),
            terminated: Arc::new(AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::from("a".repeat(64))),
            workiq_ready: Arc::new(AtomicBool::new(true)),
        })
        .unwrap();
    let next_command = |commands: &mut tokio::sync::mpsc::Receiver<ai::Command>| {
        let deadline = Instant::now() + Wait::from_secs(5);
        loop {
            match commands.try_recv() {
                Ok(
                    ai::Command::AttachWake(_)
                    | ai::Command::DailyAiLimit(_)
                    | ai::Command::ContextSources(_)
                    | ai::Command::SyncWorkiq { .. },
                ) => {}
                Ok(command) => break command,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "Missing fake-provider command");
                    std::thread::sleep(Wait::from_millis(1));
                }
                Err(error) => panic!("Fake provider disconnected: {error}"),
            }
        }
    };
    let ai::Command::Prepare(background) = next_command(&mut commands) else {
        panic!("Expected automatic research");
    };
    assert!(!background.foreground);
    occupied.store(1, Ordering::Release);
    worker
        .commands
        .send(Command::PrepareNow("fixture-1".into()))
        .unwrap();
    let ai::Command::Prepare(foreground) = next_command(&mut commands) else {
        panic!("Expected reserved foreground capacity");
    };
    assert!(foreground.foreground);
    assert_eq!(foreground.event.id, "fixture-1");
    occupied.store(2, Ordering::Release);
    let cancellation_requested = Instant::now();
    worker
        .commands
        .send(Command::PrepareNow("fixture-3".into()))
        .unwrap();
    let ai::Command::Cancel(cancelled) = next_command(&mut commands) else {
        panic!("Saturated manual demand must cancel the automatic request");
    };
    assert_eq!(cancelled, background.id | (1 << 63));
    let cancellation_ms = cancellation_requested.elapsed().as_secs_f64() * 1000.0;

    for revision in 0..80 {
        worker
            .commands
            .send(Command::Detail {
                revision,
                id: "fixture-3".into(),
            })
            .unwrap();
    }
    let deadline = Instant::now() + Wait::from_secs(5);
    loop {
        if matches!(
            worker.notices.try_recv(),
            Ok(tomlook::worker::Notice::Detail { revision: 79, .. })
        ) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Local details stalled behind the slow provider"
        );
        std::thread::sleep(Wait::from_millis(1));
    }
    while let Ok(command) = commands.try_recv() {
        assert!(
            matches!(command, ai::Command::SyncWorkiq { .. }),
            "A third provider request started before cancellation freed capacity"
        );
    }

    occupied.store(1, Ordering::Release);
    let available = Instant::now();
    background
        .reply
        .blocking_send(ai::Prepared {
            id: background.id,
            result: ai::PreparationResult::Finished(Ok(Arc::new(
                r#"{"summary":"Obsolete slow answer","sources":[],"gaps":[]}"#.into(),
            ))),
        })
        .unwrap();
    background.wake.send(Command::Wake).unwrap();
    let ai::Command::Prepare(promoted) = next_command(&mut commands) else {
        panic!("Expected promoted request after worker availability");
    };
    assert_eq!(promoted.event.id, "fixture-3");
    assert!(promoted.foreground);
    assert_ne!(promoted.id, background.id);
    let start_ms = available.elapsed().as_secs_f64() * 1000.0;
    if !cfg!(debug_assertions) {
        assert!(
            start_ms <= 100.0,
            "Promoted request took {start_ms:.2} ms after availability"
        );
    }
    eprintln!(
        "Controlled 5000-event worker: cancellation request {cancellation_ms:.2} ms; promoted dispatch after availability {start_ms:.2} ms"
    );
    occupied.store(2, Ordering::Release);
    let raw = serde_json::json!({
        "summary": "Current manual result",
        "preparation": vec!["p".repeat(2000); 8],
        "talking_points": vec!["t".repeat(2000); 8],
        "open_questions": vec!["q".repeat(2000); 8],
        "risks": ["Slow fixture provider"],
        "gaps": ["Synthetic fixture; no live evidence"],
        "sources": [{"title": "Fixture source", "url": "https://example.com/fixture"}]
    });
    let fingerprint = worker.activity.lock().unwrap().queue.jobs["fixture-3"]
        .fingerprint
        .clone();
    let mut expected = validate_prepared(&raw.to_string(), &fingerprint).unwrap();
    expected["tomlook_event_fingerprint"] = promoted.event.fingerprint().into();
    assert!(raw.to_string().len() > 48_000);
    occupied.store(1, Ordering::Release);
    promoted
        .reply
        .blocking_send(ai::Prepared {
            id: promoted.id,
            result: ai::PreparationResult::Finished(Ok(Arc::new(raw.to_string()))),
        })
        .unwrap();
    promoted.wake.send(Command::Wake).unwrap();
    let deadline = Instant::now() + Wait::from_secs(5);
    loop {
        if worker.activity.lock().unwrap().queue.jobs["fixture-3"].state == State::Completed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Large current result did not persist"
        );
        std::thread::sleep(Wait::from_millis(1));
    }
    assert_eq!(
        worker.activity.lock().unwrap().queue.jobs["fixture-1"].state,
        State::Running
    );
    worker
        .commands
        .send(Command::Detail {
            revision: 80,
            id: "fixture-3".into(),
        })
        .unwrap();
    let deadline = Instant::now() + Wait::from_secs(5);
    loop {
        if let Ok(tomlook::worker::Notice::Detail {
            revision: 80,
            payload,
        }) = worker.notices.try_recv()
        {
            assert_eq!(payload.as_ref(), Some(&expected));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Saved large detail did not return while another request was running"
        );
        std::thread::sleep(Wait::from_millis(1));
    }
    worker.stop.store(true, Ordering::Release);
    let _ = worker.commands.try_send(Command::Wake);
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(Wait::from_secs(5))
        .unwrap()
        .unwrap();
    let store = Store::open(root.path()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&store.briefing("fixture-3").unwrap().unwrap()).unwrap();
    assert_eq!(saved, expected);
    assert!(store.briefing(&background.event.id).unwrap().is_none());
    let recovered = store.queue().unwrap();
    assert_eq!(recovered.jobs["fixture-3"].state, State::Completed);
    assert_eq!(recovered.jobs["fixture-1"].state, State::Interrupted);
    assert_ne!(recovered.jobs[&background.event.id].state, State::Completed);
}

#[test]
fn invalid_saved_detail_reports_an_error_and_preserves_the_failed_ledger() {
    let root = tempfile::tempdir().unwrap();
    drop(Store::open(root.path()).unwrap());
    let connection = rusqlite::Connection::open(root.path().join("tomlook.db")).unwrap();
    connection
        .execute(
            "INSERT INTO briefings VALUES(?1,1,?2,?3)",
            rusqlite::params!["invalid-detail", "{", chrono::Utc::now().to_rfc3339()],
        )
        .unwrap();
    drop(connection);
    let mut worker = Worker::start(
        Options {
            state: root.path().into(),
            legacy_root: root.path().join("no-legacy"),
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    worker
        .commands
        .send(Command::Detail {
            revision: 1,
            id: "invalid-detail".into(),
        })
        .unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match worker.notices.try_recv() {
            Ok(tomlook::worker::Notice::Error(error)) => {
                assert!(error.contains("Saved briefing is invalid"), "{error}");
                break;
            }
            Ok(tomlook::worker::Notice::Detail { .. }) => {
                panic!("Invalid saved content must not return a successful detail")
            }
            _ => {}
        }
        assert!(Instant::now() < deadline, "Saved detail failure was hidden");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
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
        store.briefing("invalid-detail").unwrap().as_deref(),
        Some("{")
    );
    assert!(
        store
            .queue()
            .unwrap()
            .failure
            .unwrap()
            .contains("Saved briefing is invalid")
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

#[test]
fn preloaded_cached_calendar_is_handed_to_the_owning_worker() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Calendar::build(
        calendar::fixture(5000),
        calendar::fixture_coverage(),
        Vec::new(),
    )
    .unwrap();
    Store::open(root.path())
        .unwrap()
        .save_calendar(&fixture)
        .unwrap();
    let options = Options {
        state: root.path().to_owned(),
        legacy_root: root.path().join("no-legacy"),
        demo: None,
    };
    let preload = tomlook::worker::Preload::start(&options).unwrap();
    let mut worker =
        Worker::start_with(options, eframe::egui::Context::default(), Some(preload)).unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    let loaded = loop {
        if let Ok(tomlook::worker::Notice::Loaded { calendar, .. }) = worker.notices.try_recv() {
            break calendar;
        }
        assert!(
            Instant::now() < deadline,
            "preloaded calendar was not delivered"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert_eq!(loaded.events.len(), 5000);
    assert_eq!(loaded.coverage, fixture.coverage);
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
}
