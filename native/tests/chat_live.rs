use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
use tomlook::{
    ai::{Command, Engine, Notice, Question},
    chat::{Mode, Request},
};

fn send(engine: &Engine, id: u64, account: &str, mode: Mode, question: &str) -> Result<(), String> {
    engine
        .commands
        .blocking_send(Command::Chat(Box::new(Request {
            question: Question {
                id,
                question: question.into(),
                meeting: None,
                account: Some(account.into()),
                public_topic: None,
                suggest_drafts: false,
                language: tomlook::i18n::Language::Cs,
            },
            mode,
            history: Vec::new(),
            resume: None,
        })))
        .map_err(|_| "Live chat executor closed".into())
}

fn connect(engine: &Engine, account: &str) -> Result<(), String> {
    engine
        .commands
        .blocking_send(Command::Connect)
        .map_err(|_| "Connect command failed")?;
    engine
        .commands
        .blocking_send(Command::RestoreWorkiq(account.into()))
        .map_err(|_| "Restore command failed")?;
    let until = Instant::now() + Duration::from_secs(180);
    let mut external = std::collections::BTreeSet::from([
        tomlook::connectors::Kind::Github,
        tomlook::connectors::Kind::Webiq,
    ]);
    while !engine.ready.load(Ordering::Acquire)
        || !engine.workiq_ready.load(Ordering::Acquire)
        || !external.is_empty()
    {
        if Instant::now() >= until {
            return Err("Saved connections did not become ready".into());
        }
        match engine.notices.recv_timeout(Duration::from_secs(1)) {
            Ok(Notice::Connection {
                ready: false,
                message,
            }) if !message.contains("Connecting") => return Err(message),
            Ok(Notice::HostedWorkiq {
                busy: false,
                ready: false,
                message,
            }) => return Err(message),
            Ok(Notice::External {
                kind, busy: false, ..
            }) => {
                external.remove(&kind);
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("Connection notices closed".into()),
        }
    }
    Ok(())
}

fn exercise(engine: &Engine, account: &str) -> Result<Vec<serde_json::Value>, String> {
    connect(engine, account)?;
    let first = chrono::Utc::now().timestamp_millis() as u64;
    let started = Instant::now();
    send(
        engine,
        first,
        account,
        Mode::Deep,
        "Ověř přípravu na příštích 30 dní, ale tento ověřovací běh omez na nejvýše tři schůzky, kde budu pravděpodobně prezentovat nebo dělat demo. Nejprve použij uložený kalendář a Insights; pro jednu nejbližší relevantní schůzku zkus dohledat existující prezentaci/demo v povoleném místním indexu a kontext v poště a Teams přes Microsoft 365. Žádné veřejné hledání ani zápisy. Rozliš ověřené podklady, neověřenou připravenost a mezery v prohledání. Nejvýše 500 slov.",
    )?;
    send(
        engine,
        first + 1,
        account,
        Mode::Fast,
        "Použij jen uložený kalendář. Kdy mám nejbližší dvě schůzky? Napiš jejich názvy a místní datum a čas. Nic nestahuj z Microsoft 365 ani veřejného webu.",
    )?;
    let mut pending = BTreeMap::from([(first, Mode::Deep), (first + 1, Mode::Fast)]);
    let mut steps = BTreeMap::<u64, Vec<String>>::new();
    let mut outputs = Vec::new();
    let until = Instant::now() + Duration::from_secs(680);
    while !pending.is_empty() {
        if Instant::now() >= until {
            return Err("Concurrent live chats exceeded their deadline".into());
        }
        match engine.notices.recv_timeout(Duration::from_millis(250)) {
            Ok(Notice::ChatProgress { id, progress }) => {
                let values = steps.entry(id).or_default();
                if values.last() != Some(&progress.stage) {
                    values.push(progress.stage);
                }
            }
            Ok(Notice::Answer { id, result }) if pending.contains_key(&id) => {
                let mode = pending.remove(&id).unwrap();
                let answer = result?;
                if answer.text.trim().is_empty() || answer.evidence.records.is_empty() {
                    return Err("Live chat omitted its answer or native evidence".into());
                }
                outputs.push(serde_json::json!({"mode":mode,"model":mode.model(),"effort":mode.effort(),
                    "completion_seconds_from_pair_start":started.elapsed().as_secs_f64(),
                    "answer":&*answer.text,"evidence":&*answer.evidence,"stages":steps.remove(&id)}));
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("Live answer notices closed".into()),
        }
    }
    let auto_started = Instant::now();
    send(
        engine,
        first + 2,
        account,
        Mode::Auto,
        "Podle uloženého kalendáře mi stručně řekni datum a název nejbližší schůzky. Jiné zdroje nepotřebuji.",
    )?;
    let until = Instant::now() + Duration::from_secs(130);
    let mut routed = false;
    loop {
        if Instant::now() >= until {
            return Err("Auto live chat exceeded its deadline".into());
        }
        match engine.notices.recv_timeout(Duration::from_millis(250)) {
            Ok(Notice::ChatProgress { id, progress }) if id == first + 2 => {
                routed |= progress.mode == Mode::Fast;
            }
            Ok(Notice::Answer { id, result }) if id == first + 2 => {
                let answer = result?;
                if !routed || answer.evidence.records.is_empty() {
                    return Err("Auto did not route to a verified cached answer".into());
                }
                outputs.push(serde_json::json!({"mode":"auto","selected":"fast","elapsed_seconds":auto_started.elapsed().as_secs_f64(),"answer":&*answer.text,"evidence":&*answer.evidence}));
                break;
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("Auto notices closed".into()),
        }
    }
    Ok(outputs)
}

#[test]
#[ignore = "Requires explicitly authorized app-owned logins and a new private output; performs bounded real read-only Quick, Thorough and Auto requests"]
fn cached_quick_and_thorough_run_concurrently_with_native_evidence_and_auto_routing() {
    let root = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_CHAT_ROOT").expect("Set TOMLOOK_LIVE_CHAT_ROOT"),
    );
    let output = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_CHAT_OUTPUT").expect("Set TOMLOOK_LIVE_CHAT_OUTPUT"),
    );
    assert!(root.is_absolute() && output.is_absolute() && !output.exists());
    let store = tomlook::storage::Store::read_only(&root).unwrap();
    let sources = store.preferences().unwrap().sources();
    let owner = store
        .calendar_owner()
        .unwrap()
        .expect("Connect the calendar first");
    drop(store);
    let mut engine = Engine::start(root, false, eframe::egui::Context::default()).unwrap();
    engine
        .commands
        .blocking_send(Command::ContextSources(sources))
        .unwrap();
    let result = exercise(&engine, &owner);
    engine.stop.send(true).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(45))
        .expect("Owned engine must finish shutdown");
    assert!(engine.terminated.load(Ordering::Acquire));
    assert_eq!(engine.occupied.load(Ordering::Acquire), 0);
    let receipt = match &result {
        Ok(outputs) => {
            serde_json::json!({"status":"passed","scope":"bounded live acceptance, not a complete monthly readiness audit","outputs":outputs})
        }
        Err(error) => serde_json::json!({"status":"failed","error":error}),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&receipt).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    result.expect("Real chat profiles, cache tools and routing must work");
}

#[test]
#[ignore = "Requires authorized app-owned state, private query/question/expected-title inputs and a new private output; makes one read-only Quick request"]
fn quick_company_lookup_agrees_with_native_cache_and_local_search() {
    company_lookup(Mode::Fast);
}

#[test]
#[ignore = "Requires authorized app-owned state, configured Foundry routing, private query/question/expected inputs and a new private receipt; makes one read-only Auto request"]
fn auto_company_lookup_routes_to_quick_and_agrees_with_fuzzy_search() {
    company_lookup(Mode::Auto);
}

fn company_lookup(mode: Mode) {
    let root = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_CHAT_ROOT").expect("Set TOMLOOK_LIVE_CHAT_ROOT"),
    );
    let output = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_CHAT_OUTPUT").expect("Set TOMLOOK_LIVE_CHAT_OUTPUT"),
    );
    let query = std::env::var("TOMLOOK_LIVE_CHAT_QUERY").expect("Set TOMLOOK_LIVE_CHAT_QUERY");
    let question =
        std::env::var("TOMLOOK_LIVE_CHAT_QUESTION").expect("Set TOMLOOK_LIVE_CHAT_QUESTION");
    let expected: Vec<String> = serde_json::from_str(
        &std::env::var("TOMLOOK_LIVE_CHAT_EXPECTED").expect("Set TOMLOOK_LIVE_CHAT_EXPECTED"),
    )
    .expect("Expected must be a JSON array of distinctive title fragments");
    let expected_answer: Vec<String> = serde_json::from_str(
        &std::env::var("TOMLOOK_LIVE_CHAT_EXPECTED_ANSWER")
            .expect("Set TOMLOOK_LIVE_CHAT_EXPECTED_ANSWER"),
    )
    .expect("Expected answer must be a JSON array of scenario-specific answer fragments");
    assert!(root.is_absolute() && output.is_absolute() && !output.exists());
    assert!(!query.is_empty() && !question.is_empty());
    assert!(!expected.is_empty() && expected.iter().all(|title| !title.trim().is_empty()));
    assert!(
        !expected_answer.is_empty() && expected_answer.iter().all(|text| !text.trim().is_empty())
    );
    let store = tomlook::storage::Store::read_only(&root).unwrap();
    let calendar = store.calendar().unwrap();
    let sources = store.preferences().unwrap().sources();
    let account = store
        .calendar_owner()
        .unwrap()
        .expect("Connect calendar first");
    let cache = tomlook::chat_cache::Cache {
        root: root.clone(),
        account: account.clone(),
    };
    let native = cache
        .read(
            tomlook::chat_cache::SEARCH,
            serde_json::json!({"query":query}),
        )
        .unwrap();
    let local = tomlook::chat_cache::local_answer(
        &store,
        &calendar,
        tomlook::chat_cache::LocalQuery::Search(query),
        None,
        tomlook::i18n::Language::Cs,
        false,
    )
    .unwrap();
    for title in &expected {
        assert!(
            native["meetings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|meeting| meeting["title"].as_str().unwrap().contains(title)),
            "Native cache omitted expected title fragment {title}"
        );
        assert!(
            local.answer.text.contains(title),
            "Local search omitted {title}"
        );
    }
    drop(store);
    let mut engine = Engine::start(root, false, eframe::egui::Context::default()).unwrap();
    engine
        .commands
        .blocking_send(Command::ContextSources(sources))
        .unwrap();
    let result = (|| -> Result<serde_json::Value, String> {
        connect(&engine, &account)?;
        let started = Instant::now();
        let id = chrono::Utc::now().timestamp_millis() as u64;
        send(&engine, id, &account, mode, &question)?;
        let mut stages = Vec::new();
        let mut selected_quick = false;
        let mut dispatch_ms = None;
        loop {
            if started.elapsed() > Duration::from_secs(90) {
                return Err("Quick lookup exceeded its acceptance deadline".into());
            }
            match engine.notices.recv_timeout(Duration::from_millis(250)) {
                Ok(Notice::ChatProgress {
                    id: current,
                    progress,
                }) if current == id => {
                    if progress.mode == Mode::Fast && !selected_quick {
                        dispatch_ms = Some(started.elapsed().as_millis());
                        selected_quick = true;
                    }
                    if stages.last() != Some(&progress.stage) {
                        stages.push(progress.stage);
                    }
                }
                Ok(Notice::Answer {
                    id: current,
                    result,
                }) if current == id => {
                    let answer = result?;
                    if !selected_quick {
                        return Err("The targeted lookup did not dispatch to Quick".into());
                    }
                    if expected_answer
                        .iter()
                        .any(|text| !answer.text.to_lowercase().contains(&text.to_lowercase()))
                    {
                        return Err(format!(
                            "Quick omitted expected scenario details: {}",
                            answer.text
                        ));
                    }
                    if expected
                        .iter()
                        .any(|title| !answer.text.to_lowercase().contains(&title.to_lowercase()))
                    {
                        return Err(format!(
                            "Quick omitted an expected meeting: {}",
                            answer.text
                        ));
                    }
                    if !answer.evidence.records.iter().any(|record| {
                        record.succeeded && record.tool == tomlook::chat_cache::SEARCH
                    }) {
                        return Err("Quick answer lacks successful native calendar evidence".into());
                    }
                    return Ok(serde_json::json!({
                        "requested_mode":mode,"selected_mode":"fast","dispatch_ms":dispatch_ms,
                        "model":Mode::Fast.model(),"effort":Mode::Fast.effort(),
                        "elapsed_seconds":started.elapsed().as_secs_f64(),
                        "native":native,"local_answer":local.answer.text,
                        "answer":answer.text,"evidence":answer.evidence,"stages":stages
                    }));
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("Quick answer notices closed".into()),
            }
        }
    })();
    engine.stop.send(true).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(45))
        .unwrap();
    assert!(engine.terminated.load(Ordering::Acquire));
    assert_eq!(engine.occupied.load(Ordering::Acquire), 0);
    let receipt = match &result {
        Ok(value) => serde_json::json!({"status":"passed","result":value}),
        Err(error) => serde_json::json!({"status":"failed","error":error}),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&receipt).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    result.expect("The exact Quick scenario must agree with native and local searches");
}

fn interrupt_and_continue(engine: &Engine, account: &str) -> Result<serde_json::Value, String> {
    connect(engine, account)?;
    let id = chrono::Utc::now().timestamp_millis() as u64;
    let question = "Použij jen uložený kalendář a uložené Insights. Prohledej příštích 30 dní a vyber tři nejbližší schůzky vhodné k přípravě prezentace nebo dema. U každé samostatně přečti uložené Insights a rozliš doložené požadavky od odhadu. Nepoužívej jiné zdroje ani zápisy. Odpověz nejvýše 300 slovy.";
    send(engine, id, account, Mode::Deep, question)?;
    let until = Instant::now() + Duration::from_secs(100);
    let mut key = None;
    let mut cancelled = false;
    let mut interrupted = false;
    let saved = loop {
        if Instant::now() >= until {
            return Err("Research did not checkpoint and stop within its bound".into());
        }
        match engine.notices.recv_timeout(Duration::from_millis(100)) {
            Ok(Notice::ChatProgress {
                id: current,
                progress,
            }) if current == id => {
                if progress.research_key.is_some() {
                    key = progress.research_key;
                }
                if !cancelled && !progress.steps.is_empty() {
                    engine
                        .commands
                        .blocking_send(Command::Cancel(id))
                        .map_err(|_| "Cancel command failed")?;
                    cancelled = true;
                }
            }
            Ok(Notice::Answer {
                id: current,
                result,
            }) if current == id => {
                if result.is_ok() || !cancelled {
                    return Err("Research finished before interruption could be verified".into());
                }
                interrupted = true;
            }
            Ok(Notice::ChatRecovered {
                result: Ok(saved), ..
            }) if interrupted => {
                if let Some(saved) = saved
                    .into_iter()
                    .find(|saved| Some(&saved.key) == key.as_ref())
                {
                    break saved;
                }
            }
            Ok(Notice::ChatRecovered {
                result: Err(error), ..
            }) => return Err(error),
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("Interrupted research notices closed".into()),
        }
    };
    if saved.status != "interrupted" || saved.evidence.records.is_empty() {
        return Err(format!(
            "Interruption did not persist native partial evidence: status={}, records={}",
            saved.status,
            saved.evidence.records.len()
        ));
    }
    let resumed = Instant::now();
    engine
        .commands
        .blocking_send(Command::Chat(Box::new(Request {
            question: Question {
                id: id + 1,
                question: saved.question.clone(),
                meeting: None,
                account: Some(account.into()),
                public_topic: None,
                suggest_drafts: false,
                language: saved.language,
            },
            mode: Mode::Deep,
            history: saved.history.clone(),
            resume: Some(saved.key.clone()),
        })))
        .map_err(|_| "Continuation command failed")?;
    let until = Instant::now() + Duration::from_secs(650);
    loop {
        if Instant::now() >= until {
            return Err("Continuation exceeded its bound".into());
        }
        match engine.notices.recv_timeout(Duration::from_millis(100)) {
            Ok(Notice::Answer {
                id: current,
                result,
            }) if current == id + 1 => {
                let answer = result?;
                if answer.text.trim().is_empty() {
                    return Err("Continuation omitted its answer".into());
                }
                return Ok(
                    serde_json::json!({"interrupted_status":saved.status,"saved_evidence":saved.evidence,
                    "continuation_seconds":resumed.elapsed().as_secs_f64(),"answer":&*answer.text,"evidence":&*answer.evidence}),
                );
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("Continuation notices closed".into()),
        }
    }
}

#[test]
#[ignore = "Requires authorized app-owned logins and a new private output; interrupts and explicitly continues two real read-only Sol requests"]
fn research_cancellation_persists_findings_and_explicit_continuation_completes() {
    let root = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_CHAT_ROOT").expect("Set TOMLOOK_LIVE_CHAT_ROOT"),
    );
    let output = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_CHAT_OUTPUT").expect("Set TOMLOOK_LIVE_CHAT_OUTPUT"),
    );
    assert!(root.is_absolute() && output.is_absolute() && !output.exists());
    let store = tomlook::storage::Store::read_only(&root).unwrap();
    let account = store.calendar_owner().unwrap().unwrap();
    let sources = store.preferences().unwrap().sources();
    drop(store);
    let mut engine = Engine::start(root, false, eframe::egui::Context::default()).unwrap();
    engine
        .commands
        .blocking_send(Command::ContextSources(sources))
        .unwrap();
    let result = interrupt_and_continue(&engine, &account);
    engine.stop.send(true).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(45))
        .expect("Owned engine must finish shutdown");
    assert!(engine.terminated.load(Ordering::Acquire));
    assert_eq!(engine.occupied.load(Ordering::Acquire), 0);
    let receipt = match &result {
        Ok(value) => serde_json::json!({"status":"passed","result":value}),
        Err(error) => serde_json::json!({"status":"failed","error":error}),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&receipt).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    result.expect("Cancellation must abort/detach and preserve resumable findings");
}
