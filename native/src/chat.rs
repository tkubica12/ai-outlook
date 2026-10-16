use crate::{ai, calendar::Event, evidence, i18n::Language};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

pub const FAST_MODEL: &str = "gpt-6-luna";
pub const DEEP_MODEL: &str = "gpt-6.1-sol";
pub const TURN_LIMIT: usize = 32;
pub const HISTORY_BYTES: usize = 48_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Auto,
    Fast,
    Deep,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Fast => "Quick",
            Self::Deep => "Thorough",
        }
    }

    pub fn model(self) -> &'static str {
        if self == Self::Deep {
            DEEP_MODEL
        } else {
            FAST_MODEL
        }
    }

    pub fn effort(self) -> &'static str {
        if self == Self::Deep { "medium" } else { "low" }
    }

    pub fn seconds(self) -> u64 {
        if self == Self::Deep { 600 } else { 60 }
    }

    pub fn remote_reads(self) -> usize {
        if self == Self::Deep { 40 } else { 4 }
    }

    pub fn local_reads(self) -> usize {
        if self == Self::Deep { 80 } else { 12 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct History {
    pub question: String,
    pub answer: String,
}

#[derive(Clone)]
pub struct Request {
    pub question: ai::Question,
    pub mode: Mode,
    pub history: Vec<History>,
    pub resume: Option<String>,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.history.len() > TURN_LIMIT
            || self
                .history
                .iter()
                .any(|turn| turn.question.len() > 8000 || turn.answer.len() > 64_000)
            || self
                .history
                .iter()
                .map(|turn| turn.question.len() + turn.answer.len())
                .sum::<usize>()
                > HISTORY_BYTES
        {
            return Err("Conversation context exceeds its bounds; start a new chat".into());
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct Progress {
    pub mode: Mode,
    pub stage: String,
    pub steps: Vec<String>,
    pub elapsed_seconds: u64,
    pub preview: Option<Arc<crate::markdown::Document>>,
    pub research_key: Option<String>,
}

pub struct Run {
    pub id: u64,
    pub mode: Mode,
    pub local: Arc<AtomicUsize>,
    pub remote: Arc<AtomicUsize>,
    pub cancelled: std::sync::atomic::AtomicBool,
    started: Instant,
    progress: Mutex<Progress>,
    protected: Mutex<Option<BTreeSet<String>>>,
    emit: Arc<dyn Fn(Progress) + Send + Sync>,
}

impl Run {
    pub fn new(id: u64, mode: Mode, emit: Arc<dyn Fn(Progress) + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            id,
            mode,
            local: Arc::new(AtomicUsize::new(mode.local_reads())),
            remote: Arc::new(AtomicUsize::new(mode.remote_reads())),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            started: Instant::now(),
            progress: Mutex::new(Progress {
                mode,
                ..Default::default()
            }),
            protected: Mutex::new(Some(BTreeSet::new())),
            emit,
        })
    }

    pub fn stage(&self, stage: &str) {
        self.update(|progress| progress.stage = stage.into());
    }

    pub fn protect(&self, terms: impl IntoIterator<Item = String>) {
        if let Ok(mut protected) = self.protected.lock()
            && let Some(values) = protected.as_mut()
        {
            for term in terms {
                if term.trim().len() < 3 {
                    continue;
                }
                if values.len() >= 2048 {
                    *protected = None;
                    break;
                }
                values.insert(term.trim().to_lowercase());
            }
        }
    }

    pub fn protect_result(&self, value: &serde_json::Value) {
        fn collect(value: &serde_json::Value, depth: usize, terms: &mut Vec<String>) {
            if depth > 8 || terms.len() >= 2048 {
                return;
            }
            match value {
                serde_json::Value::Object(object) => {
                    for (name, value) in object {
                        if matches!(
                            name.as_str(),
                            "title" | "organizer" | "location" | "path" | "attendees"
                        ) {
                            if let Some(text) = value.as_str() {
                                terms.push(text.into());
                            }
                            if let Some(people) = value.as_array() {
                                terms.extend(
                                    people.iter().filter_map(serde_json::Value::as_str).map(
                                        |person| {
                                            person
                                                .split('<')
                                                .next()
                                                .unwrap_or(person)
                                                .trim()
                                                .to_owned()
                                        },
                                    ),
                                );
                            }
                        }
                        collect(value, depth + 1, terms);
                    }
                }
                serde_json::Value::Array(array) => {
                    for value in array.iter().take(100) {
                        collect(value, depth + 1, terms);
                    }
                }
                _ => {}
            }
        }
        let mut terms = Vec::new();
        collect(value, 0, &mut terms);
        self.protect(terms);
    }

    pub fn allows_public(&self, arguments: &serde_json::Value) -> bool {
        let query = arguments.to_string().to_lowercase();
        self.protected.lock().is_ok_and(|protected| {
            protected
                .as_ref()
                .is_some_and(|terms| !terms.iter().any(|term| query.contains(term)))
        })
    }

    pub fn completed(&self, name: &str, succeeded: bool) {
        let label = tool_label(name);
        self.update(|progress| {
            if progress.steps.len() == 24 {
                progress.steps.remove(0);
            }
            progress.steps.push(format!(
                "{label}: {}",
                if succeeded {
                    "Retrieved"
                } else {
                    "Retrieval failed"
                }
            ));
        });
    }

    pub fn preview(&self, text: &str) -> Result<(), String> {
        if text.len() > 64_000 {
            return Err("Redacted chat progress exceeded 64 KiB".into());
        }
        let document = Arc::new(crate::markdown::Document::parse(text));
        self.update(|progress| {
            progress.preview = Some(document);
            progress.stage = "Writing the answer".into();
        });
        Ok(())
    }

    pub fn research_key(&self, key: String) {
        self.update(|progress| progress.research_key = Some(key));
    }

    fn update(&self, update: impl FnOnce(&mut Progress)) {
        match self.progress.lock() {
            Ok(mut progress) => {
                update(&mut progress);
                progress.elapsed_seconds = self.started.elapsed().as_secs();
                (self.emit)(progress.clone());
            }
            Err(_) => eprintln!("Tomlook: Ask activity state is unavailable"),
        }
    }

    pub fn reserve(&self, remote: bool) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err("This question was stopped; no further read was sent".into());
        }
        let counter = if remote { &self.remote } else { &self.local };
        counter
            .try_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .map(|_| ())
            .map_err(|_| {
                if remote {
                    "This answer reached its remote-read budget"
                } else {
                    "This answer reached its local-read budget"
                }
                .into()
            })
    }
}

pub fn tool_label(name: &str) -> &'static str {
    match name {
        crate::chat_cache::SEARCH => "Searching saved meetings",
        crate::chat_cache::DETAIL => "Reading a saved meeting",
        crate::chat_cache::INSIGHTS => "Reading saved Insights",
        crate::context::SEARCH => "Searching local materials",
        crate::context::READ => "Reading a local document",
        crate::workiq::connection::SDK_TOOL => "Reading mail or calendar in Microsoft 365",
        crate::workiq::connection::CONTEXT_TOOL => "Searching Microsoft 365 context",
        crate::workiq::connection::cloud::SDK_TOOL => "Reading cloud materials",
        name if name.contains("github") => "Searching GitHub",
        name if name.starts_with("WebIQ") => "Searching public sources",
        _ => "Reading an authorized source",
    }
}

#[derive(Clone)]
pub struct Turn {
    pub id: u64,
    pub question: String,
    pub answer: Option<ai::Answer>,
    pub error: Option<String>,
    pub running: bool,
    pub progress: Progress,
    pub meetings: Vec<(String, String)>,
    pub started: Instant,
}

#[derive(Clone, Default)]
pub struct Conversation {
    pub turns: VecDeque<Turn>,
    pub revision: u64,
}

impl Conversation {
    pub fn admit(&mut self, id: u64, question: String, mode: Mode) -> Result<(), String> {
        if self.turns.len() >= TURN_LIMIT {
            return Err("This chat holds 32 questions; start a new chat to continue".into());
        }
        self.turns.push_back(Turn {
            id,
            question,
            answer: None,
            error: None,
            running: true,
            progress: Progress {
                mode,
                stage: "Preparing the request".into(),
                ..Default::default()
            },
            meetings: Vec::new(),
            started: Instant::now(),
        });
        self.revision += 1;
        Ok(())
    }

    pub fn running(&self) -> bool {
        self.turns.iter().any(|turn| turn.running)
    }

    pub fn finish(&mut self, id: u64, result: Result<ai::Answer, String>) {
        if let Some(turn) = self
            .turns
            .iter_mut()
            .find(|turn| turn.id == id && turn.running)
        {
            turn.running = false;
            turn.progress.elapsed_seconds = turn.started.elapsed().as_secs();
            turn.progress.preview = None;
            match result {
                Ok(answer) => turn.answer = Some(answer),
                Err(error) => turn.error = Some(error),
            }
            self.revision += 1;
        }
    }

    pub fn history(&self) -> Vec<History> {
        let mut remaining = HISTORY_BYTES;
        let mut turns = Vec::new();
        for turn in self.turns.iter().rev() {
            let Some(answer) = &turn.answer else {
                continue;
            };
            if remaining <= turn.question.len() + 80 {
                break;
            }
            let limit = remaining - turn.question.len();
            let mut text = answer.text.as_ref().clone();
            if text.len() > limit {
                text.truncate(text.floor_char_boundary(limit - 60));
                text.push_str("\n[Earlier answer excerpt; not the complete answer.]");
            }
            remaining -= turn.question.len() + text.len();
            turns.push(History {
                question: turn.question.clone(),
                answer: text,
            });
        }
        turns.reverse();
        turns
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MeetingRef {
    pub id: String,
    pub fingerprint: String,
}

impl From<&Event> for MeetingRef {
    fn from(event: &Event) -> Self {
        Self {
            id: event.id.clone(),
            fingerprint: event.fingerprint(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub key: String,
    pub account: String,
    pub binding: String,
    pub question: String,
    pub meeting: Option<MeetingRef>,
    pub language: Language,
    pub history: Vec<History>,
    pub answer: String,
    pub evidence: evidence::Snapshot,
    pub status: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("ask-research.db"),
        }
    }

    fn open(&self) -> Result<rusqlite::Connection, String> {
        let connection =
            rusqlite::Connection::open(&self.path).map_err(|_| "Open saved research failed")?;
        connection
            .busy_timeout(std::time::Duration::from_secs(1))
            .map_err(|_| "Configure saved research failed")?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS research(key TEXT PRIMARY KEY,account TEXT NOT NULL,payload TEXT NOT NULL,status TEXT NOT NULL,updated_at TEXT NOT NULL);")
            .map_err(|_| "Initialize saved research failed")?;
        Ok(connection)
    }

    pub async fn save(&self, checkpoint: Checkpoint) -> Result<(), String> {
        let journal = self.clone();
        tokio::task::spawn_blocking(move || {
            checkpoint.validate()?;
            let json = serde_json::to_string(&checkpoint).map_err(|_| "Encode research checkpoint failed")?;
            if json.len() > 256_000 { return Err("Research checkpoint exceeds 256 KiB".into()); }
            let mut connection = journal.open()?;
            let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_| "Lock saved research failed")?;
            tx.execute("DELETE FROM research WHERE status='completed'", []).map_err(|_| "Prune completed research failed")?;
            let count: i64 = tx.query_row("SELECT count(*) FROM research WHERE key<>?1", [&checkpoint.key], |row| row.get(0)).map_err(|_| "Read saved research count failed")?;
            if count >= 8 { return Err("Eight interrupted research tasks are saved; finish a saved task before starting another".into()); }
            tx.execute("INSERT INTO research VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload,status=excluded.status,updated_at=excluded.updated_at",
                rusqlite::params![checkpoint.key,checkpoint.account,json,checkpoint.status,checkpoint.updated_at]).map_err(|_| "Save research checkpoint failed")?;
            tx.commit().map_err(|_| "Commit research checkpoint failed".into())
        }).await.map_err(|_| "Research checkpoint worker stopped")?
    }

    pub async fn load(&self, account: String) -> Result<Vec<Checkpoint>, String> {
        let journal = self.clone();
        tokio::task::spawn_blocking(move || {
            if !journal.path.exists() { return Ok(Vec::new()); }
            let connection = journal.open()?;
            let mut query = connection.prepare("SELECT payload FROM research WHERE account=?1 AND status<>'completed' ORDER BY updated_at DESC LIMIT 8").map_err(|_| "Read saved research failed")?;
            let json = query.query_map([&account], |row| row.get::<_,String>(0)).map_err(|_| "Query saved research failed")?.collect::<Result<Vec<_>,_>>().map_err(|_| "Read research checkpoint failed")?;
            json.into_iter().map(|json| {
                if json.len() > 256_000 { return Err("Saved research exceeds its bounds".into()); }
                let checkpoint: Checkpoint = serde_json::from_str(&json).map_err(|_| "Saved research is invalid")?;
                checkpoint.validate()?;
                if checkpoint.account!=account { return Err("Saved research account binding is invalid".into()); }
                Ok(checkpoint)
            }).collect()
        }).await.map_err(|_| "Research recovery worker stopped")?
    }

    pub async fn discard(&self, account: String, key: String) -> Result<(), String> {
        let journal = self.clone();
        tokio::task::spawn_blocking(move || {
            if !journal.path.exists() {
                return Err("This saved research task is unavailable".into());
            }
            let connection = journal.open()?;
            let count = connection
                .execute(
                    "DELETE FROM research WHERE account=?1 AND key=?2",
                    [account, key],
                )
                .map_err(|_| "Discard saved research failed")?;
            if count != 1 {
                return Err("This saved research task is unavailable".into());
            }
            Ok(())
        })
        .await
        .map_err(|_| "Research recovery worker stopped")?
    }
}

impl Checkpoint {
    pub fn validate(&self) -> Result<(), String> {
        self.evidence.validate()?;
        if self.question.is_empty()
            || self.question.len() > 8000
            || self.answer.len() > 64_000
            || self.key.is_empty()
            || self.key.len() > 100
            || self.account.is_empty()
            || self.account.len() > 254
            || self.binding.len() > 1000
            || self.meeting.as_ref().is_some_and(|meeting| {
                meeting.id.is_empty()
                    || meeting.id.len() > 4096
                    || meeting.fingerprint.len() != 64
                    || !meeting
                        .fingerprint
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
            })
            || self.history.len() > TURN_LIMIT
            || self
                .history
                .iter()
                .any(|turn| turn.question.len() > 8000 || turn.answer.len() > 64_000)
            || self
                .history
                .iter()
                .map(|turn| turn.question.len() + turn.answer.len())
                .sum::<usize>()
                > HISTORY_BYTES
            || !matches!(
                self.status.as_str(),
                "running" | "interrupted" | "completed"
            )
            || chrono::DateTime::parse_from_rfc3339(&self.updated_at).is_err()
        {
            return Err("Saved research exceeds its bounds or has an invalid state".into());
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Lanes {
    fast: std::sync::atomic::AtomicBool,
    deep: std::sync::atomic::AtomicBool,
}

pub struct Permit {
    lanes: Arc<Lanes>,
    deep: bool,
}

impl Lanes {
    pub fn admit(self: &Arc<Self>, mode: Mode) -> Result<Permit, String> {
        let deep = mode == Mode::Deep;
        let slot = if deep { &self.deep } else { &self.fast };
        slot.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| if deep { "A thorough research is already running; quick questions remain available" } else { "A quick question or automatic routing is already running; wait or stop it first" })?;
        Ok(Permit {
            lanes: self.clone(),
            deep,
        })
    }
}

impl Permit {
    pub fn promote(&mut self) -> Result<(), String> {
        if !self.deep {
            self.lanes.deep.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| "Auto selected thorough research, but that lane is busy; choose Quick or wait for the running research")?;
            self.lanes.fast.store(false, Ordering::Release);
            self.deep = true;
        }
        Ok(())
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        (if self.deep {
            &self.lanes.deep
        } else {
            &self.lanes.fast
        })
        .store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_research_leaves_one_fast_lane_and_auto_promotion_is_atomic() {
        let lanes = Arc::new(Lanes::default());
        let deep = lanes.admit(Mode::Deep).unwrap();
        let mut fast = lanes.admit(Mode::Auto).unwrap();
        assert!(lanes.admit(Mode::Fast).is_err());
        assert!(fast.promote().is_err());
        drop(deep);
        fast.promote().unwrap();
        let quick = lanes.admit(Mode::Fast).unwrap();
        assert!(lanes.admit(Mode::Deep).is_err());
        drop(fast);
        assert!(lanes.admit(Mode::Deep).is_ok());
        drop(quick);
        assert!(lanes.admit(Mode::Auto).is_ok());
    }

    #[test]
    fn budgets_and_public_activity_are_bounded_without_reasoning_or_arguments() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let output = captured.clone();
        let run = Run::new(
            1,
            Mode::Fast,
            Arc::new(move |progress| output.lock().unwrap().push(progress)),
        );
        for _ in 0..Mode::Fast.remote_reads() {
            run.reserve(true).unwrap();
        }
        assert!(run.reserve(true).is_err());
        assert_eq!(run.remote.load(Ordering::Acquire), 0);
        for _ in 0..40 {
            run.completed("tomlook_calendar_search", true);
        }
        let captured = captured.lock().unwrap();
        assert_eq!(captured.last().unwrap().steps.len(), 24);
        assert!(
            captured
                .last()
                .unwrap()
                .steps
                .iter()
                .all(|step| step == "Searching saved meetings: Retrieved")
        );
        run.protect_result(&serde_json::json!({"meetings":[{"title":"Private Northstar demo","attendees":["Alex Rivers <alex@example.invalid>"]}]}));
        assert!(
            !run.allows_public(
                &serde_json::json!({"query":"Private Northstar demo public agenda"})
            )
        );
        assert!(!run.allows_public(&serde_json::json!({"query":"Alex Rivers presentation"})));
        assert!(run.allows_public(
            &serde_json::json!({"query":"generic database architecture documentation"})
        ));
    }

    #[test]
    fn conversation_correlates_concurrent_answers_and_preserves_a_long_recent_turn() {
        let mut chat = Conversation::default();
        chat.admit(1, "Research preparation".into(), Mode::Deep)
            .unwrap();
        chat.admit(2, "Next meeting".into(), Mode::Fast).unwrap();
        chat.finish(
            2,
            Ok(ai::Answer::new(
                "Soon".into(),
                Arc::new(Default::default()),
                Arc::new(Vec::new()),
            )),
        );
        assert!(chat.turns[0].running);
        assert_eq!(chat.turns[1].answer.as_ref().unwrap().text.as_str(), "Soon");
        chat.finish(1, Err("Interrupted".into()));
        assert!(!chat.running());
        chat.finish(
            1,
            Ok(ai::Answer::new(
                "Late".into(),
                Arc::new(Default::default()),
                Arc::new(Vec::new()),
            )),
        );
        assert!(chat.turns[0].answer.is_none());
        assert_eq!(chat.history().len(), 1);
        chat.admit(3, "A long follow-up".into(), Mode::Deep)
            .unwrap();
        chat.finish(
            3,
            Ok(ai::Answer::new(
                "x".repeat(60_000),
                Arc::new(Default::default()),
                Arc::new(Vec::new()),
            )),
        );
        let history = chat.history();
        assert_eq!(history.last().unwrap().question, "A long follow-up");
        assert!(
            history
                .last()
                .unwrap()
                .answer
                .contains("not the complete answer")
        );
        assert!(
            history
                .iter()
                .map(|turn| turn.question.len() + turn.answer.len())
                .sum::<usize>()
                <= HISTORY_BYTES
        );
        for id in 4..=TURN_LIMIT as u64 {
            chat.admit(id, "Bounded".into(), Mode::Fast).unwrap();
        }
        assert!(chat.admit(99, "Overflow".into(), Mode::Fast).is_err());
    }

    fn checkpoint(key: &str, account: &str) -> Checkpoint {
        Checkpoint {
            key: key.into(),
            account: account.into(),
            binding: "fixture-context".into(),
            question: "Synthetic research".into(),
            meeting: None,
            language: Language::En,
            history: Vec::new(),
            answer: "Some prior findings".into(),
            evidence: Default::default(),
            status: "running".into(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn research_survives_restart_is_account_bound_and_never_evicts_interrupted_jobs() {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::new(root.path());
        journal
            .save(checkpoint("job-1", "owner@example.invalid"))
            .await
            .unwrap();
        let reopened = Journal::new(root.path());
        assert!(
            reopened
                .load("other@example.invalid".into())
                .await
                .unwrap()
                .is_empty()
        );
        let loaded = reopened.load("owner@example.invalid".into()).await.unwrap();
        assert_eq!(loaded[0].answer, "Some prior findings");
        assert_eq!(loaded[0].status, "running");
        for id in 2..=8 {
            journal
                .save(checkpoint(&format!("job-{id}"), "owner@example.invalid"))
                .await
                .unwrap();
        }
        assert!(
            journal
                .save(checkpoint("job-9", "owner@example.invalid"))
                .await
                .unwrap_err()
                .contains("Eight")
        );
        assert_eq!(
            journal
                .load("owner@example.invalid".into())
                .await
                .unwrap()
                .len(),
            8
        );
        let mut completed = loaded[0].clone();
        completed.status = "completed".into();
        journal.save(completed).await.unwrap();
        journal
            .save(checkpoint("job-9", "owner@example.invalid"))
            .await
            .unwrap();
        assert_eq!(
            journal
                .load("owner@example.invalid".into())
                .await
                .unwrap()
                .len(),
            8
        );
        let mut invalid = checkpoint("invalid", "owner@example.invalid");
        invalid.history.push(History {
            question: "x".into(),
            answer: "x".repeat(HISTORY_BYTES + 1),
        });
        assert!(journal.save(invalid).await.is_err());
        assert!(
            journal
                .discard("other@example.invalid".into(), "job-9".into())
                .await
                .is_err()
        );
        journal
            .discard("owner@example.invalid".into(), "job-9".into())
            .await
            .unwrap();
        journal
            .save(checkpoint("replacement", "owner@example.invalid"))
            .await
            .unwrap();
        assert_eq!(
            journal
                .load("owner@example.invalid".into())
                .await
                .unwrap()
                .len(),
            8
        );
    }
}
