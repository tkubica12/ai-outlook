use serde_json::Value;
use std::{
    collections::BTreeSet,
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

pub const RECORD_LIMIT: usize = 16;
pub const EXCERPT_LIMIT: usize = 4000;
const LINK_LIMIT: usize = 4;
const URL_LIMIT: usize = 2048;

#[derive(Default)]
pub struct Redactor {
    secrets: Vec<Secret>,
}

struct Secret {
    value: String,
    prefix: Vec<usize>,
}

impl Redactor {
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        let mut values = BTreeSet::new();
        for secret in secrets.into_iter().filter(|secret| !secret.is_empty()) {
            let escaped = serde_json::to_string(&secret).expect("Strings serialize to JSON");
            values.insert(escaped[1..escaped.len() - 1].to_owned());
            values.insert(secret);
        }
        let mut secrets = values.into_iter().collect::<Vec<_>>();
        secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        let secrets = secrets
            .into_iter()
            .map(|value| {
                let bytes = value.as_bytes();
                let mut prefix = vec![0; bytes.len()];
                for index in 1..bytes.len() {
                    let mut matched = prefix[index - 1];
                    while matched > 0 && bytes[index] != bytes[matched] {
                        matched = prefix[matched - 1];
                    }
                    if bytes[index] == bytes[matched] {
                        matched += 1;
                    }
                    prefix[index] = matched;
                }
                Secret { value, prefix }
            })
            .collect();
        Self { secrets }
    }

    pub fn text(&self, text: &str) -> String {
        let mut positions = self
            .secrets
            .iter()
            .map(|secret| text.find(&secret.value))
            .collect::<Vec<_>>();
        let mut cursor = 0;
        let mut safe = String::with_capacity(text.len());
        loop {
            for (position, secret) in positions.iter_mut().zip(&self.secrets) {
                if position.is_some_and(|position| position < cursor) {
                    *position = text[cursor..]
                        .find(&secret.value)
                        .map(|position| cursor + position);
                }
            }
            let next = positions
                .iter()
                .enumerate()
                .filter_map(|(index, position)| position.map(|position| (position, index)))
                .min_by_key(|(position, index)| {
                    (
                        *position,
                        std::cmp::Reverse(self.secrets[*index].value.len()),
                    )
                });
            let Some((position, index)) = next else {
                safe.push_str(&text[cursor..]);
                return safe;
            };
            safe.push_str(&text[cursor..position]);
            safe.push_str("[REDACTED]");
            cursor = position + self.secrets[index].value.len();
        }
    }

    pub fn partial(&self, text: &str) -> String {
        let mut safe = self.text(text);
        // Withhold an incomplete credential prefix at streamed or truncated boundaries.
        let tail = self
            .secrets
            .iter()
            .map(|secret| {
                let pattern = secret.value.as_bytes();
                let start = safe.len().saturating_sub(pattern.len() - 1);
                let mut matched = 0;
                for byte in &safe.as_bytes()[start..] {
                    while matched > 0 && *byte != pattern[matched] {
                        matched = secret.prefix[matched - 1];
                    }
                    if *byte == pattern[matched] {
                        matched += 1;
                    }
                }
                matched
            })
            .max()
            .unwrap_or(0);
        if tail > 0 {
            safe.truncate(safe.len() - tail);
            safe.push_str("[REDACTED]");
        }
        safe
    }

    pub fn contains(&self, value: &str) -> bool {
        self.secrets
            .iter()
            .any(|secret| value.contains(&secret.value))
    }
}

#[derive(Clone, Debug)]
pub struct Record {
    pub ordinal: usize,
    pub tool: String,
    pub observed_at: String,
    pub succeeded: bool,
    pub excerpt: String,
    pub truncated: bool,
    pub links: Vec<String>,
}

#[derive(Clone, Default, Debug)]
pub struct Snapshot {
    pub records: Vec<Record>,
    pub omitted: usize,
}

impl Snapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.records.len() > RECORD_LIMIT
            || self.records.iter().any(|record| {
                record.tool.len() > 200
                    || record.observed_at.len() > 64
                    || record.excerpt.len() > EXCERPT_LIMIT
                    || record.links.len() > LINK_LIMIT
                    || record.links.iter().any(|link| !safe_source_url(link))
            })
        {
            return Err(
                "Tool evidence exceeded its bounds or contained an unsafe source link".into(),
            );
        }
        Ok(())
    }
}

pub fn safe_source_url(value: &str) -> bool {
    value.len() <= URL_LIMIT
        && crate::calendar::safe_url(value)
        && url::Url::parse(value).is_ok_and(|url| {
            !url.query_pairs().any(|(name, _)| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "access_token"
                        | "api_key"
                        | "apikey"
                        | "auth"
                        | "code"
                        | "key"
                        | "password"
                        | "secret"
                        | "sig"
                        | "signature"
                        | "token"
                )
            })
        })
}

pub struct Recorder {
    redactor: Arc<Redactor>,
    records: Mutex<Vec<Record>>,
    count: AtomicUsize,
    failed: AtomicBool,
}

impl Recorder {
    pub fn new(redactor: Arc<Redactor>) -> Self {
        Self {
            redactor,
            records: Mutex::new(Vec::new()),
            count: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
        }
    }

    pub fn fail(&self) {
        self.failed.store(true, Ordering::Release);
    }

    pub fn record(&self, tool: &str, value: &Value, succeeded: bool) {
        let ordinal = self.count.fetch_add(1, Ordering::AcqRel) + 1;
        if ordinal > RECORD_LIMIT {
            return;
        }
        let mut writer = BoundedWriter::default();
        if serde_json::to_writer(&mut writer, value).is_err() && !writer.truncated {
            self.fail();
            return;
        }
        let end = match std::str::from_utf8(&writer.bytes) {
            Ok(text) => text.len(),
            Err(error) => error.valid_up_to(),
        };
        let mut excerpt = self.redactor.partial(
            std::str::from_utf8(&writer.bytes[..end]).expect("UTF-8 boundary was validated"),
        );
        let expanded = excerpt.len() > EXCERPT_LIMIT;
        if expanded {
            let mut end = EXCERPT_LIMIT;
            while !excerpt.is_char_boundary(end) {
                end -= 1;
            }
            excerpt.truncate(end);
        }
        let mut links = BTreeSet::new();
        let mut visited = 0;
        if succeeded {
            collect_links(value, 0, &mut visited, &mut links, &self.redactor);
        }
        let record = Record {
            ordinal,
            tool: tool.into(),
            observed_at: chrono::Utc::now().to_rfc3339(),
            succeeded,
            excerpt,
            truncated: writer.truncated || expanded,
            links: links.into_iter().collect(),
        };
        match self.records.lock() {
            Ok(mut records) => records.push(record),
            Err(_) => self.fail(),
        }
    }

    pub fn snapshot(&self) -> Result<Arc<Snapshot>, String> {
        if self.failed.load(Ordering::Acquire) {
            return Err(
                "SDK tool evidence could not be recorded safely; no completed answer was published"
                    .into(),
            );
        }
        let mut records = self
            .records
            .lock()
            .map_err(|_| "SDK tool evidence is unavailable")?
            .clone();
        records.sort_by_key(|record| record.ordinal);
        let snapshot = Snapshot {
            records,
            omitted: self
                .count
                .load(Ordering::Acquire)
                .saturating_sub(RECORD_LIMIT),
        };
        snapshot.validate()?;
        Ok(Arc::new(snapshot))
    }
}

#[derive(Default)]
struct BoundedWriter {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let remaining = EXCERPT_LIMIT - self.bytes.len();
        self.bytes
            .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        if bytes.len() > remaining {
            self.truncated = true;
            return Err(io::Error::other("Tool excerpt reached its byte limit"));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn collect_links(
    value: &Value,
    depth: usize,
    visited: &mut usize,
    links: &mut BTreeSet<String>,
    redactor: &Redactor,
) {
    if depth > 8 || *visited >= 512 || links.len() >= LINK_LIMIT {
        return;
    }
    *visited += 1;
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                if matches!(
                    name.as_str(),
                    "url" | "webUrl" | "webLink" | "source_url" | "sourceUrl"
                ) && let Some(link) = value.as_str()
                    && safe_source_url(link)
                    && !redactor.contains(link)
                {
                    links.insert(link.into());
                }
                collect_links(value, depth + 1, visited, links, redactor);
                if *visited >= 512 || links.len() >= LINK_LIMIT {
                    break;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_links(value, depth + 1, visited, links, redactor);
                if *visited >= 512 || links.len() >= LINK_LIMIT {
                    break;
                }
            }
        }
        Value::String(text) if text.len() <= 8192 => {
            if let Ok(structured) = serde_json::from_str::<Value>(text)
                && matches!(structured, Value::Object(_) | Value::Array(_))
            {
                collect_links(&structured, depth + 1, visited, links, redactor);
            }
        }
        _ => {}
    }
}
