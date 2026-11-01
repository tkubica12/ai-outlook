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

const FEEDBACK_LIMIT: u32 = 100_000;

fn feedback_time(time: chrono::DateTime<chrono::Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn feedback_payload(payload: &str) -> Result<crate::feedback::Entry> {
    if payload.len() > 16 * 1024 {
        return Err("Saved feedback exceeds its safety limit".into());
    }
    let entry: crate::feedback::Entry =
        serde_json::from_str(payload).map_err(|_| "Saved feedback is invalid")?;
    entry.validate().map_err(|_| "Saved feedback is invalid")?;
    Ok(entry)
}

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
    pub prompts: crate::prompts::Prompts,
    pub automatic_update_check: bool,
    pub update_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub update_latest_version: String,
    pub meeting_reminders: bool,
    pub reminder_minutes: u8,
    pub remind_tentative: bool,
    /// Morning "My day" notification.
    pub morning_overview: bool,
    /// Minutes after local midnight of the morning overview (07:00–10:00 in 30-minute steps).
    pub morning_overview_time: u16,
    pub morning_overview_weekends: bool,
    /// Weekly review notification.
    pub weekly_review: bool,
    /// ISO weekday of the weekly review notification: 5 (Friday, this week) or 1 (Monday, the
    /// previous week).
    pub weekly_review_day: u8,
    /// Minutes after local midnight of the weekly review notification.
    pub weekly_review_time: u16,
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
            prompts: crate::prompts::Prompts::default(),
            automatic_update_check: true,
            update_checked_at: None,
            update_latest_version: String::new(),
            meeting_reminders: true,
            reminder_minutes: crate::reminders::DEFAULT_MINUTES,
            remind_tentative: false,
            morning_overview: true,
            morning_overview_time: crate::myday::DEFAULT_TIME,
            morning_overview_weekends: false,
            weekly_review: true,
            weekly_review_day: crate::weekly::DEFAULT_DAY,
            weekly_review_time: crate::weekly::DEFAULT_TIME,
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
    pub fn recaps(&self) -> Result<crate::recaps::Ledger> {
        let raw: Option<String> = self.connection.query_row(
            "SELECT payload FROM settings WHERE key='meeting-recaps-v1'", [], |row| row.get(0))
            .optional().map_err(|_| "Read meeting recap ledger failed")?;
        let mut ledger: crate::recaps::Ledger = match raw {
            Some(raw) if raw.len() <= 32 * 1024 * 1024 => serde_json::from_str(&raw)
                .map_err(|_| "Saved meeting recap ledger is invalid")?,
            Some(_) => return Err("Meeting recap ledger exceeds its safety limit".into()),
            None => Default::default(),
        };
        ledger.recover(chrono::Local::now().fixed_offset())?;
        Ok(ledger)
    }

    pub fn save_recaps(&self, ledger: &crate::recaps::Ledger) -> Result<()> {
        let raw = serde_json::to_string(ledger).map_err(|_| "Encode meeting recap ledger failed")?;
        if raw.len() > 32 * 1024 * 1024 || ledger.records.len() > 10_000 {
            return Err("Meeting recap ledger exceeds its safety limit; saved notes were not discarded".into());
        }
        self.connection.execute(
            "INSERT INTO settings VALUES('meeting-recaps-v1',?1) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
            [raw]).map_err(|_| "Save meeting recap ledger failed")?;
        Ok(())
    }

    pub fn event_creations(&self) -> Result<std::collections::BTreeMap<String, crate::event_creation::Record>> {
        let mut query = self.connection.prepare(
            "SELECT payload FROM settings WHERE key LIKE 'event-creation-%' ORDER BY rowid DESC LIMIT 513"
        ).map_err(|_| "Read event creation history failed")?;
        let rows = query.query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| "Read event creation history failed")?;
        let mut records = std::collections::BTreeMap::new();
        for row in rows {
            let payload = row.map_err(|_| "Read event creation history failed")?;
            if payload.len() > 64 * 1024 { return Err("Saved event creation exceeds its safety limit".into()); }
            let mut record: crate::event_creation::Record = serde_json::from_str(&payload)
                .map_err(|_| "Saved event creation is invalid")?;
            record.validate()?;
            if record.state == crate::event_creation::State::Creating {
                record.state = crate::event_creation::State::Review;
                record.message = crate::workiq::connection::calendar_write::UNCERTAIN.into();
            }
            records.insert(record.id.clone(), record);
        }
        if records.len() > 512 { return Err("Event creation history reached its safety limit".into()); }
        Ok(records)
    }

    pub fn approve_event_creation(&self, record: &crate::event_creation::Record) -> Result<()> {
        record.validate()?;
        if record.state != crate::event_creation::State::Creating
            || self.calendar_owner()?.as_deref() != Some(record.account.as_str()) {
            return Err("Calendar account or event creation approval changed".into());
        }
        let payload = serde_json::to_string(record).map_err(|_| "Encode event creation approval failed")?;
        if payload.len() > 64 * 1024 { return Err("Saved event creation exceeds its safety limit".into()); }
        self.connection.execute(
            "INSERT INTO settings(key,payload) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET payload=excluded.payload
             WHERE json_extract(settings.payload,'$.state')='NotSent'",
            params![format!("event-creation-{}", record.id),payload])
            .and_then(|count| if count == 1 { Ok(()) } else { Err(rusqlite::Error::InvalidQuery) })
            .map_err(|_| "Event creation approval was already consumed or could not be saved")?;
        Ok(())
    }

    pub fn save_event_creation(&self, record: &crate::event_creation::Record) -> Result<()> {
        let payload = serde_json::to_string(record).map_err(|_| "Encode event creation result failed")?;
        if self.connection.execute("UPDATE settings SET payload=?1 WHERE key=?2",
            params![payload,format!("event-creation-{}", record.id)])
            .map_err(|_| "Persist event creation result failed")? != 1 {
            return Err("Event creation approval no longer exists".into());
        }
        Ok(())
    }

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
                   PRIMARY KEY(source,identity));
                 CREATE TABLE IF NOT EXISTS ai_feedback(
                   account TEXT NOT NULL,document TEXT NOT NULL,document_key TEXT NOT NULL,
                   section TEXT NOT NULL,revision TEXT NOT NULL,prompt_kind TEXT NOT NULL,
                   updated_at TEXT NOT NULL,payload TEXT NOT NULL,
                   PRIMARY KEY(account,document,document_key,section,revision));
                 CREATE INDEX IF NOT EXISTS ai_feedback_recent ON ai_feedback(account,updated_at);
                 CREATE TABLE IF NOT EXISTS tasks(
                   id INTEGER PRIMARY KEY AUTOINCREMENT,account TEXT NOT NULL,status TEXT NOT NULL,
                   meeting_key TEXT,meeting_event TEXT,import_key TEXT UNIQUE,
                   updated_at TEXT NOT NULL,payload TEXT NOT NULL);
                 CREATE INDEX IF NOT EXISTS tasks_by_account ON tasks(account,status);
                 CREATE INDEX IF NOT EXISTS tasks_by_meeting ON tasks(account,meeting_key);
                 CREATE TABLE IF NOT EXISTS task_tombstones(
                   account TEXT NOT NULL,import_key TEXT NOT NULL,meeting_key TEXT NOT NULL,
                   meeting_event TEXT NOT NULL,hashes TEXT NOT NULL,created_at TEXT NOT NULL,
                   PRIMARY KEY(account,import_key));
                 CREATE TABLE IF NOT EXISTS followup_drafts(
                   account TEXT NOT NULL,meeting_key TEXT NOT NULL,updated_at TEXT NOT NULL,
                   payload TEXT NOT NULL,PRIMARY KEY(account,meeting_key));
                 CREATE TABLE IF NOT EXISTS weekly_summaries(
                   account TEXT NOT NULL,week TEXT NOT NULL,updated_at TEXT NOT NULL,
                   payload TEXT NOT NULL,PRIMARY KEY(account,week));",
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
        preferences.prompts.validate()?;
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

    /// Saves or replaces the rating of one displayed output revision; the first rating time is kept.
    pub fn save_feedback(&self, entry: &crate::feedback::Entry) -> Result<crate::feedback::Entry> {
        entry.validate()?;
        let key = &entry.key;
        let document = crate::feedback::document_code(key.document.document);
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "Begin feedback storage failed")?;
        let previous: Option<String> = transaction
            .query_row(
                "SELECT payload FROM ai_feedback WHERE account=?1 AND document=?2
                 AND document_key=?3 AND section=?4 AND revision=?5",
                params![key.document.account, document, key.document.key, key.section, key.revision],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read saved feedback failed")?;
        let mut saved = entry.clone();
        match previous {
            Some(previous) => {
                saved.created_at = feedback_payload(&previous)?.created_at.min(saved.updated_at);
            }
            None => {
                let count: u32 = transaction
                    .query_row("SELECT count(*) FROM ai_feedback", [], |row| row.get(0))
                    .map_err(|_| "Read saved feedback failed")?;
                if count >= FEEDBACK_LIMIT {
                    return Err("Saved feedback reached its safety limit; nothing was saved".into());
                }
            }
        }
        let payload = serde_json::to_string(&saved).map_err(|_| "Encode feedback failed")?;
        transaction
            .execute(
                "INSERT INTO ai_feedback(account,document,document_key,section,revision,prompt_kind,updated_at,payload)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(account,document,document_key,section,revision) DO UPDATE SET
                   prompt_kind=excluded.prompt_kind,updated_at=excluded.updated_at,payload=excluded.payload",
                params![
                    key.document.account,
                    document,
                    key.document.key,
                    key.section,
                    key.revision,
                    crate::feedback::kind_code(saved.prompt_kind),
                    feedback_time(saved.updated_at),
                    payload
                ],
            )
            .map_err(|_| "Save feedback failed")?;
        transaction.commit().map_err(|_| "Commit feedback failed")?;
        Ok(saved)
    }

    /// Removes the rating of one output revision; returns whether a rating existed.
    pub fn clear_feedback(&self, key: &crate::feedback::Key) -> Result<bool> {
        self.connection
            .execute(
                "DELETE FROM ai_feedback WHERE account=?1 AND document=?2
                 AND document_key=?3 AND section=?4 AND revision=?5",
                params![
                    key.document.account,
                    crate::feedback::document_code(key.document.document),
                    key.document.key,
                    key.section,
                    key.revision
                ],
            )
            .map(|count| count > 0)
            .map_err(|_| "Clear feedback failed".into())
    }

    /// Every saved rating of one document, across all of its output revisions.
    pub fn feedback_for_document(
        &self,
        id: &crate::feedback::DocumentId,
    ) -> Result<Vec<crate::feedback::Entry>> {
        let entries = self.feedback_rows(
            "SELECT payload FROM ai_feedback WHERE account=?1 AND document=?2 AND document_key=?3
             ORDER BY updated_at DESC LIMIT 512",
            params![id.account, crate::feedback::document_code(id.document), id.key],
        )?;
        if entries.iter().any(|entry| entry.key.document != *id) {
            return Err("Saved feedback does not match its document".into());
        }
        Ok(entries)
    }

    /// An account's ratings updated since `since`, newest first and bounded.
    pub fn feedback_since(
        &self,
        account: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<crate::feedback::Entry>> {
        let entries = self.feedback_rows(
            "SELECT payload FROM ai_feedback WHERE account=?1 AND updated_at>=?2
             ORDER BY updated_at DESC LIMIT 10000",
            params![account, feedback_time(since)],
        )?;
        if entries.iter().any(|entry| entry.key.document.account != account) {
            return Err("Saved feedback does not match its account".into());
        }
        Ok(entries)
    }

    fn feedback_rows(
        &self,
        sql: &str,
        parameters: impl rusqlite::Params,
    ) -> Result<Vec<crate::feedback::Entry>> {
        let mut statement = self
            .connection
            .prepare(sql)
            .map_err(|_| "Read saved feedback failed")?;
        let rows = statement
            .query_map(parameters, |row| row.get::<_, String>(0))
            .map_err(|_| "Read saved feedback failed")?;
        rows.map(|row| {
            row.map_err(|_| "Read saved feedback failed".to_owned())
                .and_then(|payload| feedback_payload(&payload))
        })
        .collect()
    }

    /// Occurrence keys of meeting reminders already shown, with each meeting's start.
    pub fn fired_reminders(
        &self,
    ) -> Result<std::collections::BTreeMap<String, chrono::DateTime<chrono::Utc>>> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key='meeting-reminders-fired-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read meeting reminder history failed")?;
        let fired: std::collections::BTreeMap<String, chrono::DateTime<chrono::Utc>> =
            match payload {
                Some(payload) if payload.len() <= 1024 * 1024 => serde_json::from_str(&payload)
                    .map_err(|_| "Saved meeting reminder history is invalid")?,
                Some(_) => return Err("Meeting reminder history exceeds its safety limit".into()),
                None => Default::default(),
            };
        if fired.len() > crate::reminders::FIRED_LIMIT
            || fired
                .keys()
                .any(|key| key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err("Saved meeting reminder history is invalid".into());
        }
        Ok(fired)
    }

    pub fn save_fired_reminders(
        &self,
        fired: &std::collections::BTreeMap<String, chrono::DateTime<chrono::Utc>>,
    ) -> Result<()> {
        if fired.len() > crate::reminders::FIRED_LIMIT {
            return Err("Meeting reminder history exceeds its safety limit".into());
        }
        let json = serde_json::to_string(fired)
            .map_err(|_| "Encode meeting reminder history failed")?;
        self.connection
            .execute(
                "INSERT INTO settings VALUES('meeting-reminders-fired-v1',?1)
                 ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                [json],
            )
            .map_err(|_| "Save meeting reminder history failed")?;
        Ok(())
    }

    /// Local day the morning overview was last shown.
    pub fn morning_overview_shown(&self) -> Result<Option<NaiveDate>> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key='morning-overview-shown-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read morning overview history failed")?;
        payload
            .map(|value| {
                NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                    .map_err(|_| "Saved morning overview history is invalid".to_string())
            })
            .transpose()
    }

    pub fn save_morning_overview_shown(&self, date: NaiveDate) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO settings VALUES('morning-overview-shown-v1',?1)
                 ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                [date.format("%Y-%m-%d").to_string()],
            )
            .map_err(|_| "Save morning overview history failed")?;
        Ok(())
    }

    /// Local day the weekly review notification was last shown.
    pub fn weekly_review_shown(&self) -> Result<Option<NaiveDate>> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key='weekly-review-shown-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read weekly review history failed")?;
        payload
            .map(|value| {
                NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                    .map_err(|_| "Saved weekly review history is invalid".to_string())
            })
            .transpose()
    }

    pub fn save_weekly_review_shown(&self, date: NaiveDate) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO settings VALUES('weekly-review-shown-v1',?1)
                 ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                [date.format("%Y-%m-%d").to_string()],
            )
            .map_err(|_| "Save weekly review history failed")?;
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
        if let Some(owner) = owner.as_deref().filter(|owner| *owner != account) {
            transaction.execute(
                "UPDATE briefings SET payload=json_set(payload,'$.tomlook_account',?1)
                 WHERE json_valid(payload) AND json_extract(payload,'$.tomlook_account') IS NULL",
                [owner]).map_err(|_| "Bind existing Insights to their calendar account failed")?;
        }
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

    pub fn document(
        &self,
        account: &str,
        event: &Event,
        document: crate::changes::Document,
    ) -> Result<Option<serde_json::Value>> {
        if self.calendar_owner()?.as_deref() != Some(account) {
            return Err("Saved document belongs to a different calendar account".into());
        }
        let value = match document {
            crate::changes::Document::Insights => self.briefing(&event.id)?
                .map(|raw| serde_json::from_str(&raw).map_err(|_| "Saved Insights are invalid"))
                .transpose()?,
            crate::changes::Document::Notes => self.recaps()?.record(account, event)
                .filter(|record| record.account == account).and_then(|record| record.payload.clone()),
        };
        if let Some(value) = &value { crate::changes::revision(value)?; }
        Ok(value)
    }

    pub fn retain_insight_edits(&self, id: &str, payload: &mut serde_json::Value) -> Result<()> {
        if let Some(raw) = self.briefing(id)? {
            let previous = serde_json::from_str(&raw).map_err(|_| "Saved Insights are invalid")?;
            crate::changes::retain_approved(&previous, payload)?;
        }
        Ok(())
    }

    pub fn apply_document_recommendation(
        &self,
        proposal: &crate::recommendations::Recommendation,
        event: &Event,
    ) -> Result<std::collections::BTreeMap<String, crate::recommendations::Recommendation>> {
        use crate::recommendations::{Change, Kind, State};
        let Some(Change::Document(edit)) = &proposal.manual_change else {
            return Err("This approval is not a saved-document edit".into());
        };
        let transaction = self.connection.unchecked_transaction()
            .map_err(|_| "Begin document approval failed")?;
        if self.calendar_owner()?.as_deref() != Some(proposal.account.as_str())
            || proposal.suggestion.kind != Kind::Manual || event.id != proposal.event_id {
            return Err("Document approval no longer matches its calendar account".into());
        }
        let calendar = self.calendar()?;
        let current = proposal.current(&calendar, Some(&proposal.account), chrono::Local::now().fixed_offset())
            .ok_or("Meeting changed after this proposal; nothing was saved")?;
        if crate::scheduler::fingerprint(event) != crate::scheduler::fingerprint(current) {
            return Err("Meeting changed after confirmation; nothing was saved".into());
        }
        let raw: String = transaction.query_row(
            "SELECT payload FROM settings WHERE key='recommendation-inbox-v1'", [], |row| row.get(0))
            .map_err(|_| "Read saved document proposal failed")?;
        if raw.len() > 4 * 1024 * 1024 { return Err("Saved proposal inbox exceeds its safety limit".into()); }
        let mut inbox: std::collections::BTreeMap<String, crate::recommendations::Recommendation> =
            serde_json::from_str(&raw).map_err(|_| "Saved proposal inbox is invalid")?;
        let saved = inbox.get_mut(&proposal.id).ok_or("Document proposal no longer exists")?;
        if saved.state != State::Pending || saved.account != proposal.account
            || saved.fingerprint != proposal.fingerprint || saved.manual_change != proposal.manual_change {
            return Err("Document approval was already consumed or changed; nothing was saved".into());
        }
        let original = self.document(&proposal.account, current, edit.document)?
            .ok_or("The saved document no longer exists")?;
        let mut next = edit.apply(&original, &proposal.id)?;
        next["tomlook_account"] = proposal.account.clone().into();
        crate::changes::revision(&next)?;
        let json = serde_json::to_string(&next).map_err(|_| "Encode edited document failed")?;
        match edit.document {
            crate::changes::Document::Insights => {
                transaction.execute(
                    "INSERT INTO briefings SELECT ?1,COALESCE(MAX(version),0)+1,?2,?3 FROM briefings WHERE meeting_id=?1",
                    params![event.id, json, chrono::Utc::now().to_rfc3339()])
                    .map_err(|_| "Save approved Insights failed")?;
            }
            crate::changes::Document::Notes => {
                let raw: String = transaction.query_row(
                    "SELECT payload FROM settings WHERE key='meeting-recaps-v1'", [], |row| row.get(0))
                    .map_err(|_| "Read saved notes ledger failed")?;
                if raw.len() > 32 * 1024 * 1024 { return Err("Saved notes ledger exceeds its safety limit".into()); }
                let mut ledger: crate::recaps::Ledger = serde_json::from_str(&raw)
                    .map_err(|_| "Saved notes ledger is invalid")?;
                let record = ledger.record_mut(&proposal.account, current)
                    .ok_or("Saved meeting notes no longer exist")?;
                if record.checking {
                    return Err("Meeting notes are being prepared; wait before approving their edit".into());
                }
                record.payload = Some(next.clone());
                record.message = "Approved meeting notes saved".into();
                let json = serde_json::to_string(&ledger).map_err(|_| "Encode edited notes ledger failed")?;
                if json.len() > 32 * 1024 * 1024 {
                    return Err("Meeting notes ledger exceeds its safety limit".into());
                }
                transaction.execute("UPDATE settings SET payload=?1 WHERE key='meeting-recaps-v1'", [json])
                    .map_err(|_| "Save approved meeting notes failed")?;
            }
        }
        if self.document(&proposal.account, current, edit.document)?.as_ref() != Some(&next) {
            return Err("Edited document read-back did not match; nothing was saved".into());
        }
        saved.state = State::Applied;
        saved.message = "Approved edits saved; changed sections are preserved during future AI updates".into();
        for record in inbox.values_mut().filter(|record| record.account == proposal.account
            && record.event_id == event.id && record.suggestion.kind != Kind::Manual && record.state == State::Pending) {
            record.state = State::Expired;
            record.message = "Insights changed after this recommendation; review the updated meeting".into();
        }
        let json = serde_json::to_string(&inbox).map_err(|_| "Encode consumed document approval failed")?;
        if json.len() > 4 * 1024 * 1024 { return Err("Proposal inbox exceeds its safety limit".into()); }
        transaction.execute("UPDATE settings SET payload=?1 WHERE key='recommendation-inbox-v1'", [&json])
            .map_err(|_| "Save consumed document approval failed")?;
        let saved: String = transaction.query_row(
            "SELECT payload FROM settings WHERE key='recommendation-inbox-v1'", [], |row| row.get(0))
            .map_err(|_| "Verify consumed document approval failed")?;
        if saved != json { return Err("Document approval read-back did not match".into()); }
        transaction.commit().map_err(|_| "Commit document edit and approval failed")?;
        Ok(inbox)
    }

    pub fn briefing(&self, id: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row(
                "SELECT payload FROM briefings WHERE meeting_id=?1
                 AND CASE WHEN json_valid(payload) THEN
                    (json_extract(payload,'$.tomlook_account') IS NULL OR json_extract(payload,'$.tomlook_account')=?2)
                    ELSE 1 END
                 ORDER BY version DESC LIMIT 1",
                params![id, self.calendar_owner()?],
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
        let mut payload = payload.clone();
        let account = self.calendar_owner()?;
        if let Some(account) = &account {
            payload["tomlook_account"] = account.clone().into();
        }
        let ledger = serde_json::to_string(queue).map_err(|e| e.to_string())?;
        if ledger.len() > 32 * 1024 * 1024 {
            return Err("Preparation ledger reached its safety limit".into());
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let previous: Option<String> = transaction.query_row(
            "SELECT payload FROM briefings WHERE meeting_id=?1
             AND CASE WHEN json_valid(payload) THEN
                (json_extract(payload,'$.tomlook_account') IS NULL OR json_extract(payload,'$.tomlook_account')=?2)
                ELSE 1 END
             ORDER BY version DESC LIMIT 1",
            params![id, account], |row| row.get(0)).optional().map_err(|_| "Read approved sections before preparation failed")?;
        if let Some(previous) = previous {
            let previous = serde_json::from_str(&previous).map_err(|_| "Saved Insights are invalid")?;
            crate::changes::retain_approved(&previous, &mut payload)?;
        }
        let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
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

/// Local tasks and follow-up e-mail drafts: indexed columns plus a validated payload per row.
impl Store {
    fn task_row(id: i64, account: &str, payload: &str) -> Result<crate::tasks::Task> {
        if payload.len() > 64 * 1024 {
            return Err("Saved task exceeds its safety limit".into());
        }
        let mut task: crate::tasks::Task =
            serde_json::from_str(payload).map_err(|_| "Saved task is invalid")?;
        task.id = id;
        if task.account != account {
            return Err("Saved task does not match its account".into());
        }
        task.validate().map_err(|_| "Saved task is invalid")?;
        Ok(task)
    }

    fn task_payload(task: &crate::tasks::Task) -> Result<String> {
        task.validate()?;
        let payload = serde_json::to_string(task).map_err(|_| "Encode task failed")?;
        if payload.len() > 64 * 1024 {
            return Err("Task exceeds its safety limit".into());
        }
        Ok(payload)
    }

    /// Every saved task of an account, oldest first.
    pub fn tasks(&self, account: &str) -> Result<Vec<crate::tasks::Task>> {
        let mut statement = self
            .connection
            .prepare("SELECT id,payload FROM tasks WHERE account=?1 ORDER BY id LIMIT ?2")
            .map_err(|_| "Read tasks failed")?;
        let rows = statement
            .query_map(
                params![account, crate::tasks::TASK_LIMIT as i64 + 1],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|_| "Read tasks failed")?;
        let mut tasks = Vec::new();
        for row in rows {
            let (id, payload) = row.map_err(|_| "Read tasks failed")?;
            tasks.push(Self::task_row(id, account, &payload)?);
        }
        if tasks.len() > crate::tasks::TASK_LIMIT {
            return Err("Saved tasks exceed their safety limit".into());
        }
        Ok(tasks)
    }

    fn insert_task(
        connection: &Connection,
        task: &crate::tasks::Task,
        id: Option<i64>,
    ) -> Result<i64> {
        let payload = Self::task_payload(task)?;
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM tasks WHERE account=?1",
                [&task.account],
                |row| row.get(0),
            )
            .map_err(|_| "Read tasks failed")?;
        if count >= crate::tasks::TASK_LIMIT as i64 {
            return Err(
                "The task list reached its limit of 5,000 tasks; delete completed tasks first"
                    .into(),
            );
        }
        connection
            .execute(
                "INSERT INTO tasks(id,account,status,meeting_key,meeting_event,import_key,updated_at,payload)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    id,
                    task.account,
                    task.status.code(),
                    task.meeting.as_ref().map(|meeting| &meeting.key),
                    task.meeting.as_ref().map(|meeting| &meeting.event_id),
                    task.import_key,
                    feedback_time(task.updated_at),
                    payload
                ],
            )
            .map_err(|_| "Save task failed")?;
        Ok(connection.last_insert_rowid())
    }

    /// Saves a new manual or Ask task and returns it with its identity.
    pub fn add_task(&self, task: &crate::tasks::Task) -> Result<crate::tasks::Task> {
        if task.import_key.is_some() {
            return Err("Tasks from meeting notes are imported, not added".into());
        }
        let id = Self::insert_task(&self.connection, task, None)?;
        Ok(crate::tasks::Task {
            id,
            ..task.clone()
        })
    }

    /// Adds tasks imported from meeting notes, skipping any item whose identity or text (in either
    /// language, from any revision of the same meeting's notes) already exists as a task or as a
    /// tombstone of a deleted task. Returns how many tasks were added.
    pub fn import_tasks(&self, account: &str, tasks: &[crate::tasks::Task]) -> Result<usize> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "Begin task import failed")?;
        let mut known = std::collections::BTreeMap::<(String, String), BTreeSet<String>>::new();
        let mut added = 0;
        for task in tasks {
            let (Some(key), Some(meeting)) = (&task.import_key, &task.meeting) else {
                return Err("Imported task has no source identity".into());
            };
            if task.account != account {
                return Err("Imported task belongs to a different account".into());
            }
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE import_key=?1)
                       OR EXISTS(SELECT 1 FROM task_tombstones WHERE account=?2 AND import_key=?1)",
                    params![key, account],
                    |row| row.get(0),
                )
                .map_err(|_| "Read imported tasks failed")?;
            if exists {
                continue;
            }
            let identity = (meeting.key.clone(), meeting.event_id.clone());
            if !known.contains_key(&identity) {
                let mut hashes = BTreeSet::new();
                let mut statement = transaction
                    .prepare(
                        "SELECT id,payload FROM tasks WHERE account=?1 AND (meeting_key=?2 OR meeting_event=?3)",
                    )
                    .map_err(|_| "Read imported tasks failed")?;
                let rows = statement
                    .query_map(params![account, meeting.key, meeting.event_id], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|_| "Read imported tasks failed")?;
                for row in rows {
                    let (id, payload) = row.map_err(|_| "Read imported tasks failed")?;
                    hashes.extend(Self::task_row(id, account, &payload)?.hashes);
                }
                let mut statement = transaction
                    .prepare(
                        "SELECT hashes FROM task_tombstones WHERE account=?1 AND (meeting_key=?2 OR meeting_event=?3)",
                    )
                    .map_err(|_| "Read deleted tasks failed")?;
                let rows = statement
                    .query_map(params![account, meeting.key, meeting.event_id], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|_| "Read deleted tasks failed")?;
                for row in rows {
                    let saved: Vec<String> =
                        serde_json::from_str(&row.map_err(|_| "Read deleted tasks failed")?)
                            .map_err(|_| "Saved deleted task is invalid")?;
                    hashes.extend(saved);
                }
                known.insert(identity.clone(), hashes);
            }
            let hashes = known.get_mut(&identity).expect("hashes loaded");
            if task.hashes.iter().any(|hash| hashes.contains(hash)) {
                continue;
            }
            Self::insert_task(&transaction, task, None)?;
            hashes.extend(task.hashes.iter().cloned());
            added += 1;
        }
        transaction
            .commit()
            .map_err(|_| "Commit imported tasks failed")?;
        Ok(added)
    }

    /// Replaces an existing task's editable fields; its source and identity cannot change.
    pub fn update_task(&self, task: &crate::tasks::Task) -> Result<()> {
        let payload = Self::task_payload(task)?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "Begin task update failed")?;
        let current: String = transaction
            .query_row(
                "SELECT payload FROM tasks WHERE id=?1 AND account=?2",
                params![task.id, task.account],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read task failed")?
            .ok_or("Task no longer exists")?;
        let current = Self::task_row(task.id, &task.account, &current)?;
        if current.source != task.source
            || current.import_key != task.import_key
            || current.meeting != task.meeting
            || current.created_at != task.created_at
            || current.hashes != task.hashes
        {
            return Err("A task's source cannot be changed".into());
        }
        transaction
            .execute(
                "UPDATE tasks SET status=?1,updated_at=?2,payload=?3 WHERE id=?4 AND account=?5",
                params![
                    task.status.code(),
                    feedback_time(task.updated_at),
                    payload,
                    task.id,
                    task.account
                ],
            )
            .map_err(|_| "Save task failed")?;
        transaction.commit().map_err(|_| "Commit task failed".into())
    }

    /// Removes a task and returns it for undo. Deleted imports leave a tombstone so the same
    /// item is never imported again.
    pub fn delete_task(&self, account: &str, id: i64) -> Result<crate::tasks::Task> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "Begin task deletion failed")?;
        let payload: String = transaction
            .query_row(
                "SELECT payload FROM tasks WHERE id=?1 AND account=?2",
                params![id, account],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read task failed")?
            .ok_or("Task no longer exists")?;
        let task = Self::task_row(id, account, &payload)?;
        if let (Some(key), Some(meeting)) = (&task.import_key, &task.meeting) {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO task_tombstones VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        account,
                        key,
                        meeting.key,
                        meeting.event_id,
                        serde_json::to_string(&task.hashes).map_err(|_| "Encode task failed")?,
                        feedback_time(chrono::Utc::now())
                    ],
                )
                .map_err(|_| "Save deleted task failed")?;
        }
        transaction
            .execute(
                "DELETE FROM tasks WHERE id=?1 AND account=?2",
                params![id, account],
            )
            .map_err(|_| "Delete task failed")?;
        transaction
            .commit()
            .map_err(|_| "Commit task deletion failed")?;
        Ok(task)
    }

    /// Undoes a deletion: the task returns with its identity and its tombstone is removed.
    pub fn restore_task(&self, task: &crate::tasks::Task) -> Result<()> {
        if task.id <= 0 {
            return Err("Deleted task identity is invalid".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "Begin task restore failed")?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1)",
                [task.id],
                |row| row.get(0),
            )
            .map_err(|_| "Read task failed")?;
        if exists {
            return Err("Task was already restored".into());
        }
        Self::insert_task(&transaction, task, Some(task.id))?;
        if let Some(key) = &task.import_key {
            transaction
                .execute(
                    "DELETE FROM task_tombstones WHERE account=?1 AND import_key=?2",
                    params![task.account, key],
                )
                .map_err(|_| "Restore task failed")?;
        }
        transaction
            .commit()
            .map_err(|_| "Commit task restore failed".into())
    }

    fn backfilled_accounts(&self) -> Result<BTreeSet<String>> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM settings WHERE key='task-notes-backfill-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read task import state failed")?;
        match payload {
            Some(payload) if payload.len() <= 1024 * 1024 => {
                serde_json::from_str(&payload).map_err(|_| "Saved task import state is invalid".into())
            }
            Some(_) => Err("Saved task import state exceeds its safety limit".into()),
            None => Ok(BTreeSet::new()),
        }
    }

    /// Whether existing meeting notes were already imported once for this account.
    pub fn tasks_backfilled(&self, account: &str) -> Result<bool> {
        Ok(self.backfilled_accounts()?.contains(account))
    }

    pub fn mark_tasks_backfilled(&self, account: &str) -> Result<()> {
        let mut accounts = self.backfilled_accounts()?;
        if accounts.len() >= 1000 {
            accounts.clear();
        }
        accounts.insert(account.to_owned());
        let payload =
            serde_json::to_string(&accounts).map_err(|_| "Encode task import state failed")?;
        self.connection
            .execute(
                "INSERT INTO settings VALUES('task-notes-backfill-v1',?1)
                 ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                [payload],
            )
            .map_err(|_| "Save task import state failed")?;
        Ok(())
    }

    /// Saved follow-up e-mails of an account by meeting occurrence key, newest first.
    pub fn followups(
        &self,
        account: &str,
    ) -> Result<std::collections::BTreeMap<String, crate::followup::Saved>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT meeting_key,payload FROM followup_drafts WHERE account=?1
                 ORDER BY updated_at DESC LIMIT ?2",
            )
            .map_err(|_| "Read follow-up e-mails failed")?;
        let rows = statement
            .query_map(
                params![account, crate::followup::SAVED_LIMIT as i64],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|_| "Read follow-up e-mails failed")?;
        let mut drafts = std::collections::BTreeMap::new();
        for row in rows {
            let (key, payload) = row.map_err(|_| "Read follow-up e-mails failed")?;
            if payload.len() > 256 * 1024 {
                return Err("Saved follow-up e-mail exceeds its safety limit".into());
            }
            let saved: crate::followup::Saved =
                serde_json::from_str(&payload).map_err(|_| "Saved follow-up e-mail is invalid")?;
            saved.validate()?;
            if saved.account != account || saved.key != key {
                return Err("Saved follow-up e-mail does not match its meeting".into());
            }
            drafts.insert(key, saved);
        }
        Ok(drafts)
    }

    /// Saves the latest follow-up e-mail of a meeting occurrence; the oldest drafts beyond the
    /// limit are removed.
    pub fn save_followup(&self, saved: &crate::followup::Saved) -> Result<()> {
        saved.validate()?;
        let payload =
            serde_json::to_string(saved).map_err(|_| "Encode follow-up e-mail failed")?;
        if payload.len() > 256 * 1024 {
            return Err("Follow-up e-mail exceeds its safety limit".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "Begin follow-up e-mail storage failed")?;
        transaction
            .execute(
                "INSERT INTO followup_drafts VALUES(?1,?2,?3,?4)
                 ON CONFLICT(account,meeting_key) DO UPDATE SET
                   updated_at=excluded.updated_at,payload=excluded.payload",
                params![saved.account, saved.key, feedback_time(saved.updated_at), payload],
            )
            .map_err(|_| "Save follow-up e-mail failed")?;
        transaction
            .execute(
                "DELETE FROM followup_drafts WHERE account=?1 AND meeting_key NOT IN (
                   SELECT meeting_key FROM followup_drafts WHERE account=?1
                   ORDER BY updated_at DESC LIMIT ?2)",
                params![saved.account, crate::followup::SAVED_LIMIT as i64],
            )
            .map_err(|_| "Save follow-up e-mail failed")?;
        transaction
            .commit()
            .map_err(|_| "Commit follow-up e-mail failed".into())
    }

    /// Discards a meeting's follow-up e-mail; returns whether one existed.
    pub fn delete_followup(&self, account: &str, key: &str) -> Result<bool> {
        self.connection
            .execute(
                "DELETE FROM followup_drafts WHERE account=?1 AND meeting_key=?2",
                params![account, key],
            )
            .map(|count| count > 0)
            .map_err(|_| "Discard follow-up e-mail failed".into())
    }

    /// Saved AI summaries of an account's weeks by Monday, the newest weeks within the limit.
    pub fn weekly_summaries(
        &self,
        account: &str,
    ) -> Result<std::collections::BTreeMap<NaiveDate, crate::weekly::Saved>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT week,payload FROM weekly_summaries WHERE account=?1
                 ORDER BY week DESC LIMIT ?2",
            )
            .map_err(|_| "Read weekly summaries failed")?;
        let rows = statement
            .query_map(
                params![account, crate::weekly::SAVED_LIMIT as i64],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|_| "Read weekly summaries failed")?;
        let mut summaries = std::collections::BTreeMap::new();
        for row in rows {
            let (week, payload) = row.map_err(|_| "Read weekly summaries failed")?;
            if payload.len() > 64 * 1024 {
                return Err("Saved weekly summary exceeds its safety limit".into());
            }
            let saved: crate::weekly::Saved =
                serde_json::from_str(&payload).map_err(|_| "Saved weekly summary is invalid")?;
            saved.validate()?;
            if saved.account != account || saved.week.format("%Y-%m-%d").to_string() != week {
                return Err("Saved weekly summary does not match its week".into());
            }
            summaries.insert(saved.week, saved);
        }
        Ok(summaries)
    }

    /// Saves the latest AI summary of a week; summaries of the oldest weeks beyond the limit are
    /// removed.
    pub fn save_weekly_summary(&self, saved: &crate::weekly::Saved) -> Result<()> {
        saved.validate()?;
        let payload = serde_json::to_string(saved).map_err(|_| "Encode weekly summary failed")?;
        if payload.len() > 64 * 1024 {
            return Err("Weekly summary exceeds its safety limit".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "Begin weekly summary storage failed")?;
        transaction
            .execute(
                "INSERT INTO weekly_summaries VALUES(?1,?2,?3,?4)
                 ON CONFLICT(account,week) DO UPDATE SET
                   updated_at=excluded.updated_at,payload=excluded.payload",
                params![
                    saved.account,
                    saved.week.format("%Y-%m-%d").to_string(),
                    feedback_time(saved.created_at),
                    payload
                ],
            )
            .map_err(|_| "Save weekly summary failed")?;
        transaction
            .execute(
                "DELETE FROM weekly_summaries WHERE account=?1 AND week NOT IN (
                   SELECT week FROM weekly_summaries WHERE account=?1
                   ORDER BY week DESC LIMIT ?2)",
                params![saved.account, crate::weekly::SAVED_LIMIT as i64],
            )
            .map_err(|_| "Save weekly summary failed")?;
        transaction
            .commit()
            .map_err(|_| "Commit weekly summary failed".into())
    }
}

#[cfg(test)]
mod document_edit_tests {
    use super::*;
    use crate::{changes::{Document, DocumentEdit, Section}, recommendations::{Change, Recommendation, State}};
    use serde_json::json;

    fn fixture(document: Document) -> (tempfile::TempDir, Store, Event, Recommendation) {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let now = chrono::Local::now().fixed_offset();
        let mut event = crate::calendar::fixture(1).remove(0);
        event.start = now + chrono::Duration::hours(if document == Document::Notes { -2 } else { 24 });
        event.end = event.start + chrono::Duration::hours(1);
        event.is_all_day = false;
        event.status = "accepted".into();
        event.extra.insert("type".into(), json!("singleInstance"));
        event.extra.insert("isOrganizer".into(), json!(true));
        event.extra.insert("onlineMeeting".into(), json!({"joinUrl":"https://teams.microsoft.com/l/meetup-join/example"}));
        let calendar = Calendar::build(vec![event.clone()],
            [event.start.with_timezone(&chrono::Local).date_naive()].into(), vec![]).unwrap();
        store.save_workiq_calendar(&calendar, "owner").unwrap();
        let mut value = crate::changes::fixture_document();
        value["tomlook_event_fingerprint"] = json!(event.fingerprint());
        match document {
            Document::Insights => store.save_prepared(&Default::default(), &event.id, &value).unwrap(),
            Document::Notes => {
                value["tomlook_recap"] = json!(true);
                let mut ledger = crate::recaps::Ledger::default();
                let key = ledger.begin("owner", &event, now);
                ledger.finish(&key, Ok(Some(value)), now);
                store.save_recaps(&ledger).unwrap();
            }
        }
        let value = store.document("owner", &event, document).unwrap().unwrap();
        let section = Section { name: "preparation".into(),
            en: json!(["Review the [existing draft](https://example.invalid/draft)."]),
            cs: json!(["Zkontrolovat [existujici koncept](https://example.invalid/draft)."]) };
        let edit = DocumentEdit::new(document, crate::changes::revision(&value).unwrap(), vec![section], &value).unwrap();
        let proposal = Recommendation::from_manual(&event, "owner", Change::Document(edit), now).unwrap();
        store.save_recommendations(&[(proposal.id.clone(), proposal.clone())].into()).unwrap();
        (root, store, event, proposal)
    }

    #[test]
    fn local_document_approval_is_atomic_single_use_and_survives_restart() {
        let (root, store, event, proposal) = fixture(Document::Insights);
        let before = store.document("owner", &event, Document::Insights).unwrap().unwrap();
        assert_eq!(before["preparation"], json!(["Review the existing draft."]));
        assert_eq!(store.recommendations().unwrap()[&proposal.id].state, State::Pending);
        let inbox = store.apply_document_recommendation(&proposal, &event).unwrap();
        assert_eq!(inbox[&proposal.id].state, State::Applied);
        let saved = store.document("owner", &event, Document::Insights).unwrap().unwrap();
        assert_eq!(saved["preparation"], json!(["Review the [existing draft](https://example.invalid/draft)."]));
        assert_eq!(saved["cs"]["preparation"], json!(["Zkontrolovat [existujici koncept](https://example.invalid/draft)."]));
        assert_eq!(saved["sources"], before["sources"]);
        assert!(store.apply_document_recommendation(&proposal, &event).is_err());
        drop(store);
        let store = Store::open(root.path()).unwrap();
        assert_eq!(store.document("owner", &event, Document::Insights).unwrap().unwrap(), saved);
        assert_eq!(store.recommendations().unwrap()[&proposal.id].state, State::Applied);
        store.save_recommendations(&[(proposal.id.clone(), proposal.clone())].into()).unwrap();
        assert_eq!(store.recommendations().unwrap()[&proposal.id].state, State::Applied);
        assert!(store.apply_document_recommendation(&proposal, &event).is_err());
        let versions: u32 = store.connection.query_row(
            "SELECT count(*) FROM briefings WHERE meeting_id=?1", [&event.id], |row| row.get(0)).unwrap();
        assert_eq!(versions, 2);
    }

    #[test]
    fn consumed_approval_failure_rolls_back_document_insert_and_keeps_proposal_pending() {
        let (_root, store, event, proposal) = fixture(Document::Insights);
        let before = store.briefing(&event.id).unwrap();
        store.connection.execute_batch(
            "CREATE TRIGGER reject_document_receipt BEFORE UPDATE ON settings
             WHEN NEW.key='recommendation-inbox-v1'
             BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;").unwrap();
        assert!(store.apply_document_recommendation(&proposal, &event).is_err());
        assert_eq!(store.briefing(&event.id).unwrap(), before);
        assert_eq!(store.recommendations().unwrap()[&proposal.id].state, State::Pending);
        let versions: u32 = store.connection.query_row("SELECT count(*) FROM briefings", [], |row| row.get(0)).unwrap();
        assert_eq!(versions, 1);
    }

    #[test]
    fn changed_document_or_meeting_cannot_consume_the_original_approval() {
        let (_root, mut store, event, proposal) = fixture(Document::Insights);
        let mut fresh = crate::changes::fixture_document();
        fresh["summary"] = json!("A newer analysis arrived");
        store.save_prepared(&Default::default(), &event.id, &fresh).unwrap();
        let before = store.briefing(&event.id).unwrap();
        assert!(store.apply_document_recommendation(&proposal, &event).is_err());
        assert_eq!(store.briefing(&event.id).unwrap(), before);
        assert_eq!(store.recommendations().unwrap()[&proposal.id].state, State::Pending);
        let mut changed = event.clone();
        changed.title.push_str(" rescheduled");
        let calendar = Calendar::build(vec![changed.clone()],
            [changed.start.with_timezone(&chrono::Local).date_naive()].into(), vec![]).unwrap();
        store.save_workiq_calendar(&calendar, "owner").unwrap();
        assert!(store.apply_document_recommendation(&proposal, &event).is_err());
    }

    #[test]
    fn a_refresh_adds_invitation_previews_without_expiring_approvals_or_insights() {
        let (_root, mut store, event, proposal) = fixture(Document::Insights);
        assert!(!event.extra.contains_key("bodyPreview"), "Cached before previews were synchronized");
        let mut refreshed = event.clone();
        refreshed.extra.insert("bodyPreview".into(), json!("Agenda: confirm the pilot scope"));
        let day = refreshed.start.with_timezone(&chrono::Local).date_naive();
        let calendar = Calendar::build(vec![refreshed], [day].into(), vec![]).unwrap();
        store.save_workiq_calendar(&calendar, "owner").unwrap();
        let cached = store.calendar().unwrap();
        let saved = &cached.events[cached.event_index(&event.id).unwrap()];
        assert_eq!(saved.extra["bodyPreview"], "Agenda: confirm the pilot scope",
            "A refreshed day replaces its cached meetings, so the preview appears after the next sync");
        assert_eq!(crate::scheduler::fingerprint(saved), crate::scheduler::fingerprint(&event));
        assert_eq!(saved.fingerprint(), event.fingerprint(), "Insights stay current");
        assert!(proposal.current(&cached, Some("owner"), chrono::Local::now().fixed_offset()).is_some());
        assert_eq!(store.apply_document_recommendation(&proposal, &event).unwrap()[&proposal.id].state,
            State::Applied, "An approval confirmed before the refresh still applies");
    }

    #[test]
    fn fresh_automatic_preparation_retains_only_approved_sections_and_does_not_cross_accounts() {
        let (_root, mut store, event, proposal) = fixture(Document::Insights);
        store.apply_document_recommendation(&proposal, &event).unwrap();
        let approved = store.document("owner", &event, Document::Insights).unwrap().unwrap();
        let mut fresh = crate::changes::fixture_document();
        fresh["preparation"] = json!(["Generated text must not replace the approved edit"]);
        fresh["talking_points"] = json!(["New technical detail"]);
        fresh["cs"]["talking_points"] = json!(["Novy technicky detail"]);
        store.save_prepared(&Default::default(), &event.id, &fresh).unwrap();
        let saved = store.document("owner", &event, Document::Insights).unwrap().unwrap();
        assert_eq!(saved["preparation"], approved["preparation"]);
        assert_eq!(saved["cs"]["preparation"], approved["cs"]["preparation"]);
        assert_eq!(saved["talking_points"], fresh["talking_points"]);
        assert_eq!(saved["cs"]["talking_points"], fresh["cs"]["talking_points"]);
        store.connection.execute(
            "UPDATE briefings SET payload=json_remove(payload,'$.tomlook_account')", []).unwrap();
        let calendar = store.calendar().unwrap();
        store.save_workiq_calendar(&calendar, "different-owner").unwrap();
        assert!(store.briefing(&event.id).unwrap().is_none());
        assert!(store.document("owner", &event, Document::Insights).is_err());
        assert!(store.apply_document_recommendation(&proposal, &event).is_err());
        store.save_prepared(&Default::default(), &event.id, &fresh).unwrap();
        let other = store.document("different-owner", &event, Document::Insights).unwrap().unwrap();
        assert_eq!(other["preparation"], fresh["preparation"]);
        assert!(crate::changes::approved_context(&other).is_none());
        store.save_workiq_calendar(&calendar, "owner").unwrap();
        assert_eq!(store.document("owner", &event, Document::Insights).unwrap().unwrap()["preparation"], approved["preparation"]);
    }

    #[test]
    fn editing_notes_keeps_original_sources_and_processed_once_behavior() {
        let (_root, store, event, proposal) = fixture(Document::Notes);
        let before = store.document("owner", &event, Document::Notes).unwrap().unwrap();
        store.apply_document_recommendation(&proposal, &event).unwrap();
        let ledger = store.recaps().unwrap();
        let saved = ledger.record("owner", &event).unwrap().payload.as_ref().unwrap();
        assert_eq!(saved["tomlook_recap"], true);
        assert_eq!(saved["sources"], before["sources"]);
        assert!(crate::recaps::eligible(&event, chrono::Local::now().fixed_offset()));
        assert!(ledger.due("owner", &[event], chrono::Local::now().fixed_offset()).is_none());
        assert!(crate::changes::approved_context(saved).is_some());
    }

    #[test]
    fn in_flight_notes_cannot_overwrite_a_newly_approved_local_edit() {
        let (_root, store, event, proposal) = fixture(Document::Notes);
        let before = store.document("owner", &event, Document::Notes).unwrap();
        let mut ledger = store.recaps().unwrap();
        ledger.begin("owner", &event, chrono::Local::now().fixed_offset());
        store.save_recaps(&ledger).unwrap();
        assert!(store.apply_document_recommendation(&proposal, &event).unwrap_err().contains("being prepared"));
        assert_eq!(store.document("owner", &event, Document::Notes).unwrap(), before);
        assert_eq!(store.recommendations().unwrap()[&proposal.id].state, State::Pending);
    }
}

#[cfg(test)]
mod feedback_tests {
    use super::*;
    use crate::feedback::{DOCUMENT, Output, Rating, Reason, summarize};
    use serde_json::json;

    fn output(language: crate::i18n::Language) -> (Output, serde_json::Value) {
        let mut event = crate::calendar::fixture(1).remove(0);
        event.id = "meeting-1".into();
        let payload = json!({"summary":"Agree on the pilot.","preparation":["Bring the comparison."],
            "talking_points":["Start with the workflow."],"open_questions":[],"risks":[],"gaps":[],"sources":[],
            "cs":{"summary":"Domluvit pilot.","preparation":["Přinést srovnání."],"talking_points":["Začít postupem."],
                "open_questions":[],"risks":[],"gaps":[]}});
        let output = Output::new("owner", &event, &payload, language,
            &crate::prompts::Prompts::default()).unwrap();
        (output, payload)
    }

    #[test]
    fn feedback_round_trips_upserts_clears_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let (output, payload) = output(crate::i18n::Language::En);
        let target = output.target(&payload, "talking_points").unwrap();
        let first = chrono::Utc::now() - chrono::Duration::minutes(5);
        let down = output.entry(&target, Rating::Down, [Reason::TooVague].into(), "Be specific", first).unwrap();
        assert_eq!(store.save_feedback(&down).unwrap(), down);
        assert_eq!(store.feedback_for_document(&output.id).unwrap(), vec![down.clone()]);
        let later = chrono::Utc::now();
        let up = output.entry(&target, Rating::Up, Default::default(), "", later).unwrap();
        let saved = store.save_feedback(&up).unwrap();
        assert_eq!(saved.created_at, first, "Re-rating keeps the first rating time");
        assert_eq!(saved.updated_at, later);
        assert_eq!(saved.rating, Rating::Up);
        let document = output.target(&payload, DOCUMENT).unwrap();
        store.save_feedback(&output.entry(&document, Rating::Up, Default::default(), "", later).unwrap()).unwrap();
        drop(store);
        let store = Store::open(root.path()).unwrap();
        let entries = store.feedback_for_document(&output.id).unwrap();
        assert_eq!(entries.len(), 2, "One row per document, revision and section");
        assert_eq!(crate::feedback::current(&entries, &target.key).unwrap(), &saved);
        assert!(store.clear_feedback(&target.key).unwrap());
        assert!(!store.clear_feedback(&target.key).unwrap());
        let entries = store.feedback_for_document(&output.id).unwrap();
        assert!(crate::feedback::current(&entries, &target.key).is_none());
        assert_eq!(entries.len(), 1);
        let mut invalid = down;
        invalid.key.revision = "invalid".into();
        assert!(store.save_feedback(&invalid).is_err());
    }

    #[test]
    fn new_revisions_start_unrated_but_old_feedback_counts_in_the_window() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let (output, payload) = output(crate::i18n::Language::En);
        let now = chrono::Utc::now();
        let old = output.target(&payload, "summary").unwrap();
        store.save_feedback(&output.entry(&old, Rating::Down, [Reason::Inaccurate].into(), "", now).unwrap()).unwrap();
        let ancient = output.target(&payload, "preparation").unwrap();
        store.save_feedback(&output.entry(&ancient, Rating::Down, [Reason::TooLong].into(), "",
            now - chrono::Duration::days(120)).unwrap()).unwrap();
        let mut regenerated = payload.clone();
        regenerated["summary"] = json!("A re-researched overview.");
        let current = output.target(&regenerated, "summary").unwrap();
        let entries = store.feedback_for_document(&output.id).unwrap();
        assert!(crate::feedback::current(&entries, &current.key).is_none());
        let (czech, _) = self::output(crate::i18n::Language::Cs);
        assert!(crate::feedback::current(&entries, &czech.target(&payload, "summary").unwrap().key).is_none());
        let recent = store.feedback_since("owner", now - chrono::Duration::days(crate::feedback::WINDOW_DAYS)).unwrap();
        assert_eq!(recent.len(), 1);
        let summary = summarize("owner", &recent, now);
        assert_eq!(summary.preparation.down, 1);
        assert_eq!(summary.preparation.reasons, vec![(Reason::Inaccurate, 1)]);
        assert!(store.feedback_since("other", now - chrono::Duration::days(90)).unwrap().is_empty());
    }

    #[test]
    fn existing_profiles_gain_the_feedback_table_without_losing_data() {
        let root = tempfile::tempdir().unwrap();
        {
            let connection = Connection::open(root.path().join("tomlook.db")).unwrap();
            connection.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,payload TEXT NOT NULL);
                INSERT INTO settings VALUES('preferences','{\"dark\":false}');").unwrap();
        }
        let store = Store::open(root.path()).unwrap();
        assert!(!store.preferences().unwrap().dark);
        let (output, payload) = output(crate::i18n::Language::En);
        let target = output.target(&payload, "summary").unwrap();
        store.save_feedback(&output.entry(&target, Rating::Up, Default::default(), "", chrono::Utc::now()).unwrap()).unwrap();
        drop(store);
        let store = Store::open(root.path()).unwrap();
        assert_eq!(store.feedback_for_document(&output.id).unwrap().len(), 1);
    }
}

#[cfg(test)]
mod task_tests {
    use super::*;
    use crate::{i18n::Language, tasks::{New, Status}};
    use serde_json::json;

    const ACCOUNT: &str = "owner@example.com";

    fn import(store: &Store, event: &Event, payload: &serde_json::Value, language: Language) -> usize {
        let tasks = crate::tasks::from_notes(ACCOUNT, event, payload, language, chrono::Utc::now()).unwrap();
        store.import_tasks(ACCOUNT, &tasks).unwrap()
    }

    #[test]
    fn manual_tasks_round_trip_edit_complete_reopen_delete_and_restore_across_restarts() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let now = chrono::Utc::now();
        let task = store.add_task(&New::manual("Send the pilot plan", None, Language::En).unwrap()
            .task(ACCOUNT, now).unwrap()).unwrap();
        assert!(task.id > 0);
        store.add_task(&New::manual("Another account", None, Language::En).unwrap()
            .task("other@example.com", now).unwrap()).unwrap();
        assert_eq!(store.tasks(ACCOUNT).unwrap(), vec![task.clone()]);
        let due = chrono::NaiveDate::from_ymd_opt(2026, 11, 4);
        let edited = task.edited("Send the final pilot plan", "Include the budget", due, now).unwrap();
        store.update_task(&edited).unwrap();
        let done = edited.completed(true, now);
        store.update_task(&done).unwrap();
        drop(store);
        let store = Store::open(root.path()).unwrap();
        let saved = store.tasks(ACCOUNT).unwrap().remove(0);
        assert_eq!((saved.title.as_str(), saved.note.as_str(), saved.due, saved.status),
            ("Send the final pilot plan", "Include the budget", due, Status::Done));
        store.update_task(&saved.completed(false, now)).unwrap();
        assert_eq!(store.tasks(ACCOUNT).unwrap()[0].status, Status::Open);
        let mut moved = saved.clone();
        moved.source = crate::tasks::Source::AskDraft;
        assert!(store.update_task(&moved).is_err(), "The source of a task is fixed");
        let mut foreign = saved.clone();
        foreign.account = "other@example.com".into();
        assert!(store.update_task(&foreign).is_err());
        let deleted = store.delete_task(ACCOUNT, saved.id).unwrap();
        assert!(store.tasks(ACCOUNT).unwrap().is_empty());
        assert!(store.delete_task(ACCOUNT, saved.id).is_err());
        store.restore_task(&deleted).unwrap();
        assert_eq!(store.tasks(ACCOUNT).unwrap()[0].id, saved.id, "Undo keeps the identity");
        assert!(store.restore_task(&deleted).is_err());
        assert_eq!(store.tasks("other@example.com").unwrap().len(), 1);
    }

    #[test]
    fn note_imports_are_deduplicated_across_restarts_revisions_languages_and_tombstones() {
        let root = tempfile::tempdir().unwrap();
        let mut event = crate::tasks::tests::meeting();
        let payload = crate::tasks::tests::notes();
        let store = Store::open(root.path()).unwrap();
        assert_eq!(import(&store, &event, &payload, Language::En), 3);
        assert_eq!(import(&store, &event, &payload, Language::En), 0);
        assert_eq!(import(&store, &event, &payload, Language::Cs), 0, "Language does not duplicate tasks");
        drop(store);
        let store = Store::open(root.path()).unwrap();
        assert_eq!(import(&store, &event, &payload, Language::En), 0, "Restart does not duplicate tasks");
        let tasks = store.tasks(ACCOUNT).unwrap();
        assert_eq!(tasks.iter().map(|task| task.item).collect::<Vec<_>>(), vec![Some(0), Some(1), Some(3)]);
        store.update_task(&tasks[0].completed(true, chrono::Utc::now())).unwrap();
        let deleted = store.delete_task(ACCOUNT, tasks[1].id).unwrap();
        assert_eq!(import(&store, &event, &payload, Language::En), 0, "Completed and deleted items stay handled");
        let mut revised = payload.clone();
        revised["preparation"][2] = json!("My task: share the recording");
        revised["cs"]["preparation"][2] = json!("Můj úkol: nasdílet záznam");
        revised["summary"] = json!("Edited summary");
        assert_eq!(import(&store, &event, &revised, Language::En), 1, "Only the new item of a new revision is added");
        let titles = store.tasks(ACCOUNT).unwrap().into_iter().map(|task| task.title).collect::<Vec<_>>();
        assert!(titles.contains(&"Share the recording".to_owned()));
        assert!(!titles.contains(&deleted.title), "A deleted item never returns in another revision");
        event.extra.insert("iCalUId".into(), json!("metadata-enriched-later"));
        assert_eq!(import(&store, &event, &revised, Language::En), 0, "The event identity still matches");
        store.restore_task(&deleted).unwrap();
        assert_eq!(import(&store, &event, &payload, Language::En), 0);
        assert_eq!(store.tasks(ACCOUNT).unwrap().len(), 4);
        let other = crate::tasks::from_notes("other@example.com", &event, &payload, Language::En, chrono::Utc::now()).unwrap();
        assert!(store.import_tasks(ACCOUNT, &other).is_err(), "Imports are bound to their account");
    }

    #[test]
    fn ask_drafts_become_linked_tasks_and_imports_cannot_be_added_directly() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let event = crate::tasks::tests::meeting();
        let proposal = crate::proposals::Proposal {
            kind: crate::proposals::Kind::Task,
            target: "Pilot checklist".into(),
            title: "Prepare the checklist".into(),
            body: "Three success criteria".into(),
        };
        let meeting = crate::tasks::Meeting::new(ACCOUNT, &event);
        let task = New::from_draft(&proposal, Some(meeting), Language::En).unwrap()
            .task(ACCOUNT, chrono::Utc::now()).unwrap();
        let saved = store.add_task(&task).unwrap();
        assert_eq!(crate::tasks::for_meeting(&store.tasks(ACCOUNT).unwrap(), ACCOUNT, &event)[0].id, saved.id);
        let imported = crate::tasks::from_notes(ACCOUNT, &event, &crate::tasks::tests::notes(), Language::En, chrono::Utc::now()).unwrap();
        assert!(store.add_task(&imported[0]).is_err());
    }

    #[test]
    fn follow_up_drafts_are_saved_per_occurrence_survive_restart_and_can_be_discarded() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let event = crate::tasks::tests::meeting();
        let request = crate::followup::Request::new(1, ACCOUNT, &event, &crate::tasks::tests::notes(),
            Language::En, &[ACCOUNT.into()], None).unwrap();
        let draft = crate::followup::Draft { subject: "Follow-up".into(), body: "Thanks, all.".into() };
        let mut saved = crate::followup::Saved::new(&request, draft, chrono::Utc::now());
        store.save_followup(&saved).unwrap();
        saved.body = "Thanks, all. Edited.".into();
        saved.updated_at = chrono::Utc::now();
        store.save_followup(&saved).unwrap();
        drop(store);
        let store = Store::open(root.path()).unwrap();
        let drafts = store.followups(ACCOUNT).unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[&saved.key], saved);
        assert!(store.followups("other@example.com").unwrap().is_empty());
        let mut invalid = saved.clone();
        invalid.subject.clear();
        assert!(store.save_followup(&invalid).is_err());
        assert!(store.delete_followup(ACCOUNT, &saved.key).unwrap());
        assert!(!store.delete_followup(ACCOUNT, &saved.key).unwrap());
        assert!(store.followups(ACCOUNT).unwrap().is_empty());
        for index in 0..crate::followup::SAVED_LIMIT + 3 {
            let mut other = saved.clone();
            other.key = format!("{index:064x}");
            other.updated_at = chrono::Utc::now() + chrono::Duration::seconds(index as i64);
            store.save_followup(&other).unwrap();
        }
        let kept = store.followups(ACCOUNT).unwrap();
        assert_eq!(kept.len(), crate::followup::SAVED_LIMIT);
        assert!(!kept.contains_key(&format!("{:064x}", 0)), "The oldest drafts are removed first");
    }

    #[test]
    fn existing_profiles_gain_task_tables_and_backfill_runs_once_per_account() {
        let root = tempfile::tempdir().unwrap();
        {
            let connection = Connection::open(root.path().join("tomlook.db")).unwrap();
            connection.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,payload TEXT NOT NULL);
                INSERT INTO settings VALUES('preferences','{\"dark\":false}');").unwrap();
        }
        let store = Store::open(root.path()).unwrap();
        assert!(!store.preferences().unwrap().dark);
        assert!(store.tasks(ACCOUNT).unwrap().is_empty());
        assert!(!store.tasks_backfilled(ACCOUNT).unwrap());
        store.mark_tasks_backfilled(ACCOUNT).unwrap();
        drop(store);
        let store = Store::open(root.path()).unwrap();
        assert!(store.tasks_backfilled(ACCOUNT).unwrap());
        assert!(!store.tasks_backfilled("other@example.com").unwrap());
    }
}
