use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use std::collections::BTreeSet;
use tomlook::{
    calendar::{self, Calendar, View, safe_url},
    storage::{Preferences, Store},
};

#[test]
fn monday_first_and_month_clamping() {
    let date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    assert_eq!(
        View::Month.shift(date, 1),
        NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
    );
    assert_eq!(
        View::Month.shift(date, -1),
        NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()
    );
    let matrix = View::Month.days(date);
    assert_eq!(matrix.len(), 42);
    assert_eq!(matrix[0].weekday().num_days_from_monday(), 0);
    assert_eq!(View::WorkWeek.days(date).len(), 5);
}

#[test]
fn source_urls_reject_credentials_controls_and_non_web_schemes() {
    for unsafe_url in [
        "javascript:alert(1)",
        "file:///C:/secret",
        "https://user:pass@example.com",
        "https://example.com\n",
    ] {
        assert!(!safe_url(unsafe_url));
    }

    assert!(safe_url("https://outlook.office.com/calendar/item/123"));
}

#[test]
fn categories_are_precomputed_deduplicated_and_sorted() {
    let mut events = calendar::demo(Local::now(), 4);
    for (event, category) in events.iter_mut().zip(["Work", "", "Customer", "Work"]) {
        event.category = category.into();
    }
    let calendar = Calendar::build(events, BTreeSet::new(), Vec::new()).unwrap();
    assert_eq!(calendar.categories, ["Customer", "Work"]);
}

#[test]
fn aware_time_and_midnight_are_not_rounded_into_the_next_day() {
    let mut events = calendar::demo(Local::now(), 1);
    events[0].start = DateTime::parse_from_rfc3339("2026-10-07T00:00:00+02:00").unwrap();
    events[0].end = DateTime::parse_from_rfc3339("2026-10-08T00:00:00+02:00").unwrap();
    events[0].is_all_day = true;
    let first = events[0].start.with_timezone(&Local).date_naive();
    let last = (events[0].end - Duration::milliseconds(1))
        .with_timezone(&Local)
        .date_naive();
    let calendar = Calendar::build(events, BTreeSet::new(), Vec::new()).unwrap();
    assert!(calendar.days.contains_key(&first));
    assert!(calendar.days.contains_key(&last));
    assert!(!calendar.days.contains_key(&(last + Duration::days(1))));
}

#[test]
fn dst_transition_keeps_instant_duration_and_calendar_dates() {
    let zone = chrono_tz::Europe::Prague;
    let start = zone.with_ymd_and_hms(2026, 3, 29, 1, 30, 0).unwrap();
    let end = zone.with_ymd_and_hms(2026, 3, 29, 3, 30, 0).unwrap();
    assert_eq!(end - start, Duration::hours(1));
    let mut event = calendar::demo(Local::now(), 1).remove(0);
    event.start = start.fixed_offset();
    event.end = end.fixed_offset();
    assert!(event.validate().is_ok());
    assert!(!event.all_day_like());
}

#[test]
fn overlap_lanes_are_grouped_and_reused() {
    let mut events = calendar::demo(Local::now(), 4);
    let start = events[0].start;
    for (i, event) in events.iter_mut().enumerate() {
        event.start = start + Duration::minutes(if i == 3 { 120 } else { i as i64 * 15 });
        event.end = event.start + Duration::minutes(60);
    }
    let day = start.with_timezone(&Local).date_naive();
    let calendar = Calendar::build(events, BTreeSet::new(), Vec::new()).unwrap();
    let slots = &calendar.days[&day].timed;
    assert_eq!(
        slots.iter().map(|s| s.lanes).collect::<Vec<_>>(),
        vec![3, 3, 3, 1]
    );
    assert_eq!(
        slots.iter().map(|s| s.lane).collect::<Vec<_>>(),
        vec![0, 1, 2, 0]
    );
}

#[test]
fn cache_and_settings_survive_restart_without_claiming_coverage() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(directory.path()).unwrap();
    let calendar = Calendar::build(
        calendar::demo(Local::now(), 200),
        BTreeSet::new(),
        Vec::new(),
    )
    .unwrap();
    store.save_calendar(&calendar).unwrap();
    store
        .save_preferences(&Preferences {
            dark: false,
            accent: 3,
            view: View::Month,
            paused: true,
        })
        .unwrap();
    drop(store);
    let store = Store::open(directory.path()).unwrap();
    let loaded = store.calendar().unwrap();
    assert_eq!(loaded.events.len(), 200);
    assert!(loaded.coverage.is_empty());
    assert_eq!(store.preferences().unwrap().view, View::Month);
}

#[test]
fn legacy_import_preserves_original_bytes_and_reports_invalid_rows() {
    let legacy = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::create_dir(legacy.path().join("data")).unwrap();
    let mut rows = serde_json::to_value(calendar::demo(Local::now(), 3))
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    rows.push(serde_json::json!({"id":"invalid","title":"Unzoned","start":"2026-10-07T10:00:00","end":"2026-10-07T11:00:00"}));
    let bytes = serde_json::to_vec(
        &serde_json::json!({"events": rows, "coverage": {"2026-10-07":"2026-10-07T10:00:00Z"}}),
    )
    .unwrap();
    let source = legacy.path().join("data").join("calendar-cache.json");
    std::fs::write(&source, &bytes).unwrap();
    let mut store = Store::open(state.path()).unwrap();
    let imported = store.import_legacy(legacy.path()).unwrap().unwrap();
    assert_eq!(imported.events.len(), 3);
    assert!(imported.coverage.is_empty());
    assert!(
        imported
            .warnings
            .iter()
            .any(|warning| warning.contains("1 invalid"))
    );
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    assert!(store.import_legacy(legacy.path()).unwrap().is_none());
}

#[test]
fn stress_corpus_is_valid_and_fingerprints_ignore_ai_state() {
    let mut events = calendar::demo(Local::now(), 5000);
    let fingerprint = events[0].fingerprint();
    events[0].briefing_status = "ready".into();
    assert_eq!(fingerprint, events[0].fingerprint());
    events[0].title.push_str(" changed");
    assert_ne!(fingerprint, events[0].fingerprint());
    let calendar = Calendar::build(events, BTreeSet::new(), Vec::new()).unwrap();
    assert_eq!(calendar.events.len(), 5000);
    assert_eq!(calendar.days.len(), 42);
}

#[test]
fn failed_legacy_history_import_rolls_back_calendar_and_can_be_retried() {
    let legacy = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let data = legacy.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(
        data.join("calendar-cache.json"),
        serde_json::to_vec(&calendar::demo(Local::now(), 2)).unwrap(),
    )
    .unwrap();
    std::fs::write(data.join("outlook-next.db"), b"invalid SQLite").unwrap();
    let mut store = Store::open(state.path()).unwrap();
    assert!(store.import_legacy(legacy.path()).is_err());
    assert!(store.calendar().unwrap().events.is_empty());
    std::fs::remove_file(data.join("outlook-next.db")).unwrap();
    assert_eq!(
        store
            .import_legacy(legacy.path())
            .unwrap()
            .unwrap()
            .events
            .len(),
        2
    );
}

#[test]
fn duplicate_identity_is_rejected_not_silently_overwritten() {
    let mut events = calendar::demo(Local::now(), 2);
    events[1].id = events[0].id.clone();
    assert!(Calendar::build(events, BTreeSet::new(), Vec::new()).is_err());
}
