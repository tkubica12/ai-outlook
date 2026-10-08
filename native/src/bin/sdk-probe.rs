use std::path::PathBuf;
use tomlook::copilot::{Config, Harness};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Tomlook SDK probe: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .ok_or("Usage: sdk-probe <isolated-state-directory> <installed-runtime.exe>")?;
    let root = std::path::absolute(root).map_err(|e| format!("Resolve probe directory: {e}"))?;
    let runtime = args
        .next()
        .map(PathBuf::from)
        .ok_or("Installed runtime path is required")?;
    let stored_login = match args.next().as_deref().and_then(|arg| arg.to_str()) {
        None => false,
        Some("--stored-login") => true,
        Some(_) => return Err("Optional third argument must be --stored-login".into()),
    };
    if args.next().is_some() {
        return Err("Unexpected argument".into());
    }
    let config = Config {
        runtime,
        model: "gpt-5.6-terra".into(),
        copilot_credential_env: None,
        use_stored_login: stored_login,
        servers: Default::default(),
    };
    let harness = Harness::start(&root, config).await?;
    let result = async {
        let mut health = harness.health().await?;
        let session =
            tokio::time::timeout(std::time::Duration::from_secs(30), harness.session(None))
                .await
                .map_err(|_| "Probe session creation timed out".to_string())??;
        health.session_id = Some(session.id().to_string());
        tokio::time::timeout(std::time::Duration::from_secs(5), session.disconnect())
            .await
            .map_err(|_| "Probe session detach timed out".to_string())?
            .map_err(|e| format!("Disconnect probe session: {e}"))?;
        println!(
            "{}",
            serde_json::to_string(&health).map_err(|e| e.to_string())?
        );
        Ok(())
    }
    .await;
    let stopped = harness.stop().await;
    match (result, stopped) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(stop)) => Err(format!("{error}; {stop}")),
    }
}
