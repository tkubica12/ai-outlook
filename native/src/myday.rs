//! Proactive "My day" overview: today's meetings, conflicts, meetings without Insights, missing
//! materials, tasks and mentions from the previous workday's notes, free focus time and tomorrow.
//! Everything is computed locally from cached data; nothing here calls AI or the network.

use crate::{
    calendar::{Calendar, Event},
    i18n::Language,
    recommendations::{InboxEntry, Kind, Recommendation, State},
    tasks::{Source, Status, Task},
};
use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Morning overview times offered in Settings: 07:00–10:00 in 30-minute steps.
pub const TIMES: [u16; 7] = [420, 450, 480, 510, 540, 570, 600];
/// 08:00.
pub const DEFAULT_TIME: u16 = 480;
/// A missed morning overview is still shown until this local hour, never later that day.
pub const DEADLINE_HOUR: u32 = 12;
/// Shortest free gap listed as focus time.
pub const FOCUS_MINUTES: i64 = 30;
/// Working hours in which focus time is looked for: 08:00–18:00 local time.
pub const FOCUS_START: u32 = 8 * 60;
pub const FOCUS_END: u32 = 18 * 60;
const MENTION_LIMIT: usize = 10;
const MENTION_CHARS: usize = 300;
const TITLE_CHARS: usize = 120;
/// Days of saved notes handed over for mentions; covers the previous workday seen from a Monday.
pub const NOTES_DAYS: i64 = 5;

/// Saved post-meeting notes reduced to what My day shows: mentions of me in both languages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub mentions: Vec<String>,
    pub mentions_cs: Vec<String>,
}

impl Notes {
    /// `None` unless the payload is saved post-meeting notes.
    pub fn from_payload(payload: &Value) -> Option<Self> {
        if payload.get("tomlook_recap").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let items = |value: Option<&Value>| {
            value
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|text| crate::reminders::shorten(text, MENTION_CHARS))
                .filter(|text| !text.is_empty())
                .take(MENTION_LIMIT)
                .collect::<Vec<_>>()
        };
        Some(Self {
            mentions: items(payload.get("open_questions")),
            mentions_cs: items(payload.get("cs").and_then(|czech| czech.get("open_questions"))),
        })
    }

    /// Mentions in the UI language; Czech falls back to the English notes.
    pub fn mentions(&self, language: Language) -> &[String] {
        if language == Language::Cs && !self.mentions_cs.is_empty() {
            &self.mentions_cs
        } else {
            &self.mentions
        }
    }
}

/// Snapshot of the cached data the overview is computed from; cheap to clone and share with the
/// notification thread.
#[derive(Clone, Default)]
pub struct Sources {
    pub calendar: Arc<Calendar>,
    /// Calendar owner; recommendations of other accounts are ignored.
    pub owner: Option<String>,
    /// Account of `tasks`: the calendar owner, or the fixed demo account in previews.
    pub task_account: Option<String>,
    pub tasks: Arc<Vec<Task>>,
    pub recommendations: Arc<BTreeMap<String, Recommendation>>,
    pub recommendation_view: Arc<Vec<InboxEntry>>,
    /// Meetings whose Insights are ready.
    pub ready: Arc<BTreeSet<String>>,
    /// Saved post-meeting notes by event id.
    pub notes: Arc<BTreeMap<String, Notes>>,
}

/// One meeting of today's agenda.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub event_id: String,
    pub title: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    /// All-day or multi-day items; they are listed but not counted as meetings.
    pub all_day: bool,
    /// Accepted or organized by me.
    pub attending: bool,
    pub tentative: bool,
    /// Eligible for automatic preparation (accepted/organized, not followed, not ended).
    pub eligible: bool,
    /// Insights are ready.
    pub ready: bool,
    /// Post-meeting notes are ready.
    pub notes: bool,
    /// Safe online-meeting join link.
    pub join_url: Option<String>,
}

impl Item {
    /// Eligible for preparation but without ready Insights.
    pub fn unprepared(&self) -> bool {
        self.eligible && !self.ready
    }
}

/// Two confirmed meetings that overlap today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub first: String,
    pub first_title: String,
    pub second: String,
    pub second_title: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
}

/// A pending missing-materials or preparation recommendation for one of today's meetings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Material {
    pub recommendation: String,
    pub event_id: String,
    pub title: String,
    pub kind: Kind,
    pub start: DateTime<FixedOffset>,
}

/// Why a task is part of My day, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    Overdue,
    Today,
    Notes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskItem {
    pub id: i64,
    pub title: String,
    pub due: Option<NaiveDate>,
    pub reason: Reason,
    /// Imported from the notes of a meeting that ended on the previous workday.
    pub from_notes: bool,
    pub meeting: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    pub event_id: String,
    pub title: String,
    pub start: DateTime<FixedOffset>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tomorrow {
    pub date: NaiveDate,
    pub verified: bool,
    pub meetings: usize,
    pub first: Option<DateTime<FixedOffset>>,
}

/// Everything the My day panel and the morning notification show for one local day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overview {
    pub date: NaiveDate,
    /// Today's calendar coverage was synchronized; free time is claimed only then.
    pub verified: bool,
    /// Meetings I attend or tentatively accepted: all-day items first, then by start.
    pub agenda: Vec<Item>,
    /// Timed meetings I attend.
    pub meetings: usize,
    /// Timed tentatively accepted meetings.
    pub tentative: usize,
    /// Time covered by timed meetings I attend; overlaps count once.
    pub busy_minutes: i64,
    pub first_start: Option<DateTime<FixedOffset>>,
    pub last_end: Option<DateTime<FixedOffset>>,
    pub conflicts: Vec<Conflict>,
    /// Agenda meetings without ready Insights, in time order.
    pub unprepared: Vec<String>,
    pub materials: Vec<Material>,
    pub tasks: Vec<TaskItem>,
    /// Inclusive days whose ended meetings' notes contribute tasks and mentions.
    pub notes_days: (NaiveDate, NaiveDate),
    pub mentions: Vec<Mention>,
    /// Free gaps of at least 30 minutes between 08:00 and 18:00; `None` when today is not synced.
    pub focus: Option<Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)>>,
    /// Recommendations and proposed changes waiting in Messages.
    pub approvals: usize,
    pub tomorrow: Tomorrow,
}

impl Overview {
    /// Anything to act on today; the morning notification stays quiet otherwise.
    pub fn actionable(&self) -> bool {
        self.meetings + self.tentative > 0
            || !self.conflicts.is_empty()
            || !self.unprepared.is_empty()
            || !self.materials.is_empty()
            || !self.tasks.is_empty()
            || !self.mentions.is_empty()
            || self.approvals > 0
    }

    pub fn item(&self, event_id: &str) -> Option<&Item> {
        self.agenda.iter().find(|item| item.event_id == event_id)
    }

    /// The timed meeting in progress, otherwise the next one today.
    pub fn current(&self, now: DateTime<FixedOffset>) -> Option<&Item> {
        self.agenda
            .iter()
            .filter(|item| !item.all_day && item.end > now && (item.attending || item.tentative))
            .min_by_key(|item| (item.start, !item.attending, &item.event_id))
    }

    /// Focus windows that still have at least 30 free minutes after `now`.
    pub fn focus_remaining(
        &self,
        now: DateTime<FixedOffset>,
    ) -> Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
        self.focus
            .iter()
            .flatten()
            .map(|(start, end)| ((*start).max(now), *end))
            .filter(|(start, end)| *end - *start >= Duration::minutes(FOCUS_MINUTES))
            .collect()
    }
}

/// Time `minutes` after midnight of `date` in `zone`; skips forward over a DST gap.
pub fn local_time_in<Tz: TimeZone>(zone: &Tz, date: NaiveDate, minutes: u32) -> DateTime<FixedOffset> {
    let naive = date.and_time(NaiveTime::MIN) + Duration::minutes(i64::from(minutes));
    (0..=2)
        .find_map(|hours| {
            zone.from_local_datetime(&(naive + Duration::hours(hours)))
                .earliest()
        })
        .map(|time| time.fixed_offset())
        .unwrap_or_else(|| naive.and_utc().fixed_offset())
}

/// Local time `minutes` after midnight of `date`; skips forward over a DST gap.
fn local_time(date: NaiveDate, minutes: u32) -> DateTime<FixedOffset> {
    local_time_in(&Local, date, minutes)
}

pub(crate) fn cancelled(event: &Event) -> bool {
    matches!(
        event.status.to_ascii_lowercase().as_str(),
        "cancelled" | "canceled" | "deleted"
    ) || event.extra.get("isCancelled").and_then(Value::as_bool) == Some(true)
}

pub(crate) fn tentative(event: &Event) -> bool {
    !event.accepted()
        && !event.following()
        && !cancelled(event)
        && (matches!(
            event.status.to_ascii_lowercase().as_str(),
            "tentativelyaccepted" | "tentative"
        ) || event.extra.get("showAs").and_then(Value::as_str) == Some("tentative"))
}

/// The previous workday: Friday on Mondays, otherwise yesterday.
pub fn previous_workday(today: NaiveDate) -> NaiveDate {
    today
        - Duration::days(if today.weekday() == chrono::Weekday::Mon {
            3
        } else {
            1
        })
}

/// Days whose ended meetings' notes feed My day: yesterday, or Friday through Sunday on Mondays.
pub fn notes_days(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    (previous_workday(today), today - Duration::days(1))
}

fn day_events(calendar: &Calendar, date: NaiveDate) -> impl Iterator<Item = &Event> {
    calendar
        .days
        .get(&date)
        .into_iter()
        .flat_map(|day| day.events.iter())
        .map(|index| &calendar.events[*index])
}

fn clip(title: &str) -> String {
    crate::reminders::shorten(title, TITLE_CHARS)
}

/// Computes the overview of `now`'s local day.
pub fn overview(sources: &Sources, now: DateTime<Local>, language: Language) -> Overview {
    let today = now.date_naive();
    let instant = now.fixed_offset();
    let calendar = &*sources.calendar;
    let day_start = local_time(today, 0);
    let day_end = local_time(today + Duration::days(1), 0);
    let mut agenda = day_events(calendar, today)
        // BLOCKER holds without attendees still block focus time but are not meetings.
        .filter(|event| !cancelled(event) && (event.accepted() || tentative(event)) && !crate::scheduling::is_blocker(event))
        .map(|event| Item {
            event_id: event.id.clone(),
            title: clip(&event.title),
            start: event.start,
            end: event.end,
            all_day: event.all_day_like(),
            attending: event.accepted(),
            tentative: tentative(event),
            eligible: crate::scheduler::eligible(event, instant),
            ready: sources.ready.contains(&event.id),
            notes: sources.notes.contains_key(&event.id),
            join_url: event.join_url().map(str::to_owned),
        })
        .collect::<Vec<_>>();
    agenda.sort_by(|a, b| {
        (!a.all_day, a.start, &a.event_id).cmp(&(!b.all_day, b.start, &b.event_id))
    });
    let attended = agenda
        .iter()
        .filter(|item| !item.all_day && item.attending)
        .collect::<Vec<_>>();
    let mut busy_minutes = 0;
    let mut covered: Option<DateTime<FixedOffset>> = None;
    for item in &attended {
        let start = item.start.max(day_start).max(covered.unwrap_or(day_start));
        let end = item.end.min(day_end);
        if end > start {
            busy_minutes += (end - start).num_minutes();
        }
        covered = Some(covered.map_or(end, |covered| covered.max(end)));
    }
    let conflicts = conflicts_between(
        calendar,
        attended.iter().map(|item| item.event_id.as_str()),
        day_start,
        day_end,
    );
    let unprepared = agenda
        .iter()
        .filter(|item| item.unprepared())
        .map(|item| item.event_id.clone())
        .collect();
    let notes_days = notes_days(today);
    Overview {
        date: today,
        verified: calendar.coverage.contains(&today),
        meetings: attended.len(),
        tentative: agenda
            .iter()
            .filter(|item| !item.all_day && item.tentative)
            .count(),
        busy_minutes,
        first_start: attended.iter().map(|item| item.start).min(),
        last_end: attended.iter().map(|item| item.end).max(),
        conflicts,
        unprepared,
        materials: materials(sources, today, instant),
        tasks: tasks(sources, today, notes_days),
        mentions: mentions(sources, notes_days, language),
        notes_days,
        focus: calendar
            .coverage
            .contains(&today)
            .then(|| focus(calendar, today)),
        approvals: sources
            .recommendation_view
            .iter()
            .filter_map(|entry| sources.recommendations.get(&entry.id))
            .filter(|record| record.visible(calendar, sources.owner.as_deref(), instant))
            .count(),
        tomorrow: tomorrow(calendar, today + Duration::days(1)),
        agenda,
    }
}

/// Confirmed overlaps of the given meetings with other timed busy items that intersect
/// `[day_start, day_end)`, each pair once, ordered by the overlap start. Overlap times are not
/// clipped to the window.
pub fn conflicts_between<'a>(
    calendar: &Calendar,
    ids: impl IntoIterator<Item = &'a str>,
    day_start: DateTime<FixedOffset>,
    day_end: DateTime<FixedOffset>,
) -> Vec<Conflict> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for id in ids {
        let Some(event) = calendar
            .event_index(id)
            .map(|index| &calendar.events[index])
        else {
            continue;
        };
        if !crate::recommendations::blocks_time(event) {
            continue;
        }
        for other in crate::recommendations::conflicts(calendar, event) {
            // All-day items such as office days or launch weeks are not meeting conflicts.
            if other.all_day_like() {
                continue;
            }
            let pair = if event.id < other.id {
                (event.id.clone(), other.id.clone())
            } else {
                (other.id.clone(), event.id.clone())
            };
            let start = event.start.max(other.start);
            let end = event.end.min(other.end);
            if end <= day_start || start >= day_end || !seen.insert(pair) {
                continue;
            }
            let (first, second) = if (other.start, &other.id) < (event.start, &event.id) {
                (other, event)
            } else {
                (event, other)
            };
            found.push(Conflict {
                first: first.id.clone(),
                first_title: clip(&first.title),
                second: second.id.clone(),
                second_title: clip(&second.title),
                start,
                end,
            });
        }
    }
    found.sort_by(|a, b| (a.start, &a.first, &a.second).cmp(&(b.start, &b.first, &b.second)));
    found
}

fn materials(sources: &Sources, today: NaiveDate, now: DateTime<FixedOffset>) -> Vec<Material> {
    let calendar = &*sources.calendar;
    let owner = sources.owner.as_deref();
    sources
        .recommendation_view
        .iter()
        .filter_map(|entry| sources.recommendations.get(&entry.id))
        .filter(|record| {
            matches!(record.state, State::Pending | State::NotSent)
                && matches!(
                    record.suggestion.kind,
                    Kind::MissingMaterials | Kind::Preparation | Kind::PreparationAdvance
                )
                && record.visible(calendar, owner, now)
        })
        .filter_map(|record| {
            let event = record.current(calendar, owner, now)?;
            let (first, last) = event.calendar_dates();
            (first <= today && today <= last).then(|| Material {
                recommendation: record.id.clone(),
                event_id: event.id.clone(),
                title: clip(&event.title),
                kind: record.suggestion.kind,
                start: event.start,
            })
        })
        .collect()
}

fn ended_on(calendar: &Calendar, meeting: &crate::tasks::Meeting) -> NaiveDate {
    calendar
        .event_index(&meeting.event_id)
        .map(|index| calendar.events[index].calendar_dates().1)
        .unwrap_or_else(|| meeting.start.with_timezone(&Local).date_naive())
}

fn tasks(sources: &Sources, today: NaiveDate, days: (NaiveDate, NaiveDate)) -> Vec<TaskItem> {
    let Some(account) = sources.task_account.as_deref() else {
        return Vec::new();
    };
    let calendar = &*sources.calendar;
    let mut found = sources
        .tasks
        .iter()
        .filter(|task| task.account == account && task.status == Status::Open)
        .filter_map(|task| {
            let from_notes = task.source == Source::MeetingNotes
                && task.meeting.as_ref().is_some_and(|meeting| {
                    let ended = ended_on(calendar, meeting);
                    days.0 <= ended && ended <= days.1
                });
            let reason = match task.due {
                Some(due) if due < today => Reason::Overdue,
                Some(due) if due == today => Reason::Today,
                _ if from_notes => Reason::Notes,
                _ => return None,
            };
            Some((
                (
                    reason,
                    task.due,
                    task.meeting.as_ref().map(|meeting| meeting.start),
                    task.item,
                    task.id,
                ),
                TaskItem {
                    id: task.id,
                    title: task.title.clone(),
                    due: task.due,
                    reason,
                    from_notes,
                    meeting: task.meeting.as_ref().map(|meeting| meeting.title.clone()),
                },
            ))
        })
        .collect::<Vec<_>>();
    found.sort_by_key(|(key, _)| *key);
    found.into_iter().map(|(_, task)| task).collect()
}

fn mentions(sources: &Sources, days: (NaiveDate, NaiveDate), language: Language) -> Vec<Mention> {
    let calendar = &*sources.calendar;
    let mut found = sources
        .notes
        .iter()
        .filter_map(|(id, notes)| {
            let event = &calendar.events[calendar.event_index(id)?];
            let ended = event.calendar_dates().1;
            (days.0 <= ended && ended <= days.1).then_some((event, notes))
        })
        .collect::<Vec<_>>();
    found.sort_by(|(a, _), (b, _)| (a.start, &a.id).cmp(&(b.start, &b.id)));
    found
        .into_iter()
        .flat_map(|(event, notes)| {
            notes.mentions(language).iter().map(|text| Mention {
                event_id: event.id.clone(),
                title: clip(&event.title),
                start: event.start,
                text: crate::tasks::without_label(text),
            })
        })
        .collect()
}

/// Free gaps of at least 30 minutes between 08:00 and 18:00 that no confirmed busy item covers.
/// Free, tentative, followed, declined and cancelled items do not block time.
pub fn focus(calendar: &Calendar, today: NaiveDate) -> Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    free_windows(
        day_events(calendar, today),
        today,
        local_time(today, FOCUS_START),
        local_time(today, FOCUS_END),
    )
}

/// Free gaps of at least 30 minutes in `[first, last)` of `date` that none of `events` blocks:
/// confirmed busy timed items cover their overlap, all-day busy items covering `date` the whole
/// window. Free, tentative, followed, declined and cancelled items do not block time.
pub fn free_windows<'a>(
    events: impl Iterator<Item = &'a Event>,
    date: NaiveDate,
    first: DateTime<FixedOffset>,
    last: DateTime<FixedOffset>,
) -> Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    let mut busy = events
        .filter(|event| crate::recommendations::blocks_time(event))
        .filter_map(|event| {
            if event.is_all_day {
                let (from, through) = event.calendar_dates();
                (from <= date && date <= through).then_some((first, last))
            } else {
                Some((event.start.max(first), event.end.min(last)))
            }
        })
        .filter(|(start, end)| end > start)
        .collect::<Vec<_>>();
    busy.sort();
    let mut free = Vec::new();
    let mut cursor = first;
    for (start, end) in busy.into_iter().chain([(last, last)]) {
        if start - cursor >= Duration::minutes(FOCUS_MINUTES) {
            free.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    free
}

fn tomorrow(calendar: &Calendar, date: NaiveDate) -> Tomorrow {
    let meetings = day_events(calendar, date)
        .filter(|event| !event.all_day_like() && event.accepted() && !cancelled(event) && !crate::scheduling::is_blocker(event))
        .collect::<Vec<_>>();
    Tomorrow {
        date,
        verified: calendar.coverage.contains(&date),
        meetings: meetings.len(),
        first: meetings.iter().map(|event| event.start).min(),
    }
}

/// Counted things in My day texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Noun {
    Meeting,
    Tentative,
    Conflict,
    Unprepared,
    Task,
    Mention,
    WithoutAgenda,
}

/// Czech plural form index: 0 for one, 1 for two to four, 2 for zero and five or more.
pub fn czech_form(count: usize) -> usize {
    match count {
        1 => 0,
        2..=4 => 1,
        _ => 2,
    }
}

/// "3 meetings" / "3 schůzky" with the correct singular or plural form.
pub fn count(count: usize, noun: Noun, language: Language) -> String {
    let (english, czech): ([&str; 2], [&str; 3]) = match noun {
        Noun::Meeting => (["meeting", "meetings"], ["schůzka", "schůzky", "schůzek"]),
        Noun::Tentative => (["tentative", "tentative"], ["nezávazná", "nezávazné", "nezávazných"]),
        Noun::Conflict => (["conflict", "conflicts"], ["konflikt", "konflikty", "konfliktů"]),
        Noun::Unprepared => (["not prepared", "not prepared"], ["bez přípravy"; 3]),
        Noun::Task => (["task", "tasks"], ["úkol", "úkoly", "úkolů"]),
        Noun::Mention => (["mention", "mentions"], ["zmínka", "zmínky", "zmínek"]),
        Noun::WithoutAgenda => (["without agenda"; 2], ["bez agendy"; 3]),
    };
    let word = match language {
        Language::En => english[usize::from(count != 1)],
        Language::Cs => czech[czech_form(count)],
    };
    format!("{count} {word}")
}

/// "45 min", "2 h" or "4 h 15 min" (the same in English and Czech).
pub fn duration(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// Local wall-clock time, "09:15".
pub fn clock(time: DateTime<FixedOffset>) -> String {
    time.with_timezone(&Local).format("%H:%M").to_string()
}

/// "in 25 min" / "za 25 min", "now" / "právě probíhá" while the meeting runs.
pub fn countdown(
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    now: DateTime<FixedOffset>,
    language: Language,
) -> String {
    if now >= end {
        return match language {
            Language::En => "ended",
            Language::Cs => "skončila",
        }
        .into();
    }
    if now >= start {
        return match language {
            Language::En => "now",
            Language::Cs => "právě probíhá",
        }
        .into();
    }
    let minutes = ((start - now).num_seconds() + 59) / 60;
    let left = duration(minutes.max(1));
    match language {
        Language::En => format!("in {left}"),
        Language::Cs => format!("za {left}"),
    }
}

fn no_meetings(language: Language) -> String {
    match language {
        Language::En => "No meetings today",
        Language::Cs => "Dnes žádné schůzky",
    }
    .into()
}

/// "5 meetings · 4 h 15 min · 08:30–17:00 · 1 tentative".
pub fn summary(overview: &Overview, language: Language) -> String {
    let mut parts = Vec::new();
    if overview.meetings == 0 {
        parts.push(no_meetings(language));
    } else {
        parts.push(count(overview.meetings, Noun::Meeting, language));
        parts.push(duration(overview.busy_minutes));
        if let (Some(first), Some(last)) = (overview.first_start, overview.last_end) {
            parts.push(format!("{}–{}", clock(first), clock(last)));
        }
    }
    if overview.tentative > 0 {
        parts.push(count(overview.tentative, Noun::Tentative, language));
    }
    parts.join(" · ")
}

/// "3 meetings · first at 08:30" / "3 schůzky · první v 08:30".
pub fn tomorrow_line(overview: &Overview, language: Language) -> String {
    let tomorrow = &overview.tomorrow;
    match tomorrow.first {
        Some(first) => format!(
            "{} · {} {}",
            count(tomorrow.meetings, Noun::Meeting, language),
            match language {
                Language::En => "first at",
                Language::Cs => "první v",
            },
            clock(first)
        ),
        None => match language {
            Language::En => "No meetings tomorrow",
            Language::Cs => "Zítra žádné schůzky",
        }
        .into(),
    }
}

/// Text of the morning notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digest {
    pub title: String,
    /// "6 meetings · 1 conflict · 2 not prepared · 3 tasks".
    pub summary: String,
    /// "First meeting 08:30 · Weekly planning", or the next one when shown later.
    pub next: Option<String>,
}

pub fn digest(overview: &Overview, now: DateTime<Local>, language: Language) -> Digest {
    let mut parts = vec![if overview.meetings == 0 {
        no_meetings(language)
    } else {
        count(overview.meetings, Noun::Meeting, language)
    }];
    for (amount, noun) in [
        (overview.conflicts.len(), Noun::Conflict),
        (overview.unprepared.len(), Noun::Unprepared),
        (overview.tasks.len(), Noun::Task),
    ] {
        if amount > 0 {
            parts.push(count(amount, noun, language));
        }
    }
    let instant = now.fixed_offset();
    let timed = overview
        .agenda
        .iter()
        .filter(|item| !item.all_day && item.attending)
        .collect::<Vec<_>>();
    let next = timed
        .iter()
        .find(|item| item.start >= instant)
        .map(|item| {
            let first = timed.first().is_some_and(|first| first.event_id == item.event_id);
            let label = match (language, first) {
                (Language::En, true) => "First meeting",
                (Language::En, false) => "Next meeting",
                (Language::Cs, true) => "První schůzka",
                (Language::Cs, false) => "Další schůzka",
            };
            format!("{label} {} · {}", clock(item.start), item.title)
        });
    Digest {
        title: language.text("My day").into_owned(),
        summary: parts.join(" · "),
        next,
    }
}

/// Morning notification preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub enabled: bool,
    /// Minutes after local midnight; one of [`TIMES`].
    pub minutes: u16,
    pub weekends: bool,
}

impl Schedule {
    pub fn from_preferences(preferences: &crate::storage::Preferences) -> Self {
        Self {
            enabled: preferences.morning_overview,
            minutes: normalize_time(preferences.morning_overview_time),
            weekends: preferences.morning_overview_weekends,
        }
    }

    fn includes(&self, day: NaiveDate) -> bool {
        self.weekends || day.weekday().num_days_from_monday() < 5
    }

    fn time(&self) -> NaiveTime {
        let minutes = u32::from(normalize_time(self.minutes));
        NaiveTime::from_hms_opt(minutes / 60, minutes % 60, 0).expect("valid morning time")
    }
}

pub fn normalize_time(minutes: u16) -> u16 {
    if TIMES.contains(&minutes) {
        minutes
    } else {
        DEFAULT_TIME
    }
}

/// Whether the morning overview is due at local time `now`: enabled, on a scheduled day, at or
/// after the chosen time but before noon, and not yet shown today.
pub fn due(schedule: &Schedule, shown: Option<NaiveDate>, now: NaiveDateTime) -> bool {
    next_due(schedule, shown, now) == Some(now)
}

/// The next local time the overview is due, skipping days up to and including `handled`
/// (shown, or already evaluated with nothing to show). A day missed until noon is skipped.
pub fn next_due(
    schedule: &Schedule,
    handled: Option<NaiveDate>,
    now: NaiveDateTime,
) -> Option<NaiveDateTime> {
    if !schedule.enabled {
        return None;
    }
    (0..8)
        .map(|offset| now.date() + Duration::days(offset))
        .filter(|day| handled.is_none_or(|handled| *day > handled) && schedule.includes(*day))
        .find_map(|day| {
            let deadline = day.and_hms_opt(DEADLINE_HOUR, 0, 0)?;
            (now < deadline).then(|| day.and_time(schedule.time()).max(now))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{recommendations::Suggestion, tasks::New};
    use serde_json::json;

    const ACCOUNT: &str = "owner@example.com";

    fn day(number: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, number).unwrap()
    }

    fn local(date: NaiveDate, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(&date.and_hms_opt(hour, minute, 0).unwrap())
            .earliest()
            .unwrap()
    }

    fn meeting(id: &str, title: &str, date: NaiveDate, start: (u32, u32), end: (u32, u32), status: &str) -> Event {
        Event {
            id: id.into(),
            title: title.into(),
            start: local(date, start.0, start.1).fixed_offset(),
            end: local(date, end.0, end.1).fixed_offset(),
            category: String::new(),
            attendees: Vec::new(),
            organizer: String::new(),
            location: String::new(),
            status: status.into(),
            briefing_status: String::new(),
            is_all_day: false,
            source_url: None,
            extra: BTreeMap::new(),
        }
    }

    fn notes_payload(mention: &str) -> Value {
        json!({
            "tomlook_recap": true, "summary": "Pilot agreed.",
            "preparation": ["My task: send the comparison to Alex", "My task: confirm the pilot owner"],
            "talking_points": [], "open_questions": [format!("Mention of me: {mention}")],
            "risks": [], "gaps": [], "sources": [],
            "cs": {"summary": "Pilot dohodnut.",
                "preparation": ["Můj úkol: poslat Alexovi porovnání", "Můj úkol: potvrdit vlastníka pilotu"],
                "open_questions": [format!("Zmínka: {mention}")]}
        })
    }

    /// Wednesday 11 November 2026 with conflicts, unprepared, tentative, followed, declined,
    /// cancelled, free and all-day items, yesterday's and Monday's notes, and tomorrow.
    fn wednesday(coverage: &[NaiveDate]) -> Sources {
        let wednesday = day(11);
        let mut review = meeting("b", "Customer review", wednesday, (9, 0), (10, 0), "accepted");
        review.extra.insert("onlineMeeting".into(), json!({"joinUrl": "https://teams.microsoft.com/l/meetup-join/test"}));
        let mut architecture = meeting("c", "Architecture sync", wednesday, (9, 30), (10, 30), "none");
        architecture.extra.insert("isOrganizer".into(), json!(true));
        let mut followed = meeting("e", "Town hall", wednesday, (12, 0), (13, 0), "accepted");
        followed.extra.insert("isFollowing".into(), json!(true));
        let mut cancelled = meeting("g", "Cancelled sync", wednesday, (14, 0), (15, 0), "accepted");
        cancelled.extra.insert("isCancelled".into(), json!(true));
        let mut free = meeting("h", "Office hours", wednesday, (15, 0), (16, 0), "accepted");
        free.extra.insert("showAs".into(), json!("free"));
        let mut offsite = meeting("i", "Company offsite", wednesday, (0, 0), (0, 0), "accepted");
        offsite.end = local(day(12), 0, 0).fixed_offset();
        offsite.is_all_day = true;
        offsite.extra.insert("showAs".into(), json!("free"));
        let pilot = meeting("j", "Pilot planning", day(10), (14, 0), (15, 0), "accepted");
        let old = meeting("k", "Old review", day(9), (10, 0), (11, 0), "accepted");
        let events = vec![
            meeting("a", "Standup", wednesday, (8, 30), (9, 0), "accepted"),
            review.clone(),
            architecture.clone(),
            meeting("d", "Optional chat", wednesday, (11, 0), (11, 30), "tentativelyAccepted"),
            followed,
            meeting("f", "Declined review", wednesday, (13, 0), (14, 0), "declined"),
            cancelled,
            free,
            offsite,
            pilot.clone(),
            old.clone(),
            meeting("l", "Thursday kickoff", day(12), (8, 15), (9, 0), "accepted"),
            meeting("m", "Thursday planning", day(12), (10, 0), (11, 0), "organizer"),
            meeting("n", "Thursday maybe", day(12), (7, 0), (7, 30), "tentativelyAccepted"),
        ];
        let calendar = Calendar::build(events, coverage.iter().copied().collect(), Vec::new()).unwrap();
        let now = local(wednesday, 7, 30).fixed_offset();
        let created = now.to_utc();
        let manual = |id: i64, title: &str, due: Option<NaiveDate>| {
            let mut task = New::manual(title, due, Language::En).unwrap().task(ACCOUNT, created).unwrap();
            task.id = id;
            task
        };
        let mut tasks = vec![
            manual(1, "Overdue report", Some(day(10))),
            manual(2, "Due today", Some(wednesday)),
            manual(3, "Upcoming", Some(day(13))),
            manual(4, "Undated", None),
            manual(5, "Done overdue", Some(day(9))).completed(true, created),
        ];
        let mut foreign = New::manual("Foreign", Some(wednesday), Language::En).unwrap().task("other@example.com", created).unwrap();
        foreign.id = 6;
        tasks.push(foreign);
        for (offset, event) in [(10, &pilot), (20, &old)] {
            for (index, mut task) in crate::tasks::from_notes(ACCOUNT, event, &notes_payload("x"), Language::En, created)
                .unwrap().into_iter().enumerate() {
                task.id = offset + index as i64;
                tasks.push(task);
            }
        }
        let suggestion = |kind, category: Option<&str>| Suggestion {
            kind,
            reason: "Ask for the deck".into(),
            reason_cs: "Požádat o prezentaci".into(),
            category: category.map(str::to_owned),
            minutes: None,
        };
        let materials = Recommendation::from_insights(&review, ACCOUNT, suggestion(Kind::MissingMaterials, None), now).unwrap();
        let category = Recommendation::from_insights(&architecture, ACCOUNT, suggestion(Kind::Category, Some("Customer")), now).unwrap();
        let view = [&materials, &category].map(|record| InboxEntry { id: record.id.clone(), conflicts: Vec::new() }).to_vec();
        Sources {
            calendar: Arc::new(calendar),
            owner: Some(ACCOUNT.into()),
            task_account: Some(ACCOUNT.into()),
            tasks: Arc::new(tasks),
            recommendations: Arc::new([materials, category].into_iter().map(|record| (record.id.clone(), record)).collect()),
            recommendation_view: Arc::new(view),
            ready: Arc::new(["a", "h", "i"].into_iter().map(str::to_owned).collect()),
            notes: Arc::new([
                ("j".to_string(), Notes::from_payload(&notes_payload("Jamie asked about the budget")).unwrap()),
                ("k".to_string(), Notes::from_payload(&notes_payload("Old mention")).unwrap()),
            ].into()),
        }
    }

    #[test]
    fn overview_counts_attended_meetings_conflicts_unprepared_tasks_mentions_focus_and_tomorrow() {
        let sources = wednesday(&[day(11), day(12)]);
        let now = local(day(11), 7, 30);
        let overview = overview(&sources, now, Language::En);
        assert!(overview.verified);
        assert_eq!(
            overview.agenda.iter().map(|item| item.event_id.as_str()).collect::<Vec<_>>(),
            ["i", "a", "b", "c", "d", "h"],
            "Followed, declined and cancelled items are not on the agenda; all-day items come first"
        );
        assert_eq!((overview.meetings, overview.tentative, overview.busy_minutes), (4, 1, 180));
        assert_eq!(overview.first_start, Some(local(day(11), 8, 30).fixed_offset()));
        assert_eq!(overview.last_end, Some(local(day(11), 16, 0).fixed_offset()));
        assert_eq!(overview.conflicts.len(), 1, "{:?}", overview.conflicts);
        let conflict = &overview.conflicts[0];
        assert_eq!((conflict.first.as_str(), conflict.second.as_str()), ("b", "c"));
        assert_eq!((conflict.start, conflict.end), (local(day(11), 9, 30).fixed_offset(), local(day(11), 10, 0).fixed_offset()));
        assert_eq!(overview.unprepared, ["b", "c"], "Tentative and ready meetings are not listed");
        assert!(overview.item("b").unwrap().join_url.is_some());
        assert_eq!(overview.materials.len(), 1);
        assert_eq!((overview.materials[0].event_id.as_str(), overview.materials[0].kind), ("b", Kind::MissingMaterials));
        assert_eq!(overview.approvals, 2);
        assert_eq!(
            overview.tasks.iter().map(|task| (task.id, task.reason)).collect::<Vec<_>>(),
            [(1, Reason::Overdue), (2, Reason::Today), (10, Reason::Notes), (11, Reason::Notes)],
            "Upcoming, undated, done, foreign and older notes' tasks are excluded"
        );
        assert_eq!(overview.tasks[2].meeting.as_deref(), Some("Pilot planning"));
        assert_eq!(overview.notes_days, (day(10), day(10)));
        assert_eq!(
            overview.mentions.iter().map(|mention| (mention.event_id.as_str(), mention.text.as_str())).collect::<Vec<_>>(),
            [("j", "Jamie asked about the budget")]
        );
        let czech = super::overview(&sources, now, Language::Cs);
        assert_eq!(czech.mentions[0].text, "Jamie asked about the budget");
        assert_eq!(
            overview.focus,
            Some(vec![
                (local(day(11), 8, 0).fixed_offset(), local(day(11), 8, 30).fixed_offset()),
                (local(day(11), 10, 30).fixed_offset(), local(day(11), 18, 0).fixed_offset()),
            ]),
            "Free, tentative, followed, declined, cancelled and all-day free items do not block focus time"
        );
        assert_eq!(
            overview.focus_remaining(local(day(11), 9, 15).fixed_offset()),
            [(local(day(11), 10, 30).fixed_offset(), local(day(11), 18, 0).fixed_offset())]
        );
        assert_eq!(
            overview.tomorrow,
            Tomorrow { date: day(12), verified: true, meetings: 2, first: Some(local(day(12), 8, 15).fixed_offset()) }
        );
        assert!(overview.actionable());
        assert_eq!(overview.current(now.fixed_offset()).unwrap().event_id, "a");
        assert_eq!(overview.current(local(day(11), 9, 15).fixed_offset()).unwrap().event_id, "b");
        assert_eq!(summary(&overview, Language::En), "4 meetings · 3 h · 08:30–16:00 · 1 tentative");
        assert_eq!(summary(&czech, Language::Cs), "4 schůzky · 3 h · 08:30–16:00 · 1 nezávazná");
        assert_eq!(tomorrow_line(&overview, Language::En), "2 meetings · first at 08:15");
        assert_eq!(tomorrow_line(&czech, Language::Cs), "2 schůzky · první v 08:15");
        let later = super::overview(&sources, local(day(11), 9, 45), Language::En);
        assert_eq!(later.unprepared, ["b", "c"], "Running meetings still lack Insights");
        let evening = super::overview(&sources, local(day(11), 10, 45), Language::En);
        assert!(evening.unprepared.is_empty(), "Ended meetings are no longer eligible");
    }

    #[test]
    fn morning_digest_uses_counts_plurals_and_the_next_meeting() {
        let sources = wednesday(&[day(11), day(12)]);
        let early = local(day(11), 7, 30);
        let english = digest(&overview(&sources, early, Language::En), early, Language::En);
        assert_eq!(english.title, "My day");
        assert_eq!(english.summary, "4 meetings · 1 conflict · 2 not prepared · 4 tasks");
        assert_eq!(english.next.as_deref(), Some("First meeting 08:30 · Standup"));
        let czech = digest(&overview(&sources, early, Language::Cs), early, Language::Cs);
        assert_eq!(czech.title, "Můj den");
        assert_eq!(czech.summary, "4 schůzky · 1 konflikt · 2 bez přípravy · 4 úkoly");
        assert_eq!(czech.next.as_deref(), Some("První schůzka 08:30 · Standup"));
        let late = local(day(11), 9, 15);
        let next = digest(&overview(&sources, late, Language::En), late, Language::En);
        assert_eq!(next.next.as_deref(), Some("Next meeting 09:30 · Architecture sync"));
        let empty = Sources { calendar: Arc::new(Calendar::build(Vec::new(), [day(11)].into(), Vec::new()).unwrap()), ..Default::default() };
        let quiet = overview(&empty, early, Language::Cs);
        assert!(!quiet.actionable() && quiet.verified);
        assert_eq!(digest(&quiet, early, Language::Cs).summary, "Dnes žádné schůzky");
        assert_eq!(summary(&quiet, Language::En), "No meetings today");
        assert_eq!(tomorrow_line(&quiet, Language::Cs), "Zítra žádné schůzky");
    }

    #[test]
    fn czech_plurals_distinguish_one_two_to_four_and_five_or_more() {
        for (amount, text) in [(0, "0 schůzek"), (1, "1 schůzka"), (2, "2 schůzky"), (4, "4 schůzky"), (5, "5 schůzek"), (6, "6 schůzek")] {
            assert_eq!(count(amount, Noun::Meeting, Language::Cs), text);
        }
        for (amount, conflict, task, mention) in [
            (1, "1 konflikt", "1 úkol", "1 zmínka"),
            (3, "3 konflikty", "3 úkoly", "3 zmínky"),
            (7, "7 konfliktů", "7 úkolů", "7 zmínek"),
        ] {
            assert_eq!(count(amount, Noun::Conflict, Language::Cs), conflict);
            assert_eq!(count(amount, Noun::Task, Language::Cs), task);
            assert_eq!(count(amount, Noun::Mention, Language::Cs), mention);
        }
        assert_eq!(count(2, Noun::Unprepared, Language::Cs), "2 bez přípravy");
        assert_eq!(count(5, Noun::Tentative, Language::Cs), "5 nezávazných");
        assert_eq!(
            [count(6, Noun::Meeting, Language::Cs), count(1, Noun::Conflict, Language::Cs),
                count(2, Noun::Unprepared, Language::Cs), count(3, Noun::Task, Language::Cs)].join(" · "),
            "6 schůzek · 1 konflikt · 2 bez přípravy · 3 úkoly"
        );
        assert_eq!(
            [count(6, Noun::Meeting, Language::En), count(1, Noun::Conflict, Language::En),
                count(2, Noun::Unprepared, Language::En), count(3, Noun::Task, Language::En)].join(" · "),
            "6 meetings · 1 conflict · 2 not prepared · 3 tasks"
        );
        assert_eq!(count(1, Noun::Meeting, Language::En), "1 meeting");
        assert_eq!((czech_form(1), czech_form(4), czech_form(0), czech_form(22)), (0, 1, 2, 2));
    }

    #[test]
    fn countdown_and_durations_are_bilingual() {
        let start = local(day(11), 10, 0).fixed_offset();
        let end = start + Duration::hours(1);
        assert_eq!(countdown(start, end, start - Duration::minutes(25), Language::En), "in 25 min");
        assert_eq!(countdown(start, end, start - Duration::minutes(25), Language::Cs), "za 25 min");
        assert_eq!(countdown(start, end, start - Duration::seconds(24 * 60 + 30), Language::En), "in 25 min");
        assert_eq!(countdown(start, end, start - Duration::minutes(65), Language::En), "in 1 h 5 min");
        assert_eq!(countdown(start, end, start + Duration::minutes(10), Language::En), "now");
        assert_eq!(countdown(start, end, start, Language::Cs), "právě probíhá");
        assert_eq!(countdown(start, end, end, Language::En), "ended");
        assert_eq!((duration(45), duration(120), duration(255)), ("45 min".into(), "2 h".into(), "4 h 15 min".into()));
    }

    #[test]
    fn unsynced_days_claim_no_free_time_and_all_day_busy_items_block_it() {
        let unsynced = overview(&wednesday(&[]), local(day(11), 7, 30), Language::En);
        assert!(!unsynced.verified && !unsynced.tomorrow.verified);
        assert_eq!(unsynced.focus, None);
        assert!(unsynced.focus_remaining(local(day(11), 7, 30).fixed_offset()).is_empty());
        assert_eq!(unsynced.meetings, 4, "Cached meetings are still listed");
        let mut busy = meeting("busy", "Out of office", day(11), (0, 0), (0, 0), "accepted");
        busy.end = local(day(12), 0, 0).fixed_offset();
        busy.is_all_day = true;
        let calendar = Calendar::build(vec![busy], [day(11)].into(), Vec::new()).unwrap();
        let sources = Sources { calendar: Arc::new(calendar), ..Default::default() };
        let blocked = overview(&sources, local(day(11), 7, 30), Language::En);
        assert_eq!(blocked.focus, Some(Vec::new()));
        assert_eq!((blocked.meetings, blocked.agenda.len()), (0, 1), "All-day items are listed but not counted");
        assert!(blocked.unprepared.contains(&"busy".to_string()), "Same eligibility as automatic preparation");
    }

    #[test]
    fn previous_workday_notes_cover_the_weekend_on_mondays() {
        assert_eq!(previous_workday(day(16)), day(13));
        assert_eq!(previous_workday(day(11)), day(10));
        assert_eq!(previous_workday(day(15)), day(14));
        assert_eq!(notes_days(day(16)), (day(13), day(15)));
        assert_eq!(notes_days(day(10)), (day(9), day(9)));
        let thursday = meeting("thu", "Thursday review", day(12), (15, 0), (16, 0), "accepted");
        let friday = meeting("fri", "Friday review", day(13), (15, 0), (16, 0), "accepted");
        let saturday = meeting("sat", "Weekend fix", day(14), (10, 0), (11, 0), "accepted");
        let monday = meeting("mon", "Monday sync", day(16), (9, 0), (9, 30), "accepted");
        let calendar = Calendar::build(vec![thursday.clone(), friday.clone(), saturday, monday], [day(16)].into(), Vec::new()).unwrap();
        let created = local(day(16), 7, 0).to_utc();
        let mut tasks = Vec::new();
        for (offset, event) in [(10, &thursday), (20, &friday)] {
            for (index, mut task) in crate::tasks::from_notes(ACCOUNT, event, &notes_payload("x"), Language::En, created)
                .unwrap().into_iter().enumerate() {
                task.id = offset + index as i64;
                tasks.push(task);
            }
        }
        let sources = Sources {
            calendar: Arc::new(calendar),
            task_account: Some(ACCOUNT.into()),
            tasks: Arc::new(tasks),
            notes: Arc::new([
                ("thu".to_string(), Notes::from_payload(&notes_payload("Thursday")).unwrap()),
                ("fri".to_string(), Notes::from_payload(&notes_payload("Friday")).unwrap()),
                ("sat".to_string(), Notes::from_payload(&notes_payload("Saturday")).unwrap()),
            ].into()),
            ..Default::default()
        };
        let monday = overview(&sources, local(day(16), 7, 30), Language::En);
        assert_eq!(monday.notes_days, (day(13), day(15)));
        assert_eq!(
            monday.mentions.iter().map(|mention| mention.text.as_str()).collect::<Vec<_>>(),
            ["Friday", "Saturday"]
        );
        assert_eq!(monday.tasks.iter().map(|task| task.id).collect::<Vec<_>>(), [20, 21]);
        let tuesday = overview(&sources, local(day(17), 7, 30), Language::En);
        assert!(tuesday.mentions.is_empty() && tuesday.tasks.is_empty(), "Only the previous workday counts");
    }

    #[test]
    fn notes_keep_mentions_in_both_languages_and_ignore_insights() {
        let notes = Notes::from_payload(&notes_payload("Budget")).unwrap();
        assert_eq!(notes.mentions(Language::En), ["Mention of me: Budget"]);
        assert_eq!(notes.mentions(Language::Cs), ["Zmínka: Budget"]);
        let mut english = notes_payload("Budget");
        english.as_object_mut().unwrap().remove("cs");
        assert_eq!(Notes::from_payload(&english).unwrap().mentions(Language::Cs), ["Mention of me: Budget"]);
        let mut insights = notes_payload("Budget");
        insights["tomlook_recap"] = false.into();
        assert!(Notes::from_payload(&insights).is_none());
    }

    #[test]
    fn morning_schedule_is_once_per_day_catches_up_before_noon_and_skips_weekends() {
        let schedule = Schedule { enabled: true, minutes: DEFAULT_TIME, weekends: false };
        let at = |date: u32, hour: u32, minute: u32| day(date).and_hms_opt(hour, minute, 0).unwrap();
        assert!(!due(&schedule, None, at(9, 7, 59)));
        assert_eq!(next_due(&schedule, None, at(9, 7, 59)), Some(at(9, 8, 0)));
        assert!(due(&schedule, None, at(9, 8, 0)));
        assert!(due(&schedule, Some(day(6)), at(9, 11, 59)), "A missed overview is caught up before noon");
        assert!(!due(&schedule, None, at(9, 12, 0)), "After noon the day is skipped");
        assert_eq!(next_due(&schedule, None, at(9, 12, 0)), Some(at(10, 8, 0)));
        assert!(!due(&schedule, Some(day(9)), at(9, 9, 0)), "At most once per day");
        assert_eq!(next_due(&schedule, Some(day(9)), at(9, 9, 0)), Some(at(10, 8, 0)));
        assert!(!due(&schedule, None, at(14, 9, 0)), "Workdays only by default");
        assert_eq!(next_due(&schedule, None, at(13, 12, 30)), Some(at(16, 8, 0)));
        let weekends = Schedule { weekends: true, ..schedule };
        assert!(due(&weekends, None, at(14, 9, 0)));
        assert_eq!(next_due(&Schedule { enabled: false, ..schedule }, None, at(9, 8, 0)), None);
        let late = Schedule { minutes: 600, ..schedule };
        assert!(!due(&late, None, at(9, 9, 59)) && due(&late, None, at(9, 10, 0)));
        assert_eq!((normalize_time(425), normalize_time(600), normalize_time(660)), (DEFAULT_TIME, 600, DEFAULT_TIME));
    }

    #[test]
    fn preferences_default_to_a_workday_eight_oclock_overview_and_persist_the_shown_day() {
        let old: crate::storage::Preferences = serde_json::from_str(r#"{"dark":false,"meeting_reminders":true}"#).unwrap();
        assert!(old.morning_overview && !old.morning_overview_weekends);
        assert_eq!(old.morning_overview_time, 480);
        assert_eq!(Schedule::from_preferences(&old), Schedule { enabled: true, minutes: 480, weekends: false });
        let invalid: crate::storage::Preferences = serde_json::from_str(r#"{"morning_overview_time":481}"#).unwrap();
        assert_eq!(Schedule::from_preferences(&invalid).minutes, DEFAULT_TIME);
        let root = tempfile::tempdir().unwrap();
        let store = crate::storage::Store::open(root.path()).unwrap();
        assert_eq!(store.morning_overview_shown().unwrap(), None);
        store.save_morning_overview_shown(day(11)).unwrap();
        store.save_morning_overview_shown(day(12)).unwrap();
        assert_eq!(crate::storage::Store::open(root.path()).unwrap().morning_overview_shown().unwrap(), Some(day(12)));
    }

    #[test]
    fn blocker_holds_block_focus_time_but_are_not_meetings_or_agenda_items() {
        let sources = wednesday(&[day(11), day(12)]);
        let mut events = sources.calendar.events.clone();
        let mut blocker = meeting("blocker", "BLOCKER: Pilot kickoff", day(11), (16, 30), (17, 30), "organizer");
        blocker.extra.insert("isOrganizer".into(), json!(true));
        blocker.extra.insert("showAs".into(), json!("busy"));
        let mut tomorrow = blocker.clone();
        tomorrow.id = "blocker-tomorrow".into();
        tomorrow.start += Duration::days(1);
        tomorrow.end += Duration::days(1);
        events.extend([blocker, tomorrow]);
        let calendar = Calendar::build(events, sources.calendar.coverage.clone(), Vec::new()).unwrap();
        let with = Sources { calendar: Arc::new(calendar), ..sources.clone() };
        let before = overview(&sources, local(day(11), 7, 30), Language::En);
        let after = overview(&with, local(day(11), 7, 30), Language::En);
        assert!(after.item("blocker").is_none(), "BLOCKERs are not on the agenda");
        assert_eq!((after.meetings, after.busy_minutes, after.last_end), (before.meetings, before.busy_minutes, before.last_end));
        assert_eq!(after.tomorrow.meetings, before.tomorrow.meetings);
        let free = |overview: &Overview| overview.focus.clone().unwrap().iter().map(|(start, end)| (*end - *start).num_minutes()).sum::<i64>();
        assert_eq!(free(&before) - free(&after), 60, "The hold still blocks focus time");
    }
}
