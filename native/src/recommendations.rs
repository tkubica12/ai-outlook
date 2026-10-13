use crate::calendar::{Calendar, Event};
use chrono::{DateTime, Duration, FixedOffset};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: usize = 4;
const POLICY_REVISION: u8 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Category,
    Preparation,
    PreparationAdvance,
    Travel,
    TravelAfter,
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
            Self::PreparationAdvance => "Advance preparation",
            Self::Travel => "Travel before meeting",
            Self::TravelAfter => "Travel after meeting",
            Self::MissingMaterials => "Missing materials",
            Self::PriorityConflict => "Priority conflict",
            Self::FollowUp => "Follow-up",
            Self::Transition => "Meeting transition",
            Self::Manual => "Meeting action",
        }
    }

    pub fn is_travel(self) -> bool {
        matches!(self, Self::Travel | Self::TravelAfter)
    }

    pub fn is_preparation(self) -> bool {
        matches!(self, Self::Preparation | Self::PreparationAdvance)
    }

    fn intent(self) -> Self {
        if self.is_preparation() {
            Self::Preparation
        } else {
            self
        }
    }

    fn intent_key(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Preparation | Self::PreparationAdvance => "preparation",
            Self::Travel => "travel",
            Self::TravelAfter => "travel_after",
            Self::MissingMaterials => "missing_materials",
            Self::PriorityConflict => "priority_conflict",
            Self::FollowUp => "follow_up",
            Self::Transition => "transition",
            Self::Manual => "manual",
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
            || (!self.kind.is_preparation() && !self.kind.is_travel()) && self.minutes.is_some()
            || self.kind == Kind::PreparationAdvance
                && !self
                    .minutes
                    .is_some_and(|minutes| (60..=180).contains(&minutes))
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
            .any(|previous| previous.kind.intent() == suggestion.kind.intent())
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
    #[serde(default)]
    pub policy_revision: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_change: Option<Change>,
}

impl Recommendation {
    pub fn dismissal_key(&self) -> Option<String> {
        if matches!(self.suggestion.kind, Kind::Manual | Kind::PriorityConflict) {
            return None;
        }
        let mut hash = Sha256::new();
        hash.update(b"tomlook-dismissal-v1");
        for part in [
            self.account.as_str(),
            self.event_id.as_str(),
            self.suggestion.kind.intent_key(),
            self.suggestion.category.as_deref().unwrap_or(""),
        ] {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part.as_bytes());
        }
        Some(format!("{:x}", hash.finalize()))
    }

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
        let mut action = serde_json::to_vec(&(
            account,
            &event.id,
            &fingerprint,
            suggestion.kind,
            &suggestion.category,
            suggestion.minutes,
        ))
        .map_err(|_| "Encode recommendation identity failed")?;
        if suggestion.kind == Kind::Category || suggestion.kind.is_travel() {
            action.push(POLICY_REVISION);
        }
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
            policy_revision: POLICY_REVISION,
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
            policy_revision: POLICY_REVISION,
            manual_change: Some(change),
        })
    }

    pub fn current<'a>(
        &self,
        calendar: &'a Calendar,
        owner: Option<&str>,
        now: DateTime<FixedOffset>,
    ) -> Option<&'a Event> {
        if owner != Some(self.account.as_str())
            || ((self.suggestion.kind == Kind::Category || self.suggestion.kind.is_travel())
                && self.policy_revision != POLICY_REVISION)
        {
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
                        Kind::Preparation
                            | Kind::PreparationAdvance
                            | Kind::Travel
                            | Kind::TravelAfter
                            | Kind::FollowUp
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
            (
                Change::Buffer { .. },
                Kind::Travel | Kind::TravelAfter | Kind::Preparation | Kind::PreparationAdvance,
            ) => {}
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
                || category != &buffer_category(&self.event)
                || (*end - *start).num_seconds()
                    != i64::from(self.recommendation.suggestion.minutes.unwrap_or(30)) * 60
                || *start < chrono::Local::now().fixed_offset()
                || match self.recommendation.suggestion.kind {
                    Kind::Travel => *end != self.event.start,
                    Kind::TravelAfter => *start != self.event.end,
                    _ => *end > self.event.start,
                }
                || category
                    .as_ref()
                    .is_some_and(|value| value.len() > 256 || value.chars().any(char::is_control)))
        {
            return Err(
                "The confirmed buffer must match its proposed duration and side of the meeting"
                    .into(),
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
        Kind::Travel | Kind::TravelAfter | Kind::Preparation | Kind::PreparationAdvance => {
            let minutes = recommendation.suggestion.minutes.unwrap_or(30);
            let (start, end) = if recommendation.suggestion.kind.is_travel() {
                travel_interval(
                    calendar,
                    event,
                    minutes,
                    now,
                    recommendation.suggestion.kind,
                )
            } else if recommendation.suggestion.kind == Kind::PreparationAdvance {
                advance_preparation_interval(calendar, event, minutes, now)
            } else {
                free_interval(calendar, event, minutes, now, false)
            }
            .ok_or("No free interval fits the proposed buffer; adjust the calendar first")?;
            let title = match (language, recommendation.suggestion.kind) {
                (crate::i18n::Language::Cs, Kind::Travel) => "Přejezd",
                (crate::i18n::Language::Cs, Kind::TravelAfter) => "Přejezd po schůzce",
                (crate::i18n::Language::Cs, _) => "Příprava",
                (_, Kind::Travel) => "Travel buffer",
                (_, Kind::TravelAfter) => "Travel after meeting",
                _ => "Preparation",
            }
            .to_owned();
            let category = buffer_category(event);
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

fn buffer_category(event: &Event) -> Option<String> {
    real_categories(event).into_iter().find(|category| {
        matches!(
            category.trim().to_lowercase().as_str(),
            "nejisté" | "nejiste" | "blocker"
        )
    })
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

pub fn category_meanings(categories: &[String]) -> serde_json::Value {
    let meanings = categories.iter().filter_map(|category| {
        let meaning = match category.trim().to_lowercase().as_str() {
            "office" => "In person at the user's home office or employer's premises. A customer's or partner's office is not this category. Require affirmative evidence of the home/employer office.",
            "mimo office" => "In person away from the user's home/employer office, including a customer's or partner's premises within the home city.",
            "mimo prahu" => "A trip or in-person meeting outside Prague; do not infer a route or duration from the category alone.",
            "call" => "Remote participation by video or phone. A conference link alone does not make an explicitly in-person or hybrid visit remote.",
            "nejisté" | "nejiste" => "A genuinely uncertain or tentative event. Not a generic logistics, travel or preparation category. A buffer inherits uncertainty only from an uncertain parent meeting; confirmed meetings have uncategorized buffers.",
            _ => return None,
        };
        Some((category.clone(), serde_json::Value::String(meaning.into())))
    });
    serde_json::Value::Object(meanings.collect())
}

pub fn complete_suggestions(
    calendar: &Calendar,
    event: &Event,
    mut suggestions: Vec<Suggestion>,
) -> Result<Vec<Suggestion>, String> {
    validate_suggestions(&suggestions)?;
    if event.unconfirmed() || cancelled(event) {
        return Ok(Vec::new());
    }
    let categories = known_categories(calendar);
    suggestions.retain(|suggestion| {
        suggestion.kind != Kind::Category
            || (suggestion
                .category
                .as_ref()
                .is_some_and(|category| categories.contains(category))
                && real_categories(event)
                    != suggestion.category.iter().cloned().collect::<Vec<_>>())
    });
    let offsite_category = |category: &String| {
        matches!(
            category.trim().to_lowercase().as_str(),
            "mimo office" | "mimo prahu"
        )
    };
    let effective_categories = suggestions
        .iter()
        .find_map(|suggestion| suggestion.category.as_ref())
        .map(|category| vec![category.clone()])
        .unwrap_or_else(|| real_categories(event));
    let offsite = effective_categories.iter().any(offsite_category);
    let remote = !offsite
        && effective_categories
            .iter()
            .any(|category| category.trim().eq_ignore_ascii_case("call"));
    let placeholder = logistics_buffer(event);
    let context = calendar_context(calendar, event);
    suggestions.retain(|suggestion| {
        if suggestion.kind.is_travel() && (remote || event.is_all_day || placeholder)
            || suggestion.kind.is_preparation() && (event.is_all_day || placeholder)
            || suggestion.kind == Kind::FollowUp && placeholder
        {
            return false;
        }
        match suggestion.kind {
            Kind::Travel => context["existing_travel_buffer"] != true,
            Kind::TravelAfter => context["existing_travel_buffer_after"] != true,
            _ => true,
        }
    });
    if offsite && !event.is_all_day && !placeholder {
        for (kind, existing) in [
            (Kind::Travel, context["existing_travel_buffer"] == true),
            (
                Kind::TravelAfter,
                context["existing_travel_buffer_after"] == true,
            ),
        ] {
            if existing || suggestions.iter().any(|suggestion| suggestion.kind == kind) {
                continue;
            }
            if suggestions.len() == LIMIT {
                let Some(index) = suggestions.iter().rposition(|suggestion| {
                    suggestion.kind != Kind::Category
                        && !suggestion.kind.is_preparation()
                        && !suggestion.kind.is_travel()
                }) else {
                    return Err("Offsite travel recommendations exceed their safety limit".into());
                };
                suggestions.remove(index);
            }
            let (reason, reason_cs) = if kind == Kind::Travel {
                (
                    "An in-person visit away from your home office needs time to get there. Allow an estimated 30 minutes before the meeting.",
                    "Na osobní schůzku mimo domovskou kancelář je potřeba dojet. Vyhraď si před ní přibližně 30 minut na přesun.",
                )
            } else {
                (
                    "An in-person visit away from your home office needs time for the onward journey. Allow an estimated 30 minutes after the meeting.",
                    "Po osobní schůzce mimo domovskou kancelář je potřeba čas na další cestu. Vyhraď si po ní přibližně 30 minut na přesun.",
                )
            };
            suggestions.push(Suggestion {
                kind,
                reason: reason.into(),
                reason_cs: reason_cs.into(),
                category: None,
                minutes: Some(30),
            });
        }
    }
    validate_suggestions(&suggestions)?;
    Ok(suggestions)
}

fn travel_buffer(event: &Event) -> bool {
    if !logistics_buffer(event) {
        return false;
    }
    let title = event.title.to_lowercase();
    let first = title.trim_start_matches("blocker").trim_start();
    !["příprava", "priprava", "preparation"]
        .iter()
        .any(|prefix| first.starts_with(prefix))
}

fn blocks_time(event: &Event) -> bool {
    !event.unconfirmed()
        && !cancelled(event)
        && event
            .extra
            .get("showAs")
            .and_then(serde_json::Value::as_str)
            != Some("free")
}

pub fn conflicts<'a>(calendar: &'a Calendar, event: &Event) -> Vec<&'a Event> {
    let mut conflicts = calendar
        .events
        .iter()
        .filter(|other| {
            other.id != event.id
                && blocks_time(other)
                && other.start < event.end
                && other.end > event.start
        })
        .collect::<Vec<_>>();
    conflicts.sort_by_key(|other| (other.start.max(event.start), &other.id));
    conflicts
}

pub fn conflict_dismissal_key(account: &str, first: &str, second: &str) -> String {
    let (first, second) = if first < second {
        (first, second)
    } else {
        (second, first)
    };
    let mut hash = Sha256::new();
    hash.update(b"tomlook-conflict-dismissal-v1");
    for part in [account, first, second] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxEntry {
    pub id: String,
    pub conflicts: Vec<String>,
}

pub fn inbox_view(
    calendar: &Calendar,
    inbox: &BTreeMap<String, Recommendation>,
    owner: Option<&str>,
    now: DateTime<FixedOffset>,
    dismissals: &BTreeSet<String>,
) -> Vec<InboxEntry> {
    let mut covered = dismissals.clone();
    // Preserve skips from versions that did not yet record individual conflict pairs.
    for record in inbox.values().filter(|record| {
        record.suggestion.kind == Kind::PriorityConflict && record.state == State::Dismissed
    }) {
        if let Some(event) = record.current(calendar, owner, now) {
            let keys = conflicts(calendar, event)
                .into_iter()
                .map(|other| conflict_dismissal_key(&record.account, &event.id, &other.id))
                .collect::<Vec<_>>();
            if !keys.iter().any(|key| dismissals.contains(key)) {
                covered.extend(keys);
            }
        }
    }
    let mut records = inbox
        .values()
        .filter(|record| record.visible(calendar, owner, now))
        .collect::<Vec<_>>();
    records.sort_by_key(|record| {
        (
            calendar
                .event_index(&record.event_id)
                .map(|index| calendar.events[index].start),
            record.suggestion.kind,
            &record.event_id,
            &record.id,
        )
    });
    records
        .into_iter()
        .filter_map(|record| {
            let mut entry = InboxEntry {
                id: record.id.clone(),
                conflicts: Vec::new(),
            };
            if record.suggestion.kind == Kind::PriorityConflict {
                if let Some(event) = record.current(calendar, owner, now) {
                    entry.conflicts = conflicts(calendar, event)
                        .into_iter()
                        .filter(|other| {
                            covered.insert(conflict_dismissal_key(
                                &record.account,
                                &event.id,
                                &other.id,
                            ))
                        })
                        .map(|other| other.id.clone())
                        .collect();
                }
                if entry.conflicts.is_empty()
                    && !matches!(record.state, State::Review | State::Dispatching)
                {
                    return None;
                }
            }
            Some(entry)
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct MaterialContact {
    pub person: String,
    pub email: String,
    pub organizer: bool,
    pub same_domain: bool,
}

pub fn material_contacts(
    event: &Event,
    work_email: &str,
    identity: Option<&crate::workiq::connection::Identity>,
) -> Vec<MaterialContact> {
    let own_email = crate::calendar::person_email(work_email);
    let primary_email = identity
        .filter(|identity| {
            own_email
                .as_deref()
                .is_some_and(|email| identity.account.eq_ignore_ascii_case(email))
        })
        .and_then(|identity| identity.mail.as_deref())
        .and_then(crate::calendar::person_email);
    let own_domain = primary_email
        .as_ref()
        .or(own_email.as_ref())
        .and_then(|email| email.split_once('@'))
        .map(|(_, domain)| domain);
    let mut seen = BTreeSet::new();
    let mut contacts = std::iter::once((&event.organizer, true))
        .chain(event.attendees.iter().map(|person| (person, false)))
        .filter_map(|(person, organizer)| {
            let email = crate::calendar::person_email(person)?;
            if own_email.as_ref() == Some(&email)
                || primary_email.as_ref() == Some(&email)
                || !seen.insert(email.clone())
            {
                return None;
            }
            let domain = email.split_once('@')?.1;
            Some(MaterialContact {
                person: person.clone(),
                organizer,
                same_domain: own_domain == Some(domain),
                email,
            })
        })
        .collect::<Vec<_>>();
    contacts.sort_by_key(|contact| (!contact.organizer, !contact.same_domain));
    contacts.truncate(6);
    contacts
}

pub fn calendar_context(calendar: &Calendar, event: &Event) -> serde_json::Value {
    let confirmed = calendar
        .events
        .iter()
        .filter(|other| other.id != event.id && blocks_time(other))
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
            "title":other.title.chars().take(256).collect::<String>(),
            "start":if other.is_all_day { other.start } else { other.start.with_timezone(&chrono::Local).fixed_offset() },
            "end":if other.is_all_day { other.end } else { other.end.with_timezone(&chrono::Local).fixed_offset() },
            "calendar_dates":other.calendar_dates(),
            "location":other.location.chars().take(256).collect::<String>(), "all_day":other.is_all_day,
        })
    };
    let existing_travel = confirmed
        .iter()
        .any(|other| other.end == event.start && travel_buffer(other));
    let existing_travel_after = confirmed
        .iter()
        .any(|other| other.start == event.end && travel_buffer(other));
    serde_json::json!({
        "current_start_local":(!event.is_all_day).then(|| event.start.with_timezone(&chrono::Local).fixed_offset()),
        "current_end_local":(!event.is_all_day).then(|| event.end.with_timezone(&chrono::Local).fixed_offset()),
        "current_calendar_dates":event.calendar_dates(),
        "planning_now_local":chrono::Local::now().fixed_offset(),
        "display_time_zone":"local",
        "previous_confirmed":previous.map(describe), "next_confirmed":next.map(describe),
        "overlaps_confirmed":overlaps.iter().take(6).map(|other| describe(other)).collect::<Vec<_>>(),
        "overlaps_complete":overlaps.len() <= 6, "existing_travel_buffer":existing_travel,
        "existing_travel_buffer_after":existing_travel_after,
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
    if travel {
        return travel_interval(calendar, event, minutes, now, Kind::Travel);
    }
    if event.is_all_day || !(5..=180).contains(&minutes) {
        return None;
    }
    let duration = Duration::minutes(i64::from(minutes));
    let end = event.start;
    let earliest = now + Duration::minutes(5);
    let first = (end - Duration::days(2)).max(earliest);
    let mut intervals = calendar
        .events
        .iter()
        .filter(|other| {
            blocks_time(other) && other.id != event.id && other.end > first && other.start < end
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

pub fn advance_preparation_interval(
    calendar: &Calendar,
    event: &Event,
    minutes: u16,
    now: DateTime<FixedOffset>,
) -> Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    if event.is_all_day || !(60..=180).contains(&minutes) {
        return None;
    }
    let duration = Duration::minutes(i64::from(minutes));
    let last = event.start;
    let first = (last - Duration::days(14)).max(now + Duration::minutes(5));
    let mut intervals = calendar
        .events
        .iter()
        .filter(|other| {
            other.id != event.id && blocks_time(other) && other.end > first && other.start < last
        })
        .map(|other| (other.start, other.end))
        .collect::<Vec<_>>();
    intervals.sort_by_key(|interval| interval.0);
    let mut earliest = first;
    for (start, end) in intervals {
        if let Some(interval) = first_working_interval(earliest, start.min(last), duration) {
            return Some(interval);
        }
        earliest = earliest.max(end);
    }
    first_working_interval(earliest, last, duration)
}

fn first_working_interval(
    first: DateTime<FixedOffset>,
    last: DateTime<FixedOffset>,
    duration: Duration,
) -> Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    use chrono::{Datelike, Local, TimeZone};
    let mut day = first.with_timezone(&Local).date_naive();
    let last_day = last.with_timezone(&Local).date_naive();
    while day <= last_day {
        if day.weekday().num_days_from_monday() < 5 {
            let start = Local
                .from_local_datetime(&day.and_hms_opt(8, 0, 0)?)
                .single()?
                .fixed_offset()
                .max(first);
            let mut seconds = start.timestamp().div_euclid(300) * 300;
            if seconds < start.timestamp() || start.timestamp_subsec_nanos() > 0 {
                seconds += 300;
            }
            let start = DateTime::from_timestamp(seconds, 0)?.with_timezone(start.offset());
            let end = Local
                .from_local_datetime(&day.and_hms_opt(18, 0, 0)?)
                .single()?
                .fixed_offset()
                .min(last);
            if end - start >= duration {
                return Some((start, start + duration));
            }
        }
        day = day.succ_opt()?;
    }
    None
}

pub fn travel_interval(
    calendar: &Calendar,
    event: &Event,
    minutes: u16,
    now: DateTime<FixedOffset>,
    kind: Kind,
) -> Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    use chrono::{Datelike, Local, Timelike};
    if event.is_all_day || !(5..=180).contains(&minutes) || !kind.is_travel() {
        return None;
    }
    let duration = Duration::minutes(i64::from(minutes));
    let (start, end) = if kind == Kind::TravelAfter {
        (event.end, event.end + duration)
    } else {
        (event.start - duration, event.start)
    };
    let local_start = start.with_timezone(&Local);
    let local_end = end.with_timezone(&Local);
    if start < now + Duration::minutes(5)
        || local_start.date_naive() != local_end.date_naive()
        || local_start.weekday().num_days_from_monday() >= 5
        || local_start.hour() < 8
        || local_end.time() > chrono::NaiveTime::from_hms_opt(18, 0, 0)?
        || calendar.events.iter().any(|other| {
            other.id != event.id && blocks_time(other) && other.start < end && other.end > start
        })
    {
        return None;
    }
    Some((start, end))
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
