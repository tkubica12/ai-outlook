use chrono::NaiveDate;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_DAILY_LIMIT: u32 = 200;
pub const QUOTA_REACHED: &str =
    "Daily AI request limit reached; queued research is deferred until tomorrow.";
const LOG_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Metrics {
    pub input: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub output: Option<u64>,
    pub multiplier_cost: Option<f64>,
}

impl Metrics {
    pub fn from_sdk(value: &serde_json::Value) -> Result<Self, String> {
        let data: github_copilot_sdk::session_events::AssistantUsageData =
            serde_json::from_value(value.clone()).map_err(|_| "Invalid SDK usage event")?;
        let tokens = |value: Option<i64>| {
            value
                .map(|value| u64::try_from(value).map_err(|_| "Negative SDK token count"))
                .transpose()
        };
        if data
            .cost
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        {
            return Err("Invalid SDK billing multiplier".into());
        }
        Ok(Self {
            input: tokens(data.input_tokens)?,
            cache_read: tokens(data.cache_read_tokens)?,
            cache_write: tokens(data.cache_write_tokens)?,
            output: tokens(data.output_tokens)?,
            multiplier_cost: data.cost,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    pub day: String,
    pub requests: u32,
    pub limit: u32,
    pub usage_events: usize,
    pub requests_with_usage: u32,
    pub tokens: Metrics,
}

#[derive(Serialize)]
struct Diagnostic {
    timestamp: String,
    run: String,
    component: &'static str,
    code: &'static str,
    request: Option<u64>,
    count: u64,
    http_status: Option<u16>,
}

enum LogCommand {
    Event(Diagnostic),
    Flush(tokio::sync::oneshot::Sender<Result<(), String>>),
}

pub struct Service {
    database: PathBuf,
    run: String,
    limit: AtomicU32,
    diagnostics: mpsc::SyncSender<LogCommand>,
    dropped: Arc<AtomicU64>,
    log_failed: Arc<AtomicBool>,
    usage_failed: AtomicBool,
}

impl Service {
    // Called on the SDK worker, never by rendering or an input callback.
    pub fn open(root: &Path) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(root).map_err(|_| "AI accounting directory is unavailable")?;
        let database = root.join("ai-usage.db");
        open_database(&database)?;
        let run = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "System clock precedes the epoch")?
                .as_nanos()
        );
        let (diagnostics, receiver) = mpsc::sync_channel::<LogCommand>(128);
        let dropped = Arc::new(AtomicU64::new(0));
        let lost = dropped.clone();
        let log_failed = Arc::new(AtomicBool::new(false));
        let failed = log_failed.clone();
        let path = root.join("diagnostics.jsonl");
        let mut file = append_log(&path)?;
        std::thread::Builder::new()
            .name("tomlook-diagnostics".into())
            .spawn(move || {
                for message in receiver {
                    let mut event = match message {
                        LogCommand::Event(event) => event,
                        LogCommand::Flush(reply) => {
                            let result = file
                                .flush()
                                .and_then(|()| file.sync_data())
                                .map_err(|_| "Flush diagnostic log failed".to_owned());
                            if result.is_err() {
                                failed.store(true, Ordering::Release);
                            }
                            let _ = reply.send(result);
                            continue;
                        }
                    };
                    let omitted = lost.swap(0, Ordering::AcqRel);
                    let result = (|| {
                        if omitted > 0 {
                            let code = event.code;
                            let count = event.count;
                            event.code = "diagnostic_queue_overflow";
                            event.count = omitted;
                            write_log(&path, &mut file, &event)?;
                            event.code = code;
                            event.count = count;
                        }
                        write_log(&path, &mut file, &event)
                    })();
                    if let Err(error) = result {
                        failed.store(true, Ordering::Release);
                        eprintln!("Tomlook: {error}");
                        break;
                    }
                }
            })
            .map_err(|_| "Diagnostic writer could not start")?;
        Ok(Arc::new(Self {
            database,
            run,
            limit: AtomicU32::new(DEFAULT_DAILY_LIMIT),
            diagnostics,
            dropped,
            log_failed,
            usage_failed: AtomicBool::new(false),
        }))
    }

    pub fn set_limit(&self, limit: u32) -> Result<(), String> {
        if !(1..=100_000).contains(&limit) {
            return Err("Daily AI request limit must be between 1 and 100000".into());
        }
        self.limit.store(limit, Ordering::Release);
        Ok(())
    }

    pub fn log_failed(&self) -> bool {
        self.log_failed.load(Ordering::Acquire) || self.usage_failed.load(Ordering::Acquire)
    }

    pub fn record(
        &self,
        component: &'static str,
        code: &'static str,
        request: Option<u64>,
        count: u64,
    ) {
        let event = Diagnostic {
            timestamp: chrono::Utc::now().to_rfc3339(),
            run: self.run.clone(),
            component,
            code,
            request,
            count,
            http_status: None,
        };
        self.emit(event);
    }

    fn emit(&self, event: Diagnostic) {
        match self.diagnostics.try_send(LogCommand::Event(event)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                if !self.log_failed.swap(true, Ordering::AcqRel) {
                    eprintln!("Tomlook: diagnostic writer is unavailable");
                }
            }
        }
    }

    pub async fn flush(&self) -> Result<(), String> {
        let (reply, received) = tokio::sync::oneshot::channel();
        let mut command = LogCommand::Flush(reply);
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            match self.diagnostics.try_send(command) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(next)) if tokio::time::Instant::now() < until => {
                    command = next;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(_) => return Err("Diagnostic flush could not be queued".into()),
            }
        }
        tokio::time::timeout(Duration::from_secs(2), received)
            .await
            .map_err(|_| "Diagnostic flush timed out")?
            .map_err(|_| "Diagnostic writer stopped before flushing")?
    }

    pub fn failure(&self, component: &'static str, error: &str, request: Option<u64>) {
        let http_status = error
            .split_once("HTTP ")
            .and_then(|(_, tail)| tail.get(..3))
            .and_then(|digits| digits.parse::<u16>().ok())
            .filter(|status| (100..=599).contains(status));
        let code = if http_status.is_some() {
            "http_failure"
        } else if error.contains("outcome delivery timed out") {
            "outcome_delivery_timeout"
        } else if error.contains("outcome could not reach") {
            "outcome_storage_disconnected"
        } else if error.contains("configuration could not reach") {
            "configuration_storage_disconnected"
        } else if error.contains("event stream") {
            "sdk_event_stream_failed"
        } else if error.contains("session error") {
            "sdk_session_error"
        } else if error.contains("Sending the SDK") {
            "sdk_submission_timeout"
        } else if error.contains("Detach SDK") {
            "sdk_detach_failed"
        } else if error.contains("Cancel SDK") {
            "sdk_abort_failed"
        } else if error.contains("worker failed") {
            "executor_task_failed"
        } else if error.contains("initialization") {
            "connector_initialization_failed"
        } else if error.contains("tool discovery") {
            "connector_discovery_failed"
        } else if error.contains("verification") {
            "connector_verification_failed"
        } else if error.contains("refresh token") {
            "credential_renewal_unavailable"
        } else if error.contains("timed out") || error.contains("deadline") {
            "operation_deadline"
        } else if error.contains("transport") {
            "transport_failed"
        } else if error.contains("Cancelled") || error.contains("cancelled") {
            "expected_cancellation"
        } else {
            "operation_failed_details_withheld"
        };
        self.emit(Diagnostic {
            timestamp: chrono::Utc::now().to_rfc3339(),
            run: self.run.clone(),
            component,
            code,
            request,
            count: 0,
            http_status,
        });
    }

    pub fn usage_failure(&self) {
        self.usage_failed.store(true, Ordering::Release);
        self.record("accounting", "usage_capture_failed", None, 0);
    }

    pub async fn reserve(&self, request: u64) -> Result<bool, String> {
        self.reserve_on_day(request, chrono::Local::now().date_naive())
            .await
    }

    pub async fn reserve_on_day(&self, request: u64, day: NaiveDate) -> Result<bool, String> {
        let database = self.database.clone();
        let run = self.run.clone();
        let limit = self.limit.load(Ordering::Acquire);
        tokio::task::spawn_blocking(move || reserve_at(&database, &run, request, day, limit))
            .await
            .map_err(|_| "AI budget worker stopped")?
    }

    pub async fn observe(&self, request: u64, event: &str, metrics: Metrics) -> Result<(), String> {
        if event.is_empty()
            || event.len() > 128
            || !event
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err("Invalid SDK usage event identity".into());
        }
        let database = self.database.clone();
        let run = self.run.clone();
        let event = event.to_owned();
        tokio::task::spawn_blocking(move || {
            let connection = open_database(&database)?;
            let payload = serde_json::to_string(&metrics).map_err(|_| "Encode AI usage failed")?;
            connection
                .execute(
                    "INSERT INTO usage(run,request,event,payload)
                SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM requests WHERE run=?1 AND request=?2)
                ON CONFLICT(run,event) DO NOTHING",
                    params![run, request.to_string(), event, payload],
                )
                .map_err(|_| "Persist SDK usage failed")?;
            Ok(())
        })
        .await
        .map_err(|_| "AI usage worker stopped")?
    }

    pub async fn summary(&self) -> Result<Summary, String> {
        let database = self.database.clone();
        let limit = self.limit.load(Ordering::Acquire);
        tokio::task::spawn_blocking(move || {
            summary_at(&database, chrono::Local::now().date_naive(), limit)
        })
        .await
        .map_err(|_| "AI usage summary worker stopped")?
    }
}

fn open_database(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open(path).map_err(|_| "Open AI accounting failed")?;
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(|_| "Configure AI accounting failed")?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS requests(run TEXT NOT NULL,request TEXT NOT NULL,day TEXT NOT NULL,
            PRIMARY KEY(run,request));
        CREATE INDEX IF NOT EXISTS requests_day ON requests(day);
        CREATE TABLE IF NOT EXISTS usage(run TEXT NOT NULL,request TEXT NOT NULL,event TEXT NOT NULL,payload TEXT NOT NULL,
            PRIMARY KEY(run,event));").map_err(|_| "Initialize AI accounting failed")?;
    Ok(connection)
}

fn reserve_at(
    path: &Path,
    run: &str,
    request: u64,
    day: NaiveDate,
    limit: u32,
) -> Result<bool, String> {
    let mut connection = open_database(path)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "Lock daily AI budget failed")?;
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM requests WHERE run=?1 AND request=?2)",
            params![run, request.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| "Read AI reservation failed")?;
    if exists {
        return Ok(true);
    }
    let count: u32 = transaction
        .query_row(
            "SELECT count(*) FROM requests WHERE day=?1",
            [day.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| "Read daily AI request count failed")?;
    if count >= limit {
        return Ok(false);
    }
    transaction
        .execute(
            "INSERT INTO requests VALUES(?1,?2,?3)",
            params![run, request.to_string(), day.to_string()],
        )
        .map_err(|_| "Reserve daily AI request failed")?;
    transaction
        .commit()
        .map_err(|_| "Commit daily AI reservation failed")?;
    Ok(true)
}

fn summary_at(path: &Path, day: NaiveDate, limit: u32) -> Result<Summary, String> {
    let connection = open_database(path)?;
    let requests = connection
        .query_row(
            "SELECT count(*) FROM requests WHERE day=?1",
            [day.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| "Read daily AI count failed")?;
    let reported = connection
        .query_row(
            "SELECT count(DISTINCT r.run || ':' || r.request) FROM requests r
        JOIN usage u ON u.run=r.run AND u.request=r.request WHERE r.day=?1",
            [day.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| "Read measured AI count failed")?;
    let mut summary = Summary {
        day: day.to_string(),
        limit,
        requests,
        requests_with_usage: reported,
        ..Default::default()
    };
    let mut statement = connection
        .prepare(
            "SELECT u.payload FROM usage u JOIN requests r
        ON r.run=u.run AND r.request=u.request WHERE r.day=?1",
        )
        .map_err(|_| "Read AI metrics failed")?;
    let rows = statement
        .query_map([day.to_string()], |row| row.get::<_, String>(0))
        .map_err(|_| "Query AI metrics failed")?;
    for row in rows {
        let metrics: Metrics =
            serde_json::from_str(&row.map_err(|_| "Read AI metrics row failed")?)
                .map_err(|_| "Saved AI metrics are invalid")?;
        for (total, value) in [
            (&mut summary.tokens.input, metrics.input),
            (&mut summary.tokens.cache_read, metrics.cache_read),
            (&mut summary.tokens.cache_write, metrics.cache_write),
            (&mut summary.tokens.output, metrics.output),
        ] {
            if let Some(value) = value {
                *total = Some(
                    total
                        .unwrap_or(0)
                        .checked_add(value)
                        .ok_or("AI token total overflow")?,
                );
            }
        }
        if let Some(cost) = metrics.multiplier_cost {
            let total = summary.tokens.multiplier_cost.unwrap_or(0.0) + cost;
            if !cost.is_finite() || cost < 0.0 || !total.is_finite() {
                return Err("Invalid saved AI multiplier total".into());
            }
            summary.tokens.multiplier_cost = Some(total);
        }
        summary.usage_events += 1;
    }
    Ok(summary)
}

fn append_log(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| "Open diagnostic log failed".into())
}

fn write_log(path: &Path, file: &mut File, event: &Diagnostic) -> Result<(), String> {
    if file
        .metadata()
        .map_err(|_| "Inspect diagnostic log failed")?
        .len()
        >= LOG_LIMIT
    {
        let previous = path.with_extension("previous.jsonl");
        let older = path.with_extension("older.jsonl");
        file.flush().map_err(|_| "Flush diagnostic log failed")?;
        if previous.exists() {
            std::fs::copy(&previous, &older).map_err(|_| "Rotate older diagnostic log failed")?;
        }
        std::fs::copy(path, &previous).map_err(|_| "Rotate diagnostic log failed")?;
        *file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .map_err(|_| "Reset diagnostic log failed")?;
    }
    serde_json::to_writer(&mut *file, event).map_err(|_| "Serialize diagnostic event failed")?;
    file.write_all(b"\n")
        .and_then(|()| file.flush())
        .map_err(|_| "Write diagnostic log failed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_quota_is_atomic_durable_and_resets_by_local_day() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("usage.db");
        let day = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        assert!(reserve_at(&path, "first-run", 1, day, 2).unwrap());
        assert!(reserve_at(&path, "second-run", 1, day, 2).unwrap());
        assert!(!reserve_at(&path, "third-run", 1, day, 2).unwrap());
        assert!(reserve_at(&path, "first-run", 1, day, 2).unwrap());
        assert!(reserve_at(&path, "third-run", 1, day.succ_opt().unwrap(), 2).unwrap());
        assert_eq!(summary_at(&path, day, 2).unwrap().requests, 2);
    }

    #[tokio::test]
    async fn actual_sdk_usage_deduplicates_and_missing_fields_are_not_zero() {
        let root = tempfile::tempdir().unwrap();
        let service = Service::open(root.path()).unwrap();
        assert!(service.reserve(1).await.unwrap());
        let metrics = Metrics::from_sdk(
            &serde_json::json!({"model":"fixture","inputTokens":10,"cacheReadTokens":8,
            "cacheWriteTokens":2,"outputTokens":4,"cost":0.5}),
        )
        .unwrap();
        for _ in 0..2 {
            service
                .observe(1, "fixture-event", metrics.clone())
                .await
                .unwrap();
        }
        let summary = service.summary().await.unwrap();
        assert_eq!(summary.requests, 1);
        assert_eq!(summary.usage_events, 1);
        assert_eq!(summary.tokens.input, Some(10));
        assert_eq!(summary.tokens.multiplier_cost, Some(0.5));
        let absent = Metrics::from_sdk(&serde_json::json!({"model":"fixture"})).unwrap();
        assert_eq!(absent, Metrics::default());
        assert!(
            Metrics::from_sdk(&serde_json::json!({"model":"fixture","inputTokens":-1})).is_err()
        );
    }

    #[test]
    fn diagnostic_schema_only_contains_controlled_codes_and_numeric_context() {
        let event = Diagnostic {
            timestamp: "fixture-time".into(),
            run: "fixture-run".into(),
            component: "ai",
            code: "outcome_delivery_timeout",
            request: Some(42),
            count: 0,
            http_status: None,
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["request"], 42);
        assert!(value.get("message").is_none());
        assert!(value.get("prompt").is_none());
    }

    #[tokio::test]
    async fn structured_failures_are_flushed_without_remote_content_or_secrets() {
        let root = tempfile::tempdir().unwrap();
        let service = Service::open(root.path()).unwrap();
        service.failure(
            "connector",
            "HTTP 503 private meeting title secret-token",
            Some(42),
        );
        service.flush().await.unwrap();
        let text = std::fs::read_to_string(root.path().join("diagnostics.jsonl")).unwrap();
        let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(value["http_status"], 503);
        assert_eq!(value["request"], 42);
        assert!(!text.contains("private meeting"));
        assert!(!text.contains("secret-token"));
    }
}
