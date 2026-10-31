//! Release manifest signing for Tomlook updates. Private key material is never printed.

use std::path::PathBuf;
use tomlook::updater::{TRUSTED_KEYS, signing, stage, verify_manifest};

const USAGE: &str = "Usage:
  release-sign generate --key <path>
  release-sign public-key --key <path>
  release-sign sign --key <path> --in <tomlook-update.json> --out <tomlook-update.json.sig>
  release-sign verify --in <tomlook-update.json> --sig <tomlook-update.json.sig> [--archive <zip>]";

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments
        .next()
        .and_then(|command| command.into_string().ok())
        .ok_or(USAGE)?;
    let (mut key, mut input, mut output, mut signature, mut archive) =
        (None, None, None, None, None);
    while let Some(option) = arguments.next() {
        let value = arguments
            .next()
            .map(PathBuf::from)
            .ok_or("Every option requires a value")?;
        let slot = match option.to_str() {
            Some("--key") => &mut key,
            Some("--in") => &mut input,
            Some("--out") => &mut output,
            Some("--sig") => &mut signature,
            Some("--archive") => &mut archive,
            _ => {
                return Err(format!(
                    "Unknown option {}\n{USAGE}",
                    option.to_string_lossy()
                ));
            }
        };
        if slot.replace(value).is_some() {
            return Err(format!("{} was given twice", option.to_string_lossy()));
        }
    }
    let required = |value: Option<PathBuf>, name: &str| {
        value.ok_or_else(|| format!("{command} requires {name}\n{USAGE}"))
    };
    match command.as_str() {
        "generate" => println!("{}", signing::generate(&required(key, "--key")?)?),
        "public-key" => println!("{}", signing::public_key(&required(key, "--key")?)?),
        "sign" => {
            let output = required(output, "--out")?;
            signing::sign(&required(key, "--key")?, &required(input, "--in")?, &output)?;
            println!("Signed {}", output.display());
        }
        "verify" => {
            let input = required(input, "--in")?;
            let signature = required(signature, "--sig")?;
            let read = |path: &PathBuf| {
                std::fs::read(path).map_err(|error| format!("Read {}: {error}", path.display()))
            };
            let manifest = verify_manifest(&read(&input)?, &read(&signature)?, TRUSTED_KEYS)?;
            println!(
                "Verified {} {} ({}, {} bytes) against the trusted Tomlook release keys",
                manifest.product, manifest.version, manifest.archive.name, manifest.archive.size
            );
            if let Some(archive) = archive {
                // Same size, hash and safe-extraction checks the app runs before staging.
                let directory = archive
                    .parent()
                    .ok_or("Archive path has no directory")?
                    .join(format!(".release-sign-verify-{}", std::process::id()));
                let result = stage(
                    &archive,
                    &directory,
                    &manifest,
                    &directory.join("Tomlook.exe"),
                );
                let _ = std::fs::remove_dir_all(&directory);
                result?;
                println!("Verified {} and its Tomlook.exe hash", archive.display());
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("release-sign: {error}");
        std::process::exit(1);
    }
}
