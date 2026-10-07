use crate::calendar::{Calendar, Event, View};
use chrono::NaiveDate;
use rusqlite::{Connection, OpenFlags, Transaction, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub dark: bool,
    pub accent: usize,
    pub view: View,
    pub paused: bool,
    pub prepare_enabled: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            dark: true,
            accent: 0,
            view: View::WorkWeek,
            paused: false,
            prepare_enabled: false,
        }
    }
}

pub struct Store {
    connection: Connection,
    pub root: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(|e| format!("Create Tomlook state: {e}"))?;
        let connection = Connection::open(root.join("tomlook.db")).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE IF NOT EXISTS calendar(id TEXT PRIMARY KEY,payload TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS coverage(day TEXT PRIMARY KEY);
                 CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,payload TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS briefings(
                   meeting_id TEXT NOT NULL,version INTEGER NOT NULL,payload TEXT NOT NULL,
                   created_at TEXT NOT NULL,PRIMARY KEY(meeting_id,version));
                 CREATE TABLE IF NOT EXISTS legacy_records(
                   source TEXT NOT NULL,identity TEXT NOT NULL,payload TEXT NOT NULL,
                   PRIMARY KEY(source,identity));",
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            connection,
            root: root.into(),
        })
    }

    pub fn preferences(&self) -> Result<Preferences> {
        use rusqlite::OptionalExtension;
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key='preferences'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        payload.map_or(Ok(Preferences::default()), |json| {
            serde_json::from_str(&json).map_err(|e| format!("Read saved preferences: {e}"))
        })
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        let json = serde_json::to_string(preferences).map_err(|e| e.to_string())?;
        self.connection
            .execute(
                "INSERT INTO settings VALUES('preferences',?1)
                 ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                [json],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn calendar(&self) -> Result<Calendar> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM calendar")
            .map_err(|e| e.to_string())?;
        let mut events = Vec::new();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            events.push(
                serde_json::from_str(&row.map_err(|e| e.to_string())?)
                    .map_err(|e| format!("Read saved calendar: {e}"))?,
            );
        }
        let mut statement = self
            .connection
            .prepare("SELECT day FROM coverage")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut coverage = BTreeSet::new();
        for row in rows {
            coverage.insert(
                row.map_err(|e| e.to_string())?
                    .parse::<NaiveDate>()
                    .map_err(|e| e.to_string())?,
            );
        }
        Calendar::build(events, coverage, Vec::new())
    }

    pub fn save_calendar(&mut self, calendar: &Calendar) -> Result<()> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        Self::write_calendar(&transaction, calendar)?;
        transaction.commit().map_err(|e| e.to_string())
    }

    fn write_calendar(transaction: &Transaction<'_>, calendar: &Calendar) -> Result<()> {
        transaction
            .execute_batch("DELETE FROM calendar; DELETE FROM coverage;")
            .map_err(|e| e.to_string())?;
        for event in &calendar.events {
            let json = serde_json::to_string(event).map_err(|e| e.to_string())?;
            transaction
                .execute(
                    "INSERT INTO calendar VALUES(?1,?2)",
                    params![event.id, json],
                )
                .map_err(|e| e.to_string())?;
        }
        for day in &calendar.coverage {
            transaction
                .execute("INSERT INTO coverage VALUES(?1)", [day.to_string()])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn import_legacy(&mut self, root: &Path) -> Result<Option<Calendar>> {
        let already: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM settings WHERE key='legacy_imported')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if already {
            return Ok(None);
        }
        let cache_path = root.join("data").join("calendar-cache.json");
        if !cache_path.exists() {
            return Ok(None);
        }
        if fs::metadata(&cache_path).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
            return Err(
                "Legacy calendar exceeds the 32 MiB import limit; original is untouched".into(),
            );
        }
        let bytes = fs::read(&cache_path).map_err(|e| format!("Read legacy cache: {e}"))?;
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| format!("Read legacy cache JSON: {e}"))?;
        let rows = value
            .as_array()
            .or_else(|| value.get("events").and_then(|v| v.as_array()))
            .ok_or("Legacy cache has no event array")?;
        let mut events = Vec::new();
        let mut skipped = 0;
        for row in rows {
            match serde_json::from_value::<Event>(row.clone()) {
                Ok(event) if event.validate().is_ok() => events.push(event),
                _ => skipped += 1,
            }
        }
        let mut warnings = vec![
            "Imported a read-only copy of the legacy cache; coverage must be refreshed".into(),
        ];
        if skipped > 0 {
            warnings.push(format!("{skipped} invalid or unzoned legacy meetings were not imported; the original remains untouched"));
        }
        let calendar = Calendar::build(events, BTreeSet::new(), warnings)?;
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        Self::write_calendar(&transaction, &calendar)?;
        let database = root.join("data").join("outlook-next.db");
        if database.exists() {
            Self::import_briefings(&transaction, &database)?;
        }
        transaction
            .execute("INSERT INTO settings VALUES('legacy_imported','true')", [])
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(Some(calendar))
    }

    fn import_briefings(transaction: &Transaction<'_>, path: &Path) -> Result<()> {
        let original = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Open legacy briefings read-only: {e}"))?;
        let mut statement = original
            .prepare("SELECT meeting_id,version,payload,created_at FROM briefings")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, version, payload, created) = row.map_err(|e| e.to_string())?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO briefings VALUES(?1,?2,?3,?4)",
                    params![id, version, payload, created],
                )
                .map_err(|e| e.to_string())?;
        }
        for table in ["feedback", "skill_proposals", "analysis_jobs"] {
            let mut statement = original
                .prepare(&format!("SELECT * FROM {table}"))
                .map_err(|e| e.to_string())?;
            let names = statement
                .column_names()
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>();
            let mut rows = statement.query([]).map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let mut record = serde_json::Map::new();
                for (column, name) in names.iter().enumerate() {
                    let value = match row.get_ref(column).map_err(|e| e.to_string())? {
                        rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                        rusqlite::types::ValueRef::Integer(i) => i.into(),
                        rusqlite::types::ValueRef::Text(t) => {
                            std::str::from_utf8(t).map_err(|e| e.to_string())?.into()
                        }
                        _ => {
                            return Err(
                                "Unsupported legacy record type; original is untouched".into()
                            );
                        }
                    };
                    record.insert(name.clone(), value);
                }
                let identity = record
                    .get("id")
                    .ok_or("Legacy record has no identity")?
                    .to_string();
                let payload = serde_json::to_string(&record).map_err(|e| e.to_string())?;
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO legacy_records VALUES(?1,?2,?3)",
                        params![table, identity, payload],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    pub fn briefing(&self, id: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row(
                "SELECT payload FROM briefings WHERE meeting_id=?1 ORDER BY version DESC LIMIT 1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn queue(&self) -> Result<crate::scheduler::Queue> {
        use rusqlite::OptionalExtension;
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key='preparation-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let mut queue = match payload {
            Some(payload) => serde_json::from_str(&payload)
                .map_err(|e| format!("Read preparation ledger: {e}"))?,
            None => crate::scheduler::Queue::default(),
        };
        queue.recover()?;
        Ok(queue)
    }

    pub fn save_queue(&self, queue: &crate::scheduler::Queue) -> Result<()> {
        let payload = serde_json::to_string(queue).map_err(|e| e.to_string())?;
        if payload.len() > 32 * 1024 * 1024 {
            return Err(
                "Preparation ledger reached its 32 MiB safety limit; history was not discarded"
                    .into(),
            );
        }
        self.connection.execute("INSERT INTO settings VALUES('preparation-v1',?1) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload", [payload])
            .map_err(|e| format!("Save preparation ledger: {e}"))?;
        Ok(())
    }

    pub fn save_prepared(
        &mut self,
        queue: &crate::scheduler::Queue,
        id: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        let json = serde_json::to_string(payload).map_err(|e| e.to_string())?;
        let ledger = serde_json::to_string(queue).map_err(|e| e.to_string())?;
        if ledger.len() > 32 * 1024 * 1024 {
            return Err("Preparation ledger reached its safety limit".into());
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        transaction.execute("INSERT INTO briefings SELECT ?1,COALESCE(MAX(version),0)+1,?2,?3 FROM briefings WHERE meeting_id=?1",
            params![id, json, chrono::Utc::now().to_rfc3339()]).map_err(|e| e.to_string())?;
        let saved: String = transaction
            .query_row(
                "SELECT payload FROM briefings WHERE meeting_id=?1 ORDER BY version DESC LIMIT 1",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if saved != json {
            return Err("Saved briefing read-back did not match".into());
        }
        transaction.execute("INSERT INTO settings VALUES('preparation-v1',?1) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload", [ledger]).map_err(|e| e.to_string())?;
        transaction
            .commit()
            .map_err(|e| format!("Commit prepared briefing: {e}"))
    }
}
