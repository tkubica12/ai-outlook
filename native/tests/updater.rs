use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tomlook::updater::{self, Manifest, Staged};

fn key_pair() -> (Ed25519KeyPair, String) {
    let document = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(document.as_ref()).unwrap();
    let public = STANDARD.encode(pair.public_key().as_ref());
    (pair, public)
}

fn sign(pair: &Ed25519KeyPair, message: &[u8]) -> Vec<u8> {
    format!("{}\n", STANDARD.encode(pair.sign(message).as_ref())).into_bytes()
}

fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn manifest_json(version: &str, archive: &[u8], executable: &[u8]) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "schema": 1,
        "product": "Tomlook",
        "version": version,
        "tag": format!("v{version}"),
        "published": "2026-10-31T18:00:00Z",
        "notes_url": format!("https://github.com/tkubica12/ai-outlook/releases/tag/v{version}"),
        "archive": {
            "name": format!("Tomlook-v{version}-win-x64.zip"),
            "size": archive.len(),
            "sha256": hash(archive),
        },
        "executable": { "name": "Tomlook.exe", "sha256": hash(executable) },
    }))
    .unwrap()
}

fn manifest(version: &str, archive: &[u8], executable: &[u8]) -> Manifest {
    serde_json::from_str(&manifest_json(version, archive, executable)).unwrap()
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn files(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn signatures_are_verified_before_the_manifest_is_trusted() {
    let (pair, public) = key_pair();
    let (other, other_public) = key_pair();
    let json = manifest_json("9.1.0", b"archive", b"executable");
    let signature = sign(&pair, json.as_bytes());
    let verified = updater::verify_manifest(json.as_bytes(), &signature, &[&public]).unwrap();
    assert_eq!(verified.version, "9.1.0");
    assert!(
        updater::verify_manifest(json.as_bytes(), &signature, &[&other_public, &public]).is_ok()
    );
    for (message, signature, keys) in [
        (json.clone(), signature.clone(), vec![other_public.as_str()]),
        (
            json.clone(),
            sign(&other, json.as_bytes()),
            vec![public.as_str()],
        ),
        (
            json.replace("9.1.0", "9.2.0"),
            signature.clone(),
            vec![public.as_str()],
        ),
        (format!("{json} "), signature.clone(), vec![public.as_str()]),
        (json.clone(), b"not base64".to_vec(), vec![public.as_str()]),
        (
            json.clone(),
            signature[..20].to_vec(),
            vec![public.as_str()],
        ),
        (json.clone(), signature.clone(), vec![]),
        (
            "{ not json".into(),
            signature.clone(),
            vec![public.as_str()],
        ),
    ] {
        assert_eq!(
            updater::verify_manifest(message.as_bytes(), &signature, &keys).unwrap_err(),
            updater::INVALID_SIGNATURE
        );
    }
    let oversized = vec![b' '; updater::MANIFEST_LIMIT + 1];
    assert!(updater::verify_manifest(&oversized, &sign(&pair, &oversized), &[&public]).is_err());
}

#[test]
fn embedded_trusted_keys_are_ed25519_public_keys() {
    assert!(!updater::TRUSTED_KEYS.is_empty());
    for key in updater::TRUSTED_KEYS {
        assert_eq!(STANDARD.decode(key).unwrap().len(), 32);
    }
}

#[test]
fn signed_manifests_must_still_describe_a_tomlook_release() {
    let (pair, public) = key_pair();
    let valid = manifest_json("9.1.0", b"archive", b"executable");
    let archive_hash = hash(b"archive");
    assert!(
        manifest("9.1.0", b"archive", b"executable")
            .validate()
            .is_ok()
    );
    for (from, to) in [
        ("\"schema\": 1", "\"schema\": 2"),
        ("\"product\": \"Tomlook\"", "\"product\": \"Other\""),
        ("\"tag\": \"v9.1.0\"", "\"tag\": \"v9.0.0\""),
        (
            "\"published\": \"2026-10-31T18:00:00Z\"",
            "\"published\": \"yesterday\"",
        ),
        (
            "https://github.com/tkubica12/ai-outlook/releases/tag/v9.1.0",
            "http://github.com/tkubica12/ai-outlook/releases/tag/v9.1.0",
        ),
        (
            "https://github.com/tkubica12/ai-outlook/releases/tag/v9.1.0",
            "https://example.com/tkubica12/ai-outlook/releases/tag/v9.1.0",
        ),
        ("Tomlook-v9.1.0-win-x64.zip", "Tomlook-v9.1.0-win-x64.exe"),
        (
            "\"name\": \"Tomlook.exe\"",
            "\"name\": \"..\\\\Tomlook.exe\"",
        ),
        (archive_hash.as_str(), "ABCDEF"),
    ] {
        let changed = valid.replace(from, to);
        assert_ne!(changed, valid, "{from}");
        assert!(
            updater::verify_manifest(
                changed.as_bytes(),
                &sign(&pair, changed.as_bytes()),
                &[&public]
            )
            .is_err(),
            "{from} -> {to}"
        );
    }
    let prerelease = valid.replace("9.1.0", "9.1.0-beta.1");
    assert!(
        updater::verify_manifest(
            prerelease.as_bytes(),
            &sign(&pair, prerelease.as_bytes()),
            &[&public]
        )
        .is_err()
    );
}

#[test]
fn only_strictly_newer_semantic_versions_are_offered() {
    assert!(updater::is_newer("0.8.0", "0.7.0").unwrap());
    assert!(updater::is_newer("0.10.0", "0.9.9").unwrap());
    assert!(updater::is_newer("1.0.0", "0.99.0").unwrap());
    assert!(!updater::is_newer("0.7.0", "0.7.0").unwrap());
    assert!(!updater::is_newer("0.6.9", "0.7.0").unwrap());
    assert!(!updater::is_newer("0.8.0-rc.1", "0.8.0").unwrap());
    assert!(updater::is_newer("v0.8.0", "0.7.0").is_err());
}

#[test]
fn latest_release_ignores_drafts_and_requires_exact_github_assets() {
    let release = |draft: bool, prerelease: bool, url: &str| {
        serde_json::json!({
            "tag_name": "v9.1.0",
            "draft": draft,
            "prerelease": prerelease,
            "assets": [
                {"name": "tomlook-update.json", "size": 400, "browser_download_url": url},
                {"name": "tomlook-update.json.sig", "size": 89,
                 "browser_download_url": "https://github.com/tkubica12/ai-outlook/releases/download/v9.1.0/tomlook-update.json.sig"},
            ]
        })
        .to_string()
    };
    let exact =
        "https://github.com/tkubica12/ai-outlook/releases/download/v9.1.0/tomlook-update.json";
    assert!(
        updater::parse_release(release(true, false, exact).as_bytes())
            .unwrap()
            .is_none()
    );
    assert!(
        updater::parse_release(release(false, true, exact).as_bytes())
            .unwrap()
            .is_none()
    );
    let stable = updater::parse_release(release(false, false, exact).as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(stable.version.to_string(), "9.1.0");
    assert_eq!(
        stable.asset("tomlook-update.json").unwrap(),
        (exact.to_string(), 400)
    );
    assert!(stable.asset("tomlook-update.json.sig").is_ok());
    assert!(stable.asset("Tomlook-v9.1.0-win-x64.zip").is_err());
    let moved = updater::parse_release(
        release(false, false, "https://example.com/tomlook-update.json").as_bytes(),
    )
    .unwrap()
    .unwrap();
    assert!(moved.asset("tomlook-update.json").is_err());
    assert!(updater::parse_release(br#"{"tag_name":"latest","assets":[]}"#).is_err());
    assert!(updater::parse_release(b"<html>").is_err());
}

#[test]
fn verified_archives_are_extracted_into_staging() {
    let root = tempfile::tempdir().unwrap();
    let executable = b"new tomlook executable".to_vec();
    let archive = zip(&[
        ("Tomlook.exe", executable.as_slice()),
        ("LICENSE", b"MIT".as_slice()),
        ("THIRD_PARTY_NOTICES.txt", b"notices".as_slice()),
    ]);
    let path = root.path().join("download.zip");
    std::fs::write(&path, &archive).unwrap();
    let manifest = manifest("9.1.0", &archive, &executable);
    let directory = root.path().join(".tomlook-update").join("9.1.0");
    let target = root.path().join("Tomlook.exe");
    let staged = updater::stage(&path, &directory, &manifest, &target).unwrap();
    assert_eq!(staged.version, "9.1.0");
    assert_eq!(staged.target, target);
    assert_eq!(staged.executable_sha256, hash(&executable));
    assert_eq!(
        files(&directory),
        ["LICENSE", "THIRD_PARTY_NOTICES.txt", "Tomlook.exe"]
    );
    assert_eq!(
        std::fs::read(directory.join("Tomlook.exe")).unwrap(),
        executable
    );
}

#[test]
fn archives_with_traversal_unexpected_entries_or_wrong_hashes_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let executable = b"new tomlook executable".to_vec();
    let target = root.path().join("app").join("Tomlook.exe");
    let cases: Vec<(Vec<u8>, Option<Manifest>, &str)> = vec![
        (
            zip(&[("../Tomlook.exe", executable.as_slice())]),
            None,
            updater::UNSAFE_PATH,
        ),
        (
            zip(&[
                ("..\\LICENSE", b"x".as_slice()),
                ("Tomlook.exe", executable.as_slice()),
            ]),
            None,
            updater::UNSAFE_PATH,
        ),
        (
            zip(&[("nested/Tomlook.exe", executable.as_slice())]),
            None,
            updater::UNSAFE_PATH,
        ),
        (
            zip(&[
                ("Tomlook.exe", executable.as_slice()),
                ("helper.dll", b"dll".as_slice()),
            ]),
            None,
            updater::UNEXPECTED_ENTRY,
        ),
        (
            zip(&[("Tomlook.exe", b"tampered executable".as_slice())]),
            None,
            updater::EXECUTABLE_MISMATCH,
        ),
        (
            zip(&[("Tomlook.exe", executable.as_slice())]),
            Some(manifest("9.1.0", b"another archive", &executable)),
            updater::ARCHIVE_MISMATCH,
        ),
    ];
    for (index, (archive, manifest_override, expected)) in cases.into_iter().enumerate() {
        let case = root.path().join(format!("case-{index}"));
        std::fs::create_dir_all(&case).unwrap();
        let path = case.join("download.zip");
        std::fs::write(&path, &archive).unwrap();
        let manifest =
            manifest_override.unwrap_or_else(|| manifest("9.1.0", &archive, &executable));
        let directory = case.join(updater::STAGING).join("9.1.0");
        let result = updater::stage(&path, &directory, &manifest, &target);
        assert_eq!(result.unwrap_err(), expected, "case {index}");
        assert!(!directory.exists(), "case {index} left staging data");
        assert!(
            files(&case)
                .iter()
                .all(|name| name == "download.zip" || name == updater::STAGING),
            "case {index} wrote outside staging"
        );
        assert!(files(&case.join(updater::STAGING)).is_empty());
        assert!(!case.join("Tomlook.exe").exists() && !root.path().join("Tomlook.exe").exists());
    }
}

fn staged_install(root: &Path, executable: &[u8]) -> (Staged, PathBuf) {
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    let target = app.join("Tomlook.exe");
    std::fs::write(&target, b"old executable").unwrap();
    std::fs::write(app.join("LICENSE"), b"old license").unwrap();
    let directory = app.join(".tomlook-update").join("9.1.0");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("Tomlook.exe"), executable).unwrap();
    std::fs::write(directory.join("LICENSE"), b"new license").unwrap();
    std::fs::write(directory.join("THIRD_PARTY_NOTICES.txt"), b"new notices").unwrap();
    (
        Staged {
            version: "9.1.0".into(),
            notes_url: updater::release_page("v9.1.0"),
            directory,
            target: target.clone(),
            executable_sha256: hash(executable),
        },
        app,
    )
}

#[test]
fn applying_moves_the_old_executable_aside_and_installs_notices() {
    let root = tempfile::tempdir().unwrap();
    let (staged, app) = staged_install(root.path(), b"new executable");
    std::fs::write(updater::old_executable(&staged.target), b"stale").unwrap();
    updater::apply(&staged).unwrap();
    assert_eq!(std::fs::read(&staged.target).unwrap(), b"new executable");
    assert_eq!(
        std::fs::read(app.join("Tomlook.exe.old")).unwrap(),
        b"old executable"
    );
    assert_eq!(std::fs::read(app.join("LICENSE")).unwrap(), b"new license");
    assert_eq!(
        std::fs::read(app.join("THIRD_PARTY_NOTICES.txt")).unwrap(),
        b"new notices"
    );
    assert!(!app.join(".tomlook-update").exists());
    assert!(
        updater::apply(&staged).is_err(),
        "A consumed staging directory cannot be applied twice"
    );
    assert_eq!(std::fs::read(&staged.target).unwrap(), b"new executable");
}

#[test]
fn failed_installation_rolls_back_to_the_previous_executable() {
    let root = tempfile::tempdir().unwrap();
    let (staged, app) = staged_install(root.path(), b"new executable");
    std::fs::create_dir(app.join("THIRD_PARTY_NOTICES.txt")).unwrap();
    let error = updater::apply(&staged).unwrap_err();
    assert!(error.contains("previous version was restored"), "{error}");
    assert_eq!(std::fs::read(&staged.target).unwrap(), b"old executable");
    assert_eq!(std::fs::read(app.join("LICENSE")).unwrap(), b"old license");
    assert!(!app.join("Tomlook.exe.old").exists());
    assert_eq!(
        std::fs::read(staged.directory.join("Tomlook.exe")).unwrap(),
        b"new executable"
    );

    let (tampered, _) = staged_install(&root.path().join("tampered"), b"new executable");
    std::fs::write(
        tampered.directory.join("Tomlook.exe"),
        b"changed after verification",
    )
    .unwrap();
    assert!(
        updater::apply(&tampered)
            .unwrap_err()
            .contains("changed after verification")
    );
    assert_eq!(std::fs::read(&tampered.target).unwrap(), b"old executable");
    assert!(!updater::old_executable(&tampered.target).exists());
}

#[test]
fn startup_cleanup_removes_replaced_executables_and_stale_staging() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("Tomlook.exe");
    std::fs::write(&target, b"current").unwrap();
    std::fs::write(updater::old_executable(&target), b"replaced").unwrap();
    let staging = root.path().join(updater::STAGING);
    for directory in ["0.6.0", "0.7.0", "99.0.0", "not-a-version"] {
        std::fs::create_dir_all(staging.join(directory)).unwrap();
        std::fs::write(staging.join(directory).join("Tomlook.exe"), b"x").unwrap();
    }
    std::fs::write(staging.join("99.0.0.zip.part"), b"partial").unwrap();
    updater::cleanup(&target, &semver::Version::parse("0.7.0").unwrap());
    assert!(!updater::old_executable(&target).exists());
    assert_eq!(files(&staging), ["99.0.0"]);
    assert_eq!(std::fs::read(&target).unwrap(), b"current");
    std::fs::remove_dir_all(staging.join("99.0.0")).unwrap();
    updater::cleanup(&target, &semver::Version::parse("0.7.0").unwrap());
    assert!(!staging.exists());
}

#[test]
fn only_writable_tomlook_executables_outside_cargo_targets_are_replaced() {
    let root = tempfile::tempdir().unwrap();
    let staging = updater::staging_root(&root.path().join("Tomlook.exe")).unwrap();
    assert_eq!(staging, root.path().join(updater::STAGING));
    assert!(updater::staging_root(&root.path().join("tomlook.EXE")).is_ok());
    assert!(updater::staging_root(&root.path().join("Tomlook (1).exe")).is_err());
    assert!(updater::staging_root(&root.path().join("missing").join("Tomlook.exe")).is_err());
    let project = root.path().join("native");
    std::fs::create_dir_all(project.join("target").join("debug")).unwrap();
    let development = project.join("target").join("debug").join("tomlook.exe");
    assert!(!updater::development_build(&development));
    std::fs::write(project.join("Cargo.toml"), "[package]").unwrap();
    assert!(updater::development_build(&development));
    assert!(!updater::development_build(
        &root.path().join("release").join("Tomlook.exe")
    ));
}

#[test]
fn automatic_checks_wait_a_minute_and_run_at_most_daily() {
    let now = chrono::Utc::now();
    let day = Duration::from_secs(24 * 60 * 60);
    assert_eq!(
        updater::first_check_delay(Duration::ZERO, None, now),
        Duration::from_secs(60)
    );
    assert_eq!(
        updater::first_check_delay(Duration::from_secs(90), None, now),
        Duration::ZERO
    );
    assert_eq!(
        updater::first_check_delay(Duration::ZERO, Some(now - chrono::Duration::hours(2)), now),
        day - Duration::from_secs(2 * 60 * 60)
    );
    assert_eq!(
        updater::first_check_delay(Duration::ZERO, Some(now - chrono::Duration::hours(30)), now),
        Duration::from_secs(60)
    );
    assert_eq!(
        updater::first_check_delay(Duration::ZERO, Some(now + chrono::Duration::hours(5)), now),
        day
    );
}

#[test]
fn development_builds_disable_the_background_checker_without_network() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Cargo.toml"), "[package]").unwrap();
    let executable = root.path().join("target").join("debug").join("Tomlook.exe");
    let (sender, received) = std::sync::mpsc::channel();
    let checker = updater::Updater::start(executable, move || {
        let _ = sender.send(());
    })
    .unwrap();
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    let status = checker.status(0).unwrap();
    assert_eq!(status.disabled.as_deref(), Some(updater::DEVELOPMENT));
    assert!(status.activity.is_none() && status.checked_at.is_none());
    assert!(checker.status(status.revision).is_none());
    assert!(checker.exit_handle().finish().is_none());
}

#[test]
fn legacy_preferences_enable_automatic_update_checks() {
    let preferences: tomlook::storage::Preferences =
        serde_json::from_str(r#"{"dark":false,"language":"Cs","paused":true}"#).unwrap();
    assert!(preferences.automatic_update_check);
    assert!(preferences.update_checked_at.is_none());
    assert!(preferences.update_latest_version.is_empty());
    let root = tempfile::tempdir().unwrap();
    let store = tomlook::storage::Store::open(root.path()).unwrap();
    let checked = chrono::Utc::now();
    store
        .save_preferences(&tomlook::storage::Preferences {
            automatic_update_check: false,
            update_checked_at: Some(checked),
            update_latest_version: "9.1.0".into(),
            ..Default::default()
        })
        .unwrap();
    let restored = store.preferences().unwrap();
    assert!(!restored.automatic_update_check);
    assert_eq!(restored.update_checked_at, Some(checked));
    assert_eq!(restored.update_latest_version, "9.1.0");
}

#[test]
#[cfg(windows)]
fn release_keys_are_dpapi_protected_and_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let key = root.path().join("keys").join("signing.dpapi");
    let public = updater::signing::generate(&key).unwrap();
    assert_eq!(updater::signing::public_key(&key).unwrap(), public);
    let protected = std::fs::read(&key).unwrap();
    let raw = STANDARD.decode(&public).unwrap();
    assert!(
        !protected
            .windows(raw.len())
            .any(|window| window == raw.as_slice())
    );
    assert!(
        updater::signing::generate(&key)
            .unwrap_err()
            .contains("already exists")
    );
    assert_eq!(std::fs::read(&key).unwrap(), protected);

    let manifest = root.path().join("tomlook-update.json");
    let signature = root.path().join("tomlook-update.json.sig");
    std::fs::write(&manifest, manifest_json("9.1.0", b"archive", b"executable")).unwrap();
    assert_eq!(
        updater::signing::sign(&key, &manifest, &signature).unwrap(),
        public
    );
    let verified = updater::verify_manifest(
        &std::fs::read(&manifest).unwrap(),
        &std::fs::read(&signature).unwrap(),
        &[&public],
    )
    .unwrap();
    assert_eq!(verified.tag, "v9.1.0");
    assert!(updater::signing::sign(&key, &manifest, &signature).is_err());
    std::fs::write(root.path().join("bad.json"), "{}").unwrap();
    assert!(
        updater::signing::sign(
            &key,
            &root.path().join("bad.json"),
            &root.path().join("bad.sig")
        )
        .is_err()
    );
    assert!(!root.path().join("bad.sig").exists());
}
