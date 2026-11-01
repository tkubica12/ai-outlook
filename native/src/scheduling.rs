//! Find a time for a group: a local candidate search over read-only free/busy, the locally saved
//! proposal, its BLOCKER holds in your own calendar and the proposal texts for a Teams chat or an
//! e-mail. No AI is involved; every write happens only after an explicit approval elsewhere.

use crate::{
    attendees::{Attendee, Role},
    availability::{Availability, ScheduleRequest, Status},
    calendar::{Event, person_email},
    i18n::Language,
};
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Literal title prefix of every hold, in both languages, so Outlook search and rules find it.
pub const BLOCKER_PREFIX: &str = "BLOCKER:";
/// Most candidates shown.
pub const MAX_CANDIDATES: usize = 10;
/// Most candidates one proposal can carry.
pub const MAX_SELECTED: usize = 5;
/// Most searched days.
pub const MAX_DAYS: u16 = 15;
pub const DEFAULT_DAYS: u16 = 5;
/// Local daily window in minutes after midnight.
pub const DEFAULT_WINDOW: (u16, u16) = (8 * 60, 18 * 60);
pub const DEFAULT_MINIMUM: u8 = 3;
/// Avoided when possible: a soft preference only.
pub const LUNCH: (u16, u16) = (12 * 60, 13 * 60);
pub const TOPIC_LIMIT: usize = 200;
/// Longest Teams or e-mail proposal message, in UTF-8 bytes.
pub const MESSAGE_LIMIT: usize = 4000;
/// Proposals kept per account.
pub const SAVED_LIMIT: usize = 200;
pub const MIN_MINUTES: u16 = 15;
pub const MAX_MINUTES: u16 = 240;

/// A Tomlook-style hold in your calendar: a `BLOCKER:` title and nobody invited.
pub fn is_blocker(event: &Event) -> bool {
    event.title.starts_with(BLOCKER_PREFIX) && event.attendee_count() == 0
}

pub fn blocker_title(topic: &str) -> String {
    format!("{BLOCKER_PREFIX} {}", topic.trim())
}

pub fn validate_topic(topic: &str) -> Result<(), String> {
    let topic = topic.trim();
    if topic.is_empty() || topic.len() > TOPIC_LIMIT || topic.chars().any(char::is_control) {
        return Err("Enter a topic (up to 200 bytes)".into());
    }
    Ok(())
}

pub fn validate_minutes(minutes: u16) -> Result<(), String> {
    if !(MIN_MINUTES..=MAX_MINUTES).contains(&minutes) || !minutes.is_multiple_of(15) {
        return Err("The meeting must last 15 to 240 minutes in 15-minute steps".into());
    }
    Ok(())
}

/// "30 min", "1 h", "1 h 30 min".
pub fn duration_label(minutes: u16) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// "Tue 4 Nov 10:00–10:30" or "út 4. 11. 10:00–10:30" in `zone`.
pub fn slot_label_in<Z: TimeZone>(
    zone: &Z,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    language: Language,
) -> String
where
    Z::Offset: std::fmt::Display,
{
    const DAYS: [&str; 7] = ["po", "út", "st", "čt", "pá", "so", "ne"];
    let start = start.with_timezone(zone);
    let end = end.with_timezone(zone);
    let day = match language {
        Language::En => start.format("%a %-d %b").to_string(),
        Language::Cs => format!(
            "{} {}. {}.",
            DAYS[start.weekday().num_days_from_monday() as usize],
            start.day(),
            start.month()
        ),
    };
    let end_text = if end.date_naive() == start.date_naive() {
        end.format("%H:%M").to_string()
    } else {
        end.format("%d.%m. %H:%M").to_string()
    };
    format!("{day} {}–{end_text}", start.format("%H:%M"))
}

/// [`slot_label_in`] in the Windows time zone.
pub fn slot_label(start: DateTime<Utc>, end: DateTime<Utc>, language: Language) -> String {
    slot_label_in(&chrono::Local, start, end, language)
}

/// "UTC+01:00": the offset of `zone` at `time`, so that people in other time zones read a time
/// correctly; it differs on each side of a daylight-saving change.
pub fn utc_offset_label<Z: TimeZone>(zone: &Z, time: DateTime<Utc>) -> String
where
    Z::Offset: std::fmt::Display,
{
    format!("UTC{}", time.with_timezone(zone).format("%:z"))
}

/// What to search: duration, days, the local daily window and how many candidates to show at
/// least.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub minutes: u16,
    pub first: NaiveDate,
    /// Searched days: workdays when `weekdays_only`, otherwise calendar days.
    pub days: u16,
    pub window: (u16, u16),
    pub weekdays_only: bool,
    pub minimum: u8,
}

impl Search {
    /// The next five workdays from tomorrow, 08:00–18:00.
    pub fn new(today: NaiveDate, minutes: u16) -> Self {
        let mut search = Self {
            minutes,
            first: today.succ_opt().unwrap_or(today),
            days: DEFAULT_DAYS,
            window: DEFAULT_WINDOW,
            weekdays_only: true,
            minimum: DEFAULT_MINIMUM,
        };
        while !search.included(search.first) {
            search.first = search.first.succ_opt().unwrap_or(search.first);
        }
        search
    }

    fn included(&self, day: NaiveDate) -> bool {
        !self.weekdays_only || day.weekday().num_days_from_monday() < 5
    }

    /// The searched dates, in order.
    pub fn dates(&self) -> Vec<NaiveDate> {
        let mut dates = Vec::new();
        let mut day = self.first;
        for _ in 0..(usize::from(MAX_DAYS) * 2) {
            if dates.len() >= usize::from(self.days.clamp(1, MAX_DAYS)) {
                break;
            }
            if self.included(day) {
                dates.push(day);
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        dates
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_minutes(self.minutes)?;
        if !(1..=MAX_DAYS).contains(&self.days) {
            return Err("Search 1 to 15 days".into());
        }
        let (from, to) = self.window;
        if !from.is_multiple_of(15) || !to.is_multiple_of(15) || to > 1440 || from >= to || to - from < self.minutes {
            return Err("The daily window must be longer than the meeting".into());
        }
        if !(1..=MAX_CANDIDATES as u8).contains(&self.minimum) {
            return Err("Show at least 1 to 10 candidates".into());
        }
        Ok(())
    }

    /// One read-only free/busy request for all searched days.
    pub fn schedule_request_in<Z: TimeZone>(&self, zone: &Z, schedules: Vec<String>) -> Result<ScheduleRequest, String> {
        self.validate()?;
        let dates = self.dates();
        let (Some(first), Some(last)) = (dates.first(), dates.last()) else {
            return Err("Search 1 to 15 days".into());
        };
        ScheduleRequest::range_in_zone(zone, *first, *last, schedules)
    }

    pub fn schedule_request(&self, schedules: Vec<String>) -> Result<ScheduleRequest, String> {
        self.schedule_request_in(&chrono::Local, schedules)
    }
}

/// One proposed time with everyone's free/busy status.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub you: Status,
    /// Each attendee's status, in attendee order.
    pub attendees: Vec<(String, Status)>,
    /// You and every required attendee are free (or working elsewhere).
    pub all_required: bool,
}

impl Candidate {
    pub fn slot(&self) -> Slot {
        Slot {
            start: self.start.fixed_offset(),
            end: self.end.fixed_offset(),
        }
    }

    /// "All free", "All required free · Morgan busy (optional)" or "Jamie tentative · Dana unknown".
    pub fn reason(&self, attendees: &[Attendee], language: Language) -> String {
        let mut conflicts = Vec::new();
        if !self.you.available() {
            conflicts.push(named(
                match language {
                    Language::En => "You",
                    Language::Cs => "Vy",
                },
                self.you,
                false,
                language,
            ));
        }
        let mut optional = Vec::new();
        for (email, status) in &self.attendees {
            if status.available() {
                continue;
            }
            let attendee = attendees.iter().find(|attendee| attendee.email == *email);
            let label = attendee.map_or(email.as_str(), Attendee::label);
            if attendee.is_some_and(|attendee| attendee.role == Role::Optional) {
                optional.push(named(label, *status, true, language));
            } else {
                conflicts.push(named(label, *status, false, language));
            }
        }
        if conflicts.is_empty() && optional.is_empty() {
            return match language {
                Language::En => "All free".into(),
                Language::Cs => "Všichni mají volno".into(),
            };
        }
        let mut parts = Vec::new();
        if self.all_required {
            parts.push(match language {
                Language::En => "All required free".to_owned(),
                Language::Cs => "Povinní účastníci mají volno".to_owned(),
            });
        }
        parts.extend(conflicts);
        parts.extend(optional);
        const LISTED: usize = 4;
        if parts.len() > LISTED {
            let more = parts.len() - LISTED + 1;
            parts.truncate(LISTED - 1);
            parts.push(match language {
                Language::En => format!("+{more} more"),
                Language::Cs => format!("+{more} dalších"),
            });
        }
        parts.join(" · ")
    }
}

fn named(name: &str, status: Status, optional: bool, language: Language) -> String {
    let base = match language {
        Language::En => format!("{name} {}", status.word(language)),
        Language::Cs => format!("{name}: {}", status.word(language)),
    };
    match (optional, language) {
        (false, _) => base,
        (true, Language::En) => format!("{base} (optional)"),
        (true, Language::Cs) => format!("{base} (nepovinně)"),
    }
}

/// Candidates of one search, ordered by time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidates {
    pub list: Vec<Candidate>,
    /// How many of `list` have you and every required attendee free.
    pub ideal: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Cost {
    /// Required attendees busy or out of office.
    blocked: usize,
    /// You or required attendees whose availability is unknown: unknown is never free.
    unknown: usize,
    /// You or required attendees tentatively busy.
    tentative: usize,
    /// Optional attendees who are not available.
    optional: usize,
}

impl Cost {
    fn ideal(self) -> bool {
        self.blocked == 0 && self.unknown == 0 && self.tentative == 0
    }
}

struct Opening {
    day: usize,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    cost: Cost,
    lunch: bool,
    half_hour: bool,
    you: Status,
    attendees: Vec<(String, Status)>,
}

/// [`candidates_in`] in the Windows time zone.
pub fn candidates(
    search: &Search,
    availability: &Availability,
    own: &str,
    attendees: &[Attendee],
    now: DateTime<Utc>,
) -> Result<Candidates, String> {
    candidates_in(&chrono::Local, search, availability, own, attendees, now)
}

/// Local slot search over 15-minute starts within the daily window of each searched day.
///
/// Your busy or out-of-office time is never proposed. Slots are ranked by required attendees
/// busy, then unknown (unknown is not free), then tentative, then optional attendees who are not
/// free, then the lunch hour (a soft preference), then how many candidates the day already has
/// (spreading across days), half-hour starts and time. Candidates of one day are at least an hour
/// (or the meeting's duration) apart. Every candidate with everyone required free is shown, up to
/// [`MAX_CANDIDATES`]; when fewer exist, the best alternatives fill up to `search.minimum`.
pub fn candidates_in<Z: TimeZone>(
    zone: &Z,
    search: &Search,
    availability: &Availability,
    own: &str,
    attendees: &[Attendee],
    now: DateTime<Utc>,
) -> Result<Candidates, String> {
    search.validate()?;
    if attendees.is_empty() {
        return Err("Add attendees to find a time".into());
    }
    crate::attendees::validate_list(attendees)?;
    let duration = Duration::minutes(i64::from(search.minutes));
    let mut options = Vec::new();
    for (day_index, day) in search.dates().into_iter().enumerate() {
        let midnight = day.and_hms_opt(0, 0, 0).ok_or("Invalid availability date")?;
        let window_end = midnight + Duration::minutes(i64::from(search.window.1));
        let mut minute = search.window.0;
        while minute + search.minutes <= search.window.1 {
            let local = midnight + Duration::minutes(i64::from(minute));
            let this = minute;
            minute += 15;
            // Missing or repeated local times on daylight-saving days are skipped, never guessed.
            let Some(start) = zone.from_local_datetime(&local).single() else { continue };
            let start = start.with_timezone(&Utc);
            let end = start + duration;
            if start < now || end.with_timezone(zone).naive_local() > window_end {
                continue;
            }
            let you = availability.status(own, start, end);
            if matches!(you, Status::Busy | Status::OutOfOffice) {
                continue;
            }
            let mut cost = Cost { blocked: 0, unknown: 0, tentative: 0, optional: 0 };
            match you {
                Status::Unknown => cost.unknown += 1,
                Status::Tentative => cost.tentative += 1,
                _ => {}
            }
            let statuses = attendees
                .iter()
                .map(|attendee| {
                    let status = availability.status(&attendee.email, start, end);
                    match (attendee.role, status) {
                        (_, status) if status.available() => {}
                        (Role::Optional, _) => cost.optional += 1,
                        (Role::Required, Status::Unknown) => cost.unknown += 1,
                        (Role::Required, Status::Tentative) => cost.tentative += 1,
                        (Role::Required, _) => cost.blocked += 1,
                    }
                    (attendee.email.clone(), status)
                })
                .collect();
            options.push(Opening {
                day: day_index,
                start,
                end,
                cost,
                lunch: this < LUNCH.1 && this + search.minutes > LUNCH.0,
                half_hour: this.is_multiple_of(30),
                you,
                attendees: statuses,
            });
        }
    }
    let gap = i64::from(search.minutes.max(60));
    let mut per_day = vec![0_usize; usize::from(MAX_DAYS)];
    let mut picked: Vec<usize> = Vec::new();
    while picked.len() < MAX_CANDIDATES {
        let best = options
            .iter()
            .enumerate()
            .filter(|(index, option)| {
                !picked.contains(index)
                    && !picked.iter().any(|other| {
                        options[*other].day == option.day
                            && (options[*other].start - option.start).num_minutes().abs() < gap
                    })
            })
            .min_by_key(|(_, option)| {
                (option.cost, option.lunch, per_day[option.day], !option.half_hour, option.start)
            })
            .map(|(index, _)| index);
        let Some(index) = best else { break };
        per_day[options[index].day] += 1;
        picked.push(index);
    }
    // Picks never get cheaper, so every ideal pick comes first.
    let ideal = picked.iter().filter(|index| options[**index].cost.ideal()).count();
    let keep = ideal.max(usize::from(search.minimum).min(picked.len()));
    let mut list = picked[..keep]
        .iter()
        .map(|index| {
            let option = &options[*index];
            Candidate {
                start: option.start,
                end: option.end,
                you: option.you,
                attendees: option.attendees.clone(),
                all_required: option.cost.ideal(),
            }
        })
        .collect::<Vec<_>>();
    list.sort_by_key(|candidate| candidate.start);
    Ok(Candidates { list, ideal })
}

/// Adds or removes a selected slot; at most [`MAX_SELECTED`], kept in time order.
pub fn toggle(selected: &mut Vec<Slot>, slot: Slot) -> Result<(), String> {
    if let Some(index) = selected.iter().position(|chosen| *chosen == slot) {
        selected.remove(index);
        return Ok(());
    }
    if selected.len() >= MAX_SELECTED {
        return Err("Select at most 5 times".into());
    }
    selected.push(slot);
    selected.sort_by_key(|slot| slot.start);
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slot {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
}

impl Slot {
    pub fn label(&self, language: Language) -> String {
        slot_label(self.start.with_timezone(&Utc), self.end.with_timezone(&Utc), language)
    }

    /// The label for other people: "Tue 4 Nov 10:00–10:30 (UTC+01:00)" in `zone`.
    pub fn outgoing_label_in<Z: TimeZone>(&self, zone: &Z, language: Language) -> String
    where
        Z::Offset: std::fmt::Display,
    {
        let (start, end) = (self.start.with_timezone(&Utc), self.end.with_timezone(&Utc));
        format!("{} ({})", slot_label_in(zone, start, end, language), utc_offset_label(zone, start))
    }

    pub fn minutes(&self) -> i64 {
        (self.end - self.start).num_minutes()
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum ProposalStatus {
    #[default]
    Open,
    Scheduled,
    Cancelled,
}

impl ProposalStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open proposal",
            Self::Scheduled => "Scheduled",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum BlockerState {
    /// Approved and being created.
    Creating,
    /// Created and verified in your calendar.
    Held,
    /// Definitely not created.
    NotCreated,
    /// The creation outcome is uncertain; never replayed.
    Review,
    /// Approved deletion in progress.
    Releasing,
    /// Deleted and verified.
    Released,
    /// The deletion outcome is uncertain; never replayed.
    ReleaseReview,
}

impl BlockerState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Creating => "Creating BLOCKER",
            Self::Held => "BLOCKER in your calendar",
            Self::NotCreated => "BLOCKER not created",
            Self::Review => "Check this BLOCKER in Outlook",
            Self::Releasing => "Deleting BLOCKER",
            Self::Released => "BLOCKER deleted",
            Self::ReleaseReview => "Check whether this BLOCKER was deleted in Outlook",
        }
    }
}

/// One BLOCKER of a proposal: its slot, the Graph transaction ID it was created with and, once
/// verified, its event ID (a single, non-recurring occurrence).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Blocker {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub title: String,
    pub transaction_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub state: BlockerState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

/// A Teams chat member resolved from the directory.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Member {
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Entra object ID used in the member binding.
    pub user_id: String,
}

impl Member {
    pub fn label(&self) -> &str {
        if self.name.is_empty() { &self.email } else { &self.name }
    }
}

pub fn valid_user_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
        })
}

/// The approved Teams proposal: exact members, topic (group chats only) and message.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatDraft {
    pub members: Vec<Member>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    pub message: String,
}

impl ChatDraft {
    /// One other person: Graph's one-on-one chat, which Teams reuses when it already exists.
    pub fn one_on_one(&self) -> bool {
        self.members.len() == 1
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.members.is_empty() || self.members.len() > crate::attendees::LIMIT {
            return Err("A Teams proposal needs 1 to 50 people".into());
        }
        let mut seen = BTreeSet::new();
        for member in &self.members {
            if person_email(&member.email).as_deref() != Some(member.email.as_str())
                || !valid_user_id(&member.user_id)
                || !seen.insert(member.user_id.to_ascii_lowercase())
                || member.name.len() > crate::attendees::NAME_LIMIT
                || member.name.chars().any(char::is_control)
            {
                return Err("Every Teams member must be one resolved person".into());
            }
        }
        match &self.topic {
            Some(_) if self.one_on_one() => return Err("One-on-one chats have no topic".into()),
            Some(topic) => validate_topic(topic)?,
            None if !self.one_on_one() => return Err("Group chats need a topic".into()),
            None => {}
        }
        validate_message(&self.message)
    }
}

pub fn validate_message(message: &str) -> Result<(), String> {
    if message.trim().is_empty()
        || message.len() > MESSAGE_LIMIT
        || message.chars().any(|ch| ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t')
    {
        return Err("Write a message of up to 4000 bytes".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ChatState {
    /// Approved and being sent.
    Sending,
    /// The chat exists and the message was read back.
    Sent,
    /// Definitely nothing sent (a chat may exist without the message).
    NotSent,
    /// Uncertain; never replayed.
    Review,
}

impl ChatState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sending => "Sending to Teams",
            Self::Sent => "Sent in Teams",
            Self::NotSent => "Not sent in Teams",
            Self::Review => "Check the Teams chat",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatRecord {
    pub draft: ChatDraft,
    pub state: ChatState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

/// A locally saved scheduling proposal.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Proposal {
    pub id: String,
    pub account: String,
    pub topic: String,
    pub attendees: Vec<Attendee>,
    pub minutes: u16,
    /// The proposed times, in order.
    pub candidates: Vec<Slot>,
    pub language: Language,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<Blocker>,
    /// The event-creation approval that holds the BLOCKERs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_record: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<ChatRecord>,
    /// The invitation that scheduled the meeting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_record: Option<String>,
    pub status: ProposalStatus,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

pub const UNCERTAIN_CHAT: &str = "The Teams outcome is uncertain. Open Teams before proposing again; nothing will be sent again automatically.";
pub const UNCERTAIN_RELEASE: &str = "The deletion outcome is uncertain. Check Outlook; nothing will be deleted again automatically.";

impl Proposal {
    pub fn new(
        account: &str,
        topic: &str,
        attendees: Vec<Attendee>,
        minutes: u16,
        mut candidates: Vec<Slot>,
        language: Language,
        now: DateTime<FixedOffset>,
    ) -> Result<Self, String> {
        candidates.sort();
        let identity = serde_json::to_vec(&(
            account,
            topic.trim(),
            &attendees,
            minutes,
            &candidates,
            now.timestamp_nanos_opt(),
        ))
        .map_err(|_| "Encode proposal failed")?;
        let id = format!("{:x}", Sha256::digest(identity))[..32].to_owned();
        let proposal = Self {
            id,
            account: account.to_owned(),
            topic: topic.trim().to_owned(),
            attendees,
            minutes,
            candidates,
            language,
            blockers: Vec::new(),
            hold_record: None,
            chat: None,
            meeting_record: None,
            status: ProposalStatus::Open,
            created_at: now,
            updated_at: now,
        };
        proposal.validate()?;
        Ok(proposal)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.len() != 32 || !self.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid proposal identity".into());
        }
        if self.account.trim().is_empty() || self.account.len() > 320 {
            return Err("Calendar account is unavailable".into());
        }
        validate_topic(&self.topic)?;
        if self.attendees.is_empty() {
            return Err("Add attendees to find a time".into());
        }
        crate::attendees::validate_list(&self.attendees)?;
        validate_minutes(self.minutes)?;
        if self.candidates.is_empty() || self.candidates.len() > MAX_SELECTED {
            return Err("Select 1 to 5 times".into());
        }
        if self.candidates.windows(2).any(|pair| pair[0] >= pair[1])
            || self.candidates.iter().any(|slot| slot.minutes() != i64::from(self.minutes))
        {
            return Err("Proposed times must be distinct and last the meeting's duration".into());
        }
        if self.blockers.len() > MAX_SELECTED
            || self.blockers.iter().any(|blocker| {
                blocker.transaction_id.is_empty()
                    || blocker.transaction_id.len() > 128
                    || !blocker.title.starts_with(BLOCKER_PREFIX)
                    || blocker.event_id.as_ref().is_some_and(|id| id.is_empty() || id.len() > 3000)
                    || blocker.message.len() > 2000
                    || !self.candidates.contains(&Slot { start: blocker.start, end: blocker.end })
            })
        {
            return Err("Saved BLOCKERs do not match their proposal".into());
        }
        if let Some(chat) = &self.chat {
            chat.draft.validate()?;
            if chat.message.len() > 2000
                || chat.chat_id.as_ref().is_some_and(|id| !valid_chat_id(id))
                || chat.web_url.as_ref().is_some_and(|url| url.len() > 4000 || !crate::calendar::safe_url(url))
            {
                return Err("Saved Teams proposal is invalid".into());
            }
        }
        Ok(())
    }

    /// People who receive the proposal, required first.
    pub fn attendee_lines(&self, language: Language) -> Vec<String> {
        let mut attendees = self.attendees.iter().collect::<Vec<_>>();
        attendees.sort_by_key(|attendee| attendee.role);
        attendees
            .into_iter()
            .map(|attendee| {
                let person = if attendee.name.is_empty() {
                    attendee.email.clone()
                } else {
                    // Spaces around the address survive Outlook's text-to-HTML conversion.
                    format!("{}, {}", attendee.name, attendee.email)
                };
                match (attendee.role, language) {
                    (Role::Required, _) => format!("- {person}"),
                    (Role::Optional, Language::En) => format!("- {person} (optional)"),
                    (Role::Optional, Language::Cs) => format!("- {person} (nepovinně)"),
                }
            })
            .collect()
    }

    fn numbered_times(&self, language: Language) -> Vec<String> {
        self.candidates
            .iter()
            .enumerate()
            .map(|(index, slot)| format!("{}) {}", index + 1, slot.label(language)))
            .collect()
    }

    /// The numbered times for other people, each with its UTC offset in `zone`.
    fn outgoing_times<Z: TimeZone>(&self, zone: &Z) -> String
    where
        Z::Offset: std::fmt::Display,
    {
        self.candidates
            .iter()
            .enumerate()
            .map(|(index, slot)| format!("{}) {}", index + 1, slot.outgoing_label_in(zone, self.language)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The description of every BLOCKER: topic, invitees with addresses and all proposed times.
    pub fn blocker_body(&self) -> String {
        let language = self.language;
        let duration = duration_label(self.minutes);
        let mut lines = vec![match language {
            Language::En => format!("Proposed time for: {} ({duration})", self.topic),
            Language::Cs => format!("Navržený termín pro: {} ({duration})", self.topic),
        }];
        lines.push(String::new());
        lines.push(match language {
            Language::En => "Invitees:".into(),
            Language::Cs => "Pozvaní:".into(),
        });
        lines.extend(self.attendee_lines(language));
        lines.push(String::new());
        lines.push(match language {
            Language::En => "Proposed times:".into(),
            Language::Cs => "Navržené termíny:".into(),
        });
        lines.extend(self.numbered_times(language));
        lines.push(String::new());
        lines.push(match language {
            Language::En => "Held by Tomlook until the meeting is scheduled. Confirm or release it in Tomlook.".into(),
            Language::Cs => "Blokováno Tomlookem, dokud se schůzka nenaplánuje. Potvrdit nebo uvolnit ho můžete v Tomlooku.".into(),
        });
        lines.join("\n")
    }

    /// The event-creation draft of the BLOCKERs: busy, no attendees, no Teams, no reminder.
    pub fn hold_draft(
        &self,
        private: bool,
        category: Option<String>,
    ) -> Result<crate::event_creation::Draft, String> {
        let first = self.candidates.first().ok_or("Select 1 to 5 times")?;
        let draft = crate::event_creation::Draft {
            title: blocker_title(&self.topic),
            description: self.blocker_body(),
            start: first.start,
            end: first.end,
            category,
            private,
            teams: false,
            travel_minutes: None,
            language: self.language,
            attendees: Vec::new(),
        };
        draft.validate()?;
        Ok(draft)
    }

    /// The default Teams message: a short question with the numbered times and their UTC offsets
    /// in the Windows time zone.
    pub fn chat_message(&self, members: &[Member]) -> String {
        self.chat_message_in(&chrono::Local, members)
    }

    /// [`Self::chat_message`] with the times in `zone`.
    pub fn chat_message_in<Z: TimeZone>(&self, zone: &Z, members: &[Member]) -> String
    where
        Z::Offset: std::fmt::Display,
    {
        let duration = duration_label(self.minutes);
        let times = self.outgoing_times(zone);
        match self.language {
            Language::En => {
                let greeting = match members {
                    [one] if !one.name.is_empty() => format!("Hi {},", one.name.split_whitespace().next().unwrap_or(&one.name)),
                    [_] => "Hi,".into(),
                    _ => "Hi all,".into(),
                };
                format!("{greeting} I'd like to schedule {} ({duration}). Which of these times works for you?\n{times}\nPlease reply here. Thanks!", self.topic)
            }
            Language::Cs => {
                let greeting = if members.len() == 1 { "Dobrý den," } else { "Dobrý den všem," };
                format!("{greeting} potřebuji naplánovat schůzku „{}“ ({duration}). Který z těchto termínů vám vyhovuje?\n{times}\nOdpovězte prosím tady v chatu. Děkuji!", self.topic)
            }
        }
    }

    pub fn email_subject(&self) -> String {
        match self.language {
            Language::En => format!("Scheduling: {} – which time works for you?", self.topic),
            Language::Cs => format!("Plánování: {} – který termín vám vyhovuje?", self.topic),
        }
    }

    /// The default e-mail text with the numbered times and their UTC offsets in the Windows time
    /// zone.
    pub fn email_body(&self) -> String {
        self.email_body_in(&chrono::Local)
    }

    /// [`Self::email_body`] with the times in `zone`.
    pub fn email_body_in<Z: TimeZone>(&self, zone: &Z) -> String
    where
        Z::Offset: std::fmt::Display,
    {
        let duration = duration_label(self.minutes);
        let times = self.outgoing_times(zone);
        match self.language {
            Language::En => format!("Hi all,\n\nI'd like to schedule {} ({duration}). Which of these times works for you?\n{times}\n\nPlease reply to this e-mail. Thanks!", self.topic),
            Language::Cs => format!("Dobrý den všem,\n\npotřebuji naplánovat schůzku „{}“ ({duration}). Který z těchto termínů vám vyhovuje?\n{times}\n\nOdpovězte prosím na tento e-mail. Děkuji!", self.topic),
        }
    }

    /// BLOCKERs currently in your calendar.
    pub fn held(&self) -> impl Iterator<Item = &Blocker> {
        self.blockers.iter().filter(|blocker| blocker.state == BlockerState::Held && blocker.event_id.is_some())
    }

    /// Whether any part of the proposal still waits for an outcome.
    pub fn in_flight(&self) -> bool {
        self.blockers.iter().any(|blocker| matches!(blocker.state, BlockerState::Creating | BlockerState::Releasing))
            || self.chat.as_ref().is_some_and(|chat| chat.state == ChatState::Sending)
    }

    /// Whether an outcome needs to be checked in Outlook or Teams.
    pub fn needs_review(&self) -> bool {
        self.blockers.iter().any(|blocker| matches!(blocker.state, BlockerState::Review | BlockerState::ReleaseReview))
            || self.chat.as_ref().is_some_and(|chat| chat.state == ChatState::Review)
    }

    /// Records the approved BLOCKER creation; allowed once per proposal.
    pub fn begin_hold(&mut self, record: &crate::event_creation::Record, now: DateTime<FixedOffset>) -> Result<(), String> {
        if self.status != ProposalStatus::Open || !self.blockers.is_empty() || self.hold_record.is_some() {
            return Err("BLOCKERs were already requested for this proposal".into());
        }
        let hold = record.hold.as_ref().filter(|hold| hold.proposal == self.id && hold.slots == self.candidates)
            .ok_or("BLOCKER approval does not match its proposal")?;
        self.blockers = record
            .parts()
            .into_iter()
            .zip(&hold.slots)
            .map(|(part, slot)| Blocker {
                start: slot.start,
                end: slot.end,
                title: part.title,
                transaction_id: part.transaction_id,
                event_id: None,
                state: BlockerState::Creating,
                message: String::new(),
            })
            .collect();
        self.hold_record = Some(record.id.clone());
        self.updated_at = now;
        self.validate()
    }

    /// Applies the creation outcome: verified BLOCKERs are held, the failed one is not created or
    /// uncertain, and later ones were never attempted.
    pub fn finish_hold(
        &mut self,
        state: crate::event_creation::State,
        event_ids: &[String],
        message: &str,
        now: DateTime<FixedOffset>,
    ) {
        use crate::event_creation::State;
        for (index, blocker) in self.blockers.iter_mut().enumerate() {
            if blocker.state != BlockerState::Creating {
                continue;
            }
            if let Some(id) = event_ids.get(index) {
                blocker.event_id = Some(id.clone());
                blocker.state = BlockerState::Held;
                blocker.message.clear();
            } else if index == event_ids.len() && state == State::Review {
                blocker.state = BlockerState::Review;
                blocker.message = message.chars().take(2000).collect();
            } else if index == event_ids.len() && state == State::NotSent {
                blocker.state = BlockerState::NotCreated;
                blocker.message = message.chars().take(2000).collect();
            } else {
                blocker.state = BlockerState::NotCreated;
                blocker.message = "Not attempted after an earlier BLOCKER failed".into();
            }
        }
        self.updated_at = now;
    }

    /// After a restart, anything in flight is uncertain and is never replayed.
    pub fn recover(&mut self) {
        for blocker in &mut self.blockers {
            match blocker.state {
                BlockerState::Creating => {
                    blocker.state = BlockerState::Review;
                    blocker.message = crate::workiq::connection::calendar_write::UNCERTAIN.into();
                }
                BlockerState::Releasing => {
                    blocker.state = BlockerState::ReleaseReview;
                    blocker.message = UNCERTAIN_RELEASE.into();
                }
                _ => {}
            }
        }
        if let Some(chat) = &mut self.chat
            && chat.state == ChatState::Sending
        {
            chat.state = ChatState::Review;
            chat.message = UNCERTAIN_CHAT.into();
        }
    }

    /// Marks the chosen held BLOCKERs for an approved deletion and returns them.
    pub fn begin_release(&mut self, event_ids: &[String], now: DateTime<FixedOffset>) -> Result<Vec<Blocker>, String> {
        if event_ids.is_empty() || self.in_flight() {
            return Err("Wait for the current proposal action to finish".into());
        }
        let mut chosen = Vec::new();
        for id in event_ids {
            let blocker = self
                .blockers
                .iter_mut()
                .find(|blocker| blocker.event_id.as_deref() == Some(id.as_str()) && blocker.state == BlockerState::Held)
                .ok_or("This BLOCKER is no longer held by the proposal")?;
            blocker.state = BlockerState::Releasing;
            blocker.message.clear();
            chosen.push(blocker.clone());
        }
        self.updated_at = now;
        Ok(chosen)
    }

    /// Applies deletion outcomes; a cancelled proposal closes once nothing is held any more.
    pub fn finish_release(&mut self, outcomes: &[ReleaseOutcome], cancel: bool, now: DateTime<FixedOffset>) {
        for outcome in outcomes {
            if let Some(blocker) = self.blockers.iter_mut().find(|blocker| {
                blocker.state == BlockerState::Releasing && blocker.event_id.as_deref() == Some(outcome.event_id.as_str())
            }) {
                blocker.state = outcome.state;
                blocker.message = outcome.message.chars().take(2000).collect();
            }
        }
        if cancel
            && self.status == ProposalStatus::Open
            && !self.blockers.iter().any(|blocker| matches!(blocker.state,
                BlockerState::Held | BlockerState::Creating | BlockerState::Releasing))
        {
            self.status = ProposalStatus::Cancelled;
        }
        self.updated_at = now;
    }

    /// Records an approved Teams proposal; allowed once unless nothing was created before.
    pub fn begin_chat(&mut self, draft: ChatDraft, now: DateTime<FixedOffset>) -> Result<(), String> {
        if self.status != ProposalStatus::Open || self.in_flight() {
            return Err("Wait for the current proposal action to finish".into());
        }
        if self.chat.as_ref().is_some_and(|chat| chat.state != ChatState::NotSent || chat.chat_id.is_some()) {
            return Err("This proposal was already sent to Teams; open the chat instead".into());
        }
        draft.validate()?;
        let invited = self.attendees.iter().map(|attendee| attendee.email.as_str()).collect::<BTreeSet<_>>();
        if draft.members.iter().any(|member| !invited.contains(member.email.as_str())) {
            return Err("Teams members must be the proposal's attendees".into());
        }
        self.chat = Some(ChatRecord {
            draft,
            state: ChatState::Sending,
            chat_id: None,
            web_url: None,
            message_id: None,
            message: String::new(),
        });
        self.updated_at = now;
        Ok(())
    }

    pub fn finish_chat(&mut self, result: &ChatResult, now: DateTime<FixedOffset>) {
        if let Some(chat) = &mut self.chat
            && chat.state == ChatState::Sending
        {
            chat.state = result.state;
            chat.chat_id = result.chat_id.clone().filter(|id| valid_chat_id(id));
            chat.web_url = result.web_url.clone().filter(|url| url.len() <= 4000 && crate::calendar::safe_url(url));
            chat.message_id = result.message_id.clone();
            chat.message = result.message.chars().take(2000).collect();
        }
        self.updated_at = now;
    }

    pub fn schedule(&mut self, record: &str, now: DateTime<FixedOffset>) -> Result<(), String> {
        if self.status == ProposalStatus::Cancelled {
            return Err("This proposal was cancelled".into());
        }
        self.status = ProposalStatus::Scheduled;
        self.meeting_record = Some(record.to_owned());
        self.updated_at = now;
        Ok(())
    }
}

/// Graph chat IDs such as `19:…@thread.v2`.
pub fn valid_chat_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 300
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'@' | b'.' | b'-' | b'_' | b'='))
}

/// The approved Teams proposal handed to the WorkIQ executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatRequest {
    pub proposal: String,
    pub account: String,
    pub draft: ChatDraft,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatResult {
    pub proposal: String,
    pub state: ChatState,
    pub chat_id: Option<String>,
    pub web_url: Option<String>,
    pub message_id: Option<String>,
    pub message: String,
}

/// Read-only result of the Teams check: resolved members and people who cannot be added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatCheck {
    pub members: Vec<Member>,
    /// Address and reason.
    pub unresolved: Vec<(String, String)>,
}

/// Graph `POST /chats` body: you and every member as owners, bound by user ID; the topic only for
/// group chats.
pub fn chat_body(draft: &ChatDraft, own_upn: &str) -> Result<Value, String> {
    draft.validate()?;
    if person_email(own_upn).as_deref() != Some(own_upn) {
        return Err("Your account cannot be added to the chat".into());
    }
    let member = |id: &str| {
        json!({
            "@odata.type": "#microsoft.graph.aadUserConversationMember",
            "roles": ["owner"],
            "user@odata.bind": format!("https://graph.microsoft.com/v1.0/users('{id}')"),
        })
    };
    let members = std::iter::once(member(own_upn))
        .chain(draft.members.iter().map(|person| member(&person.user_id)))
        .collect::<Vec<_>>();
    let mut body = json!({
        "chatType": if draft.one_on_one() { "oneOnOne" } else { "group" },
        "members": members,
    });
    if !draft.one_on_one()
        && let Some(topic) = &draft.topic
    {
        body["topic"] = json!(topic.trim());
    }
    Ok(body)
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '\r' => {}
            '\n' => escaped.push_str("<br>"),
            ch => escaped.push(ch),
        }
    }
    escaped
}

/// Graph `POST /chats/{id}/messages` body: the message as escaped HTML with line breaks.
pub fn message_body(message: &str) -> Result<Value, String> {
    validate_message(message)?;
    Ok(json!({"body": {"contentType": "html", "content": escape_html(message.trim())}}))
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a read-back chat message carries exactly the approved text.
pub fn message_matches(value: &Value, message: &str) -> bool {
    let Some(content) = value.pointer("/body/content").and_then(Value::as_str) else {
        return false;
    };
    let text = if value
        .pointer("/body/contentType")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("html"))
    {
        crate::html_text::extract(content)
    } else {
        content.to_owned()
    };
    normalized(&text) == normalized(message)
}

/// The consequence of holding the selected times.
pub fn hold_notice(count: usize, language: Language) -> String {
    match language {
        Language::En => format!(
            "Creates {count} BLOCKER {} in your calendar: busy, no attendees, no Teams meeting and no reminder.",
            if count == 1 { "event" } else { "events" }
        ),
        Language::Cs => format!(
            "Vytvoří ve vašem kalendáři {count} {} BLOCKER: obsazeno, bez účastníků, bez schůzky Teams a bez připomenutí.",
            match count {
                1 => "událost",
                2..=4 => "události",
                _ => "událostí",
            }
        ),
    }
}

pub fn hold_button(count: usize, language: Language) -> String {
    match language {
        Language::En => format!("Create {count} {}", if count == 1 { "BLOCKER" } else { "BLOCKERs" }),
        Language::Cs => format!(
            "Vytvořit {count} {}",
            match count {
                1 => "BLOCKER",
                2..=4 => "BLOCKERy",
                _ => "BLOCKERů",
            }
        ),
    }
}

pub fn release_notice(count: usize, language: Language) -> String {
    match language {
        Language::En => format!(
            "Deletes {count} BLOCKER {} from your calendar. Nobody is notified.",
            if count == 1 { "event" } else { "events" }
        ),
        Language::Cs => format!(
            "Smaže z vašeho kalendáře {count} {} BLOCKER. Nikdo nedostane oznámení.",
            match count {
                1 => "událost",
                2..=4 => "události",
                _ => "událostí",
            }
        ),
    }
}

pub fn release_button(count: usize, language: Language) -> String {
    match language {
        Language::En => format!("Delete {count} {}", if count == 1 { "BLOCKER" } else { "BLOCKERs" }),
        Language::Cs => format!(
            "Smazat {count} {}",
            match count {
                1 => "BLOCKER",
                2..=4 => "BLOCKERy",
                _ => "BLOCKERů",
            }
        ),
    }
}

/// The consequence of the Teams proposal.
pub fn chat_notice(draft: &ChatDraft, language: Language) -> String {
    let count = draft.members.len();
    match (draft.one_on_one(), language) {
        (true, Language::En) => "This creates (or reuses) your one-on-one Teams chat and sends this message to 1 person.".into(),
        (true, Language::Cs) => "Tímto se vytvoří (nebo použije existující) chat v Teams jen s touto osobou a zpráva se odešle 1 osobě.".into(),
        (false, Language::En) => format!("This creates a Teams chat and sends this message to {count} people."),
        (false, Language::Cs) => format!(
            "Tímto se vytvoří chat v Teams a zpráva se odešle {count} {}.",
            if count == 1 { "osobě" } else { "lidem" }
        ),
    }
}

/// An approved deletion of held BLOCKERs, matched by event ID, transaction ID and title prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseRequest {
    pub proposal: String,
    pub account: String,
    pub blockers: Vec<Blocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseOutcome {
    pub event_id: String,
    pub state: BlockerState,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseResult {
    pub proposal: String,
    pub outcomes: Vec<ReleaseOutcome>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::availability::{Item, Schedule};
    use chrono::Timelike;
    use chrono_tz::Europe::Prague;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    fn local(day: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
        Prague.from_local_datetime(&day.and_hms_opt(hour, minute, 0).unwrap()).single().unwrap().with_timezone(&Utc)
    }

    pub fn people() -> Vec<Attendee> {
        let mut people = crate::attendees::parse_input(
            "Alex Rivers <alex@example.com>; Jamie Brooks <jamie@example.com>; Morgan Lee <morgan@example.com>",
        )
        .unwrap();
        people[2].role = Role::Optional;
        people
    }

    /// Free/busy where `busy` lists (email, status, local day, from minute, to minute).
    fn availability(search: &Search, emails: &[&str], busy: &[(&str, Status, NaiveDate, u32, u32)]) -> Availability {
        let request = search.schedule_request_in(&Prague, emails.iter().map(|email| (*email).to_owned()).collect()).unwrap();
        let schedules = emails
            .iter()
            .map(|email| {
                if busy.iter().any(|(who, status, ..)| who == email && *status == Status::Unknown) {
                    return Schedule {
                        email: (*email).into(),
                        view: vec![Status::Unknown; request.cells()],
                        items: Vec::new(),
                        error: Some("Availability is not shared with you".into()),
                    };
                }
                let items = busy
                    .iter()
                    .filter(|(who, ..)| who == email)
                    .map(|(_, status, day, from, to)| Item {
                        status: *status,
                        start: local(*day, from / 60, from % 60),
                        end: local(*day, to / 60, to % 60),
                        subject: None,
                        location: None,
                    })
                    .collect();
                Schedule::from_items(&request, (*email).into(), items)
            })
            .collect();
        Availability { start: request.start, end: request.end, interval: request.interval, schedules }
    }

    const ALL: [&str; 4] = ["me@example.com", "alex@example.com", "jamie@example.com", "morgan@example.com"];

    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()
    }

    fn search() -> Search {
        Search::new(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(), 30)
    }

    fn run(search: &Search, availability: &Availability) -> Candidates {
        candidates_in(&Prague, search, availability, "me@example.com", &people(), at("2026-11-01T12:00:00+01:00")).unwrap()
    }

    fn local_minutes(time: DateTime<Utc>) -> u32 {
        let time = time.with_timezone(&Prague);
        time.hour() * 60 + time.minute()
    }

    #[test]
    fn defaults_are_five_workdays_from_tomorrow_with_an_eight_to_six_window() {
        let friday = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let search = Search::new(friday, 30);
        assert_eq!(search.first, NaiveDate::from_ymd_opt(2026, 11, 9).unwrap(), "Tomorrow is Saturday; Monday is first");
        assert_eq!(search.dates().len(), 5);
        assert_eq!((search.window, search.minimum, search.weekdays_only), ((480, 1080), 3, true));
        let mut longest = search.clone();
        longest.days = 15;
        let dates = longest.dates();
        assert_eq!(dates.len(), 15);
        assert!(dates.iter().all(|day| day.weekday().num_days_from_monday() < 5));
        assert_eq!(*dates.last().unwrap(), NaiveDate::from_ymd_opt(2026, 11, 27).unwrap());
        assert!(longest.schedule_request_in(&Prague, vec!["me@example.com".into()]).is_ok(), "15 workdays fit one request");
        for change in [
            (|search: &mut Search| search.minutes = 10) as fn(&mut Search),
            |search| search.minutes = 255,
            |search| search.days = 16,
            |search| search.days = 0,
            |search| search.window = (600, 615),
            |search| search.window = (1080, 480),
            |search| search.minimum = 11,
        ] {
            let mut invalid = search.clone();
            change(&mut invalid);
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }
        assert_eq!(slot_label_in(&Prague, at("2026-11-03T09:00:00Z"), at("2026-11-03T09:30:00Z"), Language::En), "Tue 3 Nov 10:00–10:30");
        assert_eq!(slot_label_in(&Prague, at("2026-11-03T09:00:00Z"), at("2026-11-03T09:30:00Z"), Language::Cs), "út 3. 11. 10:00–10:30");
    }

    #[test]
    fn required_attendees_must_be_free_and_optional_conflicts_rank_lower() {
        let search = search();
        let mut busy = Vec::new();
        for offset in 0..5 {
            let day = monday() + Duration::days(offset);
            // Alex (required) is busy all day except 10:00-11:00 and 15:00-16:00.
            busy.push(("alex@example.com", Status::Busy, day, 480, 600));
            busy.push(("alex@example.com", Status::Busy, day, 660, 900));
            busy.push(("alex@example.com", Status::Busy, day, 960, 1080));
            // Morgan (optional) is busy 10:00-11:00.
            busy.push(("morgan@example.com", Status::Busy, day, 600, 660));
        }
        let result = run(&search, &availability(&search, &ALL, &busy));
        assert_eq!(result.list.len(), result.ideal);
        assert!(result.ideal >= 5);
        for candidate in &result.list {
            assert!(candidate.all_required);
            let minute = local_minutes(candidate.start);
            assert!((600..660).contains(&minute) || (900..960).contains(&minute), "{minute}");
        }
        let afternoon = result.list.iter().filter(|candidate| local_minutes(candidate.start) >= 900).count();
        assert_eq!(afternoon, 5, "The afternoon without optional conflicts is offered on every day first");
        let morning = result.list.iter().find(|candidate| local_minutes(candidate.start) < 900).unwrap();
        assert_eq!(morning.reason(&people(), Language::En), "All required free · Morgan Lee busy (optional)");
        assert_eq!(morning.reason(&people(), Language::Cs), "Povinní účastníci mají volno · Morgan Lee: obsazeno (nepovinně)");
        let free = result.list.iter().find(|candidate| local_minutes(candidate.start) >= 900).unwrap();
        assert_eq!(free.reason(&people(), Language::En), "All free");
        assert_eq!(free.reason(&people(), Language::Cs), "Všichni mají volno");
    }

    #[test]
    fn your_busy_time_and_the_daily_window_are_never_proposed_and_days_are_spread() {
        let mut search = search();
        search.window = (540, 720);
        let busy = (0..5)
            .map(|offset| ("me@example.com", Status::Busy, monday() + Duration::days(offset), 540, 600))
            .collect::<Vec<_>>();
        let result = run(&search, &availability(&search, &ALL, &busy));
        assert_eq!(result.list.len(), 10);
        let days = result.list.iter().map(|candidate| candidate.start.with_timezone(&Prague).date_naive()).collect::<BTreeSet<_>>();
        assert_eq!(days.len(), 5, "Spread across every searched day");
        for candidate in &result.list {
            let minute = local_minutes(candidate.start);
            assert!(minute >= 600 && minute + 30 <= 720, "Outside the window or in your busy time: {minute}");
            assert_eq!(candidate.you, Status::Free);
        }
        for pair in result.list.windows(2) {
            if pair[0].start.with_timezone(&Prague).date_naive() == pair[1].start.with_timezone(&Prague).date_naive() {
                assert!(pair[1].start - pair[0].start >= Duration::minutes(60), "Same-day candidates are an hour apart");
            }
        }
        assert!(result.list.iter().all(|candidate| local_minutes(candidate.start).is_multiple_of(30)), "Half hours are preferred");
    }

    #[test]
    fn weekends_are_skipped_unless_allowed_and_past_times_are_never_offered() {
        let mut search = search();
        search.first = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        search.days = 3;
        let result = candidates_in(&Prague, &search, &availability(&search, &ALL, &[]), "me@example.com", &people(),
            at("2026-11-06T15:10:00+01:00")).unwrap();
        let days = result.list.iter().map(|candidate| candidate.start.with_timezone(&Prague).date_naive().weekday().num_days_from_monday()).collect::<BTreeSet<_>>();
        assert!(!days.contains(&5) && !days.contains(&6));
        assert!(result.list.iter().all(|candidate| candidate.start >= at("2026-11-06T15:10:00+01:00")));
        search.weekdays_only = false;
        let result = candidates_in(&Prague, &search, &availability(&search, &ALL, &[]), "me@example.com", &people(),
            at("2026-11-06T15:10:00+01:00")).unwrap();
        let days = result.list.iter().map(|candidate| candidate.start.with_timezone(&Prague).date_naive().weekday().num_days_from_monday()).collect::<BTreeSet<_>>();
        assert!(days.contains(&5) && days.contains(&6));
    }

    #[test]
    fn lunch_is_avoided_when_possible_but_used_when_nothing_else_fits() {
        let mut search = search();
        search.days = 1;
        search.minimum = 1;
        let day = monday();
        let open = availability(&search, &ALL, &[("alex@example.com", Status::Busy, day, 480, 690), ("alex@example.com", Status::Busy, day, 840, 1080)]);
        let result = run(&search, &open);
        assert!(result.list.iter().all(|candidate| local_minutes(candidate.start) >= 780 || local_minutes(candidate.start) + 30 <= 720),
            "Lunch is a last resort");
        assert!(result.list.iter().any(|candidate| local_minutes(candidate.start) >= 780));
        let lunch_only = availability(&search, &ALL, &[("alex@example.com", Status::Busy, day, 480, 720), ("alex@example.com", Status::Busy, day, 750, 1080)]);
        let result = run(&search, &lunch_only);
        assert_eq!(result.ideal, 1);
        assert_eq!(local_minutes(result.list[0].start), 720);
    }

    #[test]
    fn unknown_is_never_free_and_alternatives_fill_the_minimum() {
        let mut search = search();
        search.days = 2;
        let busy = [("jamie@example.com", Status::Unknown, monday(), 0, 0)];
        let result = run(&search, &availability(&search, &ALL, &busy));
        assert_eq!(result.ideal, 0, "Unknown availability is not free");
        assert_eq!(result.list.len(), 3, "The minimum is filled with the best alternatives");
        assert!(result.list.iter().all(|candidate| !candidate.all_required));
        assert_eq!(result.list[0].reason(&people(), Language::En), "Jamie Brooks unknown");
        assert_eq!(result.list[0].reason(&people(), Language::Cs), "Jamie Brooks: neznámé");
        let mut tentative = Vec::new();
        for offset in 0..2 {
            let day = monday() + Duration::days(offset);
            tentative.push(("jamie@example.com", Status::Tentative, day, 480, 1080));
            tentative.push(("alex@example.com", Status::Busy, day, 480, 1020));
        }
        tentative.push(("alex@example.com", Status::Busy, monday(), 1020, 1080));
        let result = run(&search, &availability(&search, &ALL, &tentative));
        assert_eq!(result.ideal, 0);
        let alex_free = result.list.iter().filter(|candidate| candidate.attendees[0].1 == Status::Free).collect::<Vec<_>>();
        assert_eq!(alex_free.len(), 1, "Tentative ranks above busy: the only time Alex is free is offered");
        assert!(local_minutes(alex_free[0].start) >= 1020);
        assert_eq!(alex_free[0].reason(&people(), Language::En), "Jamie Brooks tentative");
        let own_unknown = [("me@example.com", Status::Unknown, monday(), 0, 0)];
        let result = run(&search, &availability(&search, &ALL, &own_unknown));
        assert_eq!(result.ideal, 0, "Your own unknown availability is not free either");
        assert_eq!(result.list[0].reason(&people(), Language::En), "You unknown");
    }

    #[test]
    fn daylight_saving_days_use_real_local_times() {
        let mut search = Search::new(NaiveDate::from_ymd_opt(2026, 10, 24).unwrap(), 60);
        search.weekdays_only = false;
        search.first = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        search.days = 1;
        search.window = (0, 360);
        search.minimum = 10;
        let autumn = availability(&search, &ALL, &[]);
        assert_eq!(autumn.end - autumn.start, Duration::hours(25));
        let result = candidates_in(&Prague, &search, &autumn, "me@example.com", &people(), at("2026-10-01T00:00:00Z")).unwrap();
        assert!(!result.list.is_empty());
        for candidate in &result.list {
            assert_eq!(candidate.end - candidate.start, Duration::hours(1));
            let start = candidate.start.with_timezone(&Prague);
            assert!(start.hour() != 2, "Repeated 02:00-03:00 local times are never guessed: {start}");
            assert!(candidate.end.with_timezone(&Prague).naive_local() <= NaiveDate::from_ymd_opt(2026, 10, 25).unwrap().and_hms_opt(6, 0, 0).unwrap());
        }
        let mut spring = search.clone();
        spring.first = NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        let result = candidates_in(&Prague, &spring, &availability(&spring, &ALL, &[]), "me@example.com", &people(),
            at("2026-03-01T00:00:00Z")).unwrap();
        assert!(result.list.iter().all(|candidate| candidate.start.with_timezone(&Prague).hour() != 2), "Missing local times are skipped");
        let ten = local(NaiveDate::from_ymd_opt(2026, 10, 25).unwrap(), 10, 0);
        assert_eq!(ten, at("2026-10-25T09:00:00Z"), "After the change 10:00 local is 09:00 UTC");
    }

    #[test]
    fn selection_holds_one_to_five_distinct_times_in_order() {
        let slot = |hour: i64| Slot {
            start: DateTime::parse_from_rfc3339("2026-11-03T08:00:00+01:00").unwrap() + Duration::hours(hour),
            end: DateTime::parse_from_rfc3339("2026-11-03T08:30:00+01:00").unwrap() + Duration::hours(hour),
        };
        let mut selected = Vec::new();
        for hour in [4, 1, 3, 0, 2] {
            toggle(&mut selected, slot(hour)).unwrap();
        }
        assert_eq!(selected, (0..5).map(slot).collect::<Vec<_>>());
        assert_eq!(toggle(&mut selected, slot(6)), Err("Select at most 5 times".into()));
        toggle(&mut selected, slot(2)).unwrap();
        assert_eq!(selected.len(), 4, "Selecting again removes it");
    }

    pub fn proposal(language: Language) -> Proposal {
        let start = DateTime::parse_from_rfc3339("2026-11-03T10:00:00+01:00").unwrap();
        Proposal::new(
            "owner@example.com",
            "Pilot kickoff",
            people(),
            30,
            vec![
                Slot { start: start + Duration::days(1), end: start + Duration::days(1) + Duration::minutes(30) },
                Slot { start, end: start + Duration::minutes(30) },
            ],
            language,
            DateTime::parse_from_rfc3339("2026-11-01T09:00:00+01:00").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn proposals_validate_and_build_bilingual_blocker_texts() {
        let english = proposal(Language::En);
        assert_eq!(english.id.len(), 32);
        assert!(english.candidates[0] < english.candidates[1], "Times are kept in order");
        let (first, second) = (english.candidates[0].label(Language::En), english.candidates[1].label(Language::En));
        let body = english.blocker_body();
        assert_eq!(body, format!("Proposed time for: Pilot kickoff (30 min)\n\nInvitees:\n- Alex Rivers, alex@example.com\n- Jamie Brooks, jamie@example.com\n- Morgan Lee, morgan@example.com (optional)\n\nProposed times:\n1) {first}\n2) {second}\n\nHeld by Tomlook until the meeting is scheduled. Confirm or release it in Tomlook."));
        let czech = proposal(Language::Cs);
        assert!(czech.blocker_body().starts_with("Navržený termín pro: Pilot kickoff (30 min)\n\nPozvaní:\n- Alex Rivers, alex@example.com"));
        assert!(czech.blocker_body().contains(&format!("- Morgan Lee, morgan@example.com (nepovinně)\n\nNavržené termíny:\n1) {}\n2) {}",
            czech.candidates[0].label(Language::Cs), czech.candidates[1].label(Language::Cs))));
        let draft = english.hold_draft(false, None).unwrap();
        assert_eq!(draft.title, "BLOCKER: Pilot kickoff");
        assert_eq!(czech.hold_draft(true, None).unwrap().title, "BLOCKER: Pilot kickoff", "The literal word stays in Czech");
        assert!(draft.attendees.is_empty() && !draft.teams && draft.travel_minutes.is_none() && draft.category.is_none());
        let mut invalid = english.clone();
        invalid.candidates[1].end += Duration::minutes(15);
        assert!(invalid.validate().is_err(), "Every time lasts the meeting's duration");
        let mut invalid = english.clone();
        invalid.candidates.push(invalid.candidates[0]);
        assert!(invalid.validate().is_err());
        let mut invalid = english.clone();
        invalid.topic = "\u{7}".into();
        assert!(invalid.validate().is_err());
        assert!(Proposal::new("owner@example.com", "Pilot", Vec::new(), 30, english.candidates.clone(), Language::En,
            english.created_at).is_err());
        assert_eq!(hold_notice(2, Language::En), "Creates 2 BLOCKER events in your calendar: busy, no attendees, no Teams meeting and no reminder.");
        assert_eq!(hold_notice(1, Language::Cs), "Vytvoří ve vašem kalendáři 1 událost BLOCKER: obsazeno, bez účastníků, bez schůzky Teams a bez připomenutí.");
        assert_eq!(hold_notice(5, Language::Cs), "Vytvoří ve vašem kalendáři 5 událostí BLOCKER: obsazeno, bez účastníků, bez schůzky Teams a bez připomenutí.");
        assert_eq!((hold_button(2, Language::En), hold_button(1, Language::En)), ("Create 2 BLOCKERs".into(), "Create 1 BLOCKER".into()));
        assert_eq!((hold_button(3, Language::Cs), hold_button(5, Language::Cs)), ("Vytvořit 3 BLOCKERy".into(), "Vytvořit 5 BLOCKERů".into()));
        assert_eq!(release_notice(2, Language::En), "Deletes 2 BLOCKER events from your calendar. Nobody is notified.");
        assert_eq!(release_button(2, Language::Cs), "Smazat 2 BLOCKERy");
    }

    fn member(email: &str, name: &str, id: char) -> Member {
        Member { email: email.into(), name: name.into(), user_id: format!("{0}{0}{0}{0}{0}{0}{0}{0}-0000-4000-8000-00000000000{0}", id) }
    }

    #[test]
    fn chat_bodies_bind_you_and_every_member_and_give_group_chats_a_topic() {
        let proposal = proposal(Language::En);
        let members = vec![member("alex@example.com", "Alex Rivers", 'a'), member("jamie@example.com", "Jamie Brooks", 'b')];
        let group = ChatDraft { members: members.clone(), topic: Some("Pilot kickoff".into()), message: proposal.chat_message(&members) };
        let body = chat_body(&group, "owner@example.com").unwrap();
        assert_eq!(body["chatType"], "group");
        assert_eq!(body["topic"], "Pilot kickoff");
        let binds = body["members"].as_array().unwrap().iter().map(|member| {
            assert_eq!(member["@odata.type"], "#microsoft.graph.aadUserConversationMember");
            assert_eq!(member["roles"], json!(["owner"]));
            member["user@odata.bind"].as_str().unwrap().to_owned()
        }).collect::<Vec<_>>();
        assert_eq!(binds, [
            "https://graph.microsoft.com/v1.0/users('owner@example.com')".to_owned(),
            format!("https://graph.microsoft.com/v1.0/users('{}')", members[0].user_id),
            format!("https://graph.microsoft.com/v1.0/users('{}')", members[1].user_id),
        ]);
        let one = ChatDraft { members: members[..1].to_vec(), topic: None, message: proposal.chat_message(&members[..1]) };
        let body = chat_body(&one, "owner@example.com").unwrap();
        assert_eq!(body["chatType"], "oneOnOne");
        assert!(body.get("topic").is_none(), "One-on-one chats have no topic");
        assert_eq!(body["members"].as_array().unwrap().len(), 2);
        assert!(chat_body(&ChatDraft { topic: Some("x".into()), ..one.clone() }, "owner@example.com").is_err());
        assert!(chat_body(&ChatDraft { topic: None, ..group.clone() }, "owner@example.com").is_err());
        assert!(chat_body(&group, "Owner@example.com").is_err());
        let mut duplicate = group.clone();
        duplicate.members.push(members[0].clone());
        assert!(chat_body(&duplicate, "owner@example.com").is_err());
        let mut unresolved = group.clone();
        unresolved.members[0].user_id = "alex@example.com".into();
        assert!(chat_body(&unresolved, "owner@example.com").is_err(), "Members are bound by resolved IDs only");
        let message = message_body("Hi <all> & \"team\"\n1) Tue").unwrap();
        assert_eq!(message, json!({"body":{"contentType":"html","content":"Hi &lt;all&gt; &amp; &quot;team&quot;<br>1) Tue"}}));
        assert!(message_matches(&json!({"id":"1","body":{"contentType":"html","content":"<p>Hi &lt;all&gt; &amp; &quot;team&quot;<br>1) Tue</p>"}}), "Hi <all> & \"team\"\n1) Tue"));
        assert!(!message_matches(&json!({"body":{"contentType":"html","content":"Hi all"}}), "Hi <all>"));
        assert!(message_body(" ").is_err() && message_body(&"x".repeat(MESSAGE_LIMIT + 1)).is_err());
    }

    #[test]
    fn proposal_messages_and_emails_are_short_and_bilingual() {
        let english = proposal(Language::En);
        let members = vec![member("alex@example.com", "Alex Rivers", 'a'), member("jamie@example.com", "Jamie Brooks", 'b')];
        assert_eq!(english.chat_message_in(&Prague, &members), "Hi all, I'd like to schedule Pilot kickoff (30 min). Which of these times works for you?\n1) Tue 3 Nov 10:00–10:30 (UTC+01:00)\n2) Wed 4 Nov 10:00–10:30 (UTC+01:00)\nPlease reply here. Thanks!");
        assert!(english.chat_message_in(&Prague, &members[..1]).starts_with("Hi Alex, I'd like"));
        let czech = proposal(Language::Cs);
        assert_eq!(czech.chat_message_in(&Prague, &members), "Dobrý den všem, potřebuji naplánovat schůzku „Pilot kickoff“ (30 min). Který z těchto termínů vám vyhovuje?\n1) út 3. 11. 10:00–10:30 (UTC+01:00)\n2) st 4. 11. 10:00–10:30 (UTC+01:00)\nOdpovězte prosím tady v chatu. Děkuji!");
        assert_eq!(english.email_subject(), "Scheduling: Pilot kickoff – which time works for you?");
        assert_eq!(czech.email_subject(), "Plánování: Pilot kickoff – který termín vám vyhovuje?");
        assert_eq!(english.email_body_in(&Prague), "Hi all,\n\nI'd like to schedule Pilot kickoff (30 min). Which of these times works for you?\n1) Tue 3 Nov 10:00–10:30 (UTC+01:00)\n2) Wed 4 Nov 10:00–10:30 (UTC+01:00)\n\nPlease reply to this e-mail. Thanks!");
        assert_eq!(czech.email_body_in(&Prague), "Dobrý den všem,\n\npotřebuji naplánovat schůzku „Pilot kickoff“ (30 min). Který z těchto termínů vám vyhovuje?\n1) út 3. 11. 10:00–10:30 (UTC+01:00)\n2) st 4. 11. 10:00–10:30 (UTC+01:00)\n\nOdpovězte prosím na tento e-mail. Děkuji!");
        // The defaults use the Windows time zone and state each time's own offset there.
        let times = |proposal: &Proposal| proposal.candidates.iter().enumerate()
            .map(|(index, slot)| format!("{}) {} ({})", index + 1, slot.label(proposal.language),
                utc_offset_label(&chrono::Local, slot.start.with_timezone(&Utc))))
            .collect::<Vec<_>>().join("\n");
        assert_eq!(english.chat_message(&members), english.chat_message_in(&chrono::Local, &members));
        assert!(english.chat_message(&members).contains(&format!("\n{}\n", times(&english))));
        assert!(czech.email_body().contains(&format!("\n{}\n", times(&czech))));
        assert!(!english.blocker_body().contains("UTC"), "Your own BLOCKERs stay in local time");
        let to = english.attendees.iter().map(|attendee| attendee.email.as_str()).collect::<Vec<_>>();
        let link = crate::followup::mailto(&to, &english.email_subject(), &english.email_body()).unwrap();
        assert!(link.uri.starts_with("mailto:alex@example.com,jamie@example.com,morgan@example.com?subject=Scheduling%3A%20Pilot"));
        assert!(!link.body_in_clipboard);
        crate::browser::validate_mailto(&link.uri).unwrap();
        let notice = chat_notice(&ChatDraft { members: members.clone(), topic: Some("Pilot".into()), message: "x".into() }, Language::En);
        assert_eq!(notice, "This creates a Teams chat and sends this message to 2 people.");
        assert_eq!(chat_notice(&ChatDraft { members: members.clone(), topic: Some("Pilot".into()), message: "x".into() }, Language::Cs),
            "Tímto se vytvoří chat v Teams a zpráva se odešle 2 lidem.");
        assert!(chat_notice(&ChatDraft { members: members[..1].to_vec(), topic: None, message: "x".into() }, Language::En).contains("one-on-one"));
        assert_eq!(duration_label(90), "1 h 30 min");
        assert_eq!(duration_label(120), "2 h");
    }

    #[test]
    fn times_sent_to_others_carry_their_own_utc_offset_across_daylight_saving() {
        let slot = |text: &str| {
            let start = DateTime::parse_from_rfc3339(text).unwrap();
            Slot { start, end: start + Duration::minutes(30) }
        };
        // Prague leaves summer time on Sunday 25 October 2026.
        let candidates = vec![slot("2026-10-23T10:00:00+02:00"), slot("2026-10-26T10:00:00+01:00")];
        let created = DateTime::parse_from_rfc3339("2026-10-20T09:00:00+02:00").unwrap();
        let english = Proposal::new("owner@example.com", "Pilot kickoff", people(), 30, candidates.clone(), Language::En, created).unwrap();
        let czech = Proposal::new("owner@example.com", "Pilot kickoff", people(), 30, candidates, Language::Cs, created).unwrap();
        let members = vec![member("alex@example.com", "Alex Rivers", 'a')];
        assert_eq!(english.chat_message_in(&Prague, &members), "Hi Alex, I'd like to schedule Pilot kickoff (30 min). Which of these times works for you?\n1) Fri 23 Oct 10:00–10:30 (UTC+02:00)\n2) Mon 26 Oct 10:00–10:30 (UTC+01:00)\nPlease reply here. Thanks!");
        assert!(english.email_body_in(&Prague).contains("\n1) Fri 23 Oct 10:00–10:30 (UTC+02:00)\n2) Mon 26 Oct 10:00–10:30 (UTC+01:00)\n"));
        assert!(czech.chat_message_in(&Prague, &members).contains("\n1) pá 23. 10. 10:00–10:30 (UTC+02:00)\n2) po 26. 10. 10:00–10:30 (UTC+01:00)\n"));
        assert!(czech.email_body_in(&Prague).contains("\n1) pá 23. 10. 10:00–10:30 (UTC+02:00)\n2) po 26. 10. 10:00–10:30 (UTC+01:00)\n"));
        let utc = FixedOffset::east_opt(0).unwrap();
        assert!(english.email_body_in(&utc).contains("\n1) Fri 23 Oct 08:00–08:30 (UTC+00:00)\n2) Mon 26 Oct 09:00–09:30 (UTC+00:00)\n"),
            "The same instants in another zone");
        let new_york = FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(english.candidates[1].outgoing_label_in(&new_york, Language::En), "Mon 26 Oct 04:00–04:30 (UTC-05:00)");
        assert_eq!(utc_offset_label(&Prague, at("2026-10-25T00:59:00Z")), "UTC+02:00");
        assert_eq!(utc_offset_label(&Prague, at("2026-10-25T01:00:00Z")), "UTC+01:00");
        assert!(!english.blocker_body().contains("UTC") && !czech.blocker_body().contains("UTC"));
    }

    #[test]
    fn hold_release_and_chat_states_are_single_use_and_never_replayed() {
        use crate::event_creation::{Record, State};
        let mut proposal = proposal(Language::En);
        let now = proposal.created_at;
        let draft = proposal.hold_draft(false, None).unwrap();
        let record = Record::blockers("owner@example.com".into(), proposal.id.clone(), draft, proposal.candidates.clone()).unwrap();
        proposal.begin_hold(&record, now).unwrap();
        assert!(proposal.begin_hold(&record, now).is_err(), "One BLOCKER approval per proposal");
        assert_eq!(proposal.blockers.len(), 2);
        assert!(proposal.blockers.iter().all(|blocker| blocker.state == BlockerState::Creating && blocker.title == "BLOCKER: Pilot kickoff"));
        assert_eq!(proposal.blockers[1].transaction_id, format!("{}-blocker-1", record.id));
        let mut restarted = proposal.clone();
        restarted.recover();
        assert!(restarted.blockers.iter().all(|blocker| blocker.state == BlockerState::Review), "Never replayed after a restart");
        proposal.finish_hold(State::Review, &["first".into()], "Uncertain", now);
        assert_eq!(proposal.blockers[0].state, BlockerState::Held);
        assert_eq!(proposal.blockers[1].state, BlockerState::Review);
        assert!(proposal.needs_review());
        let mut partial = restarted.clone();
        partial.blockers.iter_mut().for_each(|blocker| blocker.state = BlockerState::Creating);
        partial.finish_hold(State::NotSent, &[], "Rejected", now);
        assert_eq!(partial.blockers.iter().map(|blocker| blocker.state).collect::<Vec<_>>(), [BlockerState::NotCreated, BlockerState::NotCreated]);
        assert_eq!(partial.blockers[1].message, "Not attempted after an earlier BLOCKER failed");
        assert_eq!(proposal.held().count(), 1);
        assert!(proposal.begin_release(&["unknown".into()], now).is_err());
        let chosen = proposal.begin_release(&["first".into()], now).unwrap();
        assert_eq!(chosen.len(), 1);
        assert!(proposal.begin_release(&["first".into()], now).is_err(), "Wait for the release");
        proposal.finish_release(&[ReleaseOutcome { event_id: "first".into(), state: BlockerState::Released, message: String::new() }], true, now);
        assert_eq!(proposal.blockers[0].state, BlockerState::Released);
        assert_eq!(proposal.status, ProposalStatus::Cancelled, "Nothing is held any more");
        let mut chat = self::proposal(Language::En);
        let draft = ChatDraft { members: vec![member("alex@example.com", "Alex Rivers", 'a')], topic: None, message: "Hi".into() };
        chat.begin_chat(draft.clone(), now).unwrap();
        assert!(chat.begin_chat(draft.clone(), now).is_err());
        let mut restarted = chat.clone();
        restarted.recover();
        assert_eq!(restarted.chat.as_ref().unwrap().state, ChatState::Review);
        assert!(restarted.begin_chat(draft.clone(), now).is_err(), "An uncertain chat is never sent again");
        chat.finish_chat(&ChatResult { proposal: chat.id.clone(), state: ChatState::NotSent, chat_id: None, web_url: None,
            message_id: None, message: "Rejected".into() }, now);
        chat.begin_chat(draft.clone(), now).unwrap();
        chat.finish_chat(&ChatResult { proposal: chat.id.clone(), state: ChatState::Sent, chat_id: Some("19:abc@thread.v2".into()),
            web_url: Some("https://teams.microsoft.com/l/chat/19%3Aabc".into()), message_id: Some("1".into()), message: String::new() }, now);
        assert!(chat.begin_chat(draft, now).is_err());
        let stranger = ChatDraft { members: vec![member("eve@example.com", "Eve", 'e')], topic: None, message: "Hi".into() };
        assert!(self::proposal(Language::En).begin_chat(stranger, now).is_err(), "Only the proposal's attendees");
        let mut scheduled = self::proposal(Language::En);
        scheduled.schedule("record", now).unwrap();
        assert_eq!(scheduled.status, ProposalStatus::Scheduled);
        let saved: Proposal = serde_json::from_value(serde_json::to_value(&chat).unwrap()).unwrap();
        saved.validate().unwrap();
        assert_eq!(saved, chat);
    }

    #[test]
    fn blockers_are_recognized_by_prefix_and_no_attendees() {
        let mut event = crate::calendar::fixture(1).remove(0);
        event.title = "BLOCKER: Pilot".into();
        event.attendees.clear();
        assert!(is_blocker(&event));
        event.attendees.push("alex@example.com".into());
        assert!(!is_blocker(&event), "A real meeting with attendees is never a BLOCKER");
        event.attendees.clear();
        event.title = "Blocker: Pilot".into();
        assert!(!is_blocker(&event));
    }
}
