use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tomlook::{calendar, storage::Store};

fn main() {
    if let Err(error) = run() {
        eprintln!("Tomlook fixture seed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .ok_or("Usage: seed-fixture <new-owned-state-directory> <200|5000>")?;
    let count: usize = args
        .next()
        .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
        .filter(|count| matches!(count, 200 | 5000))
        .ok_or("Event count must be 200 or 5000")?;
    if args.next().is_some() {
        return Err("Unexpected argument".into());
    }
    let root = std::path::absolute(root).map_err(|e| format!("Resolve state directory: {e}"))?;
    if root.exists()
        && std::fs::read_dir(&root)
            .map_err(|e| format!("Inspect state directory: {e}"))?
            .next()
            .is_some()
    {
        return Err("Refusing to seed a non-empty directory; use a new owned fixture state".into());
    }
    let events = calendar::fixture(count);
    let mut digest = Sha256::new();
    for event in &events {
        let json = serde_json::to_vec(event).map_err(|e| e.to_string())?;
        digest.update(json.len().to_le_bytes());
        digest.update(json);
    }
    let built = calendar::Calendar::build(events, calendar::fixture_coverage(), Vec::new())?;
    let mut store = Store::open(&root)?;
    store.save_calendar(&built)?;
    let saved = store.calendar()?;
    if saved.events.len() != count || saved.coverage != built.coverage {
        return Err("Seeded calendar read-back does not match the fixture inventory".into());
    }
    let all_day = saved.events.iter().filter(|e| e.is_all_day).count();
    let multi_day = saved
        .events
        .iter()
        .filter(|e| !e.is_all_day && e.all_day_like())
        .count();
    println!(
        "{}",
        serde_json::json!({
            "version": calendar::FIXTURE_VERSION,
            "state": root,
            "events": saved.events.len(),
            "coverage_days": saved.coverage.len(),
            "all_day": all_day,
            "multi_day": multi_day,
            "input_sha256": format!("{:x}", digest.finalize()),
        })
    );
    Ok(())
}
