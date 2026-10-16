use crate::calendar::{Calendar, Event, View};
use chrono::NaiveDate;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
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
    pub language: crate::i18n::Language,
    pub hide_unconfirmed: bool,
    pub view: View,
    pub paused: bool,
    pub prepare_enabled: bool,
    pub work_email: String,
    pub github_client_id: String,
    pub daily_ai_limit: u32,
    pub context_folders: Vec<PathBuf>,
    pub repository_collections: Vec<PathBuf>,
    pub cloud_context_folder: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            dark: true,
            accent: 0,
            language: crate::i18n::Language::En,
            hide_unconfirmed: false,
            view: View::WorkWeek,
            paused: false,
            prepare_enabled: true,
            work_email: String::new(),
            github_client_id: String::new(),
            daily_ai_limit: crate::telemetry::DEFAULT_DAILY_LIMIT,
            context_folders: Vec::new(),
            repository_collections: Vec::new(),
            cloud_context_folder: String::new(),
        }
    }
}

impl Preferences {
    pub fn sources(&self) -> crate::source_index::Config {
        crate::source_index::Config {
            folders: self.context_folders.clone(),
            repositories: self.repository_collections.clone(),
            cloud_folder: self.cloud_context_folder.clone(),
        }
    }
}

pub struct Store {
    connection: Connection,
    pub root: PathBuf,
}

impl Store {
    pub fn read_only(root: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(
            root.join("tomlook.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| "The local calendar cache is unavailable")?;
        connection
            .busy_timeout(std::time::Duration::from_millis(100))
            .map_err(|_| "Configure read-only calendar access failed")?;
        connection
            .execute_batch("PRAGMA query_only=ON; BEGIN DEFERRED;")
            .map_err(|_| "Open a consistent calendar snapshot failed")?;
        Ok(Self {
            connection,
            root: root.into(),
        })
    }

    fn read_recommendation_dismissals(connection: &Connection) -> Result<BTreeSet<String>> {
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload FROM settings WHERE key='recommendation-dismissals-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read dismissed recommendations failed")?;
        let keys: BTreeSet<String> = match payload {
            Some(payload) if payload.len() <= 1024 * 1024 => serde_json::from_str(&payload)
                .map_err(|_| "Saved dismissed recommendations are invalid")?,
            Some(_) => return Err("Dismissed recommendations reached their safety limit".into()),
            None => BTreeSet::new(),
        };
        if keys.len() > 10_000
            || keys
                .iter()
                .any(|key| key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err("Saved dismissed recommendations are invalid".into());
        }
        Ok(keys)
    }

    pub fn recommendation_dismissals(&self) -> Result<BTreeSet<String>> {
        Self::read_recommendation_dismissals(&self.connection)
    }

    pub fn recommendations(
        &self,
    ) -> Result<std::collections::BTreeMap<String, crate::recommendations::Recommendation>> {
        use rusqlite::OptionalExtension;
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key='recommendation-inbox-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read recommendation inbox failed")?;
        let mut inbox: std::collections::BTreeMap<String, crate::recommendations::Recommendation> =
            match payload {
                Some(payload) if payload.len() <= 4 * 1024 * 1024 => serde_json::from_str(&payload)
                    .map_err(|_| "Saved recommendation inbox is invalid")?,
                Some(_) => return Err("Saved recommendation inbox exceeds its safety limit".into()),
                None => Default::default(),
            };
        if inbox.len() > 512 {
            return Err("Saved recommendation inbox exceeds its entry limit".into());
        }
        let mut dismissals = self.recommendation_dismissals()?;
        dismissals.extend(
            inbox
                .values()
                .filter(|record| record.state == crate::recommendations::State::Dismissed)
                .filter_map(crate::recommendations::Recommendation::dismissal_key),
        );
        for (id, recommendation) in &mut inbox {
            if recommendation.suggestion.kind != crate::recommendations::Kind::Manual {
                recommendation.suggestion.validate()?;
            } else if recommendation.manual_change.is_none() {
                return Err("Saved manual calendar action has no exact change".into());
            }
            if id != &recommendation.id
                || id.len() != 64
                || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("Saved recommendation identity is invalid".into());
            }
            if recommendation.state == crate::recommendations::State::Dispatching {
                recommendation.state = crate::recommendations::State::Review;
                recommendation.message =
                    crate::workiq::connection::calendar_write::UNCERTAIN.into();
            } else if matches!(
                recommendation.state,
                crate::recommendations::State::Pending | crate::recommendations::State::NotSent
            ) && recommendation
                .dismissal_key()
                .is_some_and(|key| dismissals.contains(&key))
            {
                recommendation.state = crate::recommendations::State::Dismissed;
            }
        }
        Ok(inbox)
    }

    pub fn save_recommendations(
        &self,
        inbox: &std::collections::BTreeMap<String, crate::recommendations::Recommendation>,
    ) -> Result<()> {
        self.save_recommendations_with_dismissals(inbox, &BTreeSet::new())
    }

    pub fn save_recommendations_with_dismissals(
        &self,
        inbox: &std::collections::BTreeMap<String, crate::recommendations::Recommendation>,
        keys: &BTreeSet<String>,
    ) -> Result<()> {
        if keys
            .iter()
            .any(|key| key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err("Dismissed recommendations contain an invalid identity".into());
        }
        let mut next = inbox.clone();
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "Begin recommendation storage failed")?;
        let previous: Option<String> = transaction
            .query_row(
                "SELECT payload FROM settings WHERE key='recommendation-inbox-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read current recommendation states failed")?;
        if let Some(previous) = previous {
            let previous: std::collections::BTreeMap<
                String,
                crate::recommendations::Recommendation,
            > = serde_json::from_str(&previous)
                .map_err(|_| "Current recommendation states are invalid")?;
            for (id, previous) in previous {
                if let Some(next) = next.get_mut(&id) {
                    use crate::recommendations::State;
                    if (previous.state != State::Pending && next.state == State::Pending)
                        || (matches!(previous.state, State::Applied | State::Dismissed)
                            && next.state != State::Applied)
                    {
                        *next = previous;
                    }
                } else if !matches!(
                    previous.state,
                    crate::recommendations::State::Applied
                        | crate::recommendations::State::Dismissed
                        | crate::recommendations::State::Expired
                ) || previous.created_at
                    >= chrono::Local::now().fixed_offset() - chrono::Duration::days(30)
                {
                    next.insert(id, previous);
                }
            }
        }
        let mut dismissals = Self::read_recommendation_dismissals(&transaction)?;
        let previous_dismissals = dismissals.len();
        dismissals.extend(keys.iter().cloned());
        dismissals.extend(
            next.values()
                .filter(|record| record.state == crate::recommendations::State::Dismissed)
                .filter_map(crate::recommendations::Recommendation::dismissal_key),
        );
        if dismissals.len() > 10_000 {
            return Err("Dismissed recommendations reached their safety limit".into());
        }
        if dismissals.len() != previous_dismissals {
            let dismissal_payload = serde_json::to_string(&dismissals)
                .map_err(|_| "Encode dismissed recommendations failed")?;
            transaction
                .execute(
                    "INSERT INTO settings VALUES('recommendation-dismissals-v1',?1)
                 ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                    [dismissal_payload],
                )
                .map_err(|_| "Persist dismissed recommendations failed")?;
        }
        let payload =
            serde_json::to_string(&next).map_err(|_| "Encode recommendation inbox failed")?;
        if next.len() > 512 || payload.len() > 4 * 1024 * 1024 {
            return Err("Recommendation inbox reached its safety limit".into());
        }
        transaction
            .execute(
                "INSERT INTO settings VALUES('recommendation-inbox-v1',?1)
            ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                [payload],
            )
            .map_err(|_| "Persist recommendation inbox failed")?;
        transaction
            .commit()
            .map_err(|_| "Commit recommendation inbox failed")?;
        Ok(())
    }

    pub fn approve_recommendation(
        &mut self,
        operation: &crate::recommendations::Operation,
    ) -> Result<()> {
        operation.validate(&operation.recommendation.account)?;
        if self.calendar_owner()?.as_deref() != Some(operation.recommendation.account.as_str()) {
            return Err("Calendar account changed before confirmation".into());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "Lock calendar approval failed")?;
        let previous: String = transaction
            .query_row(
                "SELECT payload FROM settings WHERE key='recommendation-inbox-v1'",
                [],
                |row| row.get(0),
            )
            .map_err(|_| "Read persisted calendar proposal failed")?;
        let mut inbox: std::collections::BTreeMap<String, crate::recommendations::Recommendation> =
            serde_json::from_str(&previous).map_err(|_| "Persisted calendar proposal is invalid")?;
        let current = inbox
            .get_mut(&operation.recommendation.id)
            .ok_or("Calendar proposal no longer exists")?;
        if !matches!(
            current.state,
            crate::recommendations::State::Pending | crate::recommendations::State::NotSent
        ) || current.account != operation.recommendation.account
            || current.fingerprint != operation.recommendation.fingerprint
            || current.suggestion != operation.recommendation.suggestion
            || current.manual_change != operation.recommendation.manual_change
        {
            return Err("Calendar approval was already consumed or its proposal changed".into());
        }
        *current = operation.recommendation.clone();
        let payload = serde_json::to_string(&inbox)
            .map_err(|_| "Encode confirmed calendar proposal failed")?;
        transaction
            .execute(
                "UPDATE settings SET payload=?1 WHERE key='recommendation-inbox-v1'",
                [payload],
            )
            .map_err(|_| "Persist calendar confirmation failed")?;
        transaction.execute(
            "INSERT INTO settings VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
            params![format!("calendar-approval-{}", operation.recommendation.id),
                serde_json::to_string(operation).map_err(|_| "Encode calendar approval proof failed")?],
        ).map_err(|_| "Persist exact calendar approval failed")?;
        transaction
            .commit()
            .map_err(|_| "Commit calendar confirmation failed")?;
        Ok(())
    }

    pub fn save_calendar_event(&mut self, event: &Event, account: &str) -> Result<Calendar> {
        if self.calendar_owner()?.as_deref() != Some(account) {
            return Err("Calendar account changed before local read-back storage".into());
        }

        event.validate()?;
        let mut calendar = self.calendar()?;
        calendar.events.retain(|previous| previous.id != event.id);
        calendar.events.push(event.clone());
        let calendar = Calendar::build(calendar.events, calendar.coverage, calendar.warnings)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "Begin calendar read-back storage failed")?;
        Self::write_calendar(&transaction, &calendar)?;
        transaction
            .commit()
            .map_err(|_| "Commit calendar read-back storage failed")?;
        Ok(calendar)
    }

    pub fn calendar_approval(&self, id: &str) -> Result<crate::recommendations::Operation> {
        if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid calendar approval identity".into());
        }
        let payload: String = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key=?1",
                [format!("calendar-approval-{id}")],
                |row| row.get(0),
            )
            .map_err(|_| "Read exact calendar approval failed")?;
        let operation: crate::recommendations::Operation =
            serde_json::from_str(&payload).map_err(|_| "Saved calendar approval is invalid")?;
        if operation.recommendation.id != id {
            return Err("Saved calendar approval is invalid".into());
        }
        Ok(operation)
    }

    pub fn remove_calendar_event(&mut self, id: &str, account: &str) -> Result<Calendar> {
        if self.calendar_owner()?.as_deref() != Some(account) {
            return Err("Calendar account changed before local read-back storage".into());
        }
        let mut calendar = self.calendar()?;
        calendar.events.retain(|event| event.id != id);
        let calendar = Calendar::build(calendar.events, calendar.coverage, calendar.warnings)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "Begin calendar read-back storage failed")?;
        Self::write_calendar(&transaction, &calendar)?;
        transaction
            .commit()
            .map_err(|_| "Commit calendar read-back storage failed")?;
        Ok(calendar)
    }

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
                 CREATE TABLE IF NOT EXISTS calendar_sync(day TEXT PRIMARY KEY,refreshed_at TEXT NOT NULL);
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

    pub fn workiq_identity(&self) -> Result<Option<crate::workiq::connection::Identity>> {
        use rusqlite::OptionalExtension;
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key='workiq_identity'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        payload
            .map(|json| {
                let identity: crate::workiq::connection::Identity = serde_json::from_str(&json)
                    .map_err(|_| "Saved WorkIQ identity is invalid".to_string())?;
                identity.validate()?;
                Ok(identity)
            })
            .transpose()
    }

    pub fn save_workiq_identity(
        &self,
        identity: &crate::workiq::connection::Identity,
    ) -> Result<()> {
        identity.validate()?;
        let json = serde_json::to_string(identity).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO settings VALUES('workiq_identity',?1)
             ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                [json],
            )
            .map_err(|error| error.to_string())?;
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
        transaction
            .execute("DELETE FROM calendar_sync", [])
            .map_err(|e| e.to_string())?;
        transaction
            .execute("DELETE FROM settings WHERE key='calendar_owner'", [])
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    }

    pub fn calendar_owner(&self) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row(
                "SELECT payload FROM settings WHERE key='calendar_owner'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn save_workiq_calendar(&mut self, next: &Calendar, account: &str) -> Result<Calendar> {
        let owner = self.calendar_owner()?;
        let current = if owner.as_deref() == Some(account) {
            self.calendar()?
        } else {
            Calendar::default()
        };
        let (Some(first), Some(last)) = (next.coverage.first(), next.coverage.last()) else {
            return Err("WorkIQ calendar snapshot has no verified coverage".into());
        };
        if (0..=(*last - *first).num_days()).any(|day| {
            !next
                .coverage
                .contains(&(*first + chrono::Duration::days(day)))
        }) {
            return Err("WorkIQ calendar snapshot coverage is not contiguous".into());
        }
        let ids = next
            .events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<BTreeSet<_>>();
        let events = current
            .events
            .into_iter()
            .filter(|event| {
                let start = event.start.with_timezone(&chrono::Local).date_naive();
                let end = (event.end - chrono::Duration::milliseconds(1))
                    .with_timezone(&chrono::Local)
                    .date_naive();
                (end < *first || start > *last) && !ids.contains(event.id.as_str())
            })
            .chain(next.events.iter().cloned())
            .collect();
        let coverage = current.coverage.union(&next.coverage).copied().collect();
        let calendar = Calendar::build(events, coverage, Vec::new())?;
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        Self::write_calendar(&transaction, &calendar)?;
        if owner.as_deref() != Some(account) {
            transaction
                .execute("DELETE FROM calendar_sync", [])
                .map_err(|e| e.to_string())?;
        }
        let refreshed = chrono::Utc::now().to_rfc3339();
        for day in &next.coverage {
            transaction.execute(
                "INSERT INTO calendar_sync VALUES(?1,?2) ON CONFLICT(day) DO UPDATE SET refreshed_at=excluded.refreshed_at",
                params![day.to_string(), refreshed],
            ).map_err(|e| e.to_string())?;
        }
        transaction.execute(
            "INSERT INTO settings VALUES('calendar_owner',?1) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
            [account],
        ).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(calendar)
    }

    pub fn calendar_sync_days(&self) -> Result<crate::calendar_sync::Freshness> {
        let mut statement = self
            .connection
            .prepare("SELECT day,refreshed_at FROM calendar_sync")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut days = crate::calendar_sync::Freshness::new();
        for row in rows {
            let (day, refreshed) = row.map_err(|e| e.to_string())?;
            days.insert(
                day.parse()
                    .map_err(|e| format!("Invalid synchronized day: {e}"))?,
                refreshed
                    .parse()
                    .map_err(|e| format!("Invalid calendar refresh timestamp: {e}"))?,
            );
        }
        Ok(days)
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
