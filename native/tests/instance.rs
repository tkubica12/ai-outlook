#![cfg(windows)]

use std::{sync::mpsc, time::Duration};
use tomlook::instance::{Activation, acquire, acquire_or_restore};

#[test]
fn pending_restore_coalesces_before_the_window_binds_and_shutdown_releases_it() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("window profile \u{010D}");
    let owner = acquire(&profile).unwrap();
    let activation = Activation::new(&profile).unwrap();
    for _ in 0..20 {
        assert!(acquire_or_restore(&profile.join(".")).unwrap().is_none());
    }
    let (sender, received) = mpsc::channel();
    let mut listener = activation
        .listen(move |result| sender.send(result).unwrap())
        .unwrap();
    received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert!(
        received.recv_timeout(Duration::from_millis(50)).is_err(),
        "Pending window requests must coalesce instead of building an unbounded inbox"
    );
    assert!(acquire(&profile).unwrap_err().contains("already open"));
    listener.shutdown().unwrap();
    listener.shutdown().unwrap();
    drop(owner);
    let _next_owner = acquire_or_restore(&profile).unwrap().unwrap();
    Activation::new(&profile).unwrap();
}

#[test]
fn separate_profiles_do_not_restore_each_other_or_share_listeners() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    let _first_owner = acquire(&first).unwrap();
    let (sender, received) = mpsc::channel();
    let mut first_listener = Activation::new(&first)
        .unwrap()
        .listen(move |result| sender.send(result).unwrap())
        .unwrap();
    let _second_owner = acquire_or_restore(&second).unwrap().unwrap();
    let (sender, second_received) = mpsc::channel();
    let mut second_listener = Activation::new(&second)
        .unwrap()
        .listen(move |result| sender.send(result).unwrap())
        .unwrap();
    assert!(acquire_or_restore(&second).unwrap().is_none());
    second_received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert!(received.recv_timeout(Duration::from_millis(50)).is_err());
    first_listener.shutdown().unwrap();
    second_listener.shutdown().unwrap();
}

#[test]
fn a_signal_collision_fails_without_replacing_the_primary_listener() {
    let root = tempfile::tempdir().unwrap();
    let _owner = acquire(root.path()).unwrap();
    let activation = Activation::new(root.path()).unwrap();
    assert!(matches!(
        Activation::new(root.path()),
        Err(error) if error.contains("already exists")
    ));
    let (sender, received) = mpsc::channel();
    let mut listener = activation
        .listen(move |result| sender.send(result).unwrap())
        .unwrap();
    assert!(acquire_or_restore(root.path()).unwrap().is_none());
    received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    listener.shutdown().unwrap();
}

#[test]
fn a_secondary_launch_waits_for_primary_signal_registration() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("starting");
    let _owner = acquire(&profile).unwrap();
    let candidate = profile.clone();
    let (started, ready) = mpsc::channel();
    let requester = std::thread::spawn(move || {
        started.send(()).unwrap();
        acquire_or_restore(&candidate).unwrap().is_none()
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let activation = Activation::new(&profile).unwrap();
    assert!(requester.join().unwrap());
    let (sender, received) = mpsc::channel();
    let mut listener = activation
        .listen(move |result| sender.send(result).unwrap())
        .unwrap();
    received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    listener.shutdown().unwrap();
}

#[test]
fn profile_creation_errors_do_not_request_another_window() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("not-a-directory");
    std::fs::write(&file, "unchanged").unwrap();
    assert!(acquire_or_restore(&file).unwrap_err().contains("ownership"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "unchanged");
}

#[test]
fn an_older_instance_without_a_signal_fails_visibly_and_keeps_ownership() {
    let root = tempfile::tempdir().unwrap();
    let _owner = acquire(root.path()).unwrap();
    let started = std::time::Instant::now();
    let error = acquire_or_restore(root.path()).unwrap_err();
    assert!(error.contains("already open") && error.contains("tray icon"));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(acquire(root.path()).unwrap_err().contains("already open"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn restore_request_child() {
    let Some(profile) = std::env::var_os("TOMLOOK_TEST_WINDOW_PROFILE") else {
        return;
    };
    assert!(
        acquire_or_restore(std::path::Path::new(&profile))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_secondary_process_only_signals_the_owned_profile() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("interprocess");
    let owner = acquire(&profile).unwrap();
    let (sender, received) = mpsc::channel();
    let mut listener = Activation::new(&profile)
        .unwrap()
        .listen(move |result| sender.send(result).unwrap())
        .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "restore_request_child", "--nocapture"])
        .env("TOMLOOK_TEST_WINDOW_PROFILE", &profile)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert!(acquire(&profile).unwrap_err().contains("already open"));
    assert_eq!(
        std::fs::read_dir(&profile).unwrap().count(),
        1,
        "Secondary launches must not open a database or AI profile"
    );
    listener.shutdown().unwrap();
    drop(owner);
    acquire(&profile).unwrap();
}
