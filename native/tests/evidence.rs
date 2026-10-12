use std::sync::Arc;
use tomlook::evidence::{EXCERPT_LIMIT, RECORD_LIMIT, Recorder, Redactor, safe_source_url};

#[test]
fn observed_results_have_host_time_and_original_safe_links_only() {
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
fn original_read_provenance_is_bounded_host_owned_and_independent_of_excerpt() {
    use tomlook::evidence::source_label;
    use tomlook::workiq::connection::{CONTEXT_TOOL, SDK_TOOL};
    let recorder = Recorder::new(Arc::new(Redactor::default()));
    recorder.record_workiq(
        "/me/messages/actual?$select=body,webLink",
        &serde_json::json!({
            "body": "x".repeat(20_000),
            "resource": "/me/messages/forged",
            "webLink": "https://outlook.office.com/mail/actual"
        }),
    );
    recorder.record(
        CONTEXT_TOOL,
        &serde_json::json!({
            "answer": "[Message](https://outlook.office.com/mail/actual)"
        }),
        true,
    );
    let snapshot = recorder.snapshot().unwrap();
    let original = &snapshot.records[0];
    assert!(original.truncated);
    assert!(original.excerpt.len() <= EXCERPT_LIMIT);
    assert!(!original.excerpt.contains("actual"));
    assert_eq!(
        original.resource.as_deref(),
        Some("/me/messages/actual?$select=body,webLink")
    );
    assert_eq!(
        source_label(&original.tool, original.resource.as_deref()),
        "Message read"
    );
    assert_eq!(
        source_label(&snapshot.records[1].tool, None),
        "Workplace search summary"
    );
    assert_eq!(
        source_label(SDK_TOOL, Some("/me/events/fixture")),
        "Meeting read"
    );
    assert_eq!(
        source_label(SDK_TOOL, Some("/me/messages?$search=fixture")),
        "Mail search results"
    );
    assert_eq!(
        source_label(SDK_TOOL, Some("/me/calendarView?startDateTime=fixture")),
        "Calendar results"
    );
    assert_eq!(source_label(SDK_TOOL, None), "Mail and meetings");
    assert!(snapshot.records[1].resource.is_none());
    let saved = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        saved["records"][0]["resource"],
        original.resource.as_ref().unwrap().as_str()
    );
    assert!(saved["records"][1].get("resource").is_none());
    let mut invalid = snapshot.as_ref().clone();
    invalid.records[0].resource = Some("/users/other/messages".into());
    assert!(invalid.validate().is_err());
    invalid.records[0].resource = Some(format!("/me/messages/{}", "x".repeat(4000)));
    assert!(invalid.validate().is_err());
    invalid.records[0].resource = original.resource.clone();
    invalid.records[0].tool = CONTEXT_TOOL.into();
    assert!(invalid.validate().is_err());
}

#[test]
fn invalid_or_secret_read_provenance_cannot_be_published() {
    for resource in [
        "/me/messages/fixture-credential",
        "/me/messages?$search=fixture%2Dcredential",
        "/me/messages/fixture/send",
    ] {
        let recorder = Recorder::new(Arc::new(Redactor::new(["fixture-credential".into()])));
        recorder.record_workiq(resource, &serde_json::json!({"body": "fixture"}));
        assert!(recorder.snapshot().is_err());
    }
}

#[test]
fn hosted_context_markdown_links_are_captured_even_after_the_excerpt_limit() {
    let recorder = Recorder::new(Arc::new(Redactor::new(["fixture-credential".into()])));
    recorder.record(
        "tomlook_workiq_context",
        &serde_json::json!({"answer":format!(
            "{} [1](https://example.com/source_(original)) [2](https://example.com/source_(original)) \
             [unsafe](https://example.com?access_token=fixture) [secret](https://example.com/fixture-credential) \
             [javascript](javascript:alert(1)) [3](https://example.com/other)",
            "x".repeat(20_000)
        )}),
        true,
    );
    let snapshot = recorder.snapshot().unwrap();
    assert_eq!(
        snapshot.records[0].links,
        [
            "https://example.com/other",
            "https://example.com/source_(original)"
        ]
    );
    assert!(snapshot.records[0].truncated);
    snapshot.validate().unwrap();
    let failed = Recorder::new(Arc::new(Redactor::default()));
    failed.record(
        "tomlook_workiq_context",
        &serde_json::json!({"answer":"[1](https://example.com/failure)"}),
        false,
    );
    assert!(failed.snapshot().unwrap().records[0].links.is_empty());
}

#[test]
fn local_file_identity_survives_content_excerpt_truncation() {
    let recorder = Recorder::new(Arc::new(Redactor::new(["fixture-credential".into()])));
    recorder.record(
        tomlook::context::READ,
        &serde_json::json!({
            "root": 7,
            "folder": "D:\\demo",
            "path": "slides\\factory.html",
            "offset": 8000,
            "content": format!("fixture-credential {}", "\u{010d}".repeat(EXCERPT_LIMIT)),
        }),
        true,
    );
    let snapshot = recorder.snapshot().unwrap();
    let record = &snapshot.records[0];
    let prefix = record.excerpt.split(",\"data\":").next().unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&format!("{prefix}}}")).unwrap();
    assert_eq!(metadata["_local_source"]["folder"], "D:\\demo");
    assert_eq!(metadata["_local_source"]["path"], "slides\\factory.html");
    assert_eq!(metadata["_local_source"]["root"], 7);
    assert_eq!(metadata["_local_source"]["offset"], 8000);
    assert!(record.truncated);
    assert!(record.excerpt.len() <= EXCERPT_LIMIT);
    assert!(!record.excerpt.contains("fixture-credential"));
    assert!(record.excerpt.contains("[REDACTED]"));
    snapshot.validate().unwrap();
    recorder.record(
        tomlook::context::READ,
        &serde_json::json!("A fixture file could not be read"),
        false,
    );
    let snapshot = recorder.snapshot().unwrap();
    assert!(!snapshot.records[1].excerpt.contains("_local_source"));
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
