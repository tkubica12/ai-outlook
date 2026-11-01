//! Read-only, non-interactive MSX probe. One bounded Azure CLI token read (20 seconds) per
//! resource, then tool discovery, a WhoAmI-style user resolution and a `TOP 1` read. Prints tool
//! names, row counts and column names only: never tokens or customer values. It never signs in.
use std::{sync::Arc, time::Instant};
use tomlook::msx::{Client, DEFAULT_TENANT, Resource, Settings};

fn shape(value: &serde_json::Value) -> String {
    let columns = value["rows"]
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(serde_json::Value::as_object)
        .map(|row| row.keys().cloned().collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    format!(
        "{} row(s), truncated: {}, columns: [{columns}]",
        value["row_count"], value["truncated"]
    )
}

fn excerpt(error: &str) -> String {
    error.chars().take(160).collect()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let tenant = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_TENANT.into());
    if let Err(error) = tomlook::msx::validate_tenant(&tenant) {
        eprintln!("Usage: msx-probe [tenant-id]: {error}");
        std::process::exit(1);
    }
    let client = Arc::new(Client::new(
        &Settings {
            enabled: true,
            tenant,
        },
        Arc::new(|_| {}),
    ));
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let (_stop, shutdown) = tokio::sync::watch::channel(false);
    let started = Instant::now();
    let checked = client.check(false, cancelled, shutdown).await;
    println!(
        "check: {} after {} ms",
        match &checked {
            Ok(()) => "connected".to_owned(),
            Err(failure) => format!("{} (retry: {})", failure.message, failure.retry),
        },
        started.elapsed().as_millis()
    );
    let status = client.status();
    for resource in Resource::ALL {
        let service = status.service(resource);
        println!(
            "{}: {:?} - {}",
            resource.label(),
            service.phase,
            service.message
        );
        if let Some((discovered, allowed)) = client.tool_names(resource).await {
            println!("  tools discovered: {}", discovered.join(", "));
            println!("  read tools allowed: {}", allowed.join(", "));
        }
    }
    if client.ready(Resource::Dataverse) {
        match client.whoami().await {
            Ok(user) => println!(
                "whoami: one systemuser resolved by {} ({} row)",
                user.value.matched_by, user.source.rows
            ),
            Err(error) => println!("whoami: {}", excerpt(&error)),
        }
        match client.ai_query("SELECT TOP 1 name FROM account").await {
            Ok(value) => println!("account TOP 1: {}", shape(&value)),
            Err(error) => println!("account TOP 1: {}", excerpt(&error)),
        }
    }
    if client.ready(Resource::PowerBi) {
        match client.ai_dax("EVALUATE ROW(\"probe\", 1)").await {
            Ok(value) => println!("Power BI ROW probe: {}", shape(&value)),
            Err(error) => println!("Power BI ROW probe: {}", excerpt(&error)),
        }
    }
    client.disconnect().await;
}
