use crate::{attendees::Attendee, calendar::Event, i18n::Language};
use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Added to uncertain outcomes of meetings with attendees: Outlook may already have invited them.
pub const INVITATIONS_MAY_BE_SENT: &str =
    "Invitations may already have been sent; check Outlook before retrying.";

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
    /// Invited with the main event only; omitted when empty so earlier approvals keep their identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<Attendee>,
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
            attendees: Vec::new(),
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
        crate::attendees::validate_list(&self.attendees)?;
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
            attendees: self.attendees.clone(),
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
                    attendees: Vec::new(),
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
    /// Invitees of the main event; travel reserves never have any.
    pub attendees: Vec<Attendee>,
}

impl Part {
    pub fn body(&self) -> Value {
        let utc = |time: DateTime<FixedOffset>| {
            json!({"dateTime":time.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%S").to_string(),
                "timeZone":"UTC"})
        };
        let attendees = self
            .attendees
            .iter()
            .map(|attendee| {
                json!({"emailAddress":{"address":attendee.email,"name":attendee.label()},
                    "type":attendee.role.graph()})
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "subject":self.title, "body":{"contentType":"text","content":self.description},
            "start":utc(self.start), "end":utc(self.end), "showAs":"busy",
            "isAllDay":false, "isReminderOn":false, "attendees":attendees,
            "categories":self.category.iter().collect::<Vec<_>>(),
            "sensitivity":if self.private {"private"} else {"normal"},
            "isOnlineMeeting":self.teams, "transactionId":self.transaction_id,
        });
        if self.teams {
            body["onlineMeetingProvider"] = json!("teamsForBusiness");
        }
        body
    }

    /// Exactly the intended invitees (case-insensitive address) with their intended types.
    /// The organizer's own address may be listed by the provider; nobody else may be added.
    fn attendees_verified(&self, event: &Event) -> bool {
        if self.attendees.is_empty() {
            return event.attendee_count() == 0;
        }
        if event.attendee_count() != event.attendees.len() {
            return false;
        }
        let organizer = crate::calendar::person_email(&event.organizer);
        let roles = event
            .extra
            .get("tomlook_attendee_roles")
            .and_then(Value::as_object);
        let mut actual = std::collections::BTreeMap::new();
        for person in &event.attendees {
            let Some(email) = crate::calendar::person_email(person) else {
                return false;
            };
            if organizer.as_ref() == Some(&email)
                && !self.attendees.iter().any(|attendee| attendee.email == email)
            {
                continue;
            }
            let role = roles
                .and_then(|roles| roles.get(&email))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if actual.insert(email, role).is_some() {
                return false;
            }
        }
        actual.len() == self.attendees.len()
            && self.attendees.iter().all(|attendee| {
                actual.get(&attendee.email) == Some(&attendee.role.graph())
            })
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
            && self.attendees_verified(event)
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
    /// BLOCKERs of a scheduling proposal: one part per proposed time instead of the draft's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
}

/// The proposed times a BLOCKER approval holds in your own calendar.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Hold {
    pub proposal: String,
    pub slots: Vec<crate::scheduling::Slot>,
}

impl Record {
    pub fn new(account: String, draft: Draft) -> Result<Self, String> {
        draft.validate()?;
        Self::build(account, draft)
    }

    /// One busy BLOCKER per proposed time: no attendees, no Teams, no travel, no reminder. The
    /// approval binds the proposal, every time and the exact text.
    pub fn blockers(
        account: String,
        proposal: String,
        draft: Draft,
        slots: Vec<crate::scheduling::Slot>,
    ) -> Result<Self, String> {
        draft.validate()?;
        let first = slots.first().ok_or("Select 1 to 5 times")?;
        if slots.len() > crate::scheduling::MAX_SELECTED
            || slots.windows(2).any(|pair| pair[0] >= pair[1])
            || slots.iter().any(|slot| slot.end <= slot.start || slot.end - slot.start > Duration::hours(4))
            || (first.start, first.end) != (draft.start, draft.end)
            || !draft.title.starts_with(crate::scheduling::BLOCKER_PREFIX)
            || !draft.attendees.is_empty()
            || draft.teams
            || draft.travel_minutes.is_some()
            || proposal.len() != 32
            || !proposal.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("BLOCKER approval does not match its proposal".into());
        }
        let mut record = Self::build(account, draft)?;
        let identity = serde_json::to_vec(&(&record.account, "blockers", &proposal, &slots, &record.draft))
            .map_err(|_| "Encode event creation approval failed")?;
        record.id = format!("{:x}", Sha256::digest(identity));
        record.hold = Some(Hold { proposal, slots });
        Ok(record)
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
            hold: None,
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
        } else if let Some(hold) = &self.hold {
            Self::blockers(self.account.clone(), hold.proposal.clone(), self.draft.clone(), hold.slots.clone())?
        } else {
            Self::new(self.account.clone(), self.draft.clone())?
        };
        if self.id != expected.id || self.draft != expected.draft || self.hold != expected.hold {
            return Err("Calendar account or event creation approval changed".into());
        }
        Ok(())
    }

    pub fn parts(&self) -> Vec<Part> {
        if let Some(hold) = &self.hold {
            return hold
                .slots
                .iter()
                .enumerate()
                .map(|(index, slot)| Part {
                    transaction_id: format!("{}-blocker-{index}", self.id),
                    title: self.draft.title.clone(),
                    description: self.draft.description.clone(),
                    start: slot.start,
                    end: slot.end,
                    category: self.draft.category.clone(),
                    private: self.draft.private,
                    teams: false,
                    attendees: Vec::new(),
                })
                .collect();
        }
        self.draft
            .parts(&self.id)
            .into_iter()
            .skip(usize::from(self.parent.is_some()))
            .collect()
    }

    pub fn label(&self) -> &'static str {
        if self.hold.is_some() {
            return match self.state {
                State::Creating => "Creating BLOCKERs",
                State::Created => "BLOCKERs created",
                State::NotSent => "BLOCKERs not created",
                State::Review => "Check BLOCKERs in Outlook",
            };
        }
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

    /// An outcome that needs review; with attendees it says invitations may already be out.
    pub fn review_message(&self, message: &str) -> String {
        if self.draft.attendees.is_empty() || message.contains(INVITATIONS_MAY_BE_SENT) {
            message.to_owned()
        } else {
            format!("{message}\n{INVITATIONS_MAY_BE_SENT}")
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
            attendees: Vec::new(),
        }
    }

    pub fn invited() -> Draft {
        let mut value = draft();
        value.attendees = crate::attendees::parse_input(
            "Alex Rivers <alex@example.com>; jamie@example.com",
        )
        .unwrap();
        value.attendees[1].role = crate::attendees::Role::Optional;
        value
    }

    /// What Graph returns for a created part: its body as the provider read it back.
    pub fn readback(part: &Part, change: impl FnOnce(&mut Value)) -> Event {
        let mut body = part.body();
        body["id"] = json!("created");
        body["isOrganizer"] = json!(true);
        body["organizer"] = json!({"emailAddress":{"name":"Me","address":"me@example.com"}});
        if part.teams {
            body["onlineMeeting"] = json!({"joinUrl":"https://teams.microsoft.com/l/meetup-join/example"});
        }
        change(&mut body);
        crate::workiq::connection::calendar_write::parse_event_for_tests(body).unwrap()
    }

    #[test]
    fn invitations_go_only_to_the_main_event_and_old_approvals_keep_their_identity() {
        let value = invited();
        value.validate_categories(&["osobní".into()]).unwrap();
        let parts = value.parts("approval");
        assert_eq!(parts[0].body()["attendees"], json!([
            {"emailAddress":{"address":"alex@example.com","name":"Alex Rivers"},"type":"required"},
            {"emailAddress":{"address":"jamie@example.com","name":"jamie@example.com"},"type":"optional"}
        ]));
        for part in &parts[1..] {
            assert!(part.attendees.is_empty());
            assert_eq!(part.body()["attendees"], json!([]), "Travel reserves never invite anyone");
        }
        let plain = Record::new("owner".into(), draft()).unwrap();
        assert!(serde_json::to_value(&plain.draft).unwrap().get("attendees").is_none());
        let with = Record::new("owner".into(), value.clone()).unwrap();
        assert_ne!(plain.id, with.id, "The approval binds the exact invitee set");
        let mut changed = value.clone();
        changed.attendees[1].role = crate::attendees::Role::Required;
        assert_ne!(Record::new("owner".into(), changed).unwrap().id, with.id);
        let restored: Record = serde_json::from_value(serde_json::to_value(&with).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.draft.attendees, value.attendees);
        let mut invalid = value.clone();
        invalid.attendees.push(invalid.attendees[0].clone());
        assert!(invalid.validate().is_err());
        let mut invalid = value;
        invalid.attendees = (0..=crate::attendees::LIMIT)
            .map(|index| crate::attendees::Attendee::new(&format!("p{index}@example.com"), "", Default::default()).unwrap())
            .collect();
        assert!(invalid.validate().is_err());
        assert!(Draft::travel(&parent(), 30, Language::En).unwrap().attendees.is_empty());
    }

    #[test]
    fn readback_requires_the_exact_attendee_set_and_types_and_reserves_stay_attendee_free() {
        let parts = invited().parts("approval");
        let main = &parts[0];
        assert!(main.verified(&readback(main, |_| {})));
        assert!(main.verified(&readback(main, |body| {
            body["attendees"][0]["emailAddress"]["address"] = json!("ALEX@Example.com");
        })), "Addresses compare case-insensitively");
        assert!(main.verified(&readback(main, |body| {
            body["attendees"].as_array_mut().unwrap().push(
                json!({"emailAddress":{"address":"me@example.com","name":"Me"},"type":"required"}));
        })), "The organizer may be listed by the provider");
        for (case, change) in [
            ("missing attendee", Box::new(|body: &mut Value| { body["attendees"].as_array_mut().unwrap().pop(); }) as Box<dyn Fn(&mut Value)>),
            ("extra attendee", Box::new(|body: &mut Value| { body["attendees"].as_array_mut().unwrap().push(
                json!({"emailAddress":{"address":"eve@example.com"},"type":"required"})); })),
            ("type mismatch", Box::new(|body: &mut Value| { body["attendees"][1]["type"] = json!("required"); })),
            ("missing type", Box::new(|body: &mut Value| { body["attendees"][0]["type"] = json!(""); })),
            ("duplicate", Box::new(|body: &mut Value| { let first = body["attendees"][0].clone();
                body["attendees"].as_array_mut().unwrap().push(first); })),
            ("not organizer", Box::new(|body: &mut Value| { body["isOrganizer"] = json!(false); })),
            ("no attendees", Box::new(|body: &mut Value| { body["attendees"] = json!([]); })),
        ] {
            assert!(!main.verified(&readback(main, change)), "{case} must not verify");
        }
        for part in &parts[1..] {
            assert!(part.verified(&readback(part, |_| {})));
            assert!(!part.verified(&readback(part, |body| {
                body["attendees"] = json!([{"emailAddress":{"address":"alex@example.com"},"type":"required"}]);
            })), "Travel reserves must have zero attendees");
        }
        let plain = draft().parts("approval");
        assert!(!plain[0].verified(&readback(&plain[0], |body| {
            body["attendees"] = json!([{"emailAddress":{"address":"alex@example.com"},"type":"required"}]);
        })));
    }

    #[test]
    fn uncertain_outcomes_with_attendees_warn_that_invitations_may_be_out() {
        let record = Record::new("owner".into(), invited()).unwrap();
        let message = record.review_message("Calendar write outcome is uncertain.");
        assert_eq!(message, format!("Calendar write outcome is uncertain.\n{INVITATIONS_MAY_BE_SENT}"));
        assert_eq!(record.review_message(&message), message, "Never repeated");
        let plain = Record::new("owner".into(), draft()).unwrap();
        assert_eq!(plain.review_message("Uncertain."), "Uncertain.");
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
