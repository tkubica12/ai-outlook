use std::{collections::BTreeSet, path::PathBuf};
use tomlook::copilot::{Config, Connection, Policy, isolated_options};

#[test]
fn home_and_working_directory_are_child_only_and_app_owned() {
    let original = std::env::var_os("COPILOT_HOME");
    let root = tempfile::tempdir().unwrap();
    let runtime = PathBuf::from(r"C:\installed\copilot.exe");
    let options = isolated_options(root.path(), &runtime);
    let Some(copilot_sdk::RuntimeConnection::SpawnedStdio(process)) = &options.runtime else {
        panic!("Expected spawned stdio runtime");
    };
    assert_eq!(
        process.owned.base_directory.as_deref(),
        Some(root.path().join("copilot").as_path())
    );
    assert_eq!(process.working_directory, root.path().join("workspace"));
    assert!(process.env.is_none());
    assert!(process.args.iter().any(|arg| arg == "--no-remote-export"));
    for _ in 0..2 {
        let spaced = root.path().join("Tomlook \u{010c}esk\u{00fd} profile");
        let retry = isolated_options(&spaced, &runtime);
        let Some(copilot_sdk::RuntimeConnection::SpawnedStdio(retry_process)) = &retry.runtime
        else {
            panic!("Expected spawned stdio runtime");
        };
        assert_eq!(
            retry_process.owned.base_directory,
            Some(spaced.join("copilot"))
        );
        assert_eq!(retry_process.working_directory, spaced.join("workspace"));
    }
    assert_eq!(options.mode, copilot_sdk::ClientMode::Empty);
    assert_eq!(options.github.use_logged_in_user, Some(false));
    assert!(options.observability.disable_client_telemetry);
    assert!(process.env_remove.iter().any(|name| name == "GH_TOKEN"));
    assert!(process.env_remove.iter().any(|name| name == "GITHUB_TOKEN"));
    assert!(options.github.token.is_none());
    assert_eq!(std::env::var_os("COPILOT_HOME"), original);
}

#[test]
fn native_runtime_library_or_adjacent_wrapper_hosts_in_process_isolated() {
    let root = tempfile::tempdir().unwrap();
    let runtime_dir = root.path().join("pkg");
    std::fs::create_dir_all(&runtime_dir).unwrap();
    let library = runtime_dir.join("runtime.node");
    let wrapper = runtime_dir.join("copilot-runtime.exe");
    std::fs::write(&library, b"fixture").unwrap();
    std::fs::write(&wrapper, b"fixture").unwrap();

    for configured in [&library, &wrapper] {
        let config = Config {
            runtime: configured.clone(),
            model: "sample".into(),
            copilot_credential_env: None,
            use_stored_login: true,
            servers: Default::default(),
        };
        assert!(config.validate().is_ok());
        let options = isolated_options(root.path(), configured);
        let Some(copilot_sdk::RuntimeConnection::InProcess(in_process)) = &options.runtime else {
            panic!("Expected in-process runtime for {}", configured.display());
        };
        assert_eq!(
            in_process.owned.base_directory.as_deref(),
            Some(root.path().join("copilot").as_path())
        );
        assert_eq!(
            in_process.runtime_library_path.as_deref(),
            Some(library.as_path())
        );
        assert_eq!(in_process.eager_power_shell_resolution, Some(false));
        assert_eq!(options.mode, copilot_sdk::ClientMode::Empty);
        assert_eq!(options.github.use_logged_in_user, Some(false));
        assert!(options.observability.disable_client_telemetry);
        assert!(options.github.token.is_none());
    }
}

#[test]
fn conflicting_inherited_home_is_not_used_or_changed() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "home_and_working_directory_are_child_only_and_app_owned",
        ])
        .env("COPILOT_HOME", r"C:\conflicting personal profile")
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn only_registered_read_tools_and_exact_public_topics_are_allowed() {
    let policy = Policy {
        names: BTreeSet::from([
            "WorkIQ-Mail-GetMessage".into(),
            "WebIQ-MCP-web".into(),
            "dataverse-read_query".into(),
        ]),
        public_topic: Some("Azure Container Apps public documentation".into()),
    };
    assert!(policy.allows_tool(
        "WorkIQ-Mail-GetMessage",
        &serde_json::json!({"id":"sample"})
    ));
    assert!(!policy.allows_tool(
        "WorkIQ-Mail-SendEmailWithAttachments",
        &serde_json::json!({})
    ));
    assert!(!policy.allows_tool("powershell", &serde_json::json!({})));
    assert!(policy.allows_tool(
        "WebIQ-MCP-web",
        &serde_json::json!({"query":"Azure Container Apps public documentation"})
    ));
    assert!(!policy.allows_tool(
        "WebIQ-MCP-web",
        &serde_json::json!({"query":"private@example.com secret meeting"})
    ));
    for extra in ["location", "context", "q"] {
        let mut arguments =
            serde_json::json!({"query":"Azure Container Apps public documentation"});
        arguments[extra] = serde_json::json!("private@example.com secret meeting");
        assert!(!policy.allows_tool("WebIQ-MCP-web", &arguments));
    }
    assert!(policy.allows_tool(
        "dataverse-read_query",
        &serde_json::json!({"query":"SELECT name FROM account"})
    ));
    for query in [
        "DELETE FROM task",
        "SELECT name INTO stolen FROM account",
        "SELECT name FROM account; DELETE FROM task",
    ] {
        assert!(!policy.allows_tool("dataverse-read_query", &serde_json::json!({"query":query})));
    }
}

#[test]
fn connection_configuration_rejects_ambient_credentials_unknown_servers_and_unsafe_urls() {
    let runtime = std::env::current_exe().unwrap();
    let mut config = Config {
        runtime,
        model: "sample".into(),
        copilot_credential_env: None,
        use_stored_login: false,
        servers: Default::default(),
    };
    config.servers.insert(
        "WorkIQ-Mail".into(),
        Connection {
            url: "https://example.com/mcp".into(),
            credential_env: Some("GH_TOKEN".into()),
        },
    );
    assert!(config.validate().is_err());
    config
        .servers
        .get_mut("WorkIQ-Mail")
        .unwrap()
        .credential_env = Some("TOMLOOK_WORKIQ_TOKEN".into());
    assert!(config.validate().is_ok());
    config.servers.get_mut("WorkIQ-Mail").unwrap().url =
        "https://user:password@example.com/mcp".into();
    assert!(config.validate().is_err());
    config.servers.clear();
    config.servers.insert(
        "shell-server".into(),
        Connection {
            url: "https://example.com".into(),
            credential_env: None,
        },
    );
    assert!(config.validate().is_err());
}
