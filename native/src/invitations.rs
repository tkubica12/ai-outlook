//! Account-owned invitation work queue. Classification and local choices never write to Outlook.
use crate::{
    calendar::{Calendar, Event, person_email},
    customers,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};

pub const RECORD_LIMIT: usize = 2_000;
pub const GROUP_LIMIT: usize = 16;
pub const MEMBER_LIMIT: usize = 5_000;
pub const PAYLOAD_LIMIT: usize = 16 * 1024 * 1024;
pub const REFRESH_SECONDS: u64 = 180;
pub const TEAM_REFRESH_SECONDS: u64 = 24 * 60 * 60;

fn email(value: &str) -> bool {
    person_email(value).as_deref() == Some(value)
}

fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

pub fn key(event: &Event) -> String {
    let identity = event.extra.get("seriesMasterId").and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| event.extra.get("iCalUId").and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty()))
        .unwrap_or(&event.id);
    format!("{:x}", Sha256::digest(identity.as_bytes()))
}

pub fn revision(event: &Event) -> String {
    let mut digest = Sha256::new();
    for value in [&event.title, &event.start.to_rfc3339(), &event.end.to_rfc3339(),
        &event.organizer, &event.location]
    {
        digest.update(value.len().to_le_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub fn pending(event: &Event, now: DateTime<Utc>) -> bool {
    event.end > now && !event.following()
        && event.extra.get("isCancelled").and_then(serde_json::Value::as_bool) != Some(true)
        && event.extra.get("isOrganizer").and_then(serde_json::Value::as_bool) == Some(false)
        && matches!(event.status.to_ascii_lowercase().as_str(), "notresponded" | "none")
        && matches!(event.extra.get("type").and_then(serde_json::Value::as_str),
            Some("singleInstance" | "occurrence" | "exception"))
}

pub fn current<'a>(event: &'a Event, calendar: &'a Calendar,
    freshness: &crate::calendar_sync::Freshness, since: DateTime<Utc>, now: DateTime<Utc>,
) -> Option<&'a Event> {
    let (first, last) = event.calendar_dates();
    let newer = (0..=(last - first).num_days()).all(|offset| {
        let day = first + Duration::days(offset);
        calendar.coverage.contains(&day) && freshness.get(&day)
            .is_some_and(|stamp| *stamp > since && *stamp <= now)
    });
    if newer { calendar.event_index(&event.id).map(|index| &calendar.events[index]) }
    else { Some(event) }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub event: Event,
    pub first_seen: DateTime<Utc>,
    pub updated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub mail: String,
    pub enabled: bool,
    pub members: BTreeSet<String>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl Group {
    pub fn validate(&self) -> Result<(), String> {
        if !crate::msx::auth::uuid(&self.id) || !text(&self.name, 500) || !email(&self.mail)
            || self.members.len() > MEMBER_LIMIT || self.members.iter().any(|value| !email(value))
            || self.error.as_ref().is_some_and(|error| !text(error, 2_000))
        {
            return Err("Saved invitation team group is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrganizerRule {
    Important,
    Quiet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snooze {
    pub until: DateTime<Utc>,
    pub revision: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub account: String,
    pub records: BTreeMap<String, Record>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub people: BTreeSet<String>,
    pub excluded_people: BTreeSet<String>,
    pub groups: BTreeMap<String, Group>,
    pub discovery_queries: Vec<String>,
    pub organizers: BTreeMap<String, OrganizerRule>,
    pub quiet_series: BTreeSet<String>,
    pub snoozed: BTreeMap<String, Snooze>,
}

impl Book {
    pub fn new(account: &str) -> Self {
        Self { account: account.into(), ..Default::default() }
    }

    pub fn validate(&self) -> Result<(), String> {
        let valid_key = |key: &str| key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit());
        if (!email(&self.account) && self.account != crate::feedback::PREVIEW_ACCOUNT) || self.records.len() > RECORD_LIMIT
            || self.groups.len() > GROUP_LIMIT || self.people.len() > MEMBER_LIMIT
            || self.excluded_people.len() > MEMBER_LIMIT || self.organizers.len() > MEMBER_LIMIT
            || self.quiet_series.len() > MEMBER_LIMIT || self.snoozed.len() > MEMBER_LIMIT
            || self.people.iter().chain(&self.excluded_people).any(|value| !email(value))
            || self.organizers.keys().any(|value| !email(value))
            || self.quiet_series.iter().any(|key| !valid_key(key))
            || self.snoozed.iter().any(|(key, value)| !valid_key(key) || !valid_key(&value.revision))
        {
            return Err("Saved invitation queue is invalid or exceeds its safety limit".into());
        }
        validate_queries(&self.discovery_queries)?;
        for (id, group) in &self.groups {
            group.validate()?;
            if id != &group.id { return Err("Saved invitation team identity changed".into()); }
        }
        for (id, record) in &self.records {
            record.event.validate()?;
            if *id != key(&record.event) {
                return Err("Saved invitation identity changed".into());
            }
        }
        Ok(())
    }

    pub fn replace(&mut self, events: Vec<Event>, now: DateTime<Utc>) -> Result<(), String> {
        let mut records: BTreeMap<String, Record> = BTreeMap::new();
        for event in events.into_iter().filter(|event| pending(event, now)) {
            event.validate()?;
            let id = key(&event);
            if records.get(&id).is_some_and(|record| record.event.start <= event.start) { continue; }
            let previous = self.records.get(&id);
            let updated = previous.is_some_and(|record| revision(&record.event) != revision(&event));
            let first_seen = previous.filter(|_| !updated).map_or(now, |record| record.first_seen);
            records.insert(id, Record { event, first_seen, updated });
        }
        if records.len() > RECORD_LIMIT { return Err("Invitation queue exceeds 2,000 meetings".into()); }
        self.records = records;
        self.refreshed_at = Some(now);
        self.snoozed.retain(|key, snooze| snooze.until > now && self.records.get(key)
            .is_some_and(|record| snooze.revision == revision(&record.event)));
        self.validate()
    }

    pub fn team(&self) -> BTreeSet<String> {
        self.people.iter().chain(self.groups.values().filter(|group| group.enabled)
            .flat_map(|group| group.members.iter()))
            .filter(|email| !self.excluded_people.contains(*email)).cloned().collect()
    }

    pub fn merge_groups(&mut self, groups: Vec<Group>, discovered: bool) -> Result<(), String> {
        for mut group in groups {
            group.validate()?;
            if let Some(previous) = self.groups.get(&group.id) {
                group.enabled = previous.enabled;
                if discovered || group.error.is_some() {
                    group.members = previous.members.clone();
                    group.refreshed_at = previous.refreshed_at;
                    if discovered { group.error = previous.error.clone(); }
                }
            } else if !discovered {
                continue;
            }
            self.groups.insert(group.id.clone(), group);
        }
        self.validate()
    }

    pub fn apply(&mut self, action: LocalAction, now: DateTime<Utc>) -> Result<(), String> {
        match action {
            LocalAction::Snooze { key, until } => {
                let record = self.records.get(&key).ok_or("Invitation is no longer pending")?;
                if until <= now || until > now + Duration::days(30) {
                    return Err("Snooze must be in the next 30 days".into());
                }
                self.snoozed.insert(key, Snooze { until, revision: revision(&record.event) });
            }
            LocalAction::Unsnooze(key) => { self.snoozed.remove(&key); }
            LocalAction::QuietSeries { key, quiet } => {
                if !self.records.contains_key(&key) { return Err("Invitation is no longer pending".into()); }
                if quiet { self.quiet_series.insert(key); } else { self.quiet_series.remove(&key); }
            }
            LocalAction::Organizer { email: address, rule } => {
                if !email(&address) { return Err("Organizer email address is invalid".into()); }
                if let Some(rule) = rule { self.organizers.insert(address, rule); }
                else { self.organizers.remove(&address); }
            }
            LocalAction::People { included, excluded } => {
                self.people = included;
                self.excluded_people = excluded;
            }
            LocalAction::GroupEnabled { id, enabled } => {
                self.groups.get_mut(&id).ok_or("Team group is no longer available")?.enabled = enabled;
            }
            LocalAction::RemoveGroup(id) => { self.groups.remove(&id); }
        }
        self.validate()
    }
}

pub fn validate_queries(queries: &[String]) -> Result<(), String> {
    if queries.len() > 8 || queries.iter().any(|query|
        !text(query, 100) || query.chars().any(|ch| !ch.is_alphanumeric() && !matches!(ch, ' ' | '-' | '_')))
    {
        return Err("Use up to eight team searches, with letters, numbers, spaces or hyphens".into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub enum LocalAction {
    Snooze { key: String, until: DateTime<Utc> },
    Unsnooze(String),
    QuietSeries { key: String, quiet: bool },
    Organizer { email: String, rule: Option<OrganizerRule> },
    People { included: BTreeSet<String>, excluded: BTreeSet<String> },
    GroupEnabled { id: String, enabled: bool },
    RemoveGroup(String),
}

pub enum Command {
    Local { account: String, action: LocalAction },
    Open { account: String, key: String },
    Refresh { account: String },
    Discover { account: String, queries: Vec<String> },
    RefreshTeams { account: String },
}

impl Command {
    pub fn account(&self) -> &str {
        match self {
            Self::Local { account, .. } | Self::Open { account, .. } | Self::Refresh { account }
                | Self::Discover { account, .. } | Self::RefreshTeams { account } => account,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ReadKind {
    Pending { first: NaiveDate, last: NaiveDate },
    Teams(Vec<Group>),
    Discover(Vec<String>),
}

pub enum ReadResult {
    Pending(Vec<Event>),
    Teams(Vec<Group>),
    Discovered(Vec<Group>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub busy: bool,
    pub checked_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

pub struct Reader {
    pub progress: Progress,
    pub next: Instant,
    flight: Option<(u64, String, Instant, DateTime<Utc>)>,
    failures: u32,
}

impl Default for Reader {
    fn default() -> Self {
        Self { progress: Default::default(), next: Instant::now(), flight: None, failures: 0 }
    }
}

impl Reader {
    pub fn due(&self) -> bool { self.flight.is_none() && Instant::now() >= self.next }

    pub fn start(&mut self, id: u64, account: &str) {
        self.flight = Some((id, account.into(), Instant::now(), Utc::now()));
        self.progress.busy = true;
        self.progress.error = None;
    }

    pub fn accepts(&self, id: u64, account: &str) -> bool {
        self.flight.as_ref().is_some_and(|(active, owner, _, _)| *active == id && owner == account)
    }

    pub fn started_at(&self) -> Option<DateTime<Utc>> { self.flight.as_ref().map(|(_, _, _, stamp)| *stamp) }

    pub fn finish(&mut self, result: Result<(), String>, interval: u64) {
        self.flight = None;
        self.progress.busy = false;
        let delay = match result {
            Ok(()) => {
                self.failures = 0;
                self.progress.error = None;
                self.progress.checked_at = Some(Utc::now());
                interval
            }
            Err(error) => {
                self.failures = self.failures.saturating_add(1);
                self.progress.error = Some(error);
                30 * (1_u64 << self.failures.min(5))
            }
        };
        self.next = Instant::now() + std::time::Duration::from_secs(delay);
    }

    pub fn expire(&mut self) -> bool {
        if self.flight.as_ref().is_some_and(|(_, _, started, _)| started.elapsed().as_secs() > 240) {
            self.finish(Err("Invitation read timed out; cached data was preserved".into()), REFRESH_SECONDS);
            return true;
        }
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab { Important, Other, Snoozed }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason { Customer, Pinned, Team, External, ExternalParticipant, Quiet, Broadcast, Internal }

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Customer => "Customer meeting",
            Self::Pinned => "Highlighted organizer",
            Self::Team => "Organizer from my team",
            Self::External => "External organizer - customer not confirmed",
            Self::ExternalParticipant => "External participant - customer not confirmed",
            Self::Quiet => "Quiet by your choice",
            Self::Broadcast => "Internal broadcast or optional event",
            Self::Internal => "Other internal invitation",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflicts { Unknown, Clear, Found(usize) }

#[derive(Clone)]
pub struct Row {
    pub key: String,
    pub event: Event,
    pub tab: Tab,
    pub reason: Reason,
    pub detail: String,
    pub conflicts: Conflicts,
    pub first_seen: DateTime<Utc>,
    pub updated: bool,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub book: Arc<Book>,
    pub rows: Vec<Row>,
    pub sync: Progress,
    pub teams: Progress,
}

impl Snapshot {
    pub fn important(&self) -> usize {
        self.rows.iter().filter(|row| row.tab == Tab::Important).count()
    }
}

fn classification(book: &Book, event: &Event, customers: &customers::Book,
    detector: &customers::Detector, own: &BTreeSet<String>, team: &BTreeSet<String>,
) -> (Tab, Reason, String, u8) {
    let organizer = person_email(&event.organizer);
    let rule = organizer.as_ref().and_then(|email| book.organizers.get(email));
    if book.quiet_series.contains(&key(event)) || rule == Some(&OrganizerRule::Quiet) {
        return (Tab::Other, Reason::Quiet, String::new(), 9);
    }
    if rule == Some(&OrganizerRule::Important) {
        return (Tab::Important, Reason::Pinned, String::new(), 0);
    }
    let detection = customers::detect(customers, detector, &book.account, event);
    let names = detection.customers().filter_map(|id| customers.customers.get(id))
        .map(|customer| customer.name.clone()).collect::<Vec<_>>();
    if !names.is_empty() {
        return (Tab::Important, Reason::Customer, names.join(", "), 1);
    }
    let external = |address: &str| {
        address.split_once('@').is_some_and(|(_, domain)|
            detector.external(domain) || customers::consumer_domain(domain))
            && !own.contains(address)
    };
    let external_organizer = organizer.as_deref().is_some_and(external);
    let roles = event.extra.get("tomlook_attendee_roles").and_then(serde_json::Value::as_object);
    let external_participant = event.attendees.iter().filter_map(|person| person_email(person))
        .filter(|address| roles.and_then(|roles| roles.get(address)).and_then(serde_json::Value::as_str) != Some("resource"))
        .any(|address| external(&address));
    let optional = event.extra.get("tomlook_attendee_roles").and_then(serde_json::Value::as_object)
        .is_some_and(|roles| own.iter().any(|email|
            roles.get(email).and_then(serde_json::Value::as_str) == Some("optional")));
    let title = format!(" {} ", crate::search::words(&event.title).join(" "));
    let broadcast = event.attendee_count() > 50 || optional
        || ["webinar", "all hands", "office hours", "optional", "on demand", "enablement"]
            .iter().any(|word| title.contains(&format!(" {word} ")));
    if broadcast {
        return (Tab::Other, Reason::Broadcast, String::new(), 8);
    }
    if organizer.as_ref().is_some_and(|address| team.contains(address)) {
        let name = organizer.as_ref().and_then(|address| book.groups.values()
            .find(|group| group.enabled && group.members.contains(address)))
            .map(|group| group.name.clone()).unwrap_or_default();
        return (Tab::Important, Reason::Team, name, 2);
    }
    if external_organizer && !detection.excluded {
        return (Tab::Important, Reason::External, String::new(), 3);
    }
    if external_participant && !detection.excluded {
        return (Tab::Important, Reason::ExternalParticipant, String::new(), 4);
    }
    (Tab::Other, if broadcast { Reason::Broadcast } else { Reason::Internal }, String::new(), 8)
}

#[allow(clippy::too_many_arguments)]
pub fn snapshot(book: Arc<Book>, calendar: &Calendar, customers: &customers::Book,
    identity: Option<&crate::workiq::connection::Identity>,
    freshness: &crate::calendar_sync::Freshness, sync: Progress, teams: Progress,
    now: DateTime<Utc>,
) -> Snapshot {
    let mut own = BTreeSet::from([book.account.clone()]);
    if let Some(identity) = identity.filter(|identity| identity.account == book.account) {
        own.extend(identity.mail.iter().cloned());
    }
    let detector = customers::Detector::new(own.iter().map(String::as_str), []);
    let team = book.team();
    let mut fallback = Book::new(&book.account);
    let records = if book.refreshed_at.is_some() { &book.records } else {
        // Existing calendar data is useful at startup, but never called a fresh invitation read.
        if let Err(error) = fallback.replace(calendar.events.clone(), now) {
            return Snapshot { book, sync: Progress { error: Some(error), ..sync }, teams, ..Default::default() };
        }
        &fallback.records
    };
    let mut rows = Vec::new();
    for (id, record) in records {
        let event = match book.refreshed_at {
            Some(refreshed) => current(&record.event, calendar, freshness, refreshed, now),
            None => Some(&record.event),
        };
        let Some(event) = event.filter(|event| pending(event, now)) else { continue; };
        let (mut tab, reason, detail, rank) = classification(&book, event, customers, &detector, &own, &team);
        if book.snoozed.get(id).is_some_and(|snooze|
            snooze.until > now && snooze.revision == revision(event))
        {
            tab = Tab::Snoozed;
        }
        let (first, last) = event.calendar_dates();
        let fresh = (0..=(last - first).num_days()).all(|offset| {
            let day = first + Duration::days(offset);
            calendar.coverage.contains(&day) && freshness.get(&day)
                .is_some_and(|stamp| *stamp <= now && now - *stamp <= Duration::minutes(15))
        });
        let conflicts = if fresh {
            match crate::recommendations::conflicts(calendar, event).len() {
                0 => Conflicts::Clear,
                count => Conflicts::Found(count),
            }
        } else { Conflicts::Unknown };
        rows.push((rank, Row { key: id.clone(), event: event.clone(), tab, reason, detail,
            conflicts, first_seen: record.first_seen, updated: record.updated || revision(event) != revision(&record.event) }));
    }
    rows.sort_by(|(left_rank, left), (right_rank, right)|
        left_rank.cmp(right_rank).then(left.event.start.cmp(&right.event.start))
            .then(right.first_seen.cmp(&left.first_seen)).then(left.key.cmp(&right.key)));
    Snapshot { book, rows: rows.into_iter().map(|(_, row)| row).collect(), sync, teams }
}
