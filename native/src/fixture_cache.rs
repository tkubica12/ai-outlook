use crate::{calendar, storage::Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub events: usize,
    pub days: usize,
    pub anchor: String,
    pub sha256: String,
}

fn owned_directory(root: &Path) -> Result<(), String> {
    if !root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("tomlook-fixture-"))
    {
        return Err("Fixture profiles must have a tomlook-fixture- directory name".into());
    }
    Ok(())
}

fn fingerprint(calendar: &calendar::Calendar) -> Result<String, String> {
    let data = serde_json::to_vec(&(&calendar.events, &calendar.coverage))
        .map_err(|error| format!("Encode frozen fixture inventory: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(data)))
}

impl Manifest {
    pub fn verify(&self, calendar: &calendar::Calendar) -> Result<(), String> {
        let expected = calendar::Calendar::build(
            calendar::fixture(self.events),
            calendar::fixture_coverage(),
            Vec::new(),
        )?;
        if calendar.events.len() != self.events
            || calendar.coverage.len() != self.days
            || fingerprint(calendar)? != self.sha256
            || fingerprint(&expected)? != self.sha256
        {
            return Err("Cached calendar does not match the frozen fixture manifest".into());
        }
        Ok(())
    }
}

pub fn read(root: &Path) -> Result<Manifest, String> {
    owned_directory(root)?;
    let file = std::fs::File::open(root.join("fixture.json"))
        .map_err(|error| format!("Read fixture manifest: {error}"))?;
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Read fixture manifest: {error}"))?;
    if bytes.len() > 4096 {
        return Err("Fixture manifest exceeds its 4 KiB bound".into());
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Parse fixture manifest: {error}"))?;
    if manifest.version != "tomlook-frozen-fixture-v1"
        || !matches!(manifest.events, 200 | 5000)
        || manifest.days != 42
        || manifest.anchor != calendar::fixture_anchor().to_string()
        || manifest.sha256.len() != 64
        || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Fixture manifest is not a supported frozen corpus".into());
    }
    Ok(manifest)
}

pub fn seed(root: &Path, count: usize) -> Result<Manifest, String> {
    owned_directory(root)?;
    if !matches!(count, 200 | 5000) {
        return Err("Frozen fixture inventory must contain 200 or 5000 events".into());
    }
    match std::fs::read_dir(root) {
        Ok(mut entries) => {
            if entries
                .next()
                .transpose()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err(
                    "Fixture seeding requires a new empty profile; existing data is not replaced"
                        .into(),
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Inspect fixture directory: {error}")),
    }
    let _ownership = crate::instance::acquire(root)?;
    let calendar = calendar::Calendar::build(
        calendar::fixture(count),
        calendar::fixture_coverage(),
        Vec::new(),
    )?;
    let manifest = Manifest {
        version: "tomlook-frozen-fixture-v1".into(),
        events: count,
        days: 42,
        anchor: calendar::fixture_anchor().to_string(),
        sha256: fingerprint(&calendar)?,
    };
    let mut store = Store::open(root)?;
    store.save_calendar(&calendar)?;
    manifest.verify(&store.calendar()?)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("fixture.json"))
        .map_err(|error| format!("Create fixture manifest: {error}"))?;
    let bytes = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Persist fixture manifest: {error}"))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_inventory_survives_real_sqlite_reload_and_rejects_changes() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("tomlook-fixture-200");
        seed(&root, 200).unwrap();
        let manifest = read(&root).unwrap();
        let mut store = Store::open(&root).unwrap();
        manifest.verify(&store.calendar().unwrap()).unwrap();
        let changed = calendar::Calendar::build(
            calendar::fixture(199),
            calendar::fixture_coverage(),
            Vec::new(),
        )
        .unwrap();
        store.save_calendar(&changed).unwrap();
        assert!(manifest.verify(&store.calendar().unwrap()).is_err());
    }

    #[test]
    fn fixture_seeding_never_replaces_existing_data_or_uses_normal_profile() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("tomlook-fixture-owned");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("keep"), "existing").unwrap();
        assert!(seed(&root, 200).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("keep")).unwrap(),
            "existing"
        );
        assert!(seed(&temporary.path().join("Tomlook"), 200).is_err());
    }
}
