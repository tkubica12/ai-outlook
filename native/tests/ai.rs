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
            public_topic: None,
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
    engine.commands.blocking_send(Command::Stop).unwrap();
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
