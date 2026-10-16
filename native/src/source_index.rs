use crate::context;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub folders: Vec<PathBuf>,
    pub repositories: Vec<PathBuf>,
    pub cloud_folder: String,
}

impl Config {
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.repositories.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.folders.len() > 8 || self.repositories.len() > 4 {
            return Err(
                "Choose up to eight document folders and four repository collections".into(),
            );
        }
        if self.cloud_folder.len() > 256 || self.cloud_folder.chars().any(char::is_control) {
            return Err("Use a short cloud folder name without control characters".into());
        }
        if !self.cloud_folder.is_empty() {
            crate::workiq::connection::cloud::validate_folder(&self.cloud_folder)?;
        }
        for folder in &self.folders {
            context::approved_root(folder)?;
        }
        for collection in &self.repositories {
            local_collection(collection)?;
        }
        Ok(())
    }

    fn identity(&self) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(&(&self.folders, &self.repositories))
            .map_err(|_| "Encode source scope failed")?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Clone)]
pub struct Index {
    database: PathBuf,
    config: Config,
    scope: String,
    pub stopped: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    pub repositories: usize,
    pub documents: usize,
    pub skipped: usize,
    pub exclusions: std::collections::BTreeMap<String, usize>,
    pub truncated: bool,
    pub needs_backfill: bool,
    pub errors: Vec<String>,
    pub updated_at: String,
}

impl Report {
    pub fn label(&self) -> String {
        format!(
            "{} repos, {} indexed documents, {} excluded{}{}",
            self.repositories,
            self.documents,
            self.skipped,
            if self.truncated { " (partial)" } else { "" },
            if self.errors.is_empty() {
                ""
            } else {
                " - source errors"
            },
        )
    }
}

fn local_collection(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Repository collections must use an absolute local path".into());
    }
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    {
        return Err("Repository collections must be on a local drive".into());
    }
    context::verify_components(path)?;
    let canonical = path
        .canonicalize()
        .map_err(|_| "A repository collection is unavailable")?;
    if !canonical.is_dir() {
        return Err("A repository collection must be a directory".into());
    }
    Ok(canonical)
}

fn discover(
    collection: &Path,
    stopped: &AtomicBool,
) -> Result<(Vec<PathBuf>, bool, Vec<String>), String> {
    let collection = local_collection(collection)?;
    let mut pending = VecDeque::from([(collection, 0)]);
    let mut repositories = BTreeSet::new();
    let started = Instant::now();
    let mut visited = 0;
    let mut truncated = false;
    let mut errors = Vec::new();
    while let Some((directory, depth)) = pending.pop_front() {
        if stopped.load(Ordering::Acquire) {
            return Err("Source discovery was cancelled".into());
        }
        let marker = directory.join(".git");
        if std::fs::symlink_metadata(&marker).is_ok_and(|metadata| {
            !context::link(&metadata) && (metadata.is_dir() || metadata.is_file())
        }) {
            repositories.insert(directory);
            continue;
        }
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                errors.push(format!(
                    "Cannot discover repositories in {}",
                    directory.display()
                ));
                truncated = true;
                continue;
            }
        };
        for entry in entries {
            visited += 1;
            if visited > 30_000 || started.elapsed() > Duration::from_secs(10) {
                truncated = true;
                break;
            }
            let entry = entry.map_err(|_| "A repository collection entry could not be read")?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "node_modules"
                        | "target"
                        | "vendor"
                        | "dist"
                        | "build"
                        | "venv"
                        | "__pycache__"
                        | "packages"
                )
            {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|_| "A repository collection entry could not be inspected")?;
            if metadata.is_dir()
                && !context::link(&metadata)
                && !context::hidden_or_system(&metadata)
            {
                if depth < 8 {
                    pending.push_back((path, depth + 1));
                } else {
                    truncated = true;
                }
            }
        }
        if visited > 30_000 || started.elapsed() > Duration::from_secs(10) {
            break;
        }
    }
    Ok((repositories.into_iter().collect(), truncated, errors))
}

impl Index {
    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn has_local_sources(&self) -> bool {
        !self.config.is_empty()
    }

    pub fn cloud_folder(&self) -> &str {
        &self.config.cloud_folder
    }

    pub fn new(state: &Path, config: Config) -> Result<Self, String> {
        if !config.cloud_folder.is_empty() {
            crate::workiq::connection::cloud::validate_folder(&config.cloud_folder)?;
        }
        let scope = config.identity()?;
        Ok(Self {
            database: state.join("source-index.db"),
            config,
            scope,
            stopped: Arc::new(AtomicBool::new(false)),
        })
    }

    fn open(&self) -> Result<Connection, String> {
        let mut connection = Connection::open(&self.database).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(Duration::from_millis(100))
            .map_err(|e| e.to_string())?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA cache_size=-2048;
                 PRAGMA max_page_count=131072;
                 CREATE TABLE IF NOT EXISTS roots(
                   id INTEGER PRIMARY KEY,scope TEXT NOT NULL,path TEXT NOT NULL,
                   repository INTEGER NOT NULL,UNIQUE(scope,path));
                 CREATE TABLE IF NOT EXISTS documents(
                   id INTEGER PRIMARY KEY,root INTEGER NOT NULL,path TEXT NOT NULL,
                   stamp TEXT NOT NULL,UNIQUE(root,path));
                 CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(path,content);
                 CREATE TABLE IF NOT EXISTS reports(scope TEXT PRIMARY KEY,payload TEXT NOT NULL);",
            )
            .map_err(|e| format!("Open source index: {e}"))?;
        use sha2::{Digest, Sha256};
        let legacy = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&self.config)
                    .map_err(|_| "Encode legacy source scope failed")?
            )
        );
        if legacy != self.scope {
            let transaction = connection.transaction().map_err(|e| e.to_string())?;
            let populated: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM roots WHERE scope=?1)",
                    [&self.scope],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !populated {
                transaction
                    .execute(
                        "UPDATE roots SET scope=?1 WHERE scope=?2",
                        params![self.scope, legacy],
                    )
                    .map_err(|e| e.to_string())?;
                transaction.execute("INSERT OR REPLACE INTO reports SELECT ?1,payload FROM reports WHERE scope=?2", params![self.scope, legacy]).map_err(|e| e.to_string())?;
                transaction
                    .execute("DELETE FROM reports WHERE scope=?1", [&legacy])
                    .map_err(|e| e.to_string())?;
            }
            transaction.commit().map_err(|e| e.to_string())?;
        }
        Ok(connection)
    }

    pub fn refresh(&self) -> Result<Report, String> {
        self.config.validate()?;
        let mut report = Report::default();
        let mut roots = BTreeMap::new();
        for collection in &self.config.repositories {
            match discover(collection, &self.stopped) {
                Ok((repositories, truncated, errors)) => {
                    report.truncated |= truncated;
                    report.errors.extend(errors);
                    for root in repositories {
                        roots.insert(root, true);
                    }
                }
                Err(error) => report.errors.push(error),
            }
        }
        report.repositories = roots.values().filter(|repository| **repository).count();
        for root in &self.config.folders {
            roots.entry(context::approved_root(root)?).or_insert(false);
        }
        self.check_running()?;
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        let mut active_roots = BTreeSet::new();
        for (root, repository) in roots {
            self.check_running()?;
            let path = root.to_str().ok_or("A source path is not Unicode")?;
            transaction
                .execute(
                    "INSERT INTO roots(scope,path,repository) VALUES(?1,?2,?3)
                 ON CONFLICT(scope,path) DO UPDATE SET repository=excluded.repository",
                    params![self.scope, path, repository],
                )
                .map_err(|e| e.to_string())?;
            let id: i64 = transaction
                .query_row(
                    "SELECT id FROM roots WHERE scope=?1 AND path=?2",
                    params![self.scope, path],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            active_roots.insert(id);
            self.index_root(&transaction, id, &root, &mut report)?;
        }
        // Retain incomplete discovery results, but never retain a removed root after a complete scan.
        if !report.truncated && report.errors.is_empty() {
            let mut statement = transaction
                .prepare("SELECT id FROM roots WHERE scope=?1")
                .map_err(|e| e.to_string())?;
            let obsolete = statement
                .query_map([&self.scope], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(statement);
            for id in obsolete.into_iter().filter(|id| !active_roots.contains(id)) {
                transaction.execute("DELETE FROM search WHERE rowid IN (SELECT id FROM documents WHERE root=?1)", [id]).map_err(|e| e.to_string())?;
                transaction
                    .execute("DELETE FROM documents WHERE root=?1", [id])
                    .map_err(|e| e.to_string())?;
                transaction
                    .execute("DELETE FROM roots WHERE id=?1", [id])
                    .map_err(|e| e.to_string())?;
            }
        }
        report.updated_at = chrono::Utc::now().to_rfc3339();
        let payload = serde_json::to_string(&report).map_err(|e| e.to_string())?;
        transaction.execute(
            "INSERT INTO reports VALUES(?1,?2) ON CONFLICT(scope) DO UPDATE SET payload=excluded.payload",
            params![self.scope, payload],
        ).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(report)
    }

    fn check_running(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            Err("Source indexing was cancelled".into())
        } else {
            Ok(())
        }
    }

    fn index_root(
        &self,
        connection: &Connection,
        id: i64,
        root: &Path,
        report: &mut Report,
    ) -> Result<(), String> {
        let mut pending = VecDeque::from([(root.to_owned(), 0)]);
        let mut visited = 0;
        let mut changed = 0;
        let mut seen = BTreeSet::new();
        let started = Instant::now();
        let mut truncated = false;
        while let Some((directory, depth)) = pending.pop_front() {
            self.check_running()?;
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) => {
                    report
                        .errors
                        .push(format!("Cannot inspect {}", directory.display()));
                    truncated = true;
                    continue;
                }
            };
            for entry in entries {
                self.check_running()?;
                visited += 1;
                if visited > 10_000 || changed >= 64 || started.elapsed() > Duration::from_secs(2) {
                    truncated = true;
                    break;
                }
                let entry = entry.map_err(|_| "A source directory entry could not be read")?;
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "Source escaped its root")?;
                if context::blocked(relative) {
                    continue;
                }
                let metadata = std::fs::symlink_metadata(&path)
                    .map_err(|_| "A source file could not be inspected")?;
                if context::link(&metadata) || context::hidden_or_system(&metadata) {
                    report.skipped += 1;
                    continue;
                }
                if metadata.is_dir() {
                    if depth < 8 {
                        pending.push_back((path, depth + 1));
                    } else {
                        truncated = true;
                    }
                    continue;
                }
                if !metadata.is_file() || !context::text_file(relative) {
                    continue;
                }
                let relative = relative.to_str().ok_or("A source path is not Unicode")?;
                seen.insert(relative.to_owned());
                let stamp = format!(
                    "{}:{:?}",
                    metadata.len(),
                    metadata
                        .modified()
                        .map_err(|_| "Source timestamp unavailable")?
                );
                let previous: Option<String> = connection
                    .query_row(
                        "SELECT stamp FROM documents WHERE root=?1 AND path=?2",
                        params![id, relative],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if previous.as_deref() == Some(&stamp) {
                    continue;
                }
                changed += 1;
                // Revalidate the current file and remove obsolete indexed text when it becomes unsafe.
                let result = context::read_document(root, relative);
                connection.execute("DELETE FROM search WHERE rowid IN (SELECT id FROM documents WHERE root=?1 AND path=?2)", params![id, relative]).map_err(|e| e.to_string())?;
                connection
                    .execute(
                        "INSERT INTO documents(root,path,stamp) VALUES(?1,?2,?3)
                     ON CONFLICT(root,path) DO UPDATE SET stamp=excluded.stamp",
                        params![id, relative, stamp],
                    )
                    .map_err(|e| e.to_string())?;
                let content = match result {
                    Ok(content) => content,
                    Err(error) => {
                        report.skipped += 1;
                        *report.exclusions.entry(error).or_default() += 1;
                        continue;
                    }
                };
                let document: i64 = connection
                    .query_row(
                        "SELECT id FROM documents WHERE root=?1 AND path=?2",
                        params![id, relative],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let content = &content[..content.floor_char_boundary(16_000)];
                connection
                    .execute(
                        "INSERT INTO search(rowid,path,content) VALUES(?1,?2,?3)",
                        params![document, relative, content],
                    )
                    .map_err(|e| e.to_string())?;
            }
            if truncated
                && (visited > 10_000 || changed >= 64 || started.elapsed() > Duration::from_secs(2))
            {
                break;
            }
        }
        if !truncated {
            let mut statement = connection
                .prepare("SELECT id,path FROM documents WHERE root=?1")
                .map_err(|e| e.to_string())?;
            let documents = statement
                .query_map([id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(statement);
            for (document, _) in documents
                .into_iter()
                .filter(|(_, path)| !seen.contains(path))
            {
                connection
                    .execute("DELETE FROM search WHERE rowid=?1", [document])
                    .map_err(|e| e.to_string())?;
                connection
                    .execute("DELETE FROM documents WHERE id=?1", [document])
                    .map_err(|e| e.to_string())?;
            }
        }
        report.truncated |= truncated;
        report.needs_backfill |= changed >= 64;
        let count = connection.query_row("SELECT count(*) FROM documents JOIN search ON search.rowid=documents.id WHERE root=?1", [id], |row| row.get::<_, i64>(0)).map_err(|e| e.to_string())?;
        report.documents += usize::try_from(count).map_err(|_| "Invalid indexed document count")?;
        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Value, String> {
        let terms = query
            .split(|character: char| !character.is_alphanumeric())
            .filter(|term| term.chars().count() >= 2)
            .take(12)
            .map(|term| format!("\"{term}\"*"))
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Err("Use at least one word with two characters in source search".into());
        }
        let connection = self.open()?;
        let report: Option<String> = connection
            .query_row(
                "SELECT payload FROM reports WHERE scope=?1",
                [&self.scope],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let report = report
            .ok_or("Source indexing has not finished its first pass; check Insights activity")?;
        let mut statement = connection
            .prepare(
                "SELECT roots.id,roots.path,documents.path,snippet(search,1,'','', ' ... ',28)
             FROM search JOIN documents ON documents.id=search.rowid
             JOIN roots ON roots.id=documents.root
             WHERE search MATCH ?1 AND roots.scope=?2 ORDER BY bm25(search,4.0,1.0) LIMIT 40",
            )
            .map_err(|e| e.to_string())?;
        let matches = statement.query_map(params![terms.join(" OR "), self.scope], |row| {
            Ok(json!({"root":row.get::<_,i64>(0)?,"folder":row.get::<_,String>(1)?,
                "path":row.get::<_,String>(2)?,"excerpt":row.get::<_,String>(3)?,"text_readable":true}))
        }).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
        let mut report = serde_json::from_str::<Report>(&report).map_err(|e| e.to_string())?;
        report.errors.truncate(3);
        Ok(json!({"matches":matches,"index":report,
            "result_limit":40,"indexed_prefix_bytes":16000,"cloud_files":"Use WorkIQ for OneDrive/SharePoint; local index never follows cloud reparse points."}))
    }

    pub fn read(&self, root: usize, path: &str, offset: usize) -> Result<Value, String> {
        let root = i64::try_from(root).map_err(|_| "Unknown source root")?;
        let connection = self.open()?;
        let folder: Option<String> = connection
            .query_row(
                "SELECT path FROM roots WHERE id=?1 AND scope=?2",
                params![root, self.scope],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let folder = folder.ok_or("Unknown or no longer authorized source root")?;
        let content = context::read_document(Path::new(&folder), path)?;
        context::document_chunk(root, path, &folder, content, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_sources_count_once_and_cloud_hints_do_not_duplicate_local_indexes() {
        let collection = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let root = collection.path().join("demo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("README.md"), "Useful company demo").unwrap();
        let mut config = Config {
            folders: vec![root],
            repositories: vec![collection.path().to_owned()],
            ..Config::default()
        };
        let index = Index::new(state.path(), config.clone()).unwrap();
        let report = index.refresh().unwrap();
        assert_eq!(report.repositories, 1);
        assert_eq!(report.documents, 1);
        config.cloud_folder = "Customer materials".into();
        let changed = Index::new(state.path(), config).unwrap();
        assert_eq!(index.scope, changed.scope);
        assert_eq!(
            changed.search("company").unwrap()["matches"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        use sha2::{Digest, Sha256};
        let legacy = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&index.config).unwrap())
        );
        let connection = index.open().unwrap();
        connection
            .execute(
                "UPDATE roots SET scope=?1 WHERE scope=?2",
                params![legacy, index.scope],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE reports SET scope=?1 WHERE scope=?2",
                params![legacy, index.scope],
            )
            .unwrap();
        drop(connection);
        assert_eq!(
            index.search("company").unwrap()["matches"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn bounded_passes_backfill_and_unsafe_files_do_not_starve_later_documents() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        for number in 0..70 {
            std::fs::write(
                root.path().join(format!("demo-{number}.md")),
                "Customer demo context",
            )
            .unwrap();
        }
        std::fs::write(
            root.path().join("unsafe.json"),
            "{\"password\":\"do-not-index\"}",
        )
        .unwrap();
        let index = Index::new(
            state.path(),
            Config {
                folders: vec![root.path().to_owned()],
                ..Config::default()
            },
        )
        .unwrap();
        assert!(index.refresh().unwrap().needs_backfill);
        let report = index.refresh().unwrap();
        assert!(!report.needs_backfill);
        assert!(!report.truncated);
        assert_eq!(report.documents, 70);
        assert!(
            index.search("do-not-index").unwrap()["matches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        index.stopped.store(true, Ordering::Release);
        assert!(index.refresh().is_err());
    }

    #[test]
    fn indexes_all_repositories_and_discovers_additions_without_arbitrary_drive_reads() {
        let collection = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let create = |name: &str| {
            let root = collection.path().join(name);
            std::fs::create_dir_all(root.join(".git")).unwrap();
            std::fs::write(root.join("README.md"), format!("Company context in {name}")).unwrap();
        };
        create("alpha");
        create("beta");
        std::fs::write(
            collection.path().join("unrelated.md"),
            "Company private unrelated",
        )
        .unwrap();
        let index = Index::new(
            state.path(),
            Config {
                folders: Vec::new(),
                repositories: vec![collection.path().to_owned()],
                ..Config::default()
            },
        )
        .unwrap();
        assert_eq!(index.refresh().unwrap().repositories, 2);
        let matches = index.search("company").unwrap()["matches"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|value| value["path"] == "README.md"));
        create("gamma");
        assert_eq!(index.refresh().unwrap().repositories, 3);
        assert_eq!(
            index.search("company").unwrap()["matches"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let hit = &matches[0];
        assert!(
            index
                .read(
                    hit["root"].as_u64().unwrap() as usize,
                    "..\\unrelated.md",
                    0
                )
                .is_err()
        );
        let other = Index::new(state.path(), Config::default()).unwrap();
        assert!(
            other
                .read(hit["root"].as_u64().unwrap() as usize, "README.md", 0)
                .is_err()
        );
    }

    #[test]
    fn changed_and_deleted_documents_do_not_leave_stale_indexed_evidence() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let path = root.path().join("meeting.md");
        std::fs::write(&path, "Previous context").unwrap();
        let index = Index::new(
            state.path(),
            Config {
                folders: vec![root.path().to_owned()],
                repositories: Vec::new(),
                ..Config::default()
            },
        )
        .unwrap();
        index.refresh().unwrap();
        assert_eq!(
            index.search("previous").unwrap()["matches"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        std::fs::write(&path, "Replacement context here").unwrap();
        index.refresh().unwrap();
        assert!(
            index.search("previous").unwrap()["matches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        std::fs::remove_file(&path).unwrap();
        index.refresh().unwrap();
        assert!(
            index.search("replacement").unwrap()["matches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
