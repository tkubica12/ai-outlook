use chrono::{Duration, Local};
use std::collections::BTreeSet;
use tomlook::{
    calendar::{self, Calendar},
    storage::Store,
};

fn snapshot(day_offset: i64, title: &str) -> Calendar {
    let mut event = calendar::demo(Local::now(), 1).remove(0);
    event.id = format!("fixture-{day_offset}");
    event.title = title.into();
    event.start += Duration::days(day_offset);
    event.end += Duration::days(day_offset);
    let day = event.start.with_timezone(&Local).date_naive();
    Calendar::build(vec![event], BTreeSet::from([day]), Vec::new()).unwrap()
}

#[test]
fn snapshots_merge_replace_and_survive_restart_without_cross_account_mixing() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path()).unwrap();
    store
        .save_workiq_calendar(&snapshot(0, "Old"), "fixture@microsoft.com")
        .unwrap();
    store
        .save_workiq_calendar(&snapshot(7, "Other week"), "fixture@microsoft.com")
        .unwrap();
    let saved = store
        .save_workiq_calendar(&snapshot(0, "Updated"), "fixture@microsoft.com")
        .unwrap();
    assert_eq!(saved.events.len(), 2);
    assert!(saved.events.iter().any(|event| event.title == "Updated"));
    drop(store);
    let mut store = Store::open(root.path()).unwrap();
    assert_eq!(store.calendar().unwrap().events.len(), 2);
    assert_eq!(store.calendar_sync_days().unwrap().len(), 2);
    assert_eq!(
        store.calendar_owner().unwrap().as_deref(),
        Some("fixture@microsoft.com")
    );
    let empty = Calendar::build(Vec::new(), snapshot(0, "Unused").coverage, Vec::new()).unwrap();
    let saved = store
        .save_workiq_calendar(&empty, "fixture@microsoft.com")
        .unwrap();
    assert_eq!(saved.events.len(), 1);
    let changed_account = store
        .save_workiq_calendar(&snapshot(0, "New account"), "different@microsoft.com")
        .unwrap();
    assert_eq!(changed_account.events.len(), 1);
    assert_eq!(store.calendar_sync_days().unwrap().len(), 1);
    assert_eq!(changed_account.coverage.len(), 1);
    assert_eq!(changed_account.events[0].title, "New account");
    assert_eq!(
        store.calendar_owner().unwrap().as_deref(),
        Some("different@microsoft.com")
    );
    store
        .save_calendar(&snapshot(0, "Unowned local data"))
        .unwrap();
    assert!(store.calendar_owner().unwrap().is_none());
    assert!(store.calendar_sync_days().unwrap().is_empty());
}

#[test]
fn missing_or_partial_coverage_never_overwrites_a_valid_cache() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path()).unwrap();
    store
        .save_workiq_calendar(&snapshot(0, "Keep"), "fixture@microsoft.com")
        .unwrap();
    assert!(
        store
            .save_workiq_calendar(&Calendar::default(), "fixture@microsoft.com")
            .is_err()
    );
    let mut invalid = snapshot(0, "Bad");
    invalid
        .coverage
        .insert(*invalid.coverage.first().unwrap() + Duration::days(2));
    assert!(
        store
            .save_workiq_calendar(&invalid, "fixture@microsoft.com")
            .is_err()
    );
    assert_eq!(store.calendar().unwrap().events[0].title, "Keep");
}

#[test]
fn an_owned_empty_calendar_is_not_repopulated_from_legacy_data() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let legacy = root.path().join("legacy");
    std::fs::create_dir_all(legacy.join("data")).unwrap();
    std::fs::write(
        legacy.join("data").join("calendar-cache.json"),
        serde_json::to_vec(&snapshot(0, "Legacy").events).unwrap(),
    )
    .unwrap();
    let empty = Calendar::build(Vec::new(), snapshot(0, "Unused").coverage, Vec::new()).unwrap();
    let mut store = Store::open(&state).unwrap();
    store
        .save_workiq_calendar(&empty, "fixture@microsoft.com")
        .unwrap();
    drop(store);
    let mut worker = tomlook::worker::Worker::start(
        tomlook::worker::Options {
            state,
            legacy_root: legacy,
            demo: None,
        },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match worker.notices.try_recv() {
            Ok(tomlook::worker::Notice::Loaded {
                calendar,
                calendar_owner,
                ..
            }) => {
                assert!(calendar.events.is_empty());
                assert_eq!(calendar.coverage, empty.coverage);
                assert_eq!(calendar_owner.as_deref(), Some("fixture@microsoft.com"));
                break;
            }
            Ok(tomlook::worker::Notice::Error(error)) => panic!("{error}"),
            _ => {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
    worker
        .stop
        .store(true, std::sync::atomic::Ordering::Release);
    worker
        .commands
        .send(tomlook::worker::Command::Exit)
        .unwrap();
    worker
        .done
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap()
        .unwrap();
}
