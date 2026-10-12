use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
use tomlook::workiq::connection::{Connection, Range};

#[tokio::test]
#[ignore = "Requires authorized saved WorkIQ login and a private new output; reads a bounded calendar range without changing product cache"]
async fn saved_calendar_range_preserves_large_meetings() {
    let root = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_INSIGHTS_ROOT").unwrap());
    let output = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_CALENDAR_OUTPUT").unwrap());
    let account = std::env::var("TOMLOOK_LIVE_WORK_ACCOUNT").unwrap();
    let first = std::env::var("TOMLOOK_LIVE_CALENDAR_FIRST")
        .unwrap()
        .parse()
        .unwrap();
    let last = std::env::var("TOMLOOK_LIVE_CALENDAR_LAST")
        .unwrap()
        .parse()
        .unwrap();
    let range = Range::new(first, last).unwrap();
    assert!(root.is_absolute() && output.is_absolute() && !output.exists());
    let started = Instant::now();
    let connection = Connection::connect_saved(&root, &account, false, |_| {})
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(300), connection.calendar(range))
        .await
        .map_err(|_| "Live bounded calendar read exceeded 300 seconds".to_owned())
        .and_then(|result| result);
    let cleanup = connection.close().await;
    let large = result
        .as_ref()
        .map(|calendar| {
            calendar
                .events
                .iter()
                .filter(|event| event.attendee_count() > 500)
                .map(|event| {
                    assert_eq!(event.attendees.len(), 500);
                    serde_json::json!({
                        "event_id": event.id,
                        "start": event.start,
                        "total_participants": event.attendee_count(),
                        "retained_participants": event.attendees.len(),
                        "full_list_fingerprint": event.extra["tomlook_attendees_fingerprint"],
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let artifact = serde_json::json!({
        "observed_at": chrono::Utc::now().to_rfc3339(),
        "elapsed_ms": started.elapsed().as_millis(),
        "first": first,
        "last": last,
        "result": match &result {
            Ok(calendar) => serde_json::json!({
                "events":calendar.events.len(),
                "covered_days":calendar.coverage.len(),
                "requested_range_complete":(0..=(last-first).num_days())
                    .all(|offset| calendar.coverage.contains(&(first + chrono::Duration::days(offset)))),
            }),
            Err(error) => serde_json::json!({"error":error}),
        },
        "large_meetings": large,
        "owned_connection_cleanup": if cleanup.is_ok() {"passed"} else {"failed"},
        "product_calendar_cache_and_ledger_modified": false,
        "not_a_product_completion": true,
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&artifact).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    let reopened: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(artifact, reopened);
    cleanup.unwrap();
    let calendar = result.unwrap();
    assert_eq!(
        calendar.coverage.len() as i64,
        (last - first).num_days() + 1
    );
    assert!(
        !large.is_empty(),
        "Acceptance requires a real large-participant meeting"
    );
}
