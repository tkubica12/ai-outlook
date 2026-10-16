use std::time::Duration;
use tomlook::ai::{Command, Engine, Notice, Question};

#[test]
fn preview_never_starts_a_runtime_and_shutdown_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let mut engine =
        Engine::start(root.path().into(), true, eframe::egui::Context::default()).unwrap();
    engine.commands.blocking_send(Command::Connect).unwrap();
    let notice = engine.notices.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        matches!(notice, Notice::Connection { ready:false, message } if message.contains("never starts"))
    );
    engine
        .commands
        .blocking_send(Command::Ask(Box::new(Question {
            id: 1,
            question: "Synthetic question".into(),
            meeting: None,
            account: None,
            public_topic: None,
            suggest_drafts: false,
            language: tomlook::i18n::Language::En,
        })))
        .unwrap();
    let notice = engine.notices.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(matches!(
        notice,
        Notice::Answer {
            id: 1,
            result: Err(_)
        }
    ));
    assert!(!root.path().join("copilot").exists());
    engine
        .commands
        .blocking_send(Command::Chat(Box::new(tomlook::chat::Request {
            question: Question {
                id: 2,
                question: "Synthetic chat".into(),
                meeting: None,
                account: None,
                public_topic: None,
                suggest_drafts: false,
                language: tomlook::i18n::Language::En,
            },
            mode: tomlook::chat::Mode::Deep,
            history: Vec::new(),
            resume: None,
        })))
        .unwrap();
    assert!(matches!(
        engine.notices.recv_timeout(Duration::from_secs(3)).unwrap(),
        Notice::Answer {
            id: 2,
            result: Err(_)
        }
    ));
    engine
        .commands
        .blocking_send(Command::RecoverChat("fixture@example.invalid".into()))
        .unwrap();
    assert!(
        matches!(engine.notices.recv_timeout(Duration::from_secs(3)).unwrap(),Notice::ChatRecovered { result:Ok(saved),.. } if saved.is_empty())
    );
    assert!(!root.path().join("ask-research.db").exists());
    engine.commands.blocking_send(Command::ProbeWorkiq).unwrap();
    let notice = engine.notices.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(matches!(notice, Notice::Workiq { message } if message.contains("never starts")));
    engine
        .commands
        .blocking_send(Command::ConnectWorkiq("fixture@microsoft.com".into()))
        .unwrap();
    let notice = engine.notices.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        matches!(notice, Notice::HostedWorkiq { busy: false, ready: false, message } if message.contains("never opens"))
    );
    engine.commands.blocking_send(Command::SignInAi).unwrap();
    assert!(matches!(
        engine.notices.recv_timeout(Duration::from_secs(3)).unwrap(),
        Notice::AiLogin { busy: false, message } if message.contains("never opens")
    ));
    for input in [
        tomlook::connectors::Input::WebiqKey("synthetic-key".into()),
        tomlook::connectors::Input::GithubApp("Iv23fixture".into()),
        tomlook::connectors::Input::GithubToken("github_pat_fixture".into()),
    ] {
        let kind = input.kind();
        engine
            .commands
            .blocking_send(Command::ConnectExternal { kind, input })
            .unwrap();
        assert!(
            matches!(engine.notices.recv_timeout(Duration::from_secs(3)).unwrap(), Notice::External { busy: false, ready: false, message, .. } if message.contains("Preview"))
        );
        engine
            .commands
            .blocking_send(Command::ForgetExternal(kind))
            .unwrap();
        assert!(
            matches!(engine.notices.recv_timeout(Duration::from_secs(3)).unwrap(), Notice::External { busy: false, ready: false, message, .. } if message.contains("Preview"))
        );
    }
    assert!(!root.path().join("connector-login.db").exists());
    engine.commands.blocking_send(Command::Stop).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
}

#[test]
fn hosted_workiq_rejects_invalid_account_without_runtime_or_ai_admission() {
    let root = tempfile::tempdir().unwrap();
    let mut engine =
        Engine::start(root.path().into(), false, eframe::egui::Context::default()).unwrap();
    engine
        .commands
        .blocking_send(Command::ConnectWorkiq("invalid-account".into()))
        .unwrap();
    loop {
        let notice = engine.notices.recv_timeout(Duration::from_secs(3)).unwrap();
        if let Notice::HostedWorkiq {
            busy: false,
            message,
            ..
        } = notice
        {
            assert!(message.contains("Choose your Microsoft work email"));
            break;
        }
    }
    assert!(!engine.ready.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        engine.occupied.load(std::sync::atomic::Ordering::Acquire),
        0
    );
    assert!(!root.path().join("copilot").exists());
    engine.stop.send(true).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
}

#[test]
fn missing_setup_is_an_explicit_error_not_a_personal_profile_fallback() {
    let root = tempfile::tempdir().unwrap();
    let mut engine =
        Engine::start(root.path().into(), false, eframe::egui::Context::default()).unwrap();
    engine.commands.blocking_send(Command::Connect).unwrap();
    loop {
        let notice = engine.notices.recv_timeout(Duration::from_secs(3)).unwrap();
        if matches!(notice, Notice::Connection { ready:false, message } if message.contains("connections.json"))
        {
            break;
        }
    }
    assert!(!root.path().join("copilot").exists());
    engine.commands.blocking_send(Command::Stop).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
}

#[test]
fn hidden_notifications_and_queued_commands_cannot_block_priority_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let mut engine =
        Engine::start(root.path().into(), true, eframe::egui::Context::default()).unwrap();
    for _ in 0..100 {
        engine.commands.blocking_send(Command::Connect).unwrap();
    }

    engine.stop.send(true).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert!(engine.terminated.load(std::sync::atomic::Ordering::Acquire));
    assert!(!engine.ready.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        engine.occupied.load(std::sync::atomic::Ordering::Acquire),
        0
    );
    assert!(!root.path().join("copilot").exists());
}

#[test]
fn workiq_preflight_missing_configuration_does_not_start_copilot() {
    let root = tempfile::tempdir().unwrap();
    let mut engine =
        Engine::start(root.path().into(), false, eframe::egui::Context::default()).unwrap();
    engine.commands.blocking_send(Command::ProbeWorkiq).unwrap();
    loop {
        let notice = engine.notices.recv_timeout(Duration::from_secs(3)).unwrap();
        if matches!(notice, Notice::Workiq { message } if message.contains("connections.json")) {
            break;
        }
    }
    assert!(!root.path().join("copilot").exists());
    assert!(!engine.ready.load(std::sync::atomic::Ordering::Acquire));
    engine.stop.send(true).unwrap();
    engine
        .done
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
}
