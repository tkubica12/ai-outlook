use crate::{calendar::Event, i18n::Language};
use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Draft {
    pub title: String,
    pub description: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub category: Option<String>,
    pub private: bool,
    pub teams: bool,
    pub travel_minutes: Option<u16>,
    pub language: Language,
}

pub fn personal_category(categories: &[String]) -> Option<String> {
    categories
        .iter()
        .find(|category| {
            matches!(
                category.trim().to_lowercase().as_str(),
                "osobní" | "osobni" | "personal"
            )
        })
        .cloned()
}

pub fn local_time(day: NaiveDate, minutes: u16) -> Result<DateTime<FixedOffset>, String> {
    zoned_time(Local, day, minutes)
}

fn zoned_time(
    zone: impl TimeZone,
    day: NaiveDate,
    minutes: u16,
) -> Result<DateTime<FixedOffset>, String> {
    let naive = day
        .and_hms_opt(0, 0, 0)
        .and_then(|start| start.checked_add_signed(Duration::minutes(i64::from(minutes))))
        .ok_or("Invalid event date or time")?;
    zone.from_local_datetime(&naive)
        .single()
        .map(|time| time.fixed_offset())
        .ok_or_else(|| {
            "This local time is missing or ambiguous because of daylight saving time".into()
        })
}

impl Draft {
    pub fn travel(event: &Event, minutes: u16, language: Language) -> Result<Self, String> {
        validate_travel_parent(event)?;
        let draft = Self {
            title: event.title.clone(),
            description: String::new(),
            start: event.start,
            end: event.end,
            category: None,
            private: event.extra.get("sensitivity").and_then(Value::as_str) != Some("normal"),
            teams: false,
            travel_minutes: Some(minutes),
            language,
        };
        draft.validate_with_duration(Duration::days(366))?;
        Ok(draft)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_with_duration(Duration::days(1))
    }

    fn validate_with_duration(&self, max_duration: Duration) -> Result<(), String> {
        if self.title.trim().is_empty()
            || self.title.len() > 1000
            || self.title.chars().any(char::is_control)
        {
            return Err("Enter an event title (up to 1000 bytes)".into());
        }
        if self.description.len() > 16_000
            || self
                .description
                .chars()
                .any(|ch| ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t')
        {
            return Err("The description is too long or contains unsupported characters".into());
        }
        if self.end <= self.start || self.end - self.start > max_duration {
            return Err("The event must end after it starts and last at most 24 hours".into());
        }
        if self.category.as_ref().is_some_and(|category| {
            category.trim().is_empty()
                || category.len() > 256
                || category.chars().any(char::is_control)
        }) {
            return Err("Choose an existing category".into());
        }
        if self
            .travel_minutes
            .is_some_and(|minutes| !(15..=180).contains(&minutes) || minutes % 15 != 0)
        {
            return Err("Travel time must be 15 to 180 minutes in 15-minute steps".into());
        }
        Ok(())
    }

    pub fn validate_categories(&self, categories: &[String]) -> Result<(), String> {
        self.validate()?;
        if self
            .category
            .as_ref()
            .is_some_and(|category| !categories.contains(category))
        {
            return Err("Choose an existing category".into());
        }
        Ok(())
    }

    pub fn parts(&self, approval: &str) -> Vec<Part> {
        let mut parts = vec![Part {
            transaction_id: format!("{approval}-event"),
            title: self.title.clone(),
            description: self.description.clone(),
            start: self.start,
            end: self.end,
            category: self.category.clone(),
            private: self.private,
            teams: self.teams,
        }];
        if let Some(minutes) = self.travel_minutes {
            let duration = Duration::minutes(i64::from(minutes));
            for (side, label, start, end) in [
                (
                    "before",
                    "Travel to meeting",
                    self.start - duration,
                    self.start,
                ),
                (
                    "after",
                    "Travel from meeting",
                    self.end,
                    self.end + duration,
                ),
            ] {
                parts.push(Part {
                    transaction_id: format!("{approval}-{side}"),
                    title: self.language.text(label).into_owned(),
                    description: String::new(),
                    start,
                    end,
                    category: None,
                    private: self.private,
                    teams: false,
                });
            }
        }
        parts
    }
}

pub fn validate_travel_parent(event: &Event) -> Result<(), String> {
    crate::recommendations::validate_manual_change(
        event,
        &crate::recommendations::Change::ClearCategory,
    )?;
    if event.is_all_day || event.extra.get("isOrganizer").and_then(Value::as_bool) != Some(true) {
        return Err("Travel blocks require your own timed meeting".into());
    }
    if !matches!(
        event.extra.get("sensitivity").and_then(Value::as_str),
        Some("normal" | "private" | "personal" | "confidential")
    ) {
        return Err("Refresh the calendar first; the meeting's privacy is unavailable".into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Part {
    pub transaction_id: String,
    pub title: String,
    pub description: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub category: Option<String>,
    pub private: bool,
    pub teams: bool,
}

impl Part {
    pub fn body(&self) -> Value {
        let utc = |time: DateTime<FixedOffset>| {
            json!({"dateTime":time.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%S").to_string(),
                "timeZone":"UTC"})
        };
        let mut body = json!({
            "subject":self.title, "body":{"contentType":"text","content":self.description},
            "start":utc(self.start), "end":utc(self.end), "showAs":"busy",
            "isAllDay":false, "isReminderOn":false, "attendees":[],
            "categories":self.category.iter().collect::<Vec<_>>(),
            "sensitivity":if self.private {"private"} else {"normal"},
            "isOnlineMeeting":self.teams, "transactionId":self.transaction_id,
        });
        if self.teams {
            body["onlineMeetingProvider"] = json!("teamsForBusiness");
        }
        body
    }

    pub fn verified(&self, event: &Event) -> bool {
        let extra = &event.extra;
        let description_matches = self.description.trim().is_empty()
            || extra.get("body").is_some_and(|body| {
                let Some(content) = body["content"].as_str() else {
                    return false;
                };
                let text = if body["contentType"]
                    .as_str()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("html"))
                {
                    crate::html_text::extract(content)
                } else {
                    content.to_owned()
                };
                let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
                let expected = normalize(&self.description);
                if self.teams {
                    normalize(&text).contains(&expected)
                } else {
                    normalize(&text) == expected
                }
            });
        event.title == self.title
            && event.start == self.start
            && event.end == self.end
            && event.attendee_count() == 0
            && !event.is_all_day
            && extra.get("isOrganizer") == Some(&json!(true))
            && extra.get("showAs") == Some(&json!("busy"))
            && extra.get("isCancelled") != Some(&json!(true))
            && extra.get("transactionId") == Some(&json!(self.transaction_id))
            && extra.get("sensitivity")
                == Some(&json!(if self.private { "private" } else { "normal" }))
            && crate::recommendations::real_categories(event)
                == self.category.iter().cloned().collect::<Vec<_>>()
            && extra.get("isOnlineMeeting") == Some(&json!(self.teams))
            && (!self.teams
                || (extra.get("onlineMeetingProvider") == Some(&json!("teamsForBusiness"))
                    && extra
                        .get("onlineMeeting")
                        .and_then(|meeting| meeting["joinUrl"].as_str())
                        .is_some_and(crate::calendar::safe_url)))
            && description_matches
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum State {
    Creating,
    Created,
    NotSent,
    Review,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Creating => "Creating event",
            Self::Created => "Event created",
            Self::NotSent => "Event not created",
            Self::Review => "Check event creation in Outlook",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Record {
    pub id: String,
    pub account: String,
    pub draft: Draft,
    pub state: State,
    pub event_ids: Vec<String>,
    pub message: String,
    pub created_at: DateTime<FixedOffset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Event>,
}

impl Record {
    pub fn new(account: String, draft: Draft) -> Result<Self, String> {
        draft.validate()?;
        Self::build(account, draft)
    }

    fn build(account: String, draft: Draft) -> Result<Self, String> {
        if account.trim().is_empty() {
            return Err("Calendar account is unavailable".into());
        }
        // Identical confirmed content is one approval, including across a restart.
        let identity = serde_json::to_vec(&(&account, &draft))
            .map_err(|_| "Encode event creation approval failed")?;
        Ok(Self {
            id: format!("{:x}", Sha256::digest(identity)),
            account,
            draft,
            state: State::Creating,
            event_ids: Vec::new(),
            message: String::new(),
            created_at: Local::now().fixed_offset(),
            parent: None,
        })
    }

    pub fn travel(
        account: String,
        event: Event,
        minutes: u16,
        language: Language,
    ) -> Result<Self, String> {
        let draft = Draft::travel(&event, minutes, language)?;
        let mut record = Self::build(account, draft)?;
        let identity = serde_json::to_vec(&(
            &record.account,
            &event.id,
            crate::scheduler::fingerprint(&event),
            &record.draft,
        ))
        .map_err(|_| "Encode event creation approval failed")?;
        record.id = format!("{:x}", Sha256::digest(identity));
        record.parent = Some(event);
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), String> {
        let expected = if let Some(event) = &self.parent {
            Self::travel(
                self.account.clone(),
                event.clone(),
                self.draft
                    .travel_minutes
                    .ok_or("Travel time is unavailable")?,
                self.draft.language,
            )?
        } else {
            Self::new(self.account.clone(), self.draft.clone())?
        };
        if self.id != expected.id || self.draft != expected.draft {
            return Err("Calendar account or event creation approval changed".into());
        }
        Ok(())
    }

    pub fn parts(&self) -> Vec<Part> {
        self.draft
            .parts(&self.id)
            .into_iter()
            .skip(usize::from(self.parent.is_some()))
            .collect()
    }

    pub fn label(&self) -> &'static str {
        if self.parent.is_none() {
            return self.state.label();
        }
        match self.state {
            State::Creating => "Creating travel blocks",
            State::Created => "Travel blocks created",
            State::NotSent => "Travel blocks not created",
            State::Review => "Check travel blocks in Outlook",
        }
    }
}

pub struct CreationResult {
    pub id: String,
    pub state: State,
    pub events: Vec<Event>,
    pub message: String,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn draft() -> Draft {
        Draft {
            title: "Personal appointment".into(),
            description: "Bring notes".into(),
            start: DateTime::parse_from_rfc3339("2026-11-03T10:00:00+01:00").unwrap(),
            end: DateTime::parse_from_rfc3339("2026-11-03T11:00:00+01:00").unwrap(),
            category: Some("osobní".into()),
            private: true,
            teams: true,
            travel_minutes: Some(30),
            language: Language::En,
        }
    }

    pub fn parent() -> Event {
        let value = draft();
        let mut event = crate::calendar::fixture(1).remove(0);
        event.id = "owned-parent".into();
        event.title = value.title;
        event.start = value.start;
        event.end = value.end;
        event.is_all_day = false;
        event.status = "organizer".into();
        event.extra.insert("isOrganizer".into(), json!(true));
        event.extra.insert("isCancelled".into(), json!(false));
        event.extra.insert("type".into(), json!("singleInstance"));
        event.extra.insert("sensitivity".into(), json!("private"));
        event
    }

    #[test]
    fn existing_meeting_travel_approval_creates_only_two_buffers_and_binds_exact_parent() {
        let event = parent();
        let record = Record::travel("owner".into(), event.clone(), 45, Language::Cs).unwrap();
        record.validate().unwrap();
        let parts = record.parts();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].end, event.start);
        assert_eq!(parts[1].start, event.end);
        assert_eq!(parts[0].end - parts[0].start, Duration::minutes(45));
        assert_eq!(parts[1].end - parts[1].start, Duration::minutes(45));
        assert!(
            parts
                .iter()
                .all(|part| part.private && !part.teams && part.category.is_none())
        );
        assert!(parts.iter().all(|part| part.title != event.title));
        let mut tampered = record.clone();
        tampered.draft.teams = true;
        assert!(tampered.validate().is_err());
        let mut tampered = record.clone();
        tampered.parent.as_mut().unwrap().start += Duration::minutes(15);
        assert!(tampered.validate().is_err());
        assert_ne!(
            record.id,
            Record::new("owner".into(), record.draft.clone())
                .unwrap()
                .id
        );
        for (field, value) in [
            ("isOrganizer", json!(false)),
            ("type", json!("seriesMaster")),
            ("sensitivity", Value::Null),
            ("isCancelled", json!(true)),
        ] {
            let mut invalid = event.clone();
            invalid.extra.insert(field.into(), value);
            assert!(Record::travel("owner".into(), invalid, 30, Language::En).is_err());
        }
        let mut all_day = event;
        all_day.is_all_day = true;
        assert!(Record::travel("owner".into(), all_day, 30, Language::En).is_err());
        for sensitivity in ["normal", "private", "personal", "confidential"] {
            let mut event = parent();
            event.end = event.start + Duration::hours(36);
            event.extra.insert("sensitivity".into(), json!(sensitivity));
            let record = Record::travel("owner".into(), event.clone(), 30, Language::En).unwrap();
            record.validate().unwrap();
            assert_eq!(record.parts()[1].start, event.end);
            assert!(
                record
                    .parts()
                    .iter()
                    .all(|part| part.private == (sensitivity != "normal"))
            );
        }
    }

    #[test]
    fn existing_meeting_travel_approval_is_durable_and_old_creation_records_remain_compatible() {
        let root = tempfile::tempdir().unwrap();
        let mut store = crate::storage::Store::open(root.path()).unwrap();
        let event = parent();
        let calendar = crate::calendar::Calendar::build(
            vec![event.clone()],
            [event.start.date_naive()].into(),
            Vec::new(),
        )
        .unwrap();
        store.save_workiq_calendar(&calendar, "owner").unwrap();
        let record = Record::travel("owner".into(), event, 30, Language::En).unwrap();
        store.approve_event_creation(&record).unwrap();
        drop(store);
        let store = crate::storage::Store::open(root.path()).unwrap();
        let records = store.event_creations().unwrap();
        let recovered = &records[&record.id];
        assert!(recovered.parent.is_some());
        assert_eq!(recovered.state, State::Review);
        assert_eq!(recovered.parts().len(), 2);
        assert!(store.approve_event_creation(&record).is_err());
        let original = Record::new("owner".into(), draft()).unwrap();
        let encoded = serde_json::to_value(&original).unwrap();
        assert!(encoded.get("parent").is_none());
        let old: Record = serde_json::from_value(encoded).unwrap();
        old.validate().unwrap();
        assert_eq!(old.parts().len(), 3);
    }

    #[test]
    fn creation_has_exact_private_main_and_adjacent_adjustable_travel_without_invitees() {
        for minutes in [15, 30, 45, 180] {
            let mut draft = draft();
            draft.travel_minutes = Some(minutes);
            draft.validate_categories(&["osobní".into()]).unwrap();
            let parts = draft.parts("approval");
            assert_eq!(parts.len(), 3);
            assert_eq!(parts[1].end, draft.start);
            assert_eq!(parts[2].start, draft.end);
            for part in &parts[1..] {
                assert_eq!(part.end - part.start, Duration::minutes(i64::from(minutes)));
                assert!(part.private);
                assert!(!part.teams);
                assert_eq!(part.body()["categories"], json!([]));
                assert_eq!(part.body()["attendees"], json!([]));
                assert_eq!(part.body()["sensitivity"], "private");
                assert_eq!(part.body()["showAs"], "busy");
                assert!(part.body().get("onlineMeetingProvider").is_none());
            }
            assert_eq!(parts[0].body()["onlineMeetingProvider"], "teamsForBusiness");
            assert_eq!(parts[0].body()["start"]["dateTime"], "2026-11-03T09:00:00");
            assert_eq!(parts[0].body()["body"]["content"], "Bring notes");
            assert_ne!(parts[0].transaction_id, parts[1].transaction_id);
            assert_ne!(parts[1].transaction_id, parts[2].transaction_id);
        }
        let mut draft = draft();
        draft.travel_minutes = None;
        draft.teams = false;
        draft.private = false;
        draft.category = None;
        assert_eq!(draft.parts("approval").len(), 1);
        assert_eq!(draft.parts("approval")[0].body()["sensitivity"], "normal");
        assert!(
            draft.parts("approval")[0]
                .body()
                .get("onlineMeetingProvider")
                .is_none()
        );
    }

    #[test]
    fn creation_validates_title_duration_category_and_travel_and_selects_personal_category() {
        let mut value = draft();
        value.title.clear();
        assert!(value.validate().is_err());
        value = draft();
        value.end = value.start;
        assert!(value.validate().is_err());
        value.end = value.start + Duration::days(2);
        assert!(value.validate().is_err());
        value = draft();
        for minutes in [0, 14, 16, 181] {
            value.travel_minutes = Some(minutes);
            assert!(value.validate().is_err());
        }
        assert_eq!(
            personal_category(&["call".into(), "Osobní".into()]),
            Some("Osobní".into())
        );
        assert_eq!(
            personal_category(&["PERSONAL".into()]),
            Some("PERSONAL".into())
        );
        assert_eq!(personal_category(&["office".into()]), None);
        assert!(draft().validate_categories(&["office".into()]).is_err());
    }

    #[test]
    fn local_times_reject_dst_gaps_and_ambiguity_and_resolve_midnight_separately() {
        let zone = chrono_tz::Europe::Prague;
        let spring = NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        let autumn = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        assert!(zoned_time(zone, spring, 150).is_err());
        assert!(zoned_time(zone, autumn, 150).is_err());
        let day = NaiveDate::from_ymd_opt(2026, 3, 28).unwrap();
        assert_eq!(zoned_time(zone, day, 1440).unwrap().date_naive(), spring);
        assert_eq!(
            zoned_time(zone, spring, 240).unwrap() - zoned_time(zone, spring, 0).unwrap(),
            Duration::hours(3)
        );
    }

    #[test]
    fn creation_approval_survives_restart_without_replay_and_definitive_rejection_allows_explicit_retry()
     {
        let root = tempfile::tempdir().unwrap();
        let mut store = crate::storage::Store::open(root.path()).unwrap();
        let day = draft().start.date_naive();
        let calendar =
            crate::calendar::Calendar::build(Vec::new(), [day].into(), Vec::new()).unwrap();
        store.save_workiq_calendar(&calendar, "owner").unwrap();
        let mut record = Record::new("owner".into(), draft()).unwrap();
        store.approve_event_creation(&record).unwrap();
        assert!(store.approve_event_creation(&record).is_err());
        drop(store);
        let store = crate::storage::Store::open(root.path()).unwrap();
        let recovered = store.event_creations().unwrap();
        assert_eq!(recovered[&record.id].state, State::Review);
        assert!(store.approve_event_creation(&record).is_err());
        record.state = State::NotSent;
        store.save_event_creation(&record).unwrap();
        record.state = State::Creating;
        store.approve_event_creation(&record).unwrap();
        record.state = State::Created;
        store.save_event_creation(&record).unwrap();
        record.state = State::Creating;
        assert!(store.approve_event_creation(&record).is_err());
        let other = Record::new("other".into(), draft()).unwrap();
        assert!(store.approve_event_creation(&other).is_err());
    }
}
