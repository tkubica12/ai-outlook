use super::*;
use std::time::{Duration, Instant};
use tomlook::{
    attendees::{self, Attendee, Person},
    availability::{self, Availability, MeetingTimes, MeetingTimesRequest, ScheduleRequest, Status},
};

/// Debounce of people search and availability refreshes after typing or changing the day.
const LOOKUP_DELAY: Duration = Duration::from_millis(400);
/// People offered under the attendee field.
const SUGGESTIONS: usize = 6;

/// One debounced, cancellable read for the editor; only the latest request's answer is kept.
pub(super) struct Lookup<T> {
    pub(super) id: u64,
    pub(super) key: String,
    pending: String,
    pub(super) changed: Option<Instant>,
    pub(super) busy: bool,
    pub(super) result: Option<Result<T, String>>,
}

impl<T> Default for Lookup<T> {
    fn default() -> Self {
        Self { id: 0, key: String::new(), pending: String::new(), changed: None, busy: false, result: None }
    }
}

pub(super) enum Due {
    Idle,
    Wait(Duration),
    Send,
}

impl<T> Lookup<T> {
    pub(super) fn poll(&mut self, wanted: &str, now: Instant) -> Due {
        if wanted == self.key {
            self.changed = None;
            return Due::Idle;
        }
        if self.changed.is_none() || self.pending != wanted {
            self.pending = wanted.to_owned();
            self.changed = Some(now);
        }
        let elapsed = self.changed.map_or(LOOKUP_DELAY, |changed| now.saturating_duration_since(changed));
        if elapsed >= LOOKUP_DELAY { Due::Send } else { Due::Wait(LOOKUP_DELAY - elapsed) }
    }

    pub(super) fn start(&mut self, id: u64, key: String) {
        self.id = id;
        self.key = key;
        self.changed = None;
        self.busy = true;
    }

    pub(super) fn finish(&mut self, id: u64, result: Result<T, String>) {
        if id == self.id && self.busy {
            self.busy = false;
            self.result = Some(result);
        }
    }

    pub(super) fn clear(&mut self) {
        self.key.clear();
        self.pending.clear();
        self.changed = None;
        self.busy = false;
        self.result = None;
    }
}

pub(super) struct EventDraft {
    pub(super) parent: Option<Event>,
    account: Option<String>,
    pub(super) day: NaiveDate,
    pub(super) start: u16,
    pub(super) end: u16,
    pub(super) title: String,
    description: String,
    pub(super) category: Option<String>,
    pub(super) private: bool,
    pub(super) teams: bool,
    pub(super) travel: bool,
    pub(super) travel_minutes: u16,
    pub(super) focus_title: bool,
    pub(super) attendees: Vec<Attendee>,
    pub(super) attendee_input: String,
    attendee_message: Option<String>,
    /// People from the cached calendar, ranked once when the attendee field is first used.
    people: Option<Arc<Vec<Person>>>,
    pub(super) directory: Lookup<Vec<Person>>,
    pub(super) availability: Lookup<Arc<Availability>>,
    pub(super) times: Lookup<MeetingTimes>,
    /// Proposal and BLOCKER event whose time this meeting confirms.
    pub(super) proposal: Option<(String, String)>,
    /// **Find a time** was requested with this draft's title, attendees and duration.
    pub(super) find_time: bool,
}

/// The attendee field of the new-event editor or of **Find a time**, borrowed from either.
pub(super) struct AttendeeField<'a> {
    pub(super) attendees: &'a mut Vec<Attendee>,
    pub(super) input: &'a mut String,
    pub(super) message: &'a mut Option<String>,
    pub(super) people: &'a mut Option<Arc<Vec<Person>>>,
    pub(super) directory: &'a mut Lookup<Vec<Person>>,
}

fn selected_times(start: u16, end: u16) -> (u16, u16) {
    let first = start.min(end).min(1425);
    let last = start.max(end).min(1440);
    (first, if last > first { last } else { (first + 30).min(1440) })
}

fn clock(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Local day and 15-minute-step minutes of an interval within one day, if the editor can show it.
pub(super) fn local_interval(start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc>) -> Option<(NaiveDate, u16, u16)> {
    use chrono::Timelike;
    let start = start.with_timezone(&Local);
    let end = end.with_timezone(&Local);
    let minutes = |time: chrono::DateTime<Local>| u16::try_from(time.hour() * 60 + time.minute()).ok();
    let first = minutes(start)?;
    let last = if end.date_naive() == start.date_naive() {
        minutes(end)?
    } else if end.date_naive() == start.date_naive().succ_opt()? && minutes(end)? == 0 {
        1440
    } else {
        return None;
    };
    (first.is_multiple_of(15) && last.is_multiple_of(15) && last > first && end.second() == 0 && start.second() == 0)
        .then_some((start.date_naive(), first, last))
}

impl EventDraft {
    fn attendee_field(&mut self) -> AttendeeField<'_> {
        AttendeeField {
            attendees: &mut self.attendees,
            input: &mut self.attendee_input,
            message: &mut self.attendee_message,
            people: &mut self.people,
            directory: &mut self.directory,
        }
    }

    fn value(&self, language: tomlook::i18n::Language) -> Result<tomlook::event_creation::Draft, String> {
        if let Some(event) = &self.parent {
            return tomlook::event_creation::Draft::travel(event, self.travel_minutes, language);
        }
        let draft = tomlook::event_creation::Draft {
            title: self.title.trim().into(),
            description: self.description.clone(),
            start: tomlook::event_creation::local_time(self.day, self.start)?,
            end: tomlook::event_creation::local_time(self.day, self.end)?,
            category: self.category.clone(),
            private: self.private,
            teams: self.teams,
            travel_minutes: self.travel.then_some(self.travel_minutes),
            language,
            attendees: self.attendees.clone(),
        };
        draft.validate()?;
        Ok(draft)
    }

    fn interval(&self) -> Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
        let start = tomlook::event_creation::local_time(self.day, self.start).ok()?;
        let end = tomlook::event_creation::local_time(self.day, self.end).ok()?;
        Some((start.with_timezone(&chrono::Utc), end.with_timezone(&chrono::Utc)))
    }
}

impl Tomlook {
    pub(super) fn event_creation_record(&mut self, ui: &mut egui::Ui, record: &tomlook::event_creation::Record) {
        use tomlook::event_creation::State;
        let language = self.preferences.language;
        ui.push_id(&record.id, |ui| {
            ui.strong(format!("{}: {}", language.text(record.label()), record.draft.title));
            ui.weak(language.text(&record.message));
            match &record.hold {
                Some(hold) => for slot in &hold.slots { ui.weak(slot.label(language)); },
                None => {
                    ui.weak(format!("{} - {}",
                        record.draft.start.with_timezone(&Local).format("%d.%m.%Y %H:%M:%S%.f %:z"),
                        record.draft.end.with_timezone(&Local).format("%d.%m.%Y %H:%M:%S%.f %:z")));
                }
            }
            if !record.draft.attendees.is_empty() {
                ui.weak(format!("{}: {}", language.text("Invited"), record.draft.attendees.iter()
                    .map(|attendee| attendee.label()).collect::<Vec<_>>().join(", ")));
            }
            if record.state == State::Pending
                || (record.state == State::NotSent && record.parent.is_none() && record.hold.is_none())
            {
                for attendee in &record.draft.attendees {
                    ui.label(format!("{} <{}> ({})", attendee.label(), attendee.email, language.text(attendee.role.label())));
                }
                ui.weak(format!("{}: {}", language.text("Calendar"), record.account));
                if !record.draft.description.is_empty() { ui.label(&record.draft.description); }
                if let Some(category) = &record.draft.category {
                    ui.weak(format!("{}: {category}", language.text("Category")));
                }
                ui.weak(language.text(if record.draft.private { "Private event" } else { "Not private" }));
                if record.draft.teams { ui.label(language.text("Teams meeting")); }
                let first = record.draft.start.with_timezone(&Local).date_naive();
                let last = record.draft.end.with_timezone(&Local).date_naive();
                if (0..=(last - first).num_days()).any(|offset|
                    !self.calendar.coverage.contains(&(first + chrono::Duration::days(offset))))
                {
                    ui.weak(language.text("Not synced; availability has not been verified."));
                }
                let conflicts = self.calendar.events.iter().filter(|event|
                    event.start < record.draft.end && event.end > record.draft.start
                    && event.extra.get("showAs").and_then(serde_json::Value::as_str) != Some("free")
                    && event.extra.get("isCancelled") != Some(&serde_json::json!(true)))
                    .take(3).map(|event| event.title.as_str()).collect::<Vec<_>>();
                if !conflicts.is_empty() {
                    ui.colored_label(ui.visuals().warn_fg_color, format!("{}: {}",
                        language.text("Overlaps saved events"), conflicts.join(", ")));
                }
                if !record.draft.attendees.is_empty() {
                    ui.weak(language.text("Attendee availability has not been verified."));
                    ui.colored_label(ui.visuals().warn_fg_color,
                        RichText::new(attendees::invitation_notice(&record.draft.attendees, language)).strong());
                    if let Some(reason) = self.invitation_blocked() {
                        ui.colored_label(ui.visuals().error_fg_color, language.text(&reason));
                    }
                }
                ui.weak(language.text(if record.state == State::Pending {
                    "AI proposal awaiting your approval; nothing has been changed"
                } else { "Previous attempt; no calendar change was sent." }));
                let ready = self.calendar_owner.as_deref() == Some(record.account.as_str())
                    && self.workiq_ready && self.ai.is_some() && !self.preview
                    && (record.draft.attendees.is_empty() || self.invitation_blocked().is_none())
                    && record.draft.start > Local::now().fixed_offset()
                    && !self.activity.event_creations.values().any(|entry| entry.state == State::Creating);
                if record.draft.start <= Local::now().fixed_offset() {
                    ui.colored_label(ui.visuals().error_fg_color,
                        language.text("The proposed event starts in the past; choose a future time"));
                }
                ui.horizontal_wrapped(|ui| {
                    let label = if record.draft.attendees.is_empty() { "Approve and create event" } else { "Create and send invitations" };
                    if ui.add_enabled(ready, egui::Button::new(language.text(label))).clicked() {
                        self.command(Command::CreateEvents {
                            account: record.account.clone(), draft: Box::new(record.draft.clone()), parent: None,
                        });
                    }
                    if ui.button(language.text("Dismiss event proposal")).clicked() {
                        self.command(Command::DismissEventCreation { id: record.id.clone(), account: record.account.clone() });
                    }
                });
            }
            ui.add_space(8.0);
        });
    }

    pub(super) fn open_event_draft(&mut self, day: NaiveDate, start: u16, end: u16) {
        self.event_draft = Some(EventDraft {
            parent: None,
            account: self.calendar_owner.clone(), day, start, end,
            title: String::new(), description: String::new(), category: None,
            private: false, teams: false, travel: false, travel_minutes: 30, focus_title: true,
            attendees: Vec::new(), attendee_input: String::new(), attendee_message: None, people: None,
            directory: Lookup::default(), availability: Lookup::default(), times: Lookup::default(),
            proposal: None, find_time: false,
        });
    }

    pub(super) fn open_travel_draft(&mut self, event: Event) {
        self.open_event_draft(event.start.with_timezone(&Local).date_naive(), 0, 15);
        if let Some(draft) = &mut self.event_draft {
            draft.parent = Some(event);
            draft.travel = true;
            draft.focus_title = false;
        }
    }

    /// The user's own addresses, never offered or accepted as attendees.
    pub(super) fn invite_own(&self) -> Vec<String> {
        let mut own = self.own_emails().into_iter().map(|email| email.to_lowercase()).collect::<Vec<_>>();
        if self.preview {
            own.push(tomlook::showcase::DEMO_USER.into());
        }
        own
    }

    /// The address whose free/busy is your own row in the scheduling grid.
    pub(super) fn schedule_owner(&self) -> Option<String> {
        if self.preview {
            return Some(tomlook::showcase::DEMO_USER.into());
        }
        self.activity.identity.as_ref()
            .filter(|identity| self.calendar_owner.as_deref().is_some_and(|owner| identity.account.eq_ignore_ascii_case(owner)))
            .and_then(|identity| identity.mail.clone())
            .or_else(|| self.calendar_owner.clone())
            .and_then(|email| person_email(&email))
    }

    pub(super) fn next_lookup(&mut self) -> u64 {
        self.lookup_sequence = self.lookup_sequence.wrapping_add(1).max(1);
        self.lookup_sequence
    }

    /// Queues a read-only editor lookup without reporting a full queue as an AI error.
    pub(super) fn lookup_command(&self, command: ai::Command) -> bool {
        self.ai.as_ref().is_some_and(|engine| engine.commands.try_send(command).is_ok())
    }

    /// Why attendees cannot be invited on this connection: only the provider's definitive answer
    /// that new events cannot carry attendees blocks them; the write re-checks either way.
    fn invitation_blocked(&self) -> Option<String> {
        match &self.invitation_capability {
            Some((account, Some(Err(error))))
                if self.calendar_owner.as_deref() == Some(account.as_str())
                    && error == tomlook::workiq::connection::calendar_write::INVITATIONS_UNSUPPORTED => Some(error.clone()),
            _ => None,
        }
    }

    /// A failed (not definitive) invitation capability check, shown without blocking.
    fn invitation_check_failed(&self) -> Option<String> {
        match &self.invitation_capability {
            Some((account, Some(Err(error)))) if self.calendar_owner.as_deref() == Some(account.as_str())
                && self.invitation_blocked().is_none() => Some(error.clone()),
            _ => None,
        }
    }

    pub(super) fn receive_people(&mut self, id: u64, result: Result<Vec<Person>, String>) {
        if let Some(draft) = &mut self.event_draft { draft.directory.finish(id, result.clone()); }
        if let Some(find) = &mut self.find_time { find.directory.finish(id, result); }
    }

    pub(super) fn receive_availability(&mut self, id: u64, result: Result<Arc<Availability>, String>) {
        if let Some(draft) = &mut self.event_draft { draft.availability.finish(id, result); }
    }

    pub(super) fn receive_meeting_times(&mut self, id: u64, result: Result<MeetingTimes, String>) {
        if let Some(draft) = &mut self.event_draft { draft.times.finish(id, result); }
    }

    pub(super) fn receive_invitation_capability(&mut self, account: String, result: Result<(), String>) {
        if matches!(&self.invitation_capability, Some((pending, None)) if *pending == account) {
            self.invitation_capability = Some((account, Some(result)));
        }
    }

    /// Starts debounced people search and availability reads; the offline demo answers locally.
    fn poll_editor_lookups(&mut self, context: &egui::Context, draft: &mut EventDraft) {
        let now = Instant::now();
        let connected = self.workiq_ready || self.preview;
        if !self.preview && self.workiq_ready
            && (!draft.attendees.is_empty() || !draft.attendee_input.trim().is_empty())
            && let Some(account) = self.calendar_owner.clone()
            && self.invitation_capability.as_ref().is_none_or(|(checked, _)| *checked != account)
            && self.lookup_command(ai::Command::InvitationCapability(account.clone()))
        {
            self.invitation_capability = Some((account, None));
        }
        self.poll_directory(context, &mut draft.attendee_field());
        let emails = self.schedule_owner().into_iter()
            .chain(draft.attendees.iter().map(|attendee| attendee.email.clone()))
            .fold(Vec::new(), |mut emails, email| { if !emails.contains(&email) { emails.push(email); } emails });
        let wanted = if connected && !draft.attendees.is_empty() {
            format!("{}|{}", draft.day, emails.join(","))
        } else {
            String::new()
        };
        if wanted.is_empty() {
            draft.availability.clear();
            return;
        }
        match draft.availability.poll(&wanted, now) {
            Due::Idle => {}
            Due::Wait(remaining) => context.request_repaint_after(remaining),
            Due::Send => {
                let id = self.next_lookup();
                draft.availability.start(id, wanted);
                match ScheduleRequest::day(draft.day, emails) {
                    Err(error) => draft.availability.finish(id, Err(error)),
                    Ok(request) if self.preview => draft.availability.finish(id,
                        Ok(Arc::new(tomlook::showcase::availability(&request, &self.calendar.events)))),
                    Ok(request) => if !self.lookup_command(ai::Command::Availability { id, request: Box::new(request) }) {
                        draft.availability.finish(id, Err("Availability lookup could not be queued; try again".into()));
                    },
                }
            }
        }
    }

    fn request_meeting_times(&mut self, draft: &mut EventDraft) {
        let id = self.next_lookup();
        draft.times.start(id, format!("{}|{}|{}", draft.day, draft.start, draft.end));
        let request = MeetingTimesRequest::around(draft.day, draft.end.saturating_sub(draft.start),
            draft.attendees.clone(), chrono::Utc::now());
        match request {
            Err(error) => draft.times.finish(id, Err(error)),
            Ok(request) if self.preview => draft.times.finish(id, Ok(tomlook::showcase::meeting_times(&request, &self.calendar.events))),
            Ok(request) => if !self.lookup_command(ai::Command::MeetingTimes { id, request: Box::new(request) }) {
                draft.times.finish(id, Err("Meeting time suggestions could not be queued; try again".into()));
            },
        }
    }

    /// Debounced, superseding read-only people search for an attendee field.
    pub(super) fn poll_directory(&mut self, context: &egui::Context, field: &mut AttendeeField<'_>) {
        let connected = self.workiq_ready || self.preview;
        let query = field.input.trim();
        let wanted = if connected && !query.contains([';', ',', '\n']) && attendees::people_resource(query).is_ok() {
            query.to_owned()
        } else {
            String::new()
        };
        if wanted.is_empty() {
            field.directory.clear();
            return;
        }
        match field.directory.poll(&wanted, Instant::now()) {
            Due::Idle => {}
            Due::Wait(remaining) => context.request_repaint_after(remaining),
            Due::Send => {
                let id = self.next_lookup();
                field.directory.start(id, wanted.clone());
                if self.preview {
                    field.directory.finish(id, Ok(tomlook::showcase::people(&wanted)));
                } else if !self.lookup_command(ai::Command::People { id, query: wanted, own: self.invite_own() }) {
                    field.directory.finish(id, Err("People search could not be queued; try again".into()));
                }
            }
        }
    }

    fn attendee_suggestions(&self, field: &mut AttendeeField<'_>) -> Vec<Person> {
        let query = field.input.trim();
        if query.is_empty() || query.contains([';', ',', '\n']) {
            return Vec::new();
        }
        let own = self.invite_own();
        let people = field.people.get_or_insert_with(|| {
            Arc::new(attendees::from_calendar(&self.calendar.events, &own, Local::now().fixed_offset()))
        }).clone();
        let mut found = attendees::matching(&people, query, field.attendees, SUGGESTIONS)
            .into_iter().cloned().collect::<Vec<_>>();
        if let Some(Ok(directory)) = &field.directory.result && field.directory.key == query {
            for person in directory {
                if found.len() >= SUGGESTIONS { break; }
                if !found.iter().any(|known| known.email == person.email)
                    && !field.attendees.iter().any(|attendee| attendee.email == person.email)
                    && !own.contains(&person.email) {
                    found.push(person.clone());
                }
            }
        }
        found
    }

    pub(super) fn add_attendees(&self, field: &mut AttendeeField<'_>, candidates: Result<Vec<Attendee>, String>) {
        let result = candidates.and_then(|candidates| attendees::add(field.attendees, candidates, &self.invite_own()));
        *field.message = match result {
            Ok(outcome) => {
                if outcome.added > 0 { field.input.clear(); }
                if outcome.own > 0 {
                    Some("You are the organizer and are not invited".into())
                } else if outcome.added == 0 && outcome.duplicates > 0 {
                    field.input.clear();
                    Some("Already invited".into())
                } else {
                    None
                }
            }
            Err(error) => Some(error),
        };
    }

    /// Attendee chips with a Required/Optional toggle and remove button, the name or e-mail input
    /// and suggestions from the cached calendar and the directory.
    pub(super) fn attendee_picker(&mut self, ui: &mut egui::Ui, field: &mut AttendeeField<'_>, input_id: &str, label: egui::Id, enabled: bool) {
        let language = self.preferences.language;
        let mut remove = None;
        let mut toggle = None;
        if !field.attendees.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (index, attendee) in field.attendees.iter().enumerate() {
                    egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(attendee.label()).on_hover_text(&attendee.email);
                                if ui.small_button(language.text(attendee.role.label()))
                                    .on_hover_text(language.text("Switch between required and optional")).clicked() {
                                    toggle = Some(index);
                                }
                                if ui.small_button("×").on_hover_text(language.text("Remove attendee")).clicked() {
                                    remove = Some(index);
                                }
                            });
                        });
                }
            });
        }
        if let Some(index) = toggle && let Some(attendee) = field.attendees.get_mut(index) {
            attendee.role = attendee.role.toggled();
        }
        if let Some(index) = remove && index < field.attendees.len() {
            field.attendees.remove(index);
            *field.message = None;
        }
        let enabled = enabled && field.attendees.len() < attendees::LIMIT;
        let mut submit = false;
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 80.0).max(120.0);
            let input = ui.add_enabled(enabled, egui::TextEdit::singleline(field.input)
                .id(Id::new(input_id)).hint_text(language.text("Name or e-mail"))
                .desired_width(width).char_limit(4000)).labelled_by(label);
            if input.changed() { *field.message = None; }
            if input.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                submit = true;
                input.request_focus();
            }
            if ui.add_enabled(enabled && !field.input.trim().is_empty(), egui::Button::new(language.text("Add"))).clicked() {
                submit = true;
            }
        });
        let suggestions = if enabled { self.attendee_suggestions(field) } else { Vec::new() };
        let mut chosen = None;
        for person in &suggestions {
            let text = if person.name.is_empty() { person.email.clone() } else { format!("{} · {}", person.name, person.email) };
            if ui.add(egui::Button::new(text).frame(false).truncate()).clicked() {
                chosen = Some(person.clone());
            }
        }
        if enabled && field.directory.busy {
            ui.horizontal(|ui| { ui.spinner(); ui.weak(language.text("Searching the directory...")); });
        } else if enabled && let Some(Err(error)) = &field.directory.result && !field.input.trim().is_empty() {
            ui.weak(format!("{}: {}", language.text("Directory search is unavailable"), language.text(error)));
        }
        if let Some(person) = chosen {
            self.add_attendees(field, person.attendee().map(|attendee| vec![attendee]));
        } else if submit {
            let parsed = attendees::parse_input(field.input);
            let parsed = match (parsed, suggestions.first()) {
                (Err(_), Some(person)) => person.attendee().map(|attendee| vec![attendee]),
                (parsed, _) => parsed,
            };
            self.add_attendees(field, parsed);
        }
        if let Some(message) = field.message.as_ref() {
            ui.colored_label(ui.visuals().warn_fg_color, language.text(message));
        }
    }

    fn attendee_section(&mut self, ui: &mut egui::Ui, draft: &mut EventDraft) {
        let language = self.preferences.language;
        ui.add_space(6.0);
        let label = ui.label(language.text("Attendees"));
        let blocked = self.invitation_blocked();
        self.attendee_picker(ui, &mut draft.attendee_field(), "new-event-attendee", label.id, blocked.is_none());
        if let Some(reason) = &blocked {
            ui.colored_label(ui.visuals().warn_fg_color,
                format!("{}: {}", language.text("Invitations are unavailable"), language.text(reason)));
        } else if let Some(error) = self.invitation_check_failed() && !draft.attendees.is_empty() {
            ui.weak(format!("{}: {}", language.text("Invitation support could not be checked"), language.text(&error)));
        }
        if !draft.attendees.is_empty() {
            self.availability_section(ui, draft);
        }
    }

    fn availability_section(&mut self, ui: &mut egui::Ui, draft: &mut EventDraft) {
        let language = self.preferences.language;
        ui.add_space(4.0);
        if !(self.workiq_ready || self.preview) {
            ui.weak(language.text("Connect WorkIQ to see attendee availability."));
            return;
        }
        if draft.availability.busy || (draft.availability.result.is_none() && draft.availability.changed.is_some()) {
            ui.horizontal(|ui| { ui.spinner(); ui.weak(language.text("Checking availability...")); });
        }
        match draft.availability.result.clone() {
            Some(Ok(availability)) => {
                if let Some(start) = self.availability_grid(ui, draft, &availability) {
                    let duration = draft.end.saturating_sub(draft.start);
                    if start + duration <= 1440 {
                        draft.start = start;
                        draft.end = start + duration;
                    }
                }
                self.availability_legend(ui);
                if let Some((start, end)) = draft.interval() {
                    let you = self.schedule_owner().map(|email| availability.status(&email, start, end));
                    let people = draft.attendees.iter()
                        .map(|attendee| (attendee.label(), availability.status(&attendee.email, start, end)))
                        .collect::<Vec<_>>();
                    let all = you.is_none_or(Status::available) && people.iter().all(|(_, status)| status.available());
                    let summary = availability::summary(you, &people, language);
                    if all {
                        ui.label(RichText::new(summary).strong());
                    } else {
                        ui.colored_label(ui.visuals().warn_fg_color, summary);
                    }
                }
            }
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().warn_fg_color,
                    format!("{}: {}", language.text("Availability could not be checked"), language.text(&error)));
                ui.weak(language.text("You can still create the event; availability is not verified."));
            }
            None => {}
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!draft.times.busy, egui::Button::new(language.text("Suggest times")))
                .on_hover_text(language.text("Find times around this day when attendees are free")).clicked() {
                self.request_meeting_times(draft);
            }
            if draft.times.busy { ui.spinner(); }
            if draft.proposal.is_none() && ui.button(language.text("Find a time"))
                .on_hover_text(language.text("Search several days for times that suit the whole group")).clicked() {
                draft.find_time = true;
            }
        });
        let mut chosen = None;
        match &draft.times.result {
            Some(Ok(times)) => {
                if let Some(reason) = times.empty_reason.filter(|_| times.suggestions.is_empty()) {
                    ui.weak(language.text(reason));
                }
                for suggestion in &times.suggestions {
                    let Some((day, start, end)) = local_interval(suggestion.start, suggestion.end) else { continue };
                    let mut text = format!("{} {}–{} · {}", language.date(day, false), clock(start), clock(end),
                        language.text(&format!("{}/{} available", suggestion.available(), suggestion.attendees.len())));
                    if let Some(confidence) = suggestion.confidence {
                        text.push_str(&format!(" · {confidence:.0} %"));
                    }
                    if ui.button(text).clicked() {
                        chosen = Some((day, start, end));
                    }
                }
            }
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().warn_fg_color,
                    format!("{}: {}", language.text("Meeting times could not be suggested"), language.text(error)));
            }
            None => {}
        }
        if let Some((day, start, end)) = chosen {
            draft.day = day;
            draft.start = start;
            draft.end = end;
        }
    }

    pub(super) fn paint_status(&self, painter: &egui::Painter, rect: egui::Rect, status: Status, visuals: &egui::Visuals) {
        let accent = self.accent();
        let hatch = |stroke: Stroke| {
            let hatch = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
            let mut x = rect.left() - rect.height();
            while x < rect.right() {
                hatch.line_segment([egui::pos2(x, rect.bottom()), egui::pos2(x + rect.height(), rect.top())], stroke);
                x += 4.0;
            }
        };
        match status {
            Status::Free => {}
            Status::Busy => { painter.rect_filled(rect, 0, accent); }
            Status::Tentative => {
                // Outlook's convention: tentative time is striped in the busy color.
                painter.rect_filled(rect, 0, accent.gamma_multiply(0.25));
                hatch(Stroke::new(1.5, accent.gamma_multiply(0.9)));
            }
            Status::OutOfOffice => {
                let [r, g, b] = if self.preferences.dark { [170, 142, 242] } else { [100, 65, 187] };
                painter.rect_filled(rect, 0, Color32::from_rgb(r, g, b));
            }
            Status::WorkingElsewhere => {
                let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - rect.height() / 3.0), rect.max);
                painter.rect_filled(bar, 0, accent.gamma_multiply(0.75));
            }
            Status::Unknown => hatch(Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.7))),
        }
    }

    pub(super) fn availability_legend(&self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        ui.horizontal_wrapped(|ui| {
            for status in [Status::Busy, Status::Tentative, Status::OutOfOffice, Status::WorkingElsewhere, Status::Free, Status::Unknown] {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(11.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2, ui.visuals().faint_bg_color);
                ui.painter().rect_stroke(rect, 2, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
                self.paint_status(ui.painter(), rect.shrink(1.0), status, ui.visuals());
                ui.small(language.text(status.label()));
                ui.add_space(4.0);
            }
        });
    }

    /// Rows are you and each attendee; columns are 15- or 30-minute cells from 08:00 to 18:00,
    /// extended to the selected interval. Returns the start of a clicked column.
    fn availability_grid(&self, ui: &mut egui::Ui, draft: &EventDraft, availability: &Availability) -> Option<u16> {
        let language = self.preferences.language;
        let mut rows = Vec::new();
        if let Some(own) = self.schedule_owner() {
            rows.push((language.text("You").into_owned(), own));
        }
        rows.extend(draft.attendees.iter().map(|attendee| (attendee.label().to_owned(), attendee.email.clone())));
        let first = (draft.start / 60).min(8);
        let last = draft.end.div_ceil(60).clamp(18, 24);
        let span = (last - first) * 60;
        let cell: u16 = if span <= 12 * 60 { 15 } else { 30 };
        let columns = usize::from(span / cell);
        let (label_width, row_height, gap, header) = (112.0, 16.0, 2.0, 14.0);
        let width = ui.available_width().max(label_width + 160.0);
        let grid_height = header + rows.len() as f32 * (row_height + gap);
        let height = grid_height.min(header + 10.0 * (row_height + gap));
        let output = egui::ScrollArea::vertical().id_salt("availability-grid-rows").max_height(height)
            .auto_shrink([false, true]).show(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, grid_height), egui::Sense::click());
            let cell_width = (width - label_width) / columns as f32;
            let minute = |column: usize| first * 60 + u16::try_from(column).unwrap_or(u16::MAX).saturating_mul(cell);
            let times = (0..=columns)
                .map(|column| tomlook::event_creation::local_time(draft.day, minute(column)).ok()
                    .map(|time| time.with_timezone(&chrono::Utc)))
                .collect::<Vec<_>>();
            let painter = ui.painter_at(rect);
            let visuals = ui.visuals();
            let left = rect.left() + label_width;
            let font = egui::FontId::proportional(10.0);
            let every = if cell_width * 60.0 / f32::from(cell) >= 22.0 { 1 } else { 2 };
            for hour in first..last {
                let x = left + f32::from((hour - first) * 60 / cell) * cell_width;
                if hour.is_multiple_of(every) {
                    painter.text(egui::pos2(x + 1.0, rect.top()), egui::Align2::LEFT_TOP, format!("{hour:02}"), font.clone(), visuals.weak_text_color());
                }
            }
            for (index, (label, email)) in rows.iter().enumerate() {
                let top = rect.top() + header + index as f32 * (row_height + gap);
                let label_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), top), Vec2::new(label_width - 6.0, row_height));
                painter.with_clip_rect(label_rect).text(label_rect.left_center(), egui::Align2::LEFT_CENTER, label,
                    egui::FontId::proportional(12.0), if index == 0 { visuals.strong_text_color() } else { visuals.text_color() });
                let track = egui::Rect::from_min_size(egui::pos2(left, top), Vec2::new(width - label_width, row_height));
                painter.rect_filled(track, 2, visuals.faint_bg_color);
                let schedule = availability.schedule(email);
                for column in 0..columns {
                    let status = match (schedule, times[column], times[column + 1]) {
                        (Some(schedule), Some(from), Some(to)) => availability.schedule_status(schedule, from, to),
                        _ => Status::Unknown,
                    };
                    let cell_rect = egui::Rect::from_min_size(egui::pos2(left + column as f32 * cell_width, top),
                        Vec2::new(cell_width, row_height)).shrink2(Vec2::new(0.5, 0.0));
                    self.paint_status(&painter, cell_rect, status, visuals);
                }
            }
            for hour in first..=last {
                let x = left + f32::from((hour - first) * 60 / cell) * cell_width;
                painter.line_segment([egui::pos2(x, rect.top() + header - 2.0), egui::pos2(x, rect.bottom())],
                    Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color));
            }
            let to_x = |minutes: u16| left + f32::from(minutes.clamp(first * 60, last * 60) - first * 60) / f32::from(cell) * cell_width;
            let selection = egui::Rect::from_min_max(egui::pos2(to_x(draft.start), rect.top() + header - 2.0),
                egui::pos2(to_x(draft.end), rect.bottom()));
            painter.rect_stroke(selection, 2, Stroke::new(2.0, visuals.strong_text_color()), egui::StrokeKind::Inside);
            let cell_at = |position: egui::Pos2| {
                let column = ((position.x - left) / cell_width).floor();
                let row = ((position.y - rect.top() - header) / (row_height + gap)).floor();
                (column >= 0.0 && (column as usize) < columns && row >= 0.0 && (row as usize) < rows.len())
                    .then_some((row as usize, column as usize))
            };
            let mut response = response;
            if let Some((row, column)) = response.hover_pos().and_then(cell_at) {
                let (label, email) = &rows[row];
                let mut text = format!("{label} · {}–{}", clock(minute(column)), clock(minute(column + 1)));
                match (availability.schedule(email), times[column], times[column + 1]) {
                    (Some(schedule), Some(from), Some(to)) => {
                        let status = availability.schedule_status(schedule, from, to);
                        let items = Availability::items(schedule, from, to).take(4).collect::<Vec<_>>();
                        if let Some(error) = &schedule.error {
                            text.push_str(&format!("\n{}: {}", language.text("Unknown"), language.text(error)));
                        } else if items.iter().any(|item| item.subject.is_some() || item.location.is_some()) {
                            for item in items {
                                let details = [item.subject.as_deref(), item.location.as_deref()].into_iter().flatten().collect::<Vec<_>>();
                                text.push_str(&format!("\n{}{}", language.text(item.status.label()),
                                    if details.is_empty() { String::new() } else { format!(": {}", details.join(" · ")) }));
                            }
                        } else {
                            text.push_str(&format!("\n{}", language.text(status.label())));
                        }
                    }
                    _ => text.push_str(&format!("\n{}", language.text("Unknown"))),
                }
                text.push_str(&format!("\n{}", language.text("Click to start the meeting here")));
                response = response.on_hover_text_at_pointer(text);
            }
            response.clicked().then(|| response.interact_pointer_pos()).flatten()
                .and_then(cell_at).map(|(_, column)| minute(column))
        });
        output.inner
    }

    pub(super) fn event_creation_dialog(&mut self, context: &egui::Context) {
        let Some(mut draft) = self.event_draft.take() else { return };
        let language = self.preferences.language;
        let categories = tomlook::recommendations::known_categories(&self.calendar).into_iter()
            .filter(|category| !category.eq_ignore_ascii_case("follow")).collect::<Vec<_>>();
        let mut open = true;
        let mut cancel = false;
        let mut confirm = false;
        let travel_only = draft.parent.is_some();
        if !travel_only {
            self.poll_editor_lookups(context, &mut draft);
        }
        let body_height = (context.content_rect().height() - 230.0).max(260.0);
        egui::Window::new(language.text(if travel_only { "Add travel blocks before and after" } else { "New event" }))
            .id(Id::new("create-calendar-event")).open(&mut open)
            .default_width(500.0).resizable(false).collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                egui::ScrollArea::vertical().id_salt("new-event-body").max_height(body_height).show(ui, |ui| {
                if let Some(parent) = &draft.parent {
                    ui.strong(&parent.title);
                    ui.weak(meeting_date_label(parent, language));
                    ui.weak(language.text("The existing meeting stays unchanged; only two travel blocks are created."));
                    if parent.extra.get("sensitivity").and_then(serde_json::Value::as_str) != Some("normal") {
                        ui.weak(language.text("Private event"));
                    }
                } else {
                ui.strong(language.date(draft.day, true));
                let label = ui.label(language.text("Event title"));
                let title = ui.add(egui::TextEdit::singleline(&mut draft.title)
                    .id(Id::new("new-event-title")).desired_width(f32::INFINITY).char_limit(500))
                    .labelled_by(label.id);
                if draft.focus_title { title.request_focus(); draft.focus_title = false; }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(language.text("Start"));
                    ui.strong(clock(draft.start));
                });
                if ui.add(egui::Slider::new(&mut draft.start, 0..=1425).step_by(15.0).show_value(false)
                    .text(language.text("Start"))).changed() && draft.end <= draft.start {
                    draft.end = (draft.start + 30).min(1440);
                }
                ui.horizontal(|ui| {
                    ui.label(language.text("End"));
                    ui.strong(clock(draft.end));
                });
                ui.add(egui::Slider::new(&mut draft.end, 15..=1440).step_by(15.0).show_value(false)
                    .text(language.text("End")));
                self.attendee_section(ui, &mut draft);
                let label = ui.label(language.text("Description (optional)"));
                ui.add(egui::TextEdit::multiline(&mut draft.description).desired_rows(2)
                    .desired_width(f32::INFINITY).char_limit(8000)).labelled_by(label.id);
                ui.label(language.text("Category"));
                egui::Grid::new("new-event-categories").num_columns(3).spacing([6.0, 6.0]).show(ui, |ui| {
                    for (index, category) in std::iter::once(None).chain(categories.iter().map(Some)).enumerate() {
                        let label = category.map_or_else(|| language.text("None").into_owned(), Clone::clone);
                        let selected = draft.category.as_ref() == category;
                        let button = self.category_button(ui, label, category.map(String::as_str), selected);
                        if ui.add_sized([142.0, 30.0], button).on_hover_text(category.map_or_else(|| language.text("None").into_owned(), Clone::clone)).clicked() {
                            draft.category = category.cloned();
                        }
                        if index % 3 == 2 { ui.end_row(); }
                    }
                });
                    if ui.checkbox(&mut draft.private, language.text("Private event")).changed() && draft.private
                        && let Some(personal) = tomlook::event_creation::personal_category(&categories) {
                            draft.category = Some(personal);
                    }
                if draft.private {
                    ui.weak(language.text("Private in Outlook; delegates allowed to view private items may still see details."));
                    if tomlook::event_creation::personal_category(&categories).is_none() {
                        ui.weak(language.text("No personal category was found; choose a category above."));
                    }
                }
                ui.checkbox(&mut draft.teams, language.text("Teams meeting"));
                ui.checkbox(&mut draft.travel, language.text("Add travel blocks before and after"));
                }
                if draft.travel {
                    ui.add(egui::Slider::new(&mut draft.travel_minutes, 15..=180)
                        .step_by(15.0).suffix(" min").text(language.text("Each travel block")));
                    if let Ok(event) = draft.value(language) {
                        for part in event.parts("preview").iter().skip(1) {
                            ui.weak(format!("{}: {} - {}", part.title,
                                part.start.with_timezone(&Local).format("%d.%m. %H:%M"),
                                part.end.with_timezone(&Local).format("%d.%m. %H:%M")));
                        }
                    }
                }
                let value = draft.value(language);
                if let Ok(value) = &value {
                    let parts = value.parts("preview").into_iter().skip(usize::from(travel_only)).collect::<Vec<_>>();
                    let start = parts.iter().map(|part| part.start).min().expect("Main event exists");
                    let end = parts.iter().map(|part| part.end).max().expect("Main event exists");
                    let first = start.with_timezone(&Local).date_naive();
                    let last = end.with_timezone(&Local).date_naive();
                    let mut conflicts = BTreeSet::new();
                    for offset in 0..=(last - first).num_days() {
                        let date = first + chrono::Duration::days(offset);
                        if !self.calendar.coverage.contains(&date) {
                            ui.weak(language.text("Not synced; availability has not been verified."));
                        }
                        if let Some(day) = self.calendar.days.get(&date) {
                            for index in &day.events {
                                let event = &self.calendar.events[*index];
                                if draft.parent.as_ref().is_none_or(|parent| event.id != parent.id)
                                    && parts.iter().any(|part| event.start < part.end && event.end > part.start)
                                    && event.extra.get("showAs").and_then(serde_json::Value::as_str) != Some("free")
                                    && event.extra.get("isCancelled") != Some(&serde_json::json!(true)) {
                                    conflicts.insert(event.title.as_str());
                                }
                            }
                        }
                    }
                    if !conflicts.is_empty() {
                        ui.colored_label(ui.visuals().warn_fg_color, format!("{}: {}",
                            language.text("Overlaps saved events"), conflicts.into_iter().take(3).collect::<Vec<_>>().join(", ")));
                    }
                }
                });
                let value = draft.value(language);
                let unchanged = draft.account.is_some() && draft.account == self.calendar_owner
                    && draft.parent.as_ref().is_none_or(|event| {
                        self.calendar.event_index(&event.id).is_some_and(|index|
                            tomlook::scheduler::fingerprint(&self.calendar.events[index]) == tomlook::scheduler::fingerprint(event))
                    });
                if !unchanged && travel_only {
                    ui.weak(language.text("Meeting or account changed; close this dialog and review the current meeting."));
                }
                let invitations = !draft.attendees.is_empty();
                let ready = unchanged && self.workiq_ready && self.ai.is_some() && !self.preview
                    && !(invitations && self.invitation_blocked().is_some())
                    && !self.activity.event_creations.values().any(|record| record.state == tomlook::event_creation::State::Creating);
                if !ready { ui.weak(language.text("Connect WorkIQ before confirming a calendar change")); }
                if let Err(error) = &value && (travel_only || !draft.title.trim().is_empty()) {
                    ui.colored_label(ui.visuals().error_fg_color, language.text(error));
                }
                if invitations {
                    ui.colored_label(ui.visuals().warn_fg_color,
                        RichText::new(attendees::invitation_notice(&draft.attendees, language)).strong());
                }
                if draft.proposal.is_some() {
                    ui.weak(language.text("After the invitation is verified, Tomlook offers to delete the BLOCKERs of this proposal."));
                }
                ui.horizontal(|ui| {
                    let label = if travel_only { "Create travel blocks" } else if invitations { "Create and send invitations" } else { "Create event" };
                    confirm = ui.add_enabled(ready && value.is_ok(), egui::Button::new(language.text(label))).clicked();
                    cancel = ui.button(language.text("Back")).clicked();
                });
            });
        if confirm {
            if let (Some(account), Ok(value)) = (draft.account.clone(), draft.value(language)) {
                let record = draft.proposal.as_ref().and_then(|_| tomlook::event_creation::Record::new(account.clone(), value.clone()).ok());
                let submitted = self.worker.as_ref().is_some_and(|worker| worker.commands.try_send(Command::CreateEvents {
                    account, draft: Box::new(value), parent: draft.parent.clone().map(Box::new),
                }).is_ok());
                if submitted {
                    self.messages_open = true;
                    // A confirmed proposal time offers to release its BLOCKERs once the invitation is verified.
                    if let (Some((proposal, _)), Some(record)) = (draft.proposal.clone(), record) {
                        self.proposal_confirmation = Some((proposal, record.id));
                    }
                } else {
                    self.error = Some("Event creation could not be queued; nothing was submitted".into());
                    self.event_draft = Some(draft);
                }
            }
        } else if draft.find_time {
            let minutes = draft.end.saturating_sub(draft.start);
            self.open_find_time(draft.title.trim(), draft.attendees.clone(), minutes);
        } else if open && !cancel {
            self.event_draft = Some(draft);
        }
    }

    /// The `invite` demo scene: a new fictional meeting with three attendees, ready once its
    /// fictional availability is shown.
    pub(super) fn demo_invitation_ready(&mut self) -> bool {
        if self.event_draft.is_none() {
            use chrono::Datelike;
            let mut day = Local::now().date_naive() + chrono::Duration::days(1);
            while day.weekday().num_days_from_monday() >= 5 {
                day += chrono::Duration::days(1);
            }
            let (start, minutes) = ScheduleRequest::day(day, vec![tomlook::showcase::DEMO_USER.into()]).ok()
                .and_then(|request| {
                    let own = tomlook::availability::Schedule::from_events(&request, tomlook::showcase::DEMO_USER, &self.calendar.events);
                    let free = Availability { start: request.start, end: request.end, interval: request.interval, schedules: vec![own] };
                    [45_u16, 30].into_iter().find_map(|minutes| (52_u16..64).map(|quarter| quarter * 15).find(|start| {
                        let from = tomlook::event_creation::local_time(day, *start).ok();
                        let to = tomlook::event_creation::local_time(day, start + minutes).ok();
                        from.zip(to).is_some_and(|(from, to)| free.status(tomlook::showcase::DEMO_USER,
                            from.with_timezone(&chrono::Utc), to.with_timezone(&chrono::Utc)) == Status::Free)
                    }).map(|start| (start, minutes)))
                })
                .unwrap_or((840, 30));
            self.open_event_draft(day, start, start + minutes);
            let own = self.invite_own();
            if let Some(draft) = &mut self.event_draft {
                draft.title = "Northstar / Pilot kickoff".into();
                draft.teams = true;
                draft.focus_title = false;
                if let Ok(people) = attendees::parse_input(tomlook::showcase::INVITE_ATTENDEES) {
                    let _ = attendees::add(&mut draft.attendees, people, &own);
                }
            }
        }
        self.event_draft.as_ref().is_some_and(|draft| matches!(draft.availability.result, Some(Ok(_))))
    }

    pub(super) fn select_event_time(&mut self, ui: &mut egui::Ui, day: NaiveDate, rect: egui::Rect, cards: &[egui::Rect]) {
        if self.event_draft.is_some() || self.meeting_action.is_some() || self.meeting_menu.is_some()
            || self.find_time.is_some() || self.blocker_release.is_some()
            || self.messages_open || self.settings || self.preferences_open || self.connection_panel.is_some() || self.palette {
            return;
        }
        let (focused, events) = ui.input(|input| (input.focused, input.events.clone()));
        if !focused { self.time_drag = None; return; }
        let minute = |position: egui::Pos2, round: bool| {
            let value = (position.y - rect.top()) / 58.0 * 60.0 / 15.0;
            ((if round { value.round() } else { value.floor() }) as i32 * 15).clamp(0, if round { 1440 } else { 1425 }) as u16
        };
        for event in events {
            match event {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. }
                    if rect.contains(pos) && ui.clip_rect().contains(pos)
                        && ui.rect_contains_pointer(rect) && !cards.iter().any(|card| card.contains(pos)) => {
                    let start = minute(pos, false);
                    self.time_drag = Some((day, start, start));
                }
                egui::Event::PointerMoved(pos) => {
                    if let Some((selected_day, _, end)) = &mut self.time_drag && *selected_day == day {
                        *end = minute(pos, true);
                    }
                }
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } => {
                    if let Some((selected_day, start, _)) = self.time_drag && selected_day == day {
                        self.time_drag = None;
                        let (first, last) = selected_times(start, minute(pos, true));
                        self.open_event_draft(day, first, last);
                    }
                }
                egui::Event::PointerGone => { self.time_drag = None; }
                _ => {}
            }
        }
        if let Some((selected_day, start, end)) = self.time_drag && selected_day == day {
            let (start, end) = selected_times(start, end);
            let selection = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + f32::from(start) / 60.0 * 58.0),
                egui::pos2(rect.right(), rect.top() + f32::from(end) / 60.0 * 58.0),
            );
            ui.painter().rect_filled(selection, 4, self.accent().gamma_multiply(0.25));
            ui.painter().rect_stroke(selection, 4, Stroke::new(1.0, self.accent()), egui::StrokeKind::Inside);
        }
    }
}
