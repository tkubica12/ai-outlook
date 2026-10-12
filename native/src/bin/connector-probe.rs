use std::path::PathBuf;
use std::time::{Duration, Instant};
use tomlook::copilot::{Config, Harness};

/// No-model connector sign-in probe: runs runtime-managed MCP OAuth for one configured
/// connector in the isolated Tomlook profile and reports its status.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Tomlook connector probe: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .ok_or("Usage: connector-probe <state-directory> [connector]")?;
    let root = std::path::absolute(root).map_err(|e| e.to_string())?;
    let only = args.next().map(|s| s.to_string_lossy().into_owned());
    let config = Config::load(&root)?;
    let harness = Harness::start(&root, config).await?;
    let result = async {
        let health = harness.health().await?;
        if !health.authenticated {
            return Err(format!(
                "Isolated identity not accepted: {:?}",
                health.rejected
            ));
        }
        let session = tokio::time::timeout(Duration::from_secs(60), harness.sign_in_session())
            .await
            .map_err(|_| "Sign-in session timed out".to_string())??;
        println!("session {}", session.id());
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut status = harness.connector_status(&session).await?;
        while status.iter().any(|(_, s, _)| s == "pending") && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(500)).await;
            status = harness.connector_status(&session).await?;
        }
        println!("initial {status:?}");
        for (name, state, _) in status.clone() {
            if only.as_ref().is_some_and(|only| *only != name) || state == "connected" {
                continue;
            }
            match harness.connector_login(&session, &name).await? {
                Some(url) => {
                    println!("{name}: opening browser sign-in");
                    tomlook::browser::open(&url)?;
                }
                None => println!("{name}: cached tokens accepted"),
            }
            let deadline = Instant::now() + Duration::from_secs(300);
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let now = harness.connector_status(&session).await?;
                let current = now.iter().find(|(n, _, _)| *n == name).cloned();
                if let Some((_, state, error)) = &current
                    && (state == "connected" || state == "failed" || Instant::now() > deadline)
                {
                    println!("{name}: {state} {error:?}");
                    break;
                }
            }
        }
        println!("final {:?}", harness.connector_status(&session).await?);
        let _ = session.disconnect().await;
        Ok(())
    }
    .await;
    let stopped = harness.stop().await;
    result.and(stopped)
}
