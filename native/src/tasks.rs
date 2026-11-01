//! Local task list: manual tasks, personal tasks imported from post-meeting notes and task
//! drafts from Ask. Tasks stay on this computer; nothing is written to To Do, Planner,
//! Dataverse or any other remote service.
use crate::{calendar::Event, i18n::Language};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Characters of a single-line task title.
pub const TITLE_LIMIT: usize = 300;
/// Characters of a task note.
pub const NOTE_LIMIT: usize = 2_000;
/// Stored tasks per account.
pub const TASK_LIMIT: usize = 5_000;
/// Completed tasks remain listed for this many days.
pub const DONE_DAYS: i64 = 30;
/// Personal tasks imported from one meeting's notes.
pub const IMPORT_LIMIT: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Open,
    Done,
}

impl Status {
    pub fn code(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Manual,
    MeetingNotes,
    AskDraft,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "Added manually",
            Self::MeetingNotes => "From meeting notes",
            Self::AskDraft => "From Ask",
        }
    }
}

/// Snapshot of the meeting a task came from; `key` is the meeting-notes occurrence key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meeting {
    pub key: String,
    pub event_id: String,
    pub title: String,
    pub start: DateTime<FixedOffset>,
}

impl Meeting {
    pub fn new(account: &str, event: &Event) -> Self {
        Self {
            key: crate::recaps::key(account, event),
            event_id: event.id.clone(),
            title: clip(&single_line(&event.title), TITLE_LIMIT),
            start: event.start,
        }
    }

    /// The occurrence key may change when calendar metadata is enriched; the event identity remains.
    pub fn matches(&self, account: &str, event: &Event) -> bool {
        self.event_id == event.id || self.key == crate::recaps::key(account, event)
    }

    fn validate(&self) -> Result<(), String> {
        if !hex64(&self.key)
            || self.event_id.is_empty()
            || self.event_id.len() > 4096
            || self.event_id.chars().any(char::is_control)
            || self.title.chars().count() > TITLE_LIMIT
            || self.title.chars().any(char::is_control)
        {
            return Err("Saved task meeting link is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Storage row identity; not part of the saved payload.
    #[serde(skip)]
    pub id: i64,
    pub account: String,
    pub title: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub due: Option<NaiveDate>,
    pub status: Status,
    #[serde(default)]
    pub done_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub source: Source,
    #[serde(default)]
    pub meeting: Option<Meeting>,
    /// Index of the imported item in the notes' personal task list.
    #[serde(default)]
    pub item: Option<u32>,
    /// Revision of the notes' personal task lists when the task was imported.
    #[serde(default)]
    pub revision: Option<String>,
    /// Deduplication identity of an imported item: account, occurrence, revision and index.
    #[serde(default)]
    pub import_key: Option<String>,
    /// Normalized text digests of both language variants of an imported item.
    #[serde(default)]
    pub hashes: Vec<String>,
    /// Language of the title.
    pub language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Overdue,
    Today,
    Upcoming,
    NoDate,
    Done,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overdue => "Overdue",
            Self::Today => "Due today",
            Self::Upcoming => "Upcoming",
            Self::NoDate => "No due date",
            Self::Done => "Done",
        }
    }
}

impl Task {
    pub fn validate(&self) -> Result<(), String> {
        validate_title(&self.title)?;
        validate_note(&self.note)?;
        if self.account.is_empty()
            || self.account.len() > 320
            || self.account.chars().any(char::is_control)
        {
            return Err("Task account is invalid".into());
        }
        if (self.status == Status::Done) != self.done_at.is_some() {
            return Err("Task completion state is inconsistent".into());
        }
        if let Some(meeting) = &self.meeting {
            meeting.validate()?;
        }
        let imported = self.source == Source::MeetingNotes;
        if imported
            != (self.import_key.is_some() && self.item.is_some() && self.revision.is_some())
            || (imported && self.meeting.is_none())
            || self.import_key.as_deref().is_some_and(|key| !hex64(key))
            || self.revision.as_deref().is_some_and(|revision| !hex64(revision))
            || self.hashes.len() > 2
            || self.hashes.iter().any(|hash| !hex64(hash))
        {
            return Err("Saved task source is invalid".into());
        }
        Ok(())
    }

    pub fn group(&self, today: NaiveDate) -> Group {
        if self.status == Status::Done {
            return Group::Done;
        }
        match self.due {
            Some(due) if due < today => Group::Overdue,
            Some(due) if due == today => Group::Today,
            Some(_) => Group::Upcoming,
            None => Group::NoDate,
        }
    }

    /// Open tasks and tasks completed in the last 30 days are listed.
    pub fn listed(&self, now: DateTime<Utc>) -> bool {
        self.status == Status::Open
            || self
                .done_at
                .is_some_and(|done| done > now - Duration::days(DONE_DAYS))
    }

    pub fn edited(
        &self,
        title: &str,
        note: &str,
        due: Option<NaiveDate>,
        now: DateTime<Utc>,
    ) -> Result<Self, String> {
        let mut task = self.clone();
        task.title = title.trim().to_owned();
        task.note = normalize_note(note);
        task.due = due;
        task.updated_at = now.max(self.created_at);
        task.validate()?;
        Ok(task)
    }

    pub fn completed(&self, done: bool, now: DateTime<Utc>) -> Self {
        let mut task = self.clone();
        task.status = if done { Status::Done } else { Status::Open };
        task.done_at = done.then(|| self.done_at.unwrap_or(now));
        task.updated_at = now.max(self.created_at);
        task
    }
}

/// A task the user adds manually or from an Ask draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct New {
    pub title: String,
    pub note: String,
    pub due: Option<NaiveDate>,
    pub source: Source,
    pub meeting: Option<Meeting>,
    pub language: Language,
}

impl New {
    pub fn manual(title: &str, due: Option<NaiveDate>, language: Language) -> Result<Self, String> {
        let title = title.trim().to_owned();
        validate_title(&title)?;
        Ok(Self {
            title,
            due,
            language,
            ..Default::default()
        })
    }

    /// Title is the draft title; the note keeps the draft text and its proposed target.
    pub fn from_draft(
        proposal: &crate::proposals::Proposal,
        meeting: Option<Meeting>,
        language: Language,
    ) -> Result<Self, String> {
        if proposal.kind != crate::proposals::Kind::Task {
            return Err("Only task drafts can be added to tasks".into());
        }
        proposal.validate()?;
        let target = match language {
            Language::En => "Target",
            Language::Cs => "Cíl",
        };
        let note = format!(
            "{}\n\n{target}: {}",
            normalize_note(&proposal.body).trim(),
            single_line(&proposal.target)
        );
        Ok(Self {
            title: clip(&single_line(&proposal.title), TITLE_LIMIT),
            note: clip(&note, NOTE_LIMIT),
            due: None,
            source: Source::AskDraft,
            meeting,
            language,
        })
    }

    pub fn task(&self, account: &str, now: DateTime<Utc>) -> Result<Task, String> {
        if self.source == Source::MeetingNotes {
            return Err("Tasks from meeting notes are imported from saved notes only".into());
        }
        let task = Task {
            id: 0,
            account: account.into(),
            title: self.title.trim().into(),
            note: normalize_note(&self.note),
            due: self.due,
            status: Status::Open,
            done_at: None,
            created_at: now,
            updated_at: now,
            source: self.source,
            meeting: self.meeting.clone(),
            item: None,
            revision: None,
            import_key: None,
            hashes: Vec::new(),
            language: self.language,
        };
        task.validate()?;
        Ok(task)
    }
}

/// Identity of an Ask draft, used to avoid adding the same draft twice in one session.
pub fn draft_key(proposal: &crate::proposals::Proposal) -> String {
    digest(&(
        "tomlook-ask-draft-v1",
        &proposal.title,
        &proposal.target,
        &proposal.body,
    ))
}

/// Revision of the notes' personal task lists in both languages.
pub fn notes_revision(payload: &Value) -> String {
    digest(&(
        "tomlook-notes-tasks-v1",
        payload.get("preparation"),
        payload.get("cs").and_then(|czech| czech.get("preparation")),
    ))
}

/// Personal tasks (`preparation`) from saved post-meeting notes, ready for deduplicated import.
/// The title uses the UI language when that variant exists; the other language goes to the note.
/// Mentions and talking points are never imported. A due date is set only when the item contains
/// exactly one explicit calendar date.
pub fn from_notes(
    account: &str,
    event: &Event,
    payload: &Value,
    language: Language,
    now: DateTime<Utc>,
) -> Result<Vec<Task>, String> {
    if payload.get("tomlook_recap").and_then(Value::as_bool) != Some(true) {
        return Err("Only saved meeting notes can create tasks".into());
    }
    let Some(english) = payload.get("preparation").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let czech = payload
        .get("cs")
        .and_then(|czech| czech.get("preparation"))
        .and_then(Value::as_array);
    let meeting = Meeting::new(account, event);
    let revision = notes_revision(payload);
    let mut tasks = Vec::new();
    fn text(value: Option<&Value>) -> Option<&str> {
        value
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }
    for (index, item) in english.iter().enumerate().take(IMPORT_LIMIT) {
        let english = text(Some(item));
        let czech = text(czech.and_then(|items| items.get(index)));
        let (primary, other, title_language) = match (language, english, czech) {
            (Language::Cs, _, Some(czech)) => (czech, english, Language::Cs),
            (_, Some(english), _) => (english, czech, Language::En),
            (_, None, Some(czech)) => (czech, None, Language::Cs),
            _ => continue,
        };
        let line = single_line(&without_label(primary));
        let title = clip(&line, TITLE_LIMIT);
        if title.is_empty() {
            continue;
        }
        let mut note = Vec::new();
        if title != line {
            note.push(normalize_note(primary));
        }
        if let Some(other) = other
            .map(without_label)
            .filter(|other| single_line(other) != line)
        {
            note.push(normalize_note(&other));
        }
        let mut dates = explicit_dates(primary);
        if let Some(other) = other {
            dates.extend(explicit_dates(other));
        }
        let task = Task {
            id: 0,
            account: account.into(),
            title,
            note: clip(&note.join("\n\n"), NOTE_LIMIT),
            due: (dates.len() == 1).then(|| *dates.first().expect("one date")),
            status: Status::Open,
            done_at: None,
            created_at: now,
            updated_at: now,
            source: Source::MeetingNotes,
            meeting: Some(meeting.clone()),
            item: Some(index as u32),
            revision: Some(revision.clone()),
            import_key: Some(digest(&(
                "tomlook-task-import-v1",
                account,
                &meeting.key,
                &revision,
                index,
            ))),
            hashes: english
                .into_iter()
                .chain(czech)
                .map(text_hash)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            language: title_language,
        };
        task.validate()?;
        tasks.push(task);
    }
    Ok(tasks)
}

/// Notes items start with a short heading such as "My task:"; the task title keeps only the
/// capitalized text after it. URLs and longer leading clauses are left unchanged.
pub fn without_label(text: &str) -> String {
    match text.split_once(':') {
        Some((label, rest))
            if !rest.trim().is_empty()
                && !rest.starts_with("//")
                && label.chars().count() <= 40
                && label.split_whitespace().count() <= 4
                && !label.chars().any(|ch| ch.is_ascii_digit()) =>
        {
            let mut chars = rest.trim_start().chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
        _ => text.to_owned(),
    }
}

/// Explicit dates written with a four-digit year: ISO `2026-11-04` or `4. 11. 2026` / `4.11.2026`.
/// Ambiguous forms such as `11/04/2026` or weekday names are deliberately not interpreted.
pub fn explicit_dates(text: &str) -> BTreeSet<NaiveDate> {
    let chars = text.chars().collect::<Vec<_>>();
    let digits = |start: usize| {
        chars[start..]
            .iter()
            .take_while(|ch| ch.is_ascii_digit())
            .count()
    };
    let number = |start: usize, length: usize| {
        chars[start..start + length]
            .iter()
            .collect::<String>()
            .parse::<u32>()
            .ok()
    };
    let mut found = BTreeSet::new();
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_ascii_digit() || (index > 0 && chars[index - 1].is_ascii_digit()) {
            index += 1;
            continue;
        }
        let first = digits(index);
        let mut cursor = index + first;
        let separator = chars.get(cursor).copied();
        let date = if first == 4 && separator == Some('-') {
            (|| {
                let month_start = cursor + 1;
                (digits(month_start) == 2 && chars.get(month_start + 2) == Some(&'-'))
                    .then_some(())?;
                let day_start = month_start + 3;
                (digits(day_start) == 2).then_some(())?;
                cursor = day_start + 2;
                NaiveDate::from_ymd_opt(
                    number(index, 4)? as i32,
                    number(month_start, 2)?,
                    number(day_start, 2)?,
                )
            })()
        } else if (1..=2).contains(&first) && separator == Some('.') {
            (|| {
                let skip = |position: usize| position + usize::from(chars.get(position) == Some(&' '));
                let month_start = skip(cursor + 1);
                let month = digits(month_start);
                ((1..=2).contains(&month) && chars.get(month_start + month) == Some(&'.'))
                    .then_some(())?;
                let year_start = skip(month_start + month + 1);
                (digits(year_start) == 4).then_some(())?;
                cursor = year_start + 4;
                NaiveDate::from_ymd_opt(
                    number(year_start, 4)? as i32,
                    number(month_start, month)?,
                    number(index, first)?,
                )
            })()
        } else {
            None
        };
        found.extend(date);
        index = cursor.max(index + 1);
    }
    found
}

/// Open tasks by urgency (overdue, today, upcoming, no date), then tasks completed in the last
/// 30 days, newest first.
pub fn arranged(tasks: &[Task], today: NaiveDate, now: DateTime<Utc>) -> Vec<(Group, &Task)> {
    let mut rows = tasks
        .iter()
        .filter(|task| task.listed(now))
        .map(|task| (task.group(today), task))
        .collect::<Vec<_>>();
    rows.sort_by(|(group, task), (other_group, other)| {
        group
            .cmp(other_group)
            .then_with(|| {
                if *group == Group::Done {
                    other.done_at.cmp(&task.done_at)
                } else {
                    task.due
                        .cmp(&other.due)
                        .then(task.created_at.cmp(&other.created_at))
                }
            })
            .then(task.id.cmp(&other.id))
    });
    rows
}

pub fn open_count(tasks: &[Task], account: &str) -> usize {
    tasks
        .iter()
        .filter(|task| task.account == account && task.status == Status::Open)
        .count()
}

/// Tasks linked to one meeting occurrence, in import order.
pub fn for_meeting<'a>(tasks: &'a [Task], account: &str, event: &Event) -> Vec<&'a Task> {
    let key = crate::recaps::key(account, event);
    let mut linked = tasks
        .iter()
        .filter(|task| {
            task.account == account
                && task
                    .meeting
                    .as_ref()
                    .is_some_and(|meeting| meeting.event_id == event.id || meeting.key == key)
        })
        .collect::<Vec<_>>();
    linked.sort_by_key(|task| (task.source != Source::MeetingNotes, task.item, task.id));
    linked
}

pub fn validate_title(title: &str) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty()
        || title.chars().count() > TITLE_LIMIT
        || title.chars().any(char::is_control)
    {
        return Err("Task title must have 1-300 characters on one line".into());
    }
    Ok(())
}

pub fn validate_note(note: &str) -> Result<(), String> {
    if note.chars().count() > NOTE_LIMIT
        || note
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
    {
        return Err("Task note can have at most 2,000 characters".into());
    }
    Ok(())
}

/// Collapses whitespace and control characters into single spaces.
pub fn single_line(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Shortens text to `limit` characters, marking the cut with an ellipsis.
pub fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut clipped = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>()
        .trim_end()
        .to_owned();
    clipped.push('…');
    clipped
}

fn normalize_note(note: &str) -> String {
    note.replace("\r\n", "\n")
        .chars()
        .map(|ch| match ch {
            '\r' => '\n',
            ch if ch.is_control() && !matches!(ch, '\n' | '\t') => ' ',
            ch => ch,
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Digest of normalized text; the same item in another revision of the notes has the same digest.
pub fn text_hash(text: &str) -> String {
    digest(&("tomlook-task-text-v1", single_line(text).to_lowercase()))
}

fn digest(value: &impl Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("task identity serializes"))
    )
}

fn hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn notes() -> Value {
        json!({
            "tomlook_recap": true,
            "summary": "Pilot agreed.",
            "preparation": [
                "My task: send the architecture comparison to Alex by 2026-11-04",
                "My task: confirm the pilot owner",
                "",
                "My task: book a review on 4. 11. 2026 or 2026-11-06"
            ],
            "talking_points": ["Shared material: https://example.com/deck"],
            "open_questions": ["Mention of me: Jamie asked about the budget"],
            "risks": [], "gaps": [], "sources": [],
            "cs": {
                "summary": "Pilot dohodnut.",
                "preparation": [
                    "Můj úkol: poslat Alexovi porovnání architektury do 2026-11-04",
                    "Můj úkol: potvrdit vlastníka pilotu",
                    "",
                    "Můj úkol: domluvit revizi 4. 11. 2026 nebo 2026-11-06"
                ],
                "talking_points": ["Sdílený materiál: https://example.com/deck"],
                "open_questions": ["Zmínka: Jamie se ptal na rozpočet"],
                "risks": [], "gaps": []
            }
        })
    }

    pub(crate) fn meeting() -> Event {
        let mut event = crate::calendar::fixture(1).remove(0);
        event.title = "Pilot planning".into();
        event
    }

    #[test]
    fn notes_import_uses_the_ui_language_keeps_the_other_variant_and_never_imports_mentions() {
        let now = Utc::now();
        let english = from_notes("user@example.com", &meeting(), &notes(), Language::En, now).unwrap();
        assert_eq!(english.len(), 3, "Empty items are skipped");
        assert_eq!(english[0].title, "Send the architecture comparison to Alex by 2026-11-04");
        assert_eq!(english[0].note, "Poslat Alexovi porovnání architektury do 2026-11-04");
        assert_eq!(english[0].language, Language::En);
        assert_eq!(english[0].source, Source::MeetingNotes);
        assert_eq!(english[0].due, NaiveDate::from_ymd_opt(2026, 11, 4));
        assert_eq!(english[1].due, None, "No date is guessed");
        assert_eq!(english[2].due, None, "Two different dates are ambiguous");
        assert_eq!(english[2].item, Some(3));
        let czech = from_notes("user@example.com", &meeting(), &notes(), Language::Cs, now).unwrap();
        assert_eq!(czech[1].title, "Potvrdit vlastníka pilotu");
        assert_eq!(czech[1].note, "Confirm the pilot owner");
        assert_eq!(czech[1].language, Language::Cs);
        assert_eq!(czech[1].import_key, english[1].import_key, "Language does not change identity");
        assert_eq!(czech[1].hashes, english[1].hashes);
        for task in english.iter().chain(&czech) {
            assert!(!task.title.contains("Mention") && !task.title.contains("Zmínka"));
            assert!(!task.title.contains("Shared material"));
            task.validate().unwrap();
        }
        let mut without_czech = notes();
        without_czech.as_object_mut().unwrap().remove("cs");
        let fallback = from_notes("user@example.com", &meeting(), &without_czech, Language::Cs, now).unwrap();
        assert_eq!(fallback[1].title, "Confirm the pilot owner");
        assert_eq!(fallback[1].language, Language::En);
        assert!(fallback[1].note.is_empty());
        let mut insights = notes();
        insights["tomlook_recap"] = false.into();
        assert!(from_notes("user@example.com", &meeting(), &insights, Language::En, now).is_err());
    }

    #[test]
    fn task_titles_drop_short_note_headings_but_keep_urls_and_clauses() {
        assert_eq!(without_label("My task: send the deck"), "Send the deck");
        assert_eq!(without_label("Můj úkol: poslat prezentaci"), "Poslat prezentaci");
        assert_eq!(without_label("https://example.com/plan"), "https://example.com/plan");
        assert_eq!(without_label("Review at 10:30 with Alex"), "Review at 10:30 with Alex");
        assert_eq!(
            without_label("Send the comparison to the whole steering group: today"),
            "Send the comparison to the whole steering group: today"
        );
        assert_eq!(without_label("prepare the agenda"), "prepare the agenda");
    }

    #[test]
    fn long_items_keep_their_full_text_and_identity_depends_on_revision() {
        let now = Utc::now();
        let mut payload = notes();
        let long = format!("My task: {}", "deliver the detailed plan ".repeat(30));
        payload["preparation"][1] = long.clone().into();
        let tasks = from_notes("user@example.com", &meeting(), &payload, Language::En, now).unwrap();
        assert_eq!(tasks[1].title.chars().count(), TITLE_LIMIT);
        assert!(tasks[1].title.ends_with('…'));
        assert!(tasks[1].note.starts_with(long.trim()));
        let original = from_notes("user@example.com", &meeting(), &notes(), Language::En, now).unwrap();
        assert_ne!(original[0].revision, tasks[0].revision);
        assert_ne!(original[0].import_key, tasks[0].import_key);
        assert_eq!(original[0].hashes, tasks[0].hashes, "Unchanged items keep their text digest");
        assert_ne!(
            from_notes("other@example.com", &meeting(), &notes(), Language::En, now).unwrap()[0].import_key,
            original[0].import_key
        );
    }

    #[test]
    fn only_explicit_dates_with_a_year_are_recognized() {
        let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(explicit_dates("by 2026-11-04."), [date(2026, 11, 4)].into());
        assert_eq!(explicit_dates("do 4. 11. 2026"), [date(2026, 11, 4)].into());
        assert_eq!(explicit_dates("do 4.11.2026 a 2026-11-04T10:00"), [date(2026, 11, 4)].into());
        for vague in ["by Friday", "11/04/2026", "on 4. 11.", "2026-13-01", "12026-11-04", "31. 2. 2026", "v 10.30"] {
            assert!(explicit_dates(vague).is_empty(), "{vague}");
        }
        assert_eq!(explicit_dates("1. 2. 2026 and 2026-03-04").len(), 2);
    }

    #[test]
    fn listing_orders_open_tasks_by_urgency_then_recent_done_tasks() {
        let now = Utc::now();
        let today = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        let task = |id, title: &str, due: Option<NaiveDate>| {
            let mut task = New::manual(title, due, Language::En).unwrap().task("a@example.com", now).unwrap();
            task.id = id;
            task
        };
        let mut done = task(5, "Done recently", None).completed(true, now - Duration::days(2));
        done.done_at = Some(now - Duration::days(2));
        let mut old = task(6, "Done long ago", None).completed(true, now);
        old.done_at = Some(now - Duration::days(31));
        let tasks = vec![
            task(1, "No date", None),
            task(2, "Upcoming", Some(today + Duration::days(3))),
            task(3, "Today", Some(today)),
            task(4, "Overdue", Some(today - Duration::days(1))),
            done,
            old,
        ];
        let arranged = arranged(&tasks, today, now);
        assert_eq!(
            arranged.iter().map(|(group, task)| (*group, task.title.as_str())).collect::<Vec<_>>(),
            vec![
                (Group::Overdue, "Overdue"),
                (Group::Today, "Today"),
                (Group::Upcoming, "Upcoming"),
                (Group::NoDate, "No date"),
                (Group::Done, "Done recently"),
            ]
        );
        assert_eq!(open_count(&tasks, "a@example.com"), 4);
        assert_eq!(open_count(&tasks, "b@example.com"), 0);
    }

    #[test]
    fn edits_and_completion_are_validated_and_reversible() {
        let now = Utc::now();
        let task = New::manual("  Send notes  ", None, Language::En).unwrap().task("a@example.com", now).unwrap();
        assert_eq!(task.title, "Send notes");
        assert!(New::manual(" ", None, Language::En).is_err());
        assert!(New::manual(&"x".repeat(301), None, Language::En).is_err());
        assert!(task.edited("Two\nlines", "", None, now).is_err());
        assert!(task.edited("Ok", &"n".repeat(2001), None, now).is_err());
        let edited = task.edited("Send the notes", "Line one\r\nLine two", NaiveDate::from_ymd_opt(2026, 11, 2), now).unwrap();
        assert_eq!(edited.note, "Line one\nLine two");
        let done = edited.completed(true, now);
        assert_eq!((done.status, done.done_at.is_some()), (Status::Done, true));
        done.validate().unwrap();
        let reopened = done.completed(false, now);
        assert_eq!((reopened.status, reopened.done_at), (Status::Open, None));
        reopened.validate().unwrap();
        let mut inconsistent = reopened.clone();
        inconsistent.done_at = Some(now);
        assert!(inconsistent.validate().is_err());
    }

    #[test]
    fn ask_task_drafts_become_linked_local_tasks_and_other_kinds_are_refused() {
        use crate::proposals::{Kind, Proposal};
        let now = Utc::now();
        let proposal = Proposal {
            kind: Kind::Task,
            target: "Local task for the pilot".into(),
            title: "Prepare the pilot checklist".into(),
            body: "List the three success criteria.\nShare them with Jamie.".into(),
        };
        let event = meeting();
        let new = New::from_draft(&proposal, Some(Meeting::new("a@example.com", &event)), Language::En).unwrap();
        let task = new.task("a@example.com", now).unwrap();
        assert_eq!(task.title, "Prepare the pilot checklist");
        assert_eq!(task.note, "List the three success criteria.\nShare them with Jamie.\n\nTarget: Local task for the pilot");
        assert_eq!(task.source, Source::AskDraft);
        assert!(task.meeting.as_ref().unwrap().matches("a@example.com", &event));
        assert_eq!(for_meeting(std::slice::from_ref(&task), "a@example.com", &event).len(), 1);
        let czech = New::from_draft(&proposal, None, Language::Cs).unwrap();
        assert!(czech.note.ends_with("Cíl: Local task for the pilot"));
        assert!(czech.meeting.is_none());
        assert!(New::from_draft(&Proposal { kind: Kind::Email, ..proposal.clone() }, None, Language::En).is_err());
        assert!(New::from_draft(&Proposal { title: String::new(), ..proposal.clone() }, None, Language::En).is_err());
        assert_ne!(draft_key(&proposal), draft_key(&Proposal { body: "Other".into(), ..proposal }));
    }
}
