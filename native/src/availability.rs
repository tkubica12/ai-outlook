//! Free/busy for new meetings: typed, read-only Graph `getSchedule` and `findMeetingTimes`
//! requests, bounded response parsing and the availability summary. No AI is involved.

use crate::{
    attendees::Attendee,
    calendar::{Event, person_email},
    i18n::Language,
};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Minutes per `availabilityView` digit requested from Graph.
pub const INTERVAL_MINUTES: u16 = 15;
/// You and every attendee.
pub const MAX_SCHEDULES: usize = crate::attendees::LIMIT + 1;
/// Most suggestions shown by **Suggest times**.
pub const MAX_SUGGESTIONS: usize = 5;
/// Graph's `getSchedule` answers for at most 20 people per request; larger sets are split.
pub const SCHEDULES_PER_REQUEST: usize = 20;
/// Longest free/busy range: 15 workdays with their weekends and a margin, well below Graph's
/// 62-day limit.
pub const MAX_RANGE_DAYS: i64 = 22;
const MAX_ITEMS: usize = 200;
/// The longest range in 15-minute cells, with a 25-hour daylight-saving day of margin.
const MAX_CELLS: usize = (MAX_RANGE_DAYS as usize) * 96 + 8;
const TEXT_LIMIT: usize = 255;

/// Ordered by how strongly it blocks a meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    Free,
    WorkingElsewhere,
    Tentative,
    Unknown,
    Busy,
    OutOfOffice,
}

impl Status {
    /// One `availabilityView` digit.
    pub fn from_view(digit: char) -> Self {
        match digit {
            '0' => Self::Free,
            '1' => Self::Tentative,
            '2' => Self::Busy,
            '3' => Self::OutOfOffice,
            '4' => Self::WorkingElsewhere,
            _ => Self::Unknown,
        }
    }

    /// Graph `freeBusyStatus`.
    pub fn from_graph(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "free" => Self::Free,
            "tentative" => Self::Tentative,
            "busy" => Self::Busy,
            "oof" => Self::OutOfOffice,
            "workingelsewhere" => Self::WorkingElsewhere,
            _ => Self::Unknown,
        }
    }

    /// Free or working elsewhere: Outlook does not treat either as a conflict.
    pub fn available(self) -> bool {
        matches!(self, Self::Free | Self::WorkingElsewhere)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::WorkingElsewhere => "Working elsewhere",
            Self::Tentative => "Tentative",
            Self::Unknown => "Unknown",
            Self::Busy => "Busy",
            Self::OutOfOffice => "Out of office",
        }
    }

    /// Lower-case word for summaries and candidate reasons.
    pub fn word(self, language: Language) -> &'static str {
        match (language, self) {
            (Language::En, Self::Free) => "free",
            (Language::En, Self::WorkingElsewhere) => "working elsewhere",
            (Language::En, Self::Tentative) => "tentative",
            (Language::En, Self::Unknown) => "unknown",
            (Language::En, Self::Busy) => "busy",
            (Language::En, Self::OutOfOffice) => "out of office",
            (Language::Cs, Self::Free) => "volno",
            (Language::Cs, Self::WorkingElsewhere) => "pracuje jinde",
            (Language::Cs, Self::Tentative) => "nezávazně",
            (Language::Cs, Self::Unknown) => "neznámé",
            (Language::Cs, Self::Busy) => "obsazeno",
            (Language::Cs, Self::OutOfOffice) => "mimo kancelář",
        }
    }
}

fn midnight<Z: TimeZone>(zone: &Z, day: NaiveDate) -> Result<DateTime<Utc>, String> {
    let naive = day.and_hms_opt(0, 0, 0).ok_or("Invalid availability date")?;
    zone.from_local_datetime(&naive)
        .single()
        .map(|time| time.with_timezone(&Utc))
        .ok_or_else(|| {
            "Local midnight is missing or ambiguous because of daylight saving time".into()
        })
}

/// Graph `dateTimeTimeZone` in UTC, the convention of all Tomlook calendar requests.
fn graph_time(time: DateTime<Utc>) -> Value {
    json!({"dateTime": time.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"})
}

/// A response time; unzoned values are accepted only in UTC, never guessed.
fn parse_time(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.get("dateTime")?.as_str().filter(|text| text.len() <= 64)?;
    if let Ok(time) = DateTime::parse_from_rfc3339(text) {
        return Some(time.with_timezone(&Utc));
    }
    let zone = value
        .get("timeZone")
        .and_then(Value::as_str)
        .unwrap_or("UTC");
    if !matches!(
        zone,
        "UTC" | "utc" | "Etc/UTC" | "Coordinated Universal Time" | "tzone://Microsoft/Utc"
    ) {
        return None;
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|time| time.and_utc())
}

fn bounded_text(value: Option<&Value>) -> Option<String> {
    let text = value?
        .as_str()?
        .chars()
        .filter(|ch| !ch.is_control())
        .take(TEXT_LIMIT)
        .collect::<String>();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Read-only `POST /me/calendar/getSchedule` for one local day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleRequest {
    pub schedules: Vec<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub interval: u16,
}

impl ScheduleRequest {
    /// The whole local day in the Windows time zone, sent as UTC instants.
    pub fn day(day: NaiveDate, schedules: Vec<String>) -> Result<Self, String> {
        Self::in_zone(&chrono::Local, day, schedules)
    }

    pub fn in_zone<Z: TimeZone>(
        zone: &Z,
        day: NaiveDate,
        schedules: Vec<String>,
    ) -> Result<Self, String> {
        Self::range_in_zone(zone, day, day, schedules)
    }

    /// Whole local days `first..=last` in the Windows time zone, sent as UTC instants.
    pub fn range(first: NaiveDate, last: NaiveDate, schedules: Vec<String>) -> Result<Self, String> {
        Self::range_in_zone(&chrono::Local, first, last, schedules)
    }

    pub fn range_in_zone<Z: TimeZone>(
        zone: &Z,
        first: NaiveDate,
        last: NaiveDate,
        schedules: Vec<String>,
    ) -> Result<Self, String> {
        let next = last.succ_opt().ok_or("Invalid availability date")?;
        let request = Self {
            schedules,
            start: midnight(zone, first)?,
            end: midnight(zone, next)?,
            interval: INTERVAL_MINUTES,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schedules.is_empty() || self.schedules.len() > MAX_SCHEDULES {
            return Err("Availability can be checked for 1 to 51 people".into());
        }
        let mut seen = BTreeSet::new();
        if self.schedules.iter().any(|email| {
            person_email(email).as_deref() != Some(email.as_str()) || !seen.insert(email)
        }) {
            return Err("Availability needs valid, unique e-mail addresses".into());
        }
        if !matches!(self.interval, 15 | 30)
            || self.end <= self.start
            || self.end - self.start > Duration::days(MAX_RANGE_DAYS) + Duration::hours(2)
            || self.cells() > MAX_CELLS
        {
            return Err("Availability covers at most 22 days".into());
        }
        Ok(())
    }

    /// The same range for at most [`SCHEDULES_PER_REQUEST`] people each, in order.
    pub fn batches(&self) -> Vec<Self> {
        self.schedules
            .chunks(SCHEDULES_PER_REQUEST)
            .map(|schedules| Self {
                schedules: schedules.to_vec(),
                ..self.clone()
            })
            .collect()
    }

    pub fn cells(&self) -> usize {
        usize::try_from((self.end - self.start).num_minutes().max(0))
            .unwrap_or(usize::MAX)
            .div_ceil(usize::from(self.interval))
    }

    pub fn body(&self) -> Value {
        json!({
            "schedules": self.schedules,
            "startTime": graph_time(self.start),
            "endTime": graph_time(self.end),
            "availabilityViewInterval": self.interval,
        })
    }
}

/// One visible calendar item of a schedule; private items never carry text.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub status: Status,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub subject: Option<String>,
    pub location: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Schedule {
    pub email: String,
    /// One status per request interval from the request start.
    pub view: Vec<Status>,
    pub items: Vec<Item>,
    /// Why the schedule is unknown, for example an external or hidden calendar.
    pub error: Option<String>,
}

impl Schedule {
    fn unknown(email: String, cells: usize, error: &str) -> Self {
        Self {
            email,
            view: vec![Status::Unknown; cells],
            items: Vec::new(),
            error: Some(error.into()),
        }
    }

    /// Cells derived from items: free unless an item overlaps the cell.
    pub fn from_items(request: &ScheduleRequest, email: String, mut items: Vec<Item>) -> Self {
        items.truncate(MAX_ITEMS);
        let step = Duration::minutes(i64::from(request.interval));
        let view = (0..request.cells())
            .map(|index| {
                let start = request.start + step * index as i32;
                let end = start + step;
                items
                    .iter()
                    .filter(|item| item.start < end && item.end > start)
                    .map(|item| item.status)
                    .max()
                    .unwrap_or(Status::Free)
            })
            .collect();
        Self {
            email,
            view,
            items,
            error: None,
        }
    }

    /// Your own schedule from cached events, for the offline demo.
    pub fn from_events(request: &ScheduleRequest, email: &str, events: &[Event]) -> Self {
        let items = events
            .iter()
            .filter(|event| {
                event.extra.get("isCancelled").and_then(Value::as_bool) != Some(true)
                    && event.extra.get("showAs").and_then(Value::as_str) != Some("free")
                    && event.start.with_timezone(&Utc) < request.end
                    && event.end.with_timezone(&Utc) > request.start
            })
            .take(MAX_ITEMS)
            .map(|event| Item {
                status: match event.extra.get("showAs").and_then(Value::as_str) {
                    Some("oof") => Status::OutOfOffice,
                    Some("workingElsewhere") => Status::WorkingElsewhere,
                    _ if event.unconfirmed() => Status::Tentative,
                    _ => Status::Busy,
                },
                start: event.start.with_timezone(&Utc),
                end: event.end.with_timezone(&Utc),
                subject: Some(event.title.chars().take(TEXT_LIMIT).collect()),
                location: (!event.location.is_empty())
                    .then(|| event.location.chars().take(TEXT_LIMIT).collect()),
            })
            .collect();
        Self::from_items(request, email.to_owned(), items)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Availability {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub interval: u16,
    pub schedules: Vec<Schedule>,
}

fn schedule_error(error: &Value) -> &'static str {
    match error.get("responseCode").and_then(Value::as_str).unwrap_or_default() {
        "ErrorMailRecipientNotFound" | "ErrorInvalidSmtpAddress" => {
            "Not found in your organization's directory"
        }
        "ErrorNoFreeBusyAccess" | "ErrorProxyRequestNotAllowed" | "ErrorFreeBusyGenerationFailed" => {
            "Availability is not shared with you"
        }
        _ => "Availability is not visible",
    }
}

fn parse_item(value: &Value) -> Option<Item> {
    let status = Status::from_graph(value.get("status")?.as_str()?);
    let start = parse_time(value.get("start")?)?;
    let end = parse_time(value.get("end")?)?;
    if end <= start {
        return None;
    }
    let private = value.get("isPrivate").and_then(Value::as_bool) == Some(true);
    Some(Item {
        status,
        start,
        end,
        subject: (!private).then(|| bounded_text(value.get("subject"))).flatten(),
        location: (!private).then(|| bounded_text(value.get("location"))).flatten(),
    })
}

/// Parses a `getSchedule` response; every requested person gets a row, unknown when missing.
pub fn parse_schedule(request: &ScheduleRequest, value: &Value) -> Result<Availability, String> {
    request.validate()?;
    let rows = value
        .get("value")
        .and_then(Value::as_array)
        .ok_or("Availability response has no schedule list")?;
    if rows.len() > MAX_SCHEDULES {
        return Err("Availability response exceeded its schedule limit".into());
    }
    let cells = request.cells();
    let mut found = BTreeMap::new();
    for row in rows {
        let Some(email) = row
            .get("scheduleId")
            .and_then(Value::as_str)
            .and_then(person_email)
        else {
            continue;
        };
        if !request.schedules.contains(&email) || found.contains_key(&email) {
            continue;
        }
        let schedule = match row.get("error").filter(|error| !error.is_null()) {
            Some(error) => Schedule::unknown(email.clone(), cells, schedule_error(error)),
            None => {
                let items = row
                    .get("scheduleItems")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().take(MAX_ITEMS).filter_map(parse_item).collect::<Vec<_>>());
                match row
                    .get("availabilityView")
                    .and_then(Value::as_str)
                    .filter(|view| !view.is_empty())
                {
                    Some(view) => {
                        let mut statuses = view.chars().take(cells).map(Status::from_view).collect::<Vec<_>>();
                        statuses.resize(cells, Status::Unknown);
                        Schedule {
                            email: email.clone(),
                            view: statuses,
                            items: items.unwrap_or_default(),
                            error: None,
                        }
                    }
                    None => match items {
                        Some(items) => Schedule::from_items(request, email.clone(), items),
                        None => Schedule::unknown(email.clone(), cells, "Availability is not visible"),
                    },
                }
            }
        };
        found.insert(email, schedule);
    }
    let schedules = request
        .schedules
        .iter()
        .map(|email| {
            found.remove(email).unwrap_or_else(|| {
                Schedule::unknown(email.clone(), cells, "No availability was returned")
            })
        })
        .collect();
    Ok(Availability {
        start: request.start,
        end: request.end,
        interval: request.interval,
        schedules,
    })
}

impl Availability {
    /// Joins the answers of [`ScheduleRequest::batches`] of one range, in request order.
    pub fn merge(parts: Vec<Availability>) -> Result<Availability, String> {
        let mut parts = parts.into_iter();
        let mut merged = parts.next().ok_or("Availability response has no schedule list")?;
        for part in parts {
            if (part.start, part.end, part.interval) != (merged.start, merged.end, merged.interval) {
                return Err("Availability batches cover different ranges".into());
            }
            merged.schedules.extend(part.schedules);
        }
        if merged.schedules.len() > MAX_SCHEDULES {
            return Err("Availability response exceeded its schedule limit".into());
        }
        Ok(merged)
    }

    pub fn schedule(&self, email: &str) -> Option<&Schedule> {
        self.schedules.iter().find(|schedule| schedule.email == email)
    }

    /// The most blocking status during `[from, to)`; unknown outside the checked day.
    pub fn status(&self, email: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Status {
        self.schedule(email)
            .map_or(Status::Unknown, |schedule| self.schedule_status(schedule, from, to))
    }

    pub fn schedule_status(&self, schedule: &Schedule, from: DateTime<Utc>, to: DateTime<Utc>) -> Status {
        if schedule.error.is_some() || to <= from || from < self.start || to > self.end {
            return Status::Unknown;
        }
        let interval = i64::from(self.interval.max(1));
        let first = (from - self.start).num_minutes() / interval;
        let last = ((to - self.start).num_minutes() + interval - 1) / interval;
        (first..last)
            .map(|index| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| schedule.view.get(index))
                    .copied()
                    .unwrap_or(Status::Unknown)
            })
            .max()
            .unwrap_or(Status::Unknown)
    }

    /// Visible items overlapping `[from, to)`.
    pub fn items(
        schedule: &Schedule,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> impl Iterator<Item = &Item> {
        schedule
            .items
            .iter()
            .filter(move |item| item.start < to && item.end > from)
    }
}

/// "All available" or "3 of 5 available · Alex busy · Jamie out of office · Morgan unknown".
/// `you` is your own status when known; it is named only when you are not available.
pub fn summary(you: Option<Status>, attendees: &[(&str, Status)], language: Language) -> String {
    if attendees.is_empty() {
        return String::new();
    }
    let available = attendees.iter().filter(|(_, status)| status.available()).count();
    let total = attendees.len();
    if available == total && you.is_none_or(Status::available) {
        return match language {
            Language::En => "All available".into(),
            Language::Cs => "Všichni mají volno".into(),
        };
    }
    let mut parts = vec![match language {
        Language::En => format!("{available} of {total} available"),
        Language::Cs => match available {
            0 => format!("Volno nemá nikdo z {total}"),
            1 => format!("Volno má 1 z {total}"),
            2..=4 => format!("Volno mají {available} z {total}"),
            _ => format!("Volno má {available} z {total}"),
        },
    }];
    let named = |name: &str, status: Status| match language {
        Language::En => format!("{name} {}", status.word(language)),
        Language::Cs => format!("{name}: {}", status.word(language)),
    };
    if let Some(status) = you.filter(|status| !status.available()) {
        parts.push(named(
            match language {
                Language::En => "You",
                Language::Cs => "Vy",
            },
            status,
        ));
    }
    const NAMED: usize = 4;
    let unavailable = attendees
        .iter()
        .filter(|(_, status)| !status.available())
        .collect::<Vec<_>>();
    for (name, status) in unavailable.iter().take(NAMED) {
        parts.push(named(name, *status));
    }
    if unavailable.len() > NAMED {
        let more = unavailable.len() - NAMED;
        parts.push(match language {
            Language::En => format!("+{more} more"),
            Language::Cs => format!("+{more} dalších"),
        });
    }
    parts.join(" · ")
}

/// Local working hours searched by **Suggest times**, the same span as the scheduling grid.
pub const SUGGESTION_HOURS: (u32, u32) = (8, 18);

/// Read-only `POST /me/findMeetingTimes` around one local day for a fixed duration.
///
/// Graph's `work` activity domain depends on the mailbox's configured working hours and can
/// report the organizer unavailable all week, so the request sends explicit local 08:00-18:00
/// weekday slots with the `unrestricted` domain instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingTimesRequest {
    pub attendees: Vec<Attendee>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Local weekday working hours within `[start, end)`, never in the past.
    pub slots: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    pub minutes: u16,
}

impl MeetingTimesRequest {
    /// Weekday working hours from the day before through two days after `day`, never in the past.
    pub fn around(
        day: NaiveDate,
        minutes: u16,
        attendees: Vec<Attendee>,
        now: DateTime<Utc>,
    ) -> Result<Self, String> {
        Self::in_zone(&chrono::Local, day, minutes, attendees, now)
    }

    pub fn in_zone<Z: TimeZone>(
        zone: &Z,
        day: NaiveDate,
        minutes: u16,
        attendees: Vec<Attendee>,
        now: DateTime<Utc>,
    ) -> Result<Self, String> {
        use chrono::Datelike;
        let first = day.pred_opt().ok_or("Invalid availability date")?;
        let last = day
            .checked_add_days(chrono::Days::new(3))
            .ok_or("Invalid availability date")?;
        let quarter = Duration::minutes(15).num_seconds();
        let now = DateTime::from_timestamp(now.timestamp().div_euclid(quarter) * quarter, 0)
            .ok_or("Invalid current time")?
            + if now.timestamp().rem_euclid(quarter) == 0 {
                Duration::zero()
            } else {
                Duration::minutes(15)
            };
        let duration = Duration::minutes(i64::from(minutes));
        let at = |date: NaiveDate, hour: u32| {
            let naive = date.and_hms_opt(hour, 0, 0).ok_or("Invalid availability date")?;
            zone.from_local_datetime(&naive)
                .single()
                .map(|time| time.with_timezone(&Utc))
                .ok_or("This local time is missing or ambiguous because of daylight saving time")
        };
        let mut slots = Vec::new();
        let mut date = first;
        while date < last {
            if date.weekday().num_days_from_monday() < 5 {
                let start = at(date, SUGGESTION_HOURS.0)?.max(now);
                let end = at(date, SUGGESTION_HOURS.1)?;
                if end - start >= duration {
                    slots.push((start, end));
                }
            }
            date = date.succ_opt().ok_or("Invalid availability date")?;
        }
        if slots.is_empty() {
            return Err("Choose a day that is not in the past to find meeting times".into());
        }
        let request = Self {
            attendees,
            start: midnight(zone, first)?.max(now),
            end: midnight(zone, last)?,
            slots,
            minutes,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.attendees.is_empty() {
            return Err("Add attendees before suggesting times".into());
        }
        crate::attendees::validate_list(&self.attendees)?;
        if !(15..=1440).contains(&self.minutes)
            || !self.minutes.is_multiple_of(15)
            || self.end <= self.start
            || self.end - self.start > Duration::days(5)
            || self.slots.is_empty()
            || self.slots.len() > 5
            || self.slots.iter().any(|(start, end)| {
                *start < self.start
                    || *end > self.end
                    || *end - *start < Duration::minutes(i64::from(self.minutes))
            })
        {
            return Err("Meeting time suggestions need a 15-minute-step duration within five days".into());
        }
        Ok(())
    }

    pub fn body(&self) -> Value {
        let attendees = self
            .attendees
            .iter()
            .map(|attendee| {
                let mut address = json!({"address": attendee.email});
                if !attendee.name.is_empty() {
                    address["name"] = json!(attendee.name);
                }
                json!({"type": attendee.role.graph(), "emailAddress": address})
            })
            .collect::<Vec<_>>();
        let (hours, minutes) = (self.minutes / 60, self.minutes % 60);
        let duration = match (hours, minutes) {
            (0, minutes) => format!("PT{minutes}M"),
            (hours, 0) => format!("PT{hours}H"),
            (hours, minutes) => format!("PT{hours}H{minutes}M"),
        };
        let slots = self
            .slots
            .iter()
            .map(|(start, end)| json!({"start": graph_time(*start), "end": graph_time(*end)}))
            .collect::<Vec<_>>();
        json!({
            "attendees": attendees,
            "timeConstraint": {
                "activityDomain": "unrestricted",
                "timeSlots": slots,
            },
            "meetingDuration": duration,
            "maxCandidates": MAX_SUGGESTIONS,
            "isOrganizerOptional": false,
            "returnSuggestionReasons": false,
            "minimumAttendeePercentage": 50,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Graph's 0-100 confidence that everyone can attend.
    pub confidence: Option<f64>,
    pub organizer: Status,
    pub attendees: Vec<(String, Status)>,
}

impl Suggestion {
    pub fn available(&self) -> usize {
        self.attendees
            .iter()
            .filter(|(_, status)| status.available())
            .count()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeetingTimes {
    pub suggestions: Vec<Suggestion>,
    /// Why Graph returned no suggestion.
    pub empty_reason: Option<&'static str>,
    /// Graph's own `emptySuggestionsReason` code, when it is a short identifier.
    pub reason_code: Option<String>,
}

pub fn parse_meeting_times(value: &Value) -> Result<MeetingTimes, String> {
    let rows = value
        .get("meetingTimeSuggestions")
        .and_then(Value::as_array)
        .ok_or("Meeting time suggestions are unavailable")?;
    let mut suggestions = Vec::new();
    for row in rows.iter().take(20) {
        let Some(slot) = row.get("meetingTimeSlot") else {
            continue;
        };
        let (Some(start), Some(end)) = (
            slot.get("start").and_then(parse_time),
            slot.get("end").and_then(parse_time),
        ) else {
            continue;
        };
        if end <= start || end - start > Duration::days(1) {
            continue;
        }
        let attendees = row
            .get("attendeeAvailability")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .take(MAX_SCHEDULES)
                    .filter_map(|row| {
                        let email = row
                            .pointer("/attendee/emailAddress/address")
                            .and_then(Value::as_str)
                            .and_then(person_email)?;
                        let status = row
                            .get("availability")
                            .and_then(Value::as_str)
                            .map_or(Status::Unknown, Status::from_graph);
                        Some((email, status))
                    })
                    .collect()
            })
            .unwrap_or_default();
        suggestions.push(Suggestion {
            start,
            end,
            confidence: row
                .get("confidence")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.0, 100.0)),
            organizer: row
                .get("organizerAvailability")
                .and_then(Value::as_str)
                .map_or(Status::Unknown, Status::from_graph),
            attendees,
        });
        if suggestions.len() >= MAX_SUGGESTIONS {
            break;
        }
    }
    let code = value
        .get("emptySuggestionsReason")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let empty_reason = match code.to_ascii_lowercase().as_str() {
        "" => None,
        "attendeesunavailable" => Some("Attendees are not available in this range"),
        "attendeesunavailableorunknown" => {
            Some("Attendees are unavailable or their availability is unknown")
        }
        "organizerunavailable" => Some("You are not available in this range"),
        _ => Some("No meeting times were found in this range"),
    };
    Ok(MeetingTimes {
        suggestions,
        empty_reason: if rows.is_empty() {
            empty_reason.or(Some("No meeting times were found in this range"))
        } else {
            empty_reason
        },
        reason_code: (!code.is_empty()
            && code.len() <= 64
            && code.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then(|| code.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prague_request() -> ScheduleRequest {
        ScheduleRequest::in_zone(
            &chrono_tz::Europe::Prague,
            NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
            vec!["me@example.com".into(), "alex@example.com".into(), "dana@partner.example".into()],
        )
        .unwrap()
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn schedule_request_uses_local_day_boundaries_as_utc_and_a_15_minute_interval() {
        let request = prague_request();
        assert_eq!(request.body(), json!({
            "schedules": ["me@example.com", "alex@example.com", "dana@partner.example"],
            "startTime": {"dateTime": "2026-11-02T23:00:00", "timeZone": "UTC"},
            "endTime": {"dateTime": "2026-11-03T23:00:00", "timeZone": "UTC"},
            "availabilityViewInterval": 15,
        }));
        assert_eq!(request.cells(), 96);
        let autumn = ScheduleRequest::in_zone(&chrono_tz::Europe::Prague,
            NaiveDate::from_ymd_opt(2026, 10, 25).unwrap(), vec!["me@example.com".into()]).unwrap();
        assert_eq!(autumn.cells(), 100, "A 25-hour day has 100 quarter hours");
        for schedules in [vec![], vec!["Me@example.com".into()], vec!["me@example.com".into(), "me@example.com".into()],
            (0..=MAX_SCHEDULES).map(|index| format!("p{index}@example.com")).collect()] {
            assert!(ScheduleRequest::in_zone(&chrono_tz::Europe::Prague,
                NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(), schedules).is_err());
        }
        let mut long = prague_request();
        long.end += Duration::days(MAX_RANGE_DAYS);
        assert!(long.validate().is_err());
        let range = ScheduleRequest::range_in_zone(&chrono_tz::Europe::Prague,
            NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(), NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
            (0..45).map(|index| format!("p{index}@example.com")).collect()).unwrap();
        assert_eq!(range.cells(), 19 * 96 + 4, "Three weeks across the autumn change");
        let batches = range.batches();
        assert_eq!(batches.iter().map(|batch| batch.schedules.len()).collect::<Vec<_>>(), [20, 20, 5]);
        assert!(batches.iter().all(|batch| batch.start == range.start && batch.end == range.end));
        assert_eq!(batches.iter().flat_map(|batch| batch.schedules.clone()).collect::<Vec<_>>(), range.schedules);
    }

    #[test]
    fn schedule_response_parses_views_items_errors_and_hides_private_text() {
        let request = prague_request();
        let mut view = "0".repeat(96);
        // 09:00-10:00 local is cells 36..40; 13:00-13:30 cells 52..54.
        view.replace_range(36..40, "2222");
        view.replace_range(40..42, "11");
        view.replace_range(52..54, "33");
        view.replace_range(54..56, "44");
        let value = json!({"value": [
            {"scheduleId": "ME@example.com", "availabilityView": view, "scheduleItems": [
                {"status": "busy", "subject": "Design review", "location": "Room 2",
                 "start": {"dateTime": "2026-11-03T08:00:00.0000000", "timeZone": "UTC"},
                 "end": {"dateTime": "2026-11-03T09:00:00.0000000", "timeZone": "UTC"}},
                {"status": "tentative", "isPrivate": true, "subject": "Secret",
                 "start": {"dateTime": "2026-11-03T09:00:00", "timeZone": "UTC"},
                 "end": {"dateTime": "2026-11-03T09:30:00", "timeZone": "UTC"}},
                {"status": "busy", "subject": "Guessing forbidden",
                 "start": {"dateTime": "2026-11-03T09:00:00", "timeZone": "Pacific Standard Time"},
                 "end": {"dateTime": "2026-11-03T10:00:00", "timeZone": "Pacific Standard Time"}}
            ]},
            {"scheduleId": "dana@partner.example", "availabilityView": "",
             "error": {"message": "x", "responseCode": "ErrorMailRecipientNotFound"}},
            {"scheduleId": "stranger@example.com", "availabilityView": "2222"}
        ]});
        let availability = parse_schedule(&request, &value).unwrap();
        assert_eq!(availability.schedules.len(), 3);
        let me = &availability.schedules[0];
        assert_eq!(me.items.len(), 2, "Items in another unzoned time zone are not guessed");
        assert_eq!(me.items[0].subject.as_deref(), Some("Design review"));
        assert_eq!(me.items[0].location.as_deref(), Some("Room 2"));
        assert_eq!(me.items[1].subject, None, "Private items never show text");
        let status = |email: &str, from: &str, to: &str| availability.status(email, at(from), at(to));
        assert_eq!(status("me@example.com", "2026-11-03T09:00:00+01:00", "2026-11-03T10:00:00+01:00"), Status::Busy);
        assert_eq!(status("me@example.com", "2026-11-03T10:00:00+01:00", "2026-11-03T10:30:00+01:00"), Status::Tentative);
        assert_eq!(status("me@example.com", "2026-11-03T13:00:00+01:00", "2026-11-03T13:30:00+01:00"), Status::OutOfOffice);
        assert_eq!(status("me@example.com", "2026-11-03T13:30:00+01:00", "2026-11-03T14:00:00+01:00"), Status::WorkingElsewhere);
        assert_eq!(status("me@example.com", "2026-11-03T13:15:00+01:00", "2026-11-03T13:45:00+01:00"), Status::OutOfOffice);
        assert_eq!(status("me@example.com", "2026-11-03T14:00:00+01:00", "2026-11-03T15:00:00+01:00"), Status::Free);
        assert_eq!(status("me@example.com", "2026-11-02T23:00:00+01:00", "2026-11-03T01:00:00+01:00"), Status::Unknown,
            "Time outside the checked day is unknown");
        let dana = availability.schedule("dana@partner.example").unwrap();
        assert_eq!(dana.error.as_deref(), Some("Not found in your organization's directory"));
        assert_eq!(status("dana@partner.example", "2026-11-03T14:00:00+01:00", "2026-11-03T15:00:00+01:00"), Status::Unknown);
        let alex = availability.schedule("alex@example.com").unwrap();
        assert_eq!(alex.error.as_deref(), Some("No availability was returned"));
        assert!(availability.schedule("stranger@example.com").is_none());
        let short = parse_schedule(&request, &json!({"value": [{"scheduleId": "alex@example.com", "availabilityView": "2x"}]})).unwrap();
        let alex = short.schedule("alex@example.com").unwrap();
        assert_eq!(&alex.view[..3], &[Status::Busy, Status::Unknown, Status::Unknown]);
        let derived = parse_schedule(&request, &json!({"value": [{"scheduleId": "alex@example.com", "scheduleItems": [
            {"status": "workingElsewhere", "start": {"dateTime": "2026-11-03T08:00:00Z"}, "end": {"dateTime": "2026-11-03T09:00:00Z"}}
        ]}]})).unwrap();
        assert_eq!(derived.status("alex@example.com", at("2026-11-03T09:00:00+01:00"), at("2026-11-03T10:00:00+01:00")), Status::WorkingElsewhere);
        assert_eq!(derived.status("alex@example.com", at("2026-11-03T11:00:00+01:00"), at("2026-11-03T12:00:00+01:00")), Status::Free);
        assert!(parse_schedule(&request, &json!({"error": {}})).is_err());
        let many = (0..=MAX_SCHEDULES).map(|_| json!({"scheduleId": "alex@example.com"})).collect::<Vec<_>>();
        assert!(parse_schedule(&request, &json!({"value": many})).is_err());
        let long = json!({"value": [{"scheduleId": "alex@example.com", "availabilityView": "0", "scheduleItems": [
            {"status": "busy", "subject": "x".repeat(5000), "start": {"dateTime": "2026-11-03T08:00:00Z"}, "end": {"dateTime": "2026-11-03T09:00:00Z"}}
        ]}]});
        let parsed = parse_schedule(&request, &long).unwrap();
        assert_eq!(parsed.schedule("alex@example.com").unwrap().items[0].subject.as_ref().unwrap().chars().count(), 255);
    }

    #[test]
    fn status_digits_and_graph_names_cover_every_free_busy_state() {
        for (digit, name, status) in [('0', "free", Status::Free), ('1', "tentative", Status::Tentative),
            ('2', "busy", Status::Busy), ('3', "oof", Status::OutOfOffice),
            ('4', "workingElsewhere", Status::WorkingElsewhere), ('?', "unknown", Status::Unknown)] {
            assert_eq!(Status::from_view(digit), status);
            assert_eq!(Status::from_graph(name), status);
        }
        assert!(Status::Free.available() && Status::WorkingElsewhere.available());
        assert!(!Status::Tentative.available() && !Status::Unknown.available());
        assert!(Status::OutOfOffice > Status::Busy && Status::Busy > Status::Unknown && Status::Unknown > Status::Tentative);
    }

    #[test]
    fn summary_names_unavailable_people_with_english_and_czech_plurals() {
        use Status::*;
        let people = [("Alex", Busy), ("Jamie", OutOfOffice), ("Morgan", Unknown), ("Riley", Free), ("Kim", WorkingElsewhere)];
        assert_eq!(summary(Some(Free), &people, Language::En),
            "2 of 5 available · Alex busy · Jamie out of office · Morgan unknown");
        assert_eq!(summary(Some(Free), &people, Language::Cs),
            "Volno mají 2 z 5 · Alex: obsazeno · Jamie: mimo kancelář · Morgan: neznámé");
        assert_eq!(summary(Some(Free), &[("Alex", Free), ("Kim", WorkingElsewhere)], Language::En), "All available");
        assert_eq!(summary(None, &[("Alex", Free)], Language::Cs), "Všichni mají volno");
        assert_eq!(summary(Some(Busy), &[("Alex", Free)], Language::En), "1 of 1 available · You busy");
        assert_eq!(summary(Some(Tentative), &[("Alex", Free)], Language::Cs), "Volno má 1 z 1 · Vy: nezávazně");
        assert_eq!(summary(Some(Free), &[("Alex", Busy)], Language::Cs), "Volno nemá nikdo z 1 · Alex: obsazeno");
        let five = [("A", Free), ("B", Free), ("C", Free), ("D", Free), ("E", Free), ("F", Tentative)];
        assert_eq!(summary(Some(Free), &five, Language::Cs), "Volno má 5 z 6 · F: nezávazně");
        assert_eq!(summary(Some(Free), &five, Language::En), "5 of 6 available · F tentative");
        let busy = [("A", Busy), ("B", Busy), ("C", Busy), ("D", Busy), ("E", Busy), ("F", Busy)];
        assert_eq!(summary(Some(Free), &busy, Language::En), "0 of 6 available · A busy · B busy · C busy · D busy · +2 more");
        assert_eq!(summary(Some(Free), &busy, Language::Cs), "Volno nemá nikdo z 6 · A: obsazeno · B: obsazeno · C: obsazeno · D: obsazeno · +2 dalších");
        assert_eq!(summary(Some(Busy), &[], Language::En), "");
    }

    #[test]
    fn meeting_times_request_is_bounded_never_in_the_past_and_round_trips_suggestions() {
        let attendees = crate::attendees::parse_input("Alex Rivers <alex@example.com>; jamie@example.com").unwrap();
        let mut attendees = attendees;
        attendees[1].role = crate::attendees::Role::Optional;
        let day = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
        let request = MeetingTimesRequest::in_zone(&chrono_tz::Europe::Prague, day, 90, attendees.clone(),
            at("2026-11-03T10:07:00+01:00")).unwrap();
        assert_eq!(request.start, at("2026-11-03T10:15:00+01:00"), "Starts at the next quarter hour, not in the past");
        assert_eq!(request.end, at("2026-11-07T00:00:00+01:00"));
        let body = request.body();
        assert_eq!(body["meetingDuration"], "PT1H30M");
        assert_eq!(body["maxCandidates"], 5);
        assert_eq!(body["isOrganizerOptional"], false);
        assert_eq!(body["timeConstraint"]["activityDomain"], "unrestricted");
        assert_eq!(body["timeConstraint"]["timeSlots"][0]["start"], json!({"dateTime":"2026-11-03T09:15:00","timeZone":"UTC"}));
        assert_eq!(body["timeConstraint"]["timeSlots"][0]["end"], json!({"dateTime":"2026-11-03T17:00:00","timeZone":"UTC"}));
        assert_eq!(request.slots.len(), 4, "Tuesday to Friday working hours");
        assert_eq!(request.slots[3], (at("2026-11-06T08:00:00+01:00"), at("2026-11-06T18:00:00+01:00")));
        let weekend = MeetingTimesRequest::in_zone(&chrono_tz::Europe::Prague, NaiveDate::from_ymd_opt(2026, 11, 8).unwrap(),
            30, attendees.clone(), at("2026-11-01T08:00:00+01:00")).unwrap();
        assert_eq!(weekend.slots, vec![
            (at("2026-11-09T08:00:00+01:00"), at("2026-11-09T18:00:00+01:00")),
            (at("2026-11-10T08:00:00+01:00"), at("2026-11-10T18:00:00+01:00")),
        ], "Weekends are skipped");
        assert_eq!(body["attendees"], json!([
            {"type":"required","emailAddress":{"address":"alex@example.com","name":"Alex Rivers"}},
            {"type":"optional","emailAddress":{"address":"jamie@example.com"}}
        ]));
        let early = MeetingTimesRequest::in_zone(&chrono_tz::Europe::Prague, day, 60, attendees.clone(),
            at("2026-11-01T08:00:00+01:00")).unwrap();
        assert_eq!(early.start, at("2026-11-03T00:00:00+01:00"));
        assert_eq!(MeetingTimesRequest::in_zone(&chrono_tz::Europe::Prague, day, 30, attendees.clone(),
            at("2026-11-03T10:00:00+01:00")).unwrap().body()["meetingDuration"], "PT30M");
        assert!(MeetingTimesRequest::in_zone(&chrono_tz::Europe::Prague, day, 60, attendees.clone(),
            at("2026-11-20T08:00:00+01:00")).is_err(), "Past days have no suggestions");
        assert!(MeetingTimesRequest::in_zone(&chrono_tz::Europe::Prague, day, 60, Vec::new(),
            at("2026-11-01T08:00:00+01:00")).is_err());
        assert!(MeetingTimesRequest::in_zone(&chrono_tz::Europe::Prague, day, 50, attendees,
            at("2026-11-01T08:00:00+01:00")).is_err());
        let value = json!({"emptySuggestionsReason": "", "meetingTimeSuggestions": [
            {"confidence": 100.0, "organizerAvailability": "free",
             "meetingTimeSlot": {"start": {"dateTime": "2026-11-04T09:00:00.0000000", "timeZone": "UTC"},
                                 "end": {"dateTime": "2026-11-04T10:30:00.0000000", "timeZone": "UTC"}},
             "attendeeAvailability": [
                {"availability": "free", "attendee": {"type": "required", "emailAddress": {"address": "Alex@example.com"}}},
                {"availability": "tentative", "attendee": {"type": "optional", "emailAddress": {"address": "jamie@example.com"}}}]},
            {"confidence": 150.0, "organizerAvailability": "busy",
             "meetingTimeSlot": {"start": {"dateTime": "2026-11-04T09:00:00", "timeZone": "Pacific Standard Time"},
                                 "end": {"dateTime": "2026-11-04T10:00:00", "timeZone": "Pacific Standard Time"}}}
        ]});
        let parsed = parse_meeting_times(&value).unwrap();
        assert_eq!(parsed.suggestions.len(), 1, "Times in an unconfirmed zone are skipped");
        let suggestion = &parsed.suggestions[0];
        assert_eq!(suggestion.start, at("2026-11-04T10:00:00+01:00"));
        assert_eq!(suggestion.confidence, Some(100.0));
        assert_eq!(suggestion.organizer, Status::Free);
        assert_eq!(suggestion.attendees, vec![("alex@example.com".into(), Status::Free), ("jamie@example.com".into(), Status::Tentative)]);
        assert_eq!(suggestion.available(), 1);
        assert_eq!(parsed.empty_reason, None);
        let empty = parse_meeting_times(&json!({"emptySuggestionsReason": "attendeesUnavailable", "meetingTimeSuggestions": []})).unwrap();
        assert_eq!(empty.empty_reason, Some("Attendees are not available in this range"));
        assert_eq!(empty.reason_code.as_deref(), Some("attendeesUnavailable"));
        let empty = parse_meeting_times(&json!({"meetingTimeSuggestions": []})).unwrap();
        assert_eq!(empty.empty_reason, Some("No meeting times were found in this range"));
        assert_eq!(empty.reason_code, None);
        let odd = parse_meeting_times(&json!({"emptySuggestionsReason": "not a <code>", "meetingTimeSuggestions": []})).unwrap();
        assert_eq!(odd.reason_code, None, "Only short identifiers are kept");
        let live = parse_meeting_times(&json!({"emptySuggestionsReason": "OrganizerUnavailable", "meetingTimeSuggestions": []})).unwrap();
        assert_eq!(live.empty_reason, Some("You are not available in this range"), "Graph's reason casing varies");
        assert!(parse_meeting_times(&json!({"value": []})).is_err());
    }

    #[test]
    fn own_cached_events_become_a_schedule_without_free_or_cancelled_items() {
        let request = prague_request();
        let mut busy = crate::calendar::fixture(1).remove(0);
        busy.title = "Planning".into();
        busy.status = "accepted".into();
        busy.start = DateTime::parse_from_rfc3339("2026-11-03T09:00:00+01:00").unwrap();
        busy.end = DateTime::parse_from_rfc3339("2026-11-03T10:00:00+01:00").unwrap();
        busy.extra.insert("isOrganizer".into(), json!(true));
        busy.extra.insert("showAs".into(), json!("busy"));
        let mut free = busy.clone();
        free.start += Duration::hours(2);
        free.end += Duration::hours(2);
        free.extra.insert("showAs".into(), json!("free"));
        let mut cancelled = busy.clone();
        cancelled.start += Duration::hours(4);
        cancelled.end += Duration::hours(4);
        cancelled.extra.insert("isCancelled".into(), json!(true));
        let schedule = Schedule::from_events(&request, "me@example.com", &[busy, free, cancelled]);
        let availability = Availability { start: request.start, end: request.end, interval: request.interval, schedules: vec![schedule] };
        let status = |from: &str, to: &str| availability.status("me@example.com", at(from), at(to));
        assert_eq!(status("2026-11-03T09:30:00+01:00", "2026-11-03T10:30:00+01:00"), Status::Busy);
        assert_eq!(status("2026-11-03T11:00:00+01:00", "2026-11-03T12:00:00+01:00"), Status::Free);
        assert_eq!(status("2026-11-03T13:00:00+01:00", "2026-11-03T14:00:00+01:00"), Status::Free);
        assert_eq!(availability.schedules[0].items[0].subject.as_deref(), Some("Planning"));
    }
}
