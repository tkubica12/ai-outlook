use crate::calendar::{Calendar, Event};
use chrono::{DateTime, Duration, FixedOffset};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const LIMIT: usize = 4;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Category,
    Preparation,
    Travel,
    MissingMaterials,
    PriorityConflict,
    FollowUp,
    Transition,
    Manual,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Category => "Category",
            Self::Preparation => "Preparation",
            Self::Travel => "Travel buffer",
            Self::MissingMaterials => "Missing materials",
            Self::PriorityConflict => "Priority conflict",
            Self::FollowUp => "Follow-up",
            Self::Transition => "Meeting transition",
            Self::Manual => "Meeting action",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Suggestion {
    pub kind: Kind,
    pub reason: String,
    pub reason_cs: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub minutes: Option<u16>,
}

impl Suggestion {
    pub fn validate(&self) -> Result<(), String> {
        if self.kind == Kind::Manual {
            return Err("AI cannot propose manual meeting actions".into());
        }
        for reason in [&self.reason, &self.reason_cs] {
            if reason.trim().is_empty()
                || reason.len() > 1600
                || reason.chars().any(char::is_control)
            {
                return Err(
                    "Recommendation requires bounded English and Czech explanations".into(),
                );
            }
        }
        if self.category.as_ref().is_some_and(|category| {
            category.trim().is_empty()
                || category.len() > 256
                || category.chars().any(char::is_control)
        }) || self
            .minutes
            .is_some_and(|minutes| !(5..=180).contains(&minutes))
        {
            return Err("Recommendation category or duration exceeds its bounds".into());
        }
        if (self.kind == Kind::Category) != self.category.is_some()
            || (self.kind != Kind::Preparation && self.kind != Kind::Travel)
                && self.minutes.is_some()
        {
            return Err("Recommendation action does not match its type".into());
        }
        Ok(())
    }
}

pub fn validate_suggestions(suggestions: &[Suggestion]) -> Result<(), String> {
    if suggestions.len() > LIMIT {
        return Err("At most four recommendations per meeting are allowed".into());
    }
    for suggestion in suggestions {
        suggestion.validate()?;
    }
    if suggestions.iter().enumerate().any(|(index, suggestion)| {
        suggestions[..index]
            .iter()
            .any(|previous| previous.kind == suggestion.kind)
    }) {
        return Err("A meeting can have only one recommendation of each type".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum State {
    #[default]
    Pending,
    NotSent,
    Dismissed,
    Dispatching,
    Applied,
    Review,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Recommendation {
    pub id: String,
    pub account: String,
    pub event_id: String,
    #[serde(default)]
    pub event_title: String,
    #[serde(default)]
    pub source_url: Option<String>,
    pub fingerprint: String,
    pub suggestion: Suggestion,
    pub state: State,
    pub message: String,
    pub created_at: DateTime<FixedOffset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_change: Option<Change>,
}

impl Recommendation {
    pub fn from_insights(
        event: &Event,
        account: &str,
        suggestion: Suggestion,
        now: DateTime<FixedOffset>,
    ) -> Result<Self, String> {
        suggestion.validate()?;
        if suggestion.kind == Kind::Manual || event.unconfirmed() || cancelled(event) {
            return Err("Recommendations require a confirmed, noncancelled meeting".into());
        }
        let fingerprint = crate::scheduler::fingerprint(event);
        let action = serde_json::to_vec(&(
            account,
            &event.id,
            &fingerprint,
            suggestion.kind,
            &suggestion.category,
            suggestion.minutes,
        ))
        .map_err(|_| "Encode recommendation identity failed")?;
        Ok(Self {
            id: format!("{:x}", Sha256::digest(action)),
            account: account.into(),
            event_id: event.id.clone(),
            event_title: event.title.chars().take(256).collect(),
            source_url: event.source_url.clone(),
            fingerprint,
            suggestion,
            state: State::Pending,
            message: String::new(),
            created_at: now,
            manual_change: None,
        })
    }

    pub fn from_manual(
        event: &Event,
        account: &str,
        change: Change,
        now: DateTime<FixedOffset>,
    ) -> Result<Self, String> {
        validate_manual_change(event, &change)?;
        if account.is_empty() {
            return Err("Calendar account is unavailable".into());
        }
        let fingerprint = crate::scheduler::fingerprint(event);
        let identity = serde_json::to_vec(&(account, &event.id, &fingerprint, &change))
            .map_err(|_| "Encode manual calendar approval failed")?;
        Ok(Self {
            id: format!("{:x}", Sha256::digest(identity)),
            account: account.into(),
            event_id: event.id.clone(),
            event_title: event.title.chars().take(256).collect(),
            source_url: event.source_url.clone(),
            fingerprint,
            suggestion: Suggestion {
                kind: Kind::Manual,
                reason: "Manually confirmed meeting action.".into(),
                reason_cs: "Ručně potvrzená změna schůzky.".into(),
                category: None,
                minutes: None,
            },
            state: State::Pending,
            message: String::new(),
            created_at: now,
            manual_change: Some(change),
        })
    }

    pub fn current<'a>(
        &self,
        calendar: &'a Calendar,
        owner: Option<&str>,
        now: DateTime<FixedOffset>,
    ) -> Option<&'a Event> {
        if owner != Some(self.account.as_str()) {
            return None;
        }
        calendar
            .event_index(&self.event_id)
            .map(|index| &calendar.events[index])
            .filter(|event| {
                crate::scheduler::fingerprint(event) == self.fingerprint
                    && (self.suggestion.kind == Kind::Manual || !event.unconfirmed())
                    && !cancelled(event)
                    && (!matches!(
                        self.suggestion.kind,
                        Kind::Preparation | Kind::Travel | Kind::FollowUp
                    ) || !logistics_buffer(event))
                    && match self.suggestion.kind {
                        Kind::Manual => self
                            .manual_change
                            .as_ref()
                            .is_some_and(|change| validate_manual_change(event, change).is_ok()),
                        Kind::FollowUp => event.end + Duration::days(2) > now,
                        _ => event.end > now,
                    }
            })
    }

    pub fn visible(
        &self,
        calendar: &Calendar,
        owner: Option<&str>,
        now: DateTime<FixedOffset>,
    ) -> bool {
        if owner != Some(self.account.as_str()) {
            return false;
        }
        match self.state {
            State::Review | State::Dispatching => true,
            State::Pending | State::NotSent => {
                self.current(calendar, owner, now).is_some_and(|event| {
                    if self.suggestion.kind == Kind::FollowUp {
                        event.end <= now
                    } else {
                        event.start < now + Duration::days(14)
                    }
                })
            }
            _ => false,
        }
    }
}

fn cancelled(event: &Event) -> bool {
    matches!(
        event.status.to_ascii_lowercase().as_str(),
        "cancelled" | "canceled" | "deleted"
    ) || event
        .extra
        .get("isCancelled")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

fn logistics_buffer(event: &Event) -> bool {
    let title = event.title.to_lowercase();
    let mut words = title
        .split(|ch: char| !ch.is_alphabetic())
        .filter(|word| !word.is_empty());
    let first = match words.next() {
        Some("blocker") => words.next(),
        first => first,
    };
    matches!(
        first,
        Some("přejezd" | "přesun" | "presun" | "rezerva" | "příprava" | "priprava" | "preparation")
    ) || (matches!(first, Some("travel" | "transfer" | "reserve"))
        && words.next() == Some("buffer"))
}

pub fn real_categories(event: &Event) -> Vec<String> {
    event
        .extra
        .get("categories")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| {
            if event.category.is_empty() {
                Vec::new()
            } else {
                vec![event.category.clone()]
            }
        })
}

pub fn category_replacement(category: &str, known: &[String]) -> Result<serde_json::Value, String> {
    if category.trim().is_empty()
        || category.len() > 256
        || category.chars().any(char::is_control)
        || !known.iter().any(|known| known == category)
    {
        return Err(
            "Choose an existing category; no new category or calendar change was submitted".into(),
        );
    }
    Ok(serde_json::json!({"categories":[category]}))
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Change {
    Category(String),
    ClearCategory,
    Respond {
        response: MeetingResponse,
        comment: String,
        send_response: bool,
    },
    Delete,
    Buffer {
        title: String,
        start: DateTime<FixedOffset>,
        end: DateTime<FixedOffset>,
        category: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum MeetingResponse {
    Accept,
    Tentative,
    Decline,
    Cancel,
}

impl MeetingResponse {
    pub fn label(self) -> &'static str {
        match self {
            Self::Accept => "Accept invitation",
            Self::Tentative => "Tentatively accept",
            Self::Decline => "Decline invitation",
            Self::Cancel => "Cancel organized meeting",
        }
    }

    pub fn action(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Tentative => "tentativelyAccept",
            Self::Decline => "decline",
            Self::Cancel => "cancel",
        }
    }

    pub fn status(self) -> &'static str {
        match self {
            Self::Accept => "accepted",
            Self::Tentative => "tentativelyAccepted",
            Self::Decline => "declined",
            Self::Cancel => "cancelled",
        }
    }
}

pub fn validate_manual_change(event: &Event, change: &Change) -> Result<(), String> {
    event.validate()?;
    if cancelled(event) {
        return Err("Cancelled meetings cannot be changed".into());
    }
    if !matches!(
        event.extra.get("type").and_then(serde_json::Value::as_str),
        Some("singleInstance" | "occurrence" | "exception")
    ) {
        return Err("Refresh the calendar first; actions require one verified occurrence, not a whole series".into());
    }
    match change {
        Change::Category(category)
            if !category.trim().is_empty()
                && category.len() <= 256
                && !category.chars().any(char::is_control) =>
        {
            Ok(())
        }
        Change::ClearCategory => Ok(()),
        Change::Delete
            if event
                .extra
                .get("isOrganizer")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && event.attendee_count() == 0 =>
        {
            Ok(())
        }
        Change::Respond {
            response,
            comment,
            send_response,
        } => {
            if comment.len() > 4096
                || comment
                    .chars()
                    .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
            {
                return Err("Comment exceeds its 4096-byte limit or contains unsupported control characters".into());
            }
            if !send_response && !comment.is_empty() {
                return Err("A comment requires sending the response".into());
            }
            let organizer = event
                .extra
                .get("isOrganizer")
                .and_then(serde_json::Value::as_bool);
            match response {
                MeetingResponse::Cancel
                    if organizer == Some(true) && event.attendee_count() > 0 && *send_response =>
                {
                    Ok(())
                }
                MeetingResponse::Accept | MeetingResponse::Tentative | MeetingResponse::Decline
                    if organizer == Some(false) && !event.organizer.is_empty() =>
                {
                    Ok(())
                }
                _ => Err("Meeting action does not match your organizer or attendee role".into()),
            }
        }
        _ => Err("Meeting action does not match your organizer or attendee role".into()),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Operation {
    pub recommendation: Recommendation,
    pub event: Event,
    pub change: Change,
}

impl Operation {
    pub fn validate(&self, account: &str) -> Result<(), String> {
        if self.recommendation.suggestion.kind != Kind::Manual {
            self.recommendation.suggestion.validate()?;
        }
        if self.recommendation.state != State::Dispatching
            || self.recommendation.account != account
            || self.event.id != self.recommendation.event_id
            || crate::scheduler::fingerprint(&self.event) != self.recommendation.fingerprint
        {
            return Err(
                "Calendar approval no longer matches its account, meeting or proposal".into(),
            );
        }
        self.event.validate()?;
        if self.recommendation.id.len() != 64
            || !self
                .recommendation
                .id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Invalid calendar approval identity".into());
        }
        match (&self.change, self.recommendation.suggestion.kind) {
            (change, Kind::Manual)
                if self.recommendation.manual_change.as_ref() == Some(change) =>
            {
                validate_manual_change(&self.event, change)?;
            }
            (Change::Category(category), Kind::Category)
                if self.recommendation.suggestion.category.as_ref() == Some(category) => {}
            (Change::Buffer { .. }, Kind::Travel | Kind::Preparation) => {}
            _ => return Err("Calendar approval does not match the proposed action".into()),
        }
        if let Change::Buffer {
            title,
            start,
            end,
            category,
        } = &self.change
            && (title.trim().is_empty()
                || logistics_buffer(&self.event)
                || self.event.is_all_day
                || title.len() > 200
                || title.chars().any(char::is_control)
                || *end <= *start
                || *end - *start > Duration::hours(3)
                || (*end - *start).num_seconds()
                    != i64::from(self.recommendation.suggestion.minutes.unwrap_or(30)) * 60
                || *start < chrono::Local::now().fixed_offset()
                || *end > self.event.start
                || (self.recommendation.suggestion.kind == Kind::Travel
                    && *end != self.event.start)
                || category
                    .as_ref()
                    .is_some_and(|value| value.len() > 256 || value.chars().any(char::is_control)))
        {
            return Err(
                "The confirmed buffer must be a bounded future interval before its meeting".into(),
            );
        }
        Ok(())
    }
}

pub fn compact(
    inbox: &mut std::collections::BTreeMap<String, Recommendation>,
    calendar: &Calendar,
    owner: Option<&str>,
    now: DateTime<FixedOffset>,
) {
    inbox.retain(|_, recommendation| {
        !matches!(
            recommendation.state,
            State::Applied | State::Dismissed | State::Expired
        ) || recommendation.created_at >= now - Duration::days(30)
            || recommendation.current(calendar, owner, now).is_some()
    });
}

pub fn proposed_change(
    recommendation: &Recommendation,
    calendar: &Calendar,
    owner: Option<&str>,
    now: DateTime<FixedOffset>,
    language: crate::i18n::Language,
) -> Result<Change, String> {
    let event = recommendation
        .current(calendar, owner, now)
        .ok_or("Recommendation expired after a calendar change")?;
    if recommendation.suggestion.kind == Kind::Manual {
        return recommendation
            .manual_change
            .clone()
            .ok_or("Manual calendar approval has no action".into());
    }
    match recommendation.suggestion.kind {
        Kind::Category => {
            let category = recommendation
                .suggestion
                .category
                .clone()
                .ok_or("Category recommendation has no category")?;
            category_replacement(&category, &known_categories(calendar))?;
            Ok(Change::Category(category))
        }
        Kind::Travel | Kind::Preparation => {
            let minutes = recommendation.suggestion.minutes.unwrap_or(30);
            let (start, end) = free_interval(
                calendar,
                event,
                minutes,
                now,
                recommendation.suggestion.kind == Kind::Travel,
            )
            .ok_or("No free interval fits the proposed buffer; adjust the calendar first")?;
            let title = match (language, recommendation.suggestion.kind) {
                (crate::i18n::Language::Cs, Kind::Travel) => "Přejezd",
                (crate::i18n::Language::Cs, _) => "Příprava",
                (_, Kind::Travel) => "Travel buffer",
                _ => "Preparation",
            }
            .to_owned();
            let category = known_categories(calendar).into_iter().find(|category| {
                matches!(
                    category.to_lowercase().as_str(),
                    "nejisté" | "blocker" | "rezerva"
                )
            });
            Ok(Change::Buffer {
                title,
                start,
                end,
                category,
            })
        }
        _ => Err("This recommendation is informational; open its Insights".into()),
    }
}

pub fn known_categories(calendar: &Calendar) -> Vec<String> {
    calendar
        .events
        .iter()
        .flat_map(real_categories)
        .filter(|category| !category.is_empty() && category.len() <= 256)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(64)
        .collect()
}

pub fn calendar_context(calendar: &Calendar, event: &Event) -> serde_json::Value {
    let confirmed = calendar
        .events
        .iter()
        .filter(|other| {
            other.id != event.id
                && !other.unconfirmed()
                && !cancelled(other)
                && other
                    .extra
                    .get("showAs")
                    .and_then(serde_json::Value::as_str)
                    != Some("free")
        })
        .collect::<Vec<_>>();
    let nearby = |other: &&Event| {
        other.end >= event.start - Duration::days(2) && other.start <= event.end + Duration::days(2)
    };
    let previous = confirmed
        .iter()
        .copied()
        .filter(|other| other.end <= event.start)
        .filter(nearby)
        .max_by_key(|other| other.end);
    let next = confirmed
        .iter()
        .copied()
        .filter(|other| other.start >= event.end)
        .filter(nearby)
        .min_by_key(|other| other.start);
    let overlaps = confirmed
        .iter()
        .copied()
        .filter(|other| other.start < event.end && other.end > event.start)
        .collect::<Vec<_>>();
    let describe = |other: &Event| {
        serde_json::json!({
            "title":other.title.chars().take(256).collect::<String>(), "start":other.start, "end":other.end,
            "location":other.location.chars().take(256).collect::<String>(), "all_day":other.is_all_day,
        })
    };
    let previous_title = previous
        .map(|other| other.title.to_lowercase())
        .unwrap_or_default();
    let existing_travel = previous.is_some_and(|other| other.end == event.start)
        && [
            "přejezd", "přesun", "presun", "rezerva", "travel", "transfer",
        ]
        .iter()
        .any(|word| previous_title.contains(word));
    serde_json::json!({
        "previous_confirmed":previous.map(describe), "next_confirmed":next.map(describe),
        "overlaps_confirmed":overlaps.iter().take(6).map(|other| describe(other)).collect::<Vec<_>>(),
        "overlaps_complete":overlaps.len() <= 6, "existing_travel_buffer":existing_travel,
        "current_title_is_buffer":logistics_buffer(event),
        "day_covered":calendar.coverage.contains(&event.start.with_timezone(&chrono::Local).date_naive()),
        "position_and_traffic_verified":false,
    })
}

pub fn protected_calendar_context(event: &Event) -> Vec<String> {
    let Some(context) = event.extra.get("tomlook_calendar_context") else {
        return Vec::new();
    };
    let mut meetings = ["previous_confirmed", "next_confirmed"]
        .iter()
        .filter_map(|key| context.get(*key))
        .collect::<Vec<_>>();
    if let Some(overlaps) = context
        .get("overlaps_confirmed")
        .and_then(serde_json::Value::as_array)
    {
        meetings.extend(overlaps.iter().take(6));
    }
    meetings
        .into_iter()
        .flat_map(|meeting| {
            ["title", "location"].iter().filter_map(move |key| {
                meeting
                    .get(*key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
        })
        .collect()
}

pub fn free_interval(
    calendar: &Calendar,
    event: &Event,
    minutes: u16,
    now: DateTime<FixedOffset>,
    travel: bool,
) -> Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    if event.is_all_day || !(5..=180).contains(&minutes) {
        return None;
    }
    let duration = Duration::minutes(i64::from(minutes));
    let end = event.start;
    let earliest = now + Duration::minutes(5);
    let first = if travel {
        end - duration
    } else {
        end - Duration::days(2)
    }
    .max(earliest);
    let mut intervals = calendar
        .events
        .iter()
        .filter(|other| {
            !other.unconfirmed()
                && !cancelled(other)
                && other.id != event.id
                && other
                    .extra
                    .get("showAs")
                    .and_then(serde_json::Value::as_str)
                    != Some("free")
                && other.end > first
                && other.start < end
        })
        .map(|other| (other.start, other.end))
        .collect::<Vec<_>>();
    intervals.sort_by_key(|interval| interval.0);
    let mut latest = end;
    for (start, finish) in intervals.into_iter().rev() {
        if let Some(interval) = working_interval(finish.max(first), latest, duration) {
            return Some(interval);
        }
        latest = latest.min(start);
    }
    working_interval(first, latest, duration)
}

fn working_interval(
    first: DateTime<FixedOffset>,
    last: DateTime<FixedOffset>,
    duration: Duration,
) -> Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    use chrono::{Datelike, Local, TimeZone};
    let mut day = last.with_timezone(&Local).date_naive();
    let first_day = first.with_timezone(&Local).date_naive();
    while day >= first_day {
        if day.weekday().num_days_from_monday() < 5 {
            let begin = Local
                .from_local_datetime(&day.and_hms_opt(8, 0, 0)?)
                .single()?
                .fixed_offset()
                .max(first);
            let end = Local
                .from_local_datetime(&day.and_hms_opt(18, 0, 0)?)
                .single()?
                .fixed_offset()
                .min(last);
            if end - begin >= duration {
                return Some((end - duration, end));
            }
        }
        day = day.pred_opt()?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn category_updates_replace_the_whole_list_and_never_invent_categories() {
        assert_eq!(
            category_replacement("Office", &["Office".into(), "Call".into()]).unwrap(),
            serde_json::json!({"categories":["Office"]})
        );
        assert!(category_replacement("Invented", &["Office".into()]).is_err());
    }
    #[test]
    fn duplicate_explanations_share_an_action_identity_and_changed_events_expire() {
        let now = chrono::Local::now().fixed_offset();
        let mut event = crate::calendar::fixture(1).remove(0);
        event.start = now + Duration::hours(1);
        event.end = event.start + Duration::hours(1);
        let suggestion = Suggestion {
            kind: Kind::Category,
            reason: "An office visit.".into(),
            reason_cs: "Navsteva kancelare.".into(),
            category: Some("Office".into()),
            minutes: None,
        };
        let first =
            Recommendation::from_insights(&event, "owner", suggestion.clone(), now).unwrap();
        let mut revised = suggestion;
        revised.reason = "Different wording, same action.".into();
        let second = Recommendation::from_insights(&event, "owner", revised, now).unwrap();
        assert_eq!(first.id, second.id);
        let mut calendar = Calendar::build(vec![event], Default::default(), Vec::new()).unwrap();
        assert!(first.current(&calendar, Some("owner"), now).is_some());
        assert!(first.current(&calendar, Some("other"), now).is_none());
        calendar.events[0].title.push_str(" changed");
        assert!(first.current(&calendar, Some("owner"), now).is_none());
    }
}
