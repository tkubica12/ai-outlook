use std::path::PathBuf;
use tomlook::{source_index::Index, storage::Store};

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let root = PathBuf::from(arguments.next().ok_or("Usage: source-index <state> [--repositories <path>] [--cloud-folder <name>] [--query <terms>]")?);
    let store = Store::open(&root)?;
    let mut preferences = store.preferences()?;
    let mut save = false;
    let mut query = None;
    while let Some(argument) = arguments.next() {
        let value = arguments.next().ok_or("Every option requires a value")?;
        match argument.as_str() {
            "--repositories" => {
                let path = PathBuf::from(value);
                if !preferences.repository_collections.contains(&path) {
                    preferences.repository_collections.push(path);
                }
                save = true;
            }
            "--cloud-folder" => {
                preferences.cloud_context_folder = value;
                save = true;
            }
            "--query" => query = Some(value),
            _ => return Err("Unknown source-index option".into()),
        }
    }
    let config = preferences.sources();
    config.validate()?;
    if save {
        store.save_preferences(&preferences)?;
    }
    let index = Index::new(&root, config)?;
    let report = index.refresh()?;
    println!("{}", report.label());
    if let Some(query) = query {
        let result = index.search(&query)?;
        println!(
            "Matching documents: {}",
            result["matches"]
                .as_array()
                .ok_or("Invalid search result")?
                .len()
        );
    }
    let incomplete = !report.errors.is_empty();
    for error in report.errors {
        eprintln!("{error}");
    }
    if incomplete {
        return Err("Source coverage is incomplete; review the reported source errors".into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Source index failed: {error}");
        std::process::exit(1);
    }
}
