fn main() {
    let result = run();
    if let Err(error) = result {
        eprintln!("Calendar fixture: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map(std::path::PathBuf::from)
        .ok_or("Usage: calendar-fixture <new-tomlook-fixture-directory> <200|5000>")?;
    let count = args
        .next()
        .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
        .ok_or("Fixture count must be 200 or 5000")?;
    if args.next().is_some() {
        return Err("Unexpected fixture argument".into());
    }
    let root = std::path::absolute(root).map_err(|error| error.to_string())?;
    let manifest = tomlook::fixture_cache::seed(&root, count)?;
    println!(
        "{}",
        serde_json::to_string(&manifest).map_err(|error| error.to_string())?
    );
    Ok(())
}
