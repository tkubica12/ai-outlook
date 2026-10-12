use std::time::{Duration, Instant};
use tomlook::workiq::connection::Connection;

#[tokio::main]
async fn main() {
    let result = async {
        let mut args = std::env::args_os().skip(1);
        let root =
            std::path::PathBuf::from(args.next().ok_or("Expected app-owned state directory")?);
        let account = args
            .next()
            .ok_or("Expected work account")?
            .into_string()
            .map_err(|_| "Invalid work account")?;
        if args.next().is_some() {
            return Err("Unexpected diagnostic arguments".into());
        }
        let connection = Connection::connect_saved(&root, &account, false, |_| {}).await?;
        let started = Instant::now();
        let result = connection.context("Locate HTML presentations in my OneDrive folder ZakaznickeProjekty. Return existing original file links, not general information. Do not modify anything.").await;
        let elapsed_ms = started.elapsed().as_millis();
        let closed = connection.close().await;
        closed?;
        match result {
            Ok(value) => {
                println!(
                    "{}",
                    serde_json::json!({
                        "outcome": "context_read_verified",
                        "elapsed_ms": elapsed_ms,
                        "response_bytes": serde_json::to_vec(&value).map_err(|_| "Encode context result failed")?.len(),
                        "answer_bytes": value.get("answer").and_then(serde_json::Value::as_str).map(str::len),
                        "metadata_bytes": value.get("metadata").map(serde_json::to_vec).transpose().map_err(|_| "Encode context metadata failed")?.map(|bytes| bytes.len())
                    })
                );
                Ok(())
            }
            Err(error) => Err(format!("{error}; elapsed {elapsed_ms} ms")),
        }
    };
    match tokio::time::timeout(Duration::from_secs(220), result).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("WorkIQ context diagnostic: {error}");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("WorkIQ context diagnostic exceeded its deadline");
            std::process::exit(1);
        }
    }
}
