use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    match run().await {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(text) => {
                println!("{text}");
                if !report.authenticated_read_verified {
                    std::process::exit(2);
                }
            }
            Err(error) => {
                eprintln!("Tomlook WorkIQ probe serialization failed: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("Tomlook WorkIQ probe: {error}");
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<tomlook::workiq::ProbeReport, String> {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .ok_or("Usage: workiq-probe <Tomlook-state-directory> [WorkIQ-Calendar]")?;
    let root = std::path::absolute(root).map_err(|e| format!("Resolve state directory: {e}"))?;
    let connector = args
        .next()
        .map(|value| value.into_string().map_err(|_| "Invalid connector name"))
        .transpose()?
        .unwrap_or_else(|| "WorkIQ-Calendar".into());
    if args.next().is_some() {
        return Err("Unexpected arguments; no credential or runtime options are accepted".into());
    }
    tomlook::workiq::probe(&root, &connector).await
}
