use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
use tomlook::{
    ai::{Answer, Command, Engine, Notice, Preparation, PreparationResult},
    calendar::Event,
    storage::Preferences,
    worker::validate_researched,
};

fn live_insight(engine: &Engine, event: Event, account: String) -> Result<Answer, String> {
    engine
        .commands
        .blocking_send(Command::Connect)
        .map_err(|_| "AI executor closed before connecting")?;
    engine
        .commands
        .blocking_send(Command::RestoreWorkiq(account.clone()))
        .map_err(|_| "AI executor closed before restoring WorkIQ")?;
    let connection_deadline = Instant::now() + Duration::from_secs(180);
    while !engine.ready.load(Ordering::Acquire) || !engine.workiq_ready.load(Ordering::Acquire) {
        if Instant::now() >= connection_deadline {
            return Err("Saved connections did not become ready within 180 seconds".into());
        }
        match engine.notices.recv_timeout(Duration::from_secs(1)) {
            Ok(Notice::Connection {
                ready: false,
                message,
            }) if message != "Connecting isolated SDK on its own worker..." => {
                return Err(message);
            }
            Ok(Notice::HostedWorkiq {
                busy: false,
                ready: false,
                message,
            }) => return Err(message),
            Ok(Notice::Stopped) => return Err("AI executor stopped during connection".into()),
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("AI notices closed during connection".into());
            }
        }
    }
    let (reply, mut received) = tokio::sync::mpsc::channel(1);
    let (wake, _wake_receiver) = mpsc::sync_channel(1);
    engine
        .commands
        .blocking_send(Command::Prepare(Box::new(Preparation {
            id: 1,
            event,
            foreground: true,
            account: Some(account),
            connector_revision: None,
            previous: None,
            reply,
            wake,
        })))
        .map_err(|_| "AI executor closed before preparing")?;
    let deadline = Instant::now() + Duration::from_secs(620);
    loop {
        match received.try_recv() {
            Ok(prepared) if prepared.id != 1 => {
                return Err("AI returned a different request identity".into());
            }
            Ok(prepared) => {
                return match prepared.result {
                    PreparationResult::Researched(answer) => answer,
                    PreparationResult::NotAdmitted(reason) => Err(reason),
                    PreparationResult::Cancelled(reason) => Err(reason),
                    PreparationResult::QuotaDeferred(_) => {
                        Err(tomlook::telemetry::QUOTA_REACHED.into())
                    }
                    PreparationResult::Finished(_) | PreparationResult::Translated { .. } => {
                        Err("Live Insights did not return captured research evidence".into())
                    }
                };
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                return Err("Research completion channel closed".into());
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
        }
        if Instant::now() >= deadline {
            return Err("Live Insights exceeded their setup and generation deadlines".into());
        }
        match engine.notices.recv_timeout(Duration::from_millis(200)) {
            Ok(Notice::Stopped) => return Err("AI executor stopped before completion".into()),
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("AI notices closed before completion".into());
            }
        }
    }
}

#[test]
#[ignore = "Requires explicit app-owned saved logins, a cached meeting and a new private output file; performs one real read-only model request"]
fn saved_login_produces_a_persisted_insight_with_native_evidence() {
    let root = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_INSIGHTS_ROOT").expect("Set TOMLOOK_LIVE_INSIGHTS_ROOT"),
    );
    let output = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_INSIGHTS_OUTPUT").expect("Set TOMLOOK_LIVE_INSIGHTS_OUTPUT"),
    );
    let meeting =
        std::env::var("TOMLOOK_LIVE_INSIGHTS_MEETING").expect("Set TOMLOOK_LIVE_INSIGHTS_MEETING");
    assert!(root.is_absolute() && output.is_absolute());
    assert!(!output.exists(), "Never overwrite previous live evidence");
    let raw_output = output.with_extension("raw.json");
    assert!(
        !raw_output.exists(),
        "Never overwrite previous raw live evidence"
    );
    let connection = rusqlite::Connection::open_with_flags(
        root.join("tomlook.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let owner: String = connection
        .query_row(
            "SELECT payload FROM settings WHERE key='calendar_owner'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let preferences: String = connection
        .query_row(
            "SELECT payload FROM settings WHERE key='preferences'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let preferences: Preferences = serde_json::from_str(&preferences).unwrap();
    assert_eq!(owner, preferences.work_email);
    let payload: String = connection
        .query_row(
            "SELECT payload FROM calendar WHERE id=?1",
            [&meeting],
            |row| row.get(0),
        )
        .unwrap();
    let event: Event = serde_json::from_str(&payload).unwrap();
    event.validate().unwrap();
    assert_eq!(event.id, meeting);
    let fingerprint = event.fingerprint();
    drop(connection);
    let started = Instant::now();
    let mut engine = Engine::start(root, false, eframe::egui::Context::default()).unwrap();
    let result = live_insight(&engine, event, owner);
    engine.stop.send(true).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(45))
        .expect("Owned AI executor must finish shutdown");
    assert!(engine.terminated.load(Ordering::Acquire));
    assert_eq!(engine.occupied.load(Ordering::Acquire), 0);
    let raw = match &result {
        Ok(answer) => serde_json::json!({
            "diagnostic": "unvalidated_live_response",
            "answer": &*answer.text,
            "retrieved_evidence": &*answer.evidence,
            "not_a_completed_product_insight": true
        }),
        Err(error) => serde_json::json!({
            "diagnostic": "failed_live_request",
            "error": error,
            "not_a_completed_product_insight": true
        }),
    };
    persist(&raw_output, &raw);
    let answer = result.expect("A real native research response must complete");
    let insight = validate_researched(answer, &fingerprint)
        .expect("A real Insight with captured native evidence must validate");
    for field in [
        "preparation",
        "talking_points",
        "open_questions",
        "risks",
        "sources",
        "gaps",
    ] {
        assert!(insight[field].is_array(), "Missing Insights array: {field}");
    }
    assert!(
        ["preparation", "talking_points", "open_questions"]
            .iter()
            .any(|field| !insight[field].as_array().unwrap().is_empty()),
        "Insights must contain something actionable, not only a summary"
    );
    let sources = insight["sources"].as_array().unwrap();
    assert!(
        !sources.is_empty(),
        "The selected live meeting needs original source links"
    );
    let records = insight["retrieved_evidence"]["records"].as_array().unwrap();
    assert!(
        sources.iter().any(|source| {
            let url = source["url"].as_str().unwrap();
            records.iter().any(|record| {
                record["succeeded"] == true
                    && (record["links"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|link| link.as_str() == Some(url))
                        || record["excerpt"].as_str().unwrap().contains(url))
            })
        }),
        "At least one original source must match actual captured tool evidence"
    );
    let evidence = serde_json::json!({
        "diagnostic": "single_live_insight",
        "created_at": chrono::Utc::now().to_rfc3339(),
        "elapsed_ms": started.elapsed().as_millis(),
        "meeting_id": meeting,
        "local_index_included": false,
        "product_calendar_and_ledger_modified": false,
        "insight": insight,
    });
    persist(&output, &evidence);
    println!(
        "One real Insight persisted with validated native evidence; production ledger unchanged."
    );
}

fn persist(output: &std::path::Path, evidence: &serde_json::Value) {
    let bytes = serde_json::to_vec_pretty(evidence).unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(persisted, *evidence);
}
