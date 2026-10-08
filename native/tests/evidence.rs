use std::sync::Arc;
use tomlook::evidence::{EXCERPT_LIMIT, RECORD_LIMIT, Recorder, Redactor, safe_source_url};

#[test]
fn observed_results_have_host_time_and_original_safe_structured_links_only() {
    let recorder = Recorder::new(Arc::new(Redactor::new(["fixture-credential".into()])));
    recorder.record(
        "WorkIQ-Mail-GetMessage",
        &serde_json::json!({
            "title": "Synthetic source", "url": "https://example.com/source",
            "body": "Fixture passage with fixture-credential",
            "items": [
                {"webLink": "https://example.com/other"},
                {"url": "https://example.com/source"},
                {"url": "javascript:alert(1)"},
                {"url": "https://user:password@example.com/source"},
                {"url": "https://example.com?access_token=not-a-real-token"},
                {"url": "https://example.com/fixture-credential"}
            ]
        }),
        true,
    );
    let snapshot = recorder.snapshot().unwrap();
    assert_eq!(snapshot.records.len(), 1);
    let record = &snapshot.records[0];
    assert_eq!(record.ordinal, 1);
    assert!(chrono::DateTime::parse_from_rfc3339(&record.observed_at).is_ok());
    assert_eq!(
        record.links,
        ["https://example.com/other", "https://example.com/source"]
    );
    assert!(!record.excerpt.contains("fixture-credential"));
    assert!(record.excerpt.contains("[REDACTED]"));
    assert!(record.succeeded);
    assert!(!record.truncated);
    assert_eq!(snapshot.omitted, 0);
    assert!(!safe_source_url("https://example.com?API%5FKEY=fixture"));
}

#[test]
fn nested_mcp_json_is_inspected_but_failures_are_not_supporting_links() {
    let recorder = Recorder::new(Arc::new(Redactor::default()));
    recorder.record("WebIQ-MCP-web", &serde_json::json!({
        "content": [{"type": "text", "text": "{\"url\":\"https://example.com/article\",\"text\":\"Fixture article\"}"}]
    }), true);
    recorder.record(
        "WorkIQ-Mail-GetMessage",
        &serde_json::json!({"url":"https://example.com/failure","error":"Unavailable fixture"}),
        false,
    );
    let snapshot = recorder.snapshot().unwrap();
    assert_eq!(snapshot.records[0].links, ["https://example.com/article"]);
    assert!(!snapshot.records[1].succeeded);
    assert!(snapshot.records[1].links.is_empty());
}

#[test]
fn capture_limits_are_exact_utf8_safe_and_omissions_are_visible() {
    let recorder = Recorder::new(Arc::new(Redactor::default()));
    for _ in 0..RECORD_LIMIT + 3 {
        recorder.record(
            "WorkIQ-Mail-GetMessage",
            &serde_json::json!({"body":"\u{010d}".repeat(EXCERPT_LIMIT)}),
            true,
        );
    }
    let snapshot = recorder.snapshot().unwrap();
    assert_eq!(snapshot.records.len(), RECORD_LIMIT);
    assert_eq!(snapshot.omitted, 3);
    assert!(
        snapshot
            .records
            .iter()
            .all(|record| record.truncated && record.excerpt.len() <= EXCERPT_LIMIT)
    );
    snapshot.validate().unwrap();
    let expanded = Recorder::new(Arc::new(Redactor::new(["x".into()])));
    expanded.record(
        "WorkIQ-Mail-GetMessage",
        &serde_json::json!("x".repeat(EXCERPT_LIMIT - 2)),
        true,
    );
    let snapshot = expanded.snapshot().unwrap();
    assert!(snapshot.records[0].truncated);
    assert!(snapshot.records[0].excerpt.len() <= EXCERPT_LIMIT);
}

#[test]
fn streamed_and_escaped_credentials_are_redacted_across_chunk_boundaries() {
    let redactor = Redactor::new(["fixture-secret".into(), "fixture-quote\"\\value".into()]);
    assert_eq!(redactor.partial("Before fixture-sec"), "Before [REDACTED]");
    assert_eq!(
        redactor.text("Before fixture-secret after"),
        "Before [REDACTED] after"
    );
    let encoded = serde_json::to_string("fixture-quote\"\\value").unwrap();
    assert!(!redactor.text(&encoded).contains("fixture-quote"));
    assert_eq!(redactor.partial("No matching tail"), "No matching tail");
    let overlapping = Redactor::new(["ab".into(), "abc".into(), "bc".into()]);
    assert_eq!(
        overlapping.text("abc ab bc"),
        "[REDACTED] [REDACTED] [REDACTED]"
    );
    let compound = Redactor::new(["a".into(), "E".into(), "D".into()]);
    assert_eq!(compound.text("aED"), "[REDACTED][REDACTED][REDACTED]");
    let unicode = Redactor::new(["\u{010d}secret".into()]);
    assert_eq!(unicode.partial("Before \u{010d}sec"), "Before [REDACTED]");
    let recorder = Recorder::new(Arc::new(redactor));
    recorder.fail();
    assert!(recorder.snapshot().is_err());
}
