use super::*;
use super::event_editor::{AttendeeField, Due, Lookup};
use std::time::Instant;
use tomlook::{
    attendees::{self, Attendee, Person, Role},
    availability::Availability,
    scheduling::{self, BlockerState, Candidates, ChatCheck, ChatDraft, ChatState, Proposal, ProposalStatus, Search, Slot},
};

/// Why every write of **Find a time** is unavailable in the offline demo.
const DEMO_WRITES: &str = "The offline demo never changes a calendar or contacts anyone.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Choose,
    Hold,
    Chat,
    Email,
}

/// Candidates of the availability answer with this ID for this search and these attendees.
type Computed = (u64, Search, Vec<Attendee>, Result<Candidates, String>);

/// The **Find a time** dialog: topic, attendees, search, candidates and the proposal actions.
pub(super) struct FindTime {
    pub(super) topic: String,
    pub(super) attendees: Vec<Attendee>,
    pub(super) attendee_input: String,
    attendee_message: Option<String>,
    people: Option<Arc<Vec<Person>>>,
    pub(super) directory: Lookup<Vec<Person>>,
    pub(super) search: Search,
    pub(super) availability: Lookup<Arc<Availability>>,
    computed: Option<Computed>,
    pub(super) selected: Vec<Slot>,
    pub(super) step: Step,
    /// The saved proposal once BLOCKERs or a Teams proposal were approved; the times are fixed.
    pub(super) proposal: Option<Proposal>,
    pub(super) private: bool,
    pub(super) category: Option<String>,
    pub(super) chat_check: Lookup<ChatCheck>,
    pub(super) message: String,
    pub(super) email_subject: String,
    pub(super) email_body: String,
    pub(super) email_excluded: BTreeSet<String>,
    pub(super) status: Option<String>,
    focus_topic: bool,
    /// Scrolls the opened action panel into view once its content is shown.
    scroll_to_step: bool,
    /// When BLOCKERs or a Teams proposal were last submitted; their button stays disabled until
    /// the storage worker shows the approval (or briefly, if it rejected it with an error).
    submitted: Option<Instant>,
}

impl FindTime {
    fn attendee_field(&mut self) -> AttendeeField<'_> {
        AttendeeField {
            attendees: &mut self.attendees,
            input: &mut self.attendee_input,
            message: &mut self.attendee_message,
            people: &mut self.people,
            directory: &mut self.directory,
        }
    }

    pub(super) fn candidates(&self) -> Option<&Result<Candidates, String>> {
        self.computed.as_ref().map(|(_, _, _, result)| result)
    }

    fn locked(&self) -> bool {
        self.proposal.is_some()
    }

    /// A submission is still on its way to the storage worker.
    fn awaiting(&self) -> bool {
        self.submitted.is_some_and(|submitted| submitted.elapsed() < std::time::Duration::from_secs(5))
    }
}

/// An approved deletion of a proposal's held BLOCKERs, before confirmation.
pub(super) struct ReleaseDraft {
    pub(super) proposal: String,
    pub(super) events: Vec<String>,
    /// Closes the proposal as cancelled; otherwise its meeting was scheduled.
    pub(super) cancel: bool,
}

impl Tomlook {
    /// Opens **Find a time** with an optional topic, attendees and duration.
    pub(super) fn open_find_time(&mut self, topic: &str, attendees: Vec<Attendee>, minutes: u16) {
        let minutes = (minutes / 15 * 15).clamp(scheduling::MIN_MINUTES, scheduling::MAX_MINUTES);
        self.event_draft = None;
        self.meeting_menu = None;
        self.find_time = Some(FindTime {
            topic: topic.chars().take(scheduling::TOPIC_LIMIT).collect(),
            attendees,
            attendee_input: String::new(),
            attendee_message: None,
            people: None,
            directory: Lookup::default(),
            search: Search::new(Local::now().date_naive(), minutes),
            availability: Lookup::default(),
            computed: None,
            selected: Vec::new(),
            step: Step::Choose,
            proposal: None,
            private: false,
            category: None,
            chat_check: Lookup::default(),
            message: String::new(),
            email_subject: String::new(),
            email_body: String::new(),
            email_excluded: BTreeSet::new(),
            status: None,
            focus_topic: topic.trim().is_empty(),
            scroll_to_step: false,
            submitted: None,
        });
    }

    /// "Find another time with these people": the meeting's attendees and organizer except you,
    /// with their roles, its title and duration.
    pub(super) fn open_find_time_for(&mut self, event: &Event) {
        let own = self.invite_own();
        let roles = event.extra.get("tomlook_attendee_roles").and_then(serde_json::Value::as_object);
        let mut list = Vec::new();
        for person in std::iter::once(&event.organizer).chain(event.attendees.iter()) {
            if list.len() >= attendees::LIMIT {
                break;
            }
            let Some(email) = person_email(person) else { continue };
            let role = roles.and_then(|roles| roles.get(&email)).and_then(serde_json::Value::as_str);
            if role == Some("resource") {
                continue;
            }
            let name = person.rsplit_once('<').map_or("", |(name, _)| name);
            if let Ok(mut attendee) = Attendee::new(&email, name, Role::Required) {
                if role == Some("optional") {
                    attendee.role = Role::Optional;
                }
                // Duplicates and your own mailboxes are skipped.
                let _ = attendees::add(&mut list, vec![attendee], &own);
            }
        }
        let minutes = u16::try_from((event.end - event.start).num_minutes()).unwrap_or(scheduling::MAX_MINUTES);
        self.open_find_time(&event.title, list, minutes);
    }

    /// Whether a meeting has someone besides you to find another time with.
    pub(super) fn has_other_people(&self, event: &Event) -> bool {
        let own = self.invite_own();
        !event.is_all_day && std::iter::once(&event.organizer).chain(event.attendees.iter())
            .filter_map(|person| person_email(person))
            .any(|email| !own.iter().any(|own| own.eq_ignore_ascii_case(&email)))
    }

    /// The open proposal and BLOCKER of a Tomlook BLOCKER event.
    pub(super) fn blocker_proposal(&self, event_id: &str) -> Option<(&Proposal, &scheduling::Blocker)> {
        let proposal = self.activity.proposals.get(self.activity.blockers.get(event_id)?)?;
        let blocker = proposal.blockers.iter().find(|blocker| blocker.event_id.as_deref() == Some(event_id))?;
        Some((proposal, blocker))
    }

    /// "Confirm this time": the new-event editor with the proposal's topic and attendees at this
    /// BLOCKER's time.
    pub(super) fn confirm_blocker(&mut self, event: &Event) {
        let Some((proposal, _)) = self.blocker_proposal(&event.id) else { return };
        let (id, topic, attendees, private) = (proposal.id.clone(), proposal.topic.clone(), proposal.attendees.clone(),
            event.extra.get("sensitivity").and_then(serde_json::Value::as_str) == Some("private"));
        let Some((day, start, end)) = super::event_editor::local_interval(event.start.to_utc(), event.end.to_utc()) else {
            self.error = Some("This BLOCKER's time cannot be edited here; schedule the meeting in Outlook".into());
            return;
        };
        self.meeting_menu = None;
        self.open_event_draft(day, start, end);
        if let Some(draft) = &mut self.event_draft {
            draft.title = topic;
            draft.attendees = attendees;
            draft.private = private;
            draft.focus_title = false;
            draft.proposal = Some((id, event.id.clone()));
        }
    }

    /// Asks to delete every held BLOCKER of a proposal.
    pub(super) fn open_release(&mut self, proposal: &str, cancel: bool) {
        let Some(saved) = self.activity.proposals.get(proposal) else { return };
        self.meeting_menu = None;
        self.blocker_release = Some(ReleaseDraft {
            proposal: proposal.into(),
            events: saved.held().filter_map(|blocker| blocker.event_id.clone()).collect(),
            cancel,
        });
    }

    pub(super) fn receive_group_availability(&mut self, id: u64, result: Result<Arc<Availability>, String>) {
        if let Some(find) = &mut self.find_time { find.availability.finish(id, result); }
    }

    pub(super) fn receive_chat_check(&mut self, id: u64, result: Result<ChatCheck, String>) {
        if let Some(find) = &mut self.find_time {
            find.chat_check.finish(id, result);
            find.scroll_to_step = true;
        }
    }

    /// Writes need a connected, non-demo calendar owner and no event creation in progress.
    fn proposal_writes(&self) -> Result<String, &'static str> {
        if self.preview {
            return Err(DEMO_WRITES);
        }
        let account = self.calendar_owner.clone().filter(|_| self.workiq_ready && self.ai.is_some())
            .ok_or("Connect WorkIQ before confirming a calendar change")?;
        if self.activity.event_creations.values().any(|record| record.state == tomlook::event_creation::State::Creating) {
            return Err("Wait for the current event creation to finish");
        }
        Ok(account)
    }

    fn proposal_account(&self) -> String {
        self.calendar_owner.clone().unwrap_or_else(|| tomlook::feedback::PREVIEW_ACCOUNT.into())
    }

    /// The fixed proposal of this dialog, or a new one for the selected times.
    fn proposal_of(&self, find: &FindTime) -> Result<Proposal, String> {
        if let Some(proposal) = &find.proposal {
            return Ok(self.activity.proposals.get(&proposal.id).cloned().unwrap_or_else(|| proposal.clone()));
        }
        scheduling::validate_topic(&find.topic)?;
        Proposal::new(&self.proposal_account(), &find.topic, find.attendees.clone(), find.search.minutes,
            find.selected.clone(), self.preferences.language, Local::now().fixed_offset())
    }

    /// Debounced, cancellable read-only free/busy over the searched days and the Teams check.
    fn poll_find_time(&mut self, context: &egui::Context, find: &mut FindTime) {
        if !find.locked() {
            self.poll_directory(context, &mut find.attendee_field());
        }
        let connected = self.workiq_ready || self.preview;
        let emails = self.schedule_owner().into_iter()
            .chain(find.attendees.iter().map(|attendee| attendee.email.clone()))
            .fold(Vec::new(), |mut emails, email| { if !emails.contains(&email) { emails.push(email); } emails });
        let dates = find.search.dates();
        let wanted = match (dates.first(), dates.last()) {
            (Some(first), Some(last)) if connected && !find.attendees.is_empty() && find.search.validate().is_ok() =>
                format!("{first}|{last}|{}", emails.join(",")),
            _ => String::new(),
        };
        if wanted.is_empty() {
            find.availability.clear();
            find.computed = None;
        } else {
            match find.availability.poll(&wanted, Instant::now()) {
                Due::Idle => {}
                Due::Wait(remaining) => context.request_repaint_after(remaining),
                Due::Send => {
                    let id = self.next_lookup();
                    find.availability.start(id, wanted);
                    match find.search.schedule_request(emails) {
                        Err(error) => find.availability.finish(id, Err(error)),
                        Ok(request) if self.preview => find.availability.finish(id,
                            Ok(Arc::new(tomlook::showcase::group_availability(&request, &self.calendar.events)))),
                        Ok(request) => if !self.lookup_command(ai::Command::GroupAvailability { id, request: Box::new(request) }) {
                            find.availability.finish(id, Err("Availability lookup could not be queued; try again".into()));
                        },
                    }
                }
            }
        }
        // Candidates are computed locally once per answer, search and attendee set.
        if let Some(result) = &find.availability.result
            && !find.availability.busy
            && find.computed.as_ref().is_none_or(|(id, search, attendees, _)|
                *id != find.availability.id || *search != find.search || *attendees != find.attendees)
        {
            let candidates = match (result, self.schedule_owner()) {
                (Ok(availability), Some(own)) => scheduling::candidates(&find.search, availability, &own,
                    &find.attendees, chrono::Utc::now()),
                (Ok(_), None) => Err("Your own availability is unknown; connect WorkIQ again".into()),
                (Err(error), _) => Err(error.clone()),
            };
            if !find.locked() {
                if let Ok(candidates) = &candidates {
                    find.selected.retain(|slot| candidates.list.iter().any(|candidate| candidate.slot() == *slot));
                } else {
                    find.selected.clear();
                }
            }
            find.computed = Some((find.availability.id, find.search.clone(), find.attendees.clone(), candidates));
        }
        if find.step == Step::Chat && connected {
            let emails = find.attendees.iter().map(|attendee| attendee.email.clone()).collect::<Vec<_>>();
            let key = emails.join(",");
            if find.chat_check.key != key && !find.chat_check.busy {
                let id = self.next_lookup();
                find.chat_check.start(id, key);
                find.message.clear();
                find.scroll_to_step = true;
                if self.preview {
                    find.chat_check.finish(id, Ok(tomlook::showcase::chat_check(&emails)));
                } else if let Some(account) = self.calendar_owner.clone() {
                    if !self.lookup_command(ai::Command::ChatCheck { id, account, emails }) {
                        find.chat_check.finish(id, Err("The Teams check could not be queued; try again".into()));
                    }
                } else {
                    find.chat_check.finish(id, Err("Connect WorkIQ to propose in Teams".into()));
                }
            }
        }
    }

    pub(super) fn find_time_dialog(&mut self, context: &egui::Context) {
        let Some(mut find) = self.find_time.take() else { return };
        let language = self.preferences.language;
        self.poll_find_time(context, &mut find);
        let mut open = true;
        let mut close = false;
        let body_height = (context.content_rect().height() - 160.0).max(320.0);
        egui::Window::new(language.text("Find a time"))
            .id(Id::new("find-a-time")).open(&mut open)
            // A scrolling body never grows a window beyond its default height; allow the screen.
            .default_size([640.0, body_height + 40.0]).resizable(false).collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                egui::ScrollArea::vertical().id_salt("find-a-time-body").max_height(body_height).show(ui, |ui| {
                    self.find_time_inputs(ui, &mut find);
                    ui.separator();
                    self.find_time_candidates(ui, &mut find);
                    ui.separator();
                    close = self.find_time_actions(ui, &mut find);
                    match find.step {
                        Step::Choose => {}
                        Step::Hold => self.hold_step(ui, &mut find),
                        Step::Chat => self.chat_step(ui, &mut find),
                        Step::Email => self.email_step(ui, &mut find),
                    }
                    if find.scroll_to_step && find.step != Step::Choose {
                        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                        find.scroll_to_step = false;
                    }
                });
            });
        if open && !close && self.find_time.is_none() {
            self.find_time = Some(find);
        }
    }

    fn find_time_inputs(&mut self, ui: &mut egui::Ui, find: &mut FindTime) {
        let language = self.preferences.language;
        let locked = find.locked();
        let label = ui.label(language.text("Topic"));
        let topic = ui.add_enabled(!locked, egui::TextEdit::singleline(&mut find.topic)
            .id(Id::new("find-time-topic")).desired_width(f32::INFINITY).char_limit(scheduling::TOPIC_LIMIT))
            .labelled_by(label.id);
        if find.focus_topic {
            topic.request_focus();
            find.focus_topic = false;
        }
        ui.add_space(4.0);
        let label = ui.label(language.text("Attendees"));
        let mut field = find.attendee_field();
        self.attendee_picker(ui, &mut field, "find-time-attendee", label.id, !locked);
        ui.add_space(4.0);
        ui.add_enabled_ui(!locked, |ui| {
            egui::Grid::new("find-time-search").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(language.text("Duration"));
                ui.add(egui::Slider::new(&mut find.search.minutes, scheduling::MIN_MINUTES..=scheduling::MAX_MINUTES)
                    .step_by(15.0).suffix(" min"));
                ui.end_row();
                ui.label(language.text("From"));
                ui.horizontal(|ui| {
                    let today = Local::now().date_naive();
                    if ui.add_enabled(find.search.first > today, egui::Button::new("‹"))
                        .on_hover_text(language.text("Previous day")).clicked() {
                        find.search.first = find.search.first.pred_opt().unwrap_or(find.search.first);
                    }
                    ui.label(language.date(find.search.first, true));
                    if ui.button("›").on_hover_text(language.text("Next day")).clicked() {
                        find.search.first = find.search.first.succ_opt().unwrap_or(find.search.first);
                    }
                });
                ui.end_row();
                ui.label(language.text(if find.search.weekdays_only { "Workdays" } else { "Days" }));
                ui.add(egui::Slider::new(&mut find.search.days, 1..=scheduling::MAX_DAYS));
                ui.end_row();
                ui.label(language.text("Daily window"));
                ui.horizontal(|ui| {
                    let (mut from, mut to) = find.search.window;
                    ui.add(egui::Slider::new(&mut from, 0..=1425).step_by(15.0).show_value(false));
                    ui.strong(format!("{:02}:{:02}–{:02}:{:02}", from / 60, from % 60, to / 60, to % 60));
                    ui.add(egui::Slider::new(&mut to, 15..=1440).step_by(15.0).show_value(false));
                    if to <= from { to = (from + 15).min(1440); }
                    find.search.window = (from, to);
                });
                ui.end_row();
                ui.label(language.text("Show at least"));
                ui.add(egui::Slider::new(&mut find.search.minimum, 1..=scheduling::MAX_CANDIDATES as u8));
                ui.end_row();
            });
            ui.checkbox(&mut find.search.weekdays_only, language.text("Weekdays only"));
        });
        if let Err(error) = find.search.validate() {
            ui.colored_label(ui.visuals().warn_fg_color, language.text(&error));
        }
    }

    fn find_time_candidates(&mut self, ui: &mut egui::Ui, find: &mut FindTime) {
        let language = self.preferences.language;
        if find.attendees.is_empty() {
            ui.weak(language.text("Add attendees to find a time."));
            return;
        }
        if !(self.workiq_ready || self.preview) {
            ui.weak(language.text("Connect WorkIQ to see attendee availability."));
            return;
        }
        if find.availability.busy || (find.availability.result.is_none() && find.availability.changed.is_some()) {
            ui.horizontal(|ui| { ui.spinner(); ui.weak(language.text("Checking availability...")); });
            return;
        }
        let Some(result) = find.candidates().cloned() else { return };
        let candidates = match result {
            Ok(candidates) => candidates,
            Err(error) => {
                ui.colored_label(ui.visuals().warn_fg_color,
                    format!("{}: {}", language.text("Availability could not be checked"), language.text(&error)));
                return;
            }
        };
        if candidates.list.is_empty() {
            ui.weak(language.text("No free time was found in this range; try more days or a longer daily window."));
            return;
        }
        ui.strong(language.text("Candidates"));
        if candidates.ideal == 0 {
            ui.colored_label(ui.visuals().warn_fg_color,
                language.text("No time works for every required attendee; these are the best alternatives."));
        }
        let own = self.schedule_owner();
        let locked = find.locked();
        let mut toggled = None;
        for candidate in &candidates.list {
            let slot = candidate.slot();
            let mut checked = find.selected.contains(&slot);
            ui.horizontal(|ui| {
                let enabled = !locked && (checked || find.selected.len() < scheduling::MAX_SELECTED);
                if ui.add_enabled(enabled, egui::Checkbox::new(&mut checked, scheduling::slot_label(candidate.start, candidate.end, language)))
                    .changed() {
                    toggled = Some(slot);
                }
                let people = own.iter().map(|_| (language.text("You").into_owned(), candidate.you))
                    .chain(candidate.attendees.iter().map(|(email, status)| {
                        let label = find.attendees.iter().find(|attendee| attendee.email == *email)
                            .map_or(email.as_str(), Attendee::label).to_owned();
                        (label, *status)
                    })).collect::<Vec<_>>();
                for (name, status) in people {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(11.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2, ui.visuals().faint_bg_color);
                    ui.painter().rect_stroke(rect, 2, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
                    self.paint_status(ui.painter(), rect.shrink(1.0), status, ui.visuals());
                    response.on_hover_text(format!("{name}: {}", language.text(status.label())));
                }
                let reason = candidate.reason(&find.attendees, language);
                if candidate.all_required {
                    ui.weak(reason);
                } else {
                    ui.colored_label(ui.visuals().warn_fg_color, reason);
                }
            });
        }
        if let Some(slot) = toggled
            && let Err(error) = scheduling::toggle(&mut find.selected, slot) {
            find.status = Some(error);
        }
        self.availability_legend(ui);
        ui.weak(language.text("Select up to 5 times. Unknown availability is never treated as free."));
    }

    /// The action buttons; `true` closes the dialog.
    fn find_time_actions(&mut self, ui: &mut egui::Ui, find: &mut FindTime) -> bool {
        let language = self.preferences.language;
        let count = find.selected.len();
        let topic = scheduling::validate_topic(&find.topic);
        let mut close = false;
        ui.horizontal_wrapped(|ui| {
            let one = count == 1 && topic.is_ok();
            if ui.add_enabled(one, egui::Button::new(language.text("Create meeting")))
                .on_disabled_hover_text(language.text("Select exactly one time"))
                .on_hover_text(language.text("Opens the new event with these attendees and this time"))
                .clicked()
                && let Some(slot) = find.selected.first()
                && let Some((day, start, end)) = super::event_editor::local_interval(slot.start.to_utc(), slot.end.to_utc())
            {
                let (title, people) = (find.topic.trim().to_owned(), find.attendees.clone());
                self.open_event_draft(day, start, end);
                if let Some(draft) = &mut self.event_draft {
                    draft.title = title;
                    draft.attendees = people;
                    draft.focus_title = false;
                }
                close = true;
            }
            let some = count > 0 && topic.is_ok();
            for (step, label) in [
                (Step::Hold, "Hold with BLOCKERs"),
                (Step::Chat, "Propose in a Teams group chat"),
                (Step::Email, "Propose by e-mail"),
            ] {
                if ui.add_enabled(some, egui::Button::new(language.text(label)).selected(find.step == step))
                    .on_disabled_hover_text(language.text("Enter a topic and select 1 to 5 times"))
                    .clicked() {
                    find.step = if find.step == step { Step::Choose } else { step };
                    find.scroll_to_step = true;
                    find.status = None;
                    if step == Step::Email {
                        find.email_subject.clear();
                        find.email_body.clear();
                    }
                }
            }
        });
        if let Err(error) = &topic && count > 0 {
            ui.weak(language.text(error));
        }
        if let Some(status) = &find.status {
            ui.colored_label(self.accent(), language.text(status));
        }
        close
    }

    /// Busy cached events overlapping a proposed time.
    fn slot_conflicts(&self, slot: &Slot) -> Vec<String> {
        self.calendar.events.iter()
            .filter(|event| !event.is_all_day && event.start < slot.end && event.end > slot.start
                && event.extra.get("showAs").and_then(serde_json::Value::as_str) != Some("free")
                && event.extra.get("isCancelled") != Some(&serde_json::json!(true))
                && !event.unconfirmed())
            .map(|event| event.title.clone())
            .take(3)
            .collect()
    }

    fn hold_step(&mut self, ui: &mut egui::Ui, find: &mut FindTime) {
        let language = self.preferences.language;
        ui.add_space(6.0);
        let proposal = match self.proposal_of(find) {
            Ok(proposal) => proposal,
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, language.text(&error));
                return;
            }
        };
        if !proposal.blockers.is_empty() {
            self.proposal_progress(ui, &proposal);
            return;
        }
        let count = proposal.candidates.len();
        ui.colored_label(ui.visuals().warn_fg_color, RichText::new(scheduling::hold_notice(count, language)).strong());
        for slot in &proposal.candidates {
            ui.label(format!("{} · {}", scheduling::blocker_title(&proposal.topic), slot.label(language)));
            let conflicts = self.slot_conflicts(slot);
            if !conflicts.is_empty() {
                ui.colored_label(ui.visuals().warn_fg_color, format!("{}: {}", language.text("Overlaps saved events"), conflicts.join(", ")));
            }
            if !self.calendar.coverage.contains(&slot.start.with_timezone(&Local).date_naive()) {
                ui.weak(language.text("Not synced; availability has not been verified."));
            }
        }
        ui.checkbox(&mut find.private, language.text("Private event"));
        let categories = tomlook::recommendations::known_categories(&self.calendar).into_iter()
            .filter(|category| !category.eq_ignore_ascii_case("follow")).collect::<Vec<_>>();
        egui::ComboBox::from_label(language.text("Category"))
            .selected_text(find.category.clone().unwrap_or_else(|| language.text("None").into_owned()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut find.category, None, language.text("None"));
                for category in categories {
                    ui.selectable_value(&mut find.category, Some(category.clone()), category);
                }
            });
        ui.weak(language.text("The description lists the topic, the invitees and every proposed time. Nobody is invited."));
        let ready = self.proposal_writes();
        if let Err(reason) = ready {
            ui.weak(language.text(reason));
        }
        let awaiting = find.awaiting();
        if awaiting {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        if ui.add_enabled(ready.is_ok() && !awaiting, egui::Button::new(scheduling::hold_button(count, language))).clicked()
            && let Ok(account) = ready
        {
            let sent = self.worker.as_ref().is_some_and(|worker| worker.commands.try_send(Command::HoldTimes {
                account, proposal: Box::new(proposal.clone()), private: find.private, category: find.category.clone(),
            }).is_ok());
            if sent {
                find.proposal = Some(proposal);
                find.submitted = Some(Instant::now());
                find.status = Some("BLOCKERs are being created; nothing else is sent.".into());
            } else {
                find.status = Some("Event creation could not be queued; nothing was submitted".into());
            }
        }
    }

    /// Progress and outcomes of a saved proposal's BLOCKERs and Teams message.
    fn proposal_progress(&mut self, ui: &mut egui::Ui, proposal: &Proposal) {
        let language = self.preferences.language;
        for blocker in &proposal.blockers {
            let slot = Slot { start: blocker.start, end: blocker.end };
            ui.horizontal_wrapped(|ui| {
                if blocker.state == BlockerState::Creating || blocker.state == BlockerState::Releasing { ui.spinner(); }
                ui.label(format!("{} · {}", slot.label(language), language.text(blocker.state.label())));
            });
            if !blocker.message.is_empty() && blocker.state != BlockerState::Held {
                ui.weak(language.text(&blocker.message));
            }
        }
        let held = proposal.held().count();
        if held > 0 && held == proposal.candidates.len() && proposal.chat.is_none() {
            ui.colored_label(self.accent(), language.text("Your time is protected. Next, propose these times in Teams or by e-mail."));
        }
    }

    fn chat_step(&mut self, ui: &mut egui::Ui, find: &mut FindTime) {
        let language = self.preferences.language;
        ui.add_space(6.0);
        let proposal = match self.proposal_of(find) {
            Ok(proposal) => proposal,
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, language.text(&error));
                return;
            }
        };
        if let Some(chat) = &proposal.chat {
            ui.horizontal(|ui| {
                if chat.state == ChatState::Sending { ui.spinner(); }
                ui.strong(language.text(chat.state.label()));
            });
            if !chat.message.is_empty() { ui.weak(language.text(&chat.message)); }
            if let Some(url) = &chat.web_url
                && ui.button(language.text("Open the chat in Teams")).clicked()
                && let Err(error) = tomlook::browser::open(url) {
                self.error = Some(error);
            }
            if chat.state != ChatState::NotSent || chat.chat_id.is_some() {
                return;
            }
        }
        if !(self.workiq_ready || self.preview) {
            ui.weak(language.text("Connect WorkIQ to propose in Teams."));
            return;
        }
        if find.chat_check.busy {
            ui.horizontal(|ui| { ui.spinner(); ui.weak(language.text("Checking Teams and the directory...")); });
            return;
        }
        let check = match &find.chat_check.result {
            Some(Ok(check)) => check.clone(),
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().warn_fg_color,
                    format!("{}: {}", language.text("Teams proposals are unavailable"), language.text(error)));
                ui.weak(language.text("Use Propose by e-mail instead."));
                return;
            }
            None => return,
        };
        if !check.unresolved.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, language.text("These people cannot be added to a Teams chat:"));
            for (email, reason) in &check.unresolved {
                ui.weak(format!("{email}: {}", language.text(reason)));
            }
            ui.weak(language.text("Nothing is sent to a reduced group. Use Propose by e-mail instead."));
            return;
        }
        if find.message.is_empty() {
            find.message = proposal.chat_message(&check.members);
        }
        let one = check.members.len() == 1;
        let topic = (!one).then(|| proposal.topic.clone());
        ui.label(format!("{}: {}", language.text("Members"),
            std::iter::once(language.text("You").into_owned())
                .chain(check.members.iter().map(|member| format!("{} ({})", member.label(), member.email)))
                .collect::<Vec<_>>().join(", ")));
        match &topic {
            Some(topic) => { ui.label(format!("{}: {topic}", language.text("Chat topic"))); }
            None => { ui.weak(language.text("With one person, Tomlook uses your one-on-one chat, which has no topic.")); }
        }
        let label = ui.label(language.text("Message"));
        ui.add(egui::TextEdit::multiline(&mut find.message).id(Id::new("find-time-chat-message"))
            .desired_rows(6).desired_width(f32::INFINITY).char_limit(scheduling::MESSAGE_LIMIT)).labelled_by(label.id);
        let draft = ChatDraft { members: check.members.clone(), topic, message: find.message.trim().to_owned() };
        let valid = draft.validate();
        if let Err(error) = &valid {
            ui.colored_label(ui.visuals().error_fg_color, language.text(error));
        }
        ui.colored_label(ui.visuals().warn_fg_color, RichText::new(scheduling::chat_notice(&draft, language)).strong());
        let ready = self.proposal_writes();
        if let Err(reason) = ready {
            ui.weak(language.text(reason));
        }
        let awaiting = find.awaiting();
        if awaiting {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        if ui.add_enabled(ready.is_ok() && valid.is_ok() && !awaiting, egui::Button::new(language.text("Create chat and send"))).clicked()
            && let Ok(account) = ready
        {
            let sent = self.worker.as_ref().is_some_and(|worker| worker.commands.try_send(Command::ProposeChat {
                account, proposal: Box::new(proposal.clone()), draft: Box::new(draft),
            }).is_ok());
            if sent {
                find.proposal = Some(proposal);
                find.submitted = Some(Instant::now());
                find.status = Some("Sending the proposal to Teams...".into());
            } else {
                find.status = Some("The Teams proposal could not be queued; nothing was sent".into());
            }
        }
    }

    fn email_step(&mut self, ui: &mut egui::Ui, find: &mut FindTime) {
        let language = self.preferences.language;
        ui.add_space(6.0);
        let proposal = match self.proposal_of(find) {
            Ok(proposal) => proposal,
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, language.text(&error));
                return;
            }
        };
        if find.email_subject.is_empty() && find.email_body.is_empty() {
            find.email_subject = proposal.email_subject();
            find.email_body = proposal.email_body();
        }
        ui.weak(language.text("Tomlook never sends e-mail: copy the text or open a new message in your e-mail app."));
        ui.label(language.text("Recipients"));
        for attendee in &find.attendees {
            let mut included = !find.email_excluded.contains(&attendee.email);
            if ui.checkbox(&mut included, format!("{} ({})", attendee.label(), attendee.email)).changed() {
                if included { find.email_excluded.remove(&attendee.email); } else { find.email_excluded.insert(attendee.email.clone()); }
            }
        }
        let label = ui.label(language.text("Subject"));
        ui.add(egui::TextEdit::singleline(&mut find.email_subject).desired_width(f32::INFINITY)
            .char_limit(tomlook::followup::SUBJECT_LIMIT)).labelled_by(label.id);
        let label = ui.label(language.text("Message"));
        ui.add(egui::TextEdit::multiline(&mut find.email_body).desired_rows(8).desired_width(f32::INFINITY)
            .char_limit(tomlook::followup::BODY_LIMIT)).labelled_by(label.id);
        let to = find.attendees.iter().filter(|attendee| !find.email_excluded.contains(&attendee.email))
            .map(|attendee| attendee.email.as_str()).collect::<Vec<_>>();
        let valid = !find.email_subject.trim().is_empty() && !find.email_body.trim().is_empty();
        let (mut copy, mut open) = (false, false);
        ui.horizontal(|ui| {
            copy = ui.add_enabled(valid, egui::Button::new(language.text("Copy")))
                .on_hover_text(language.text("Copies the subject and message to the clipboard")).clicked();
            open = ui.add_enabled(valid && !to.is_empty(), egui::Button::new(language.text("Open in Outlook")))
                .on_hover_text(language.text("Opens a new message in your default e-mail app. Nothing is sent automatically."))
                .clicked();
        });
        if copy {
            ui.ctx().copy_text(tomlook::followup::clipboard_text(&find.email_subject, &find.email_body, language));
            find.status = Some("Copied to the clipboard.".into());
        }
        if open {
            match tomlook::followup::mailto(&to, &find.email_subject, &find.email_body) {
                Ok(link) => {
                    if link.body_in_clipboard {
                        ui.ctx().copy_text(find.email_body.clone());
                    }
                    find.status = Some(match super::followup::open_mail_link(&link.uri) {
                        Ok(()) if link.body_in_clipboard => "The message is too long for an e-mail link. The body was copied to the clipboard; paste it into the new message.".into(),
                        Ok(()) => "Opened in your e-mail app. Nothing was sent.".into(),
                        Err(error) => error,
                    });
                }
                Err(error) => find.status = Some(error),
            }
        }
    }

    /// Confirms deleting a proposal's held BLOCKERs.
    pub(super) fn blocker_release_dialog(&mut self, context: &egui::Context) {
        let Some(draft) = self.blocker_release.take() else { return };
        let language = self.preferences.language;
        let Some(proposal) = self.activity.proposals.get(&draft.proposal).cloned() else { return };
        let blockers = proposal.blockers.iter()
            .filter(|blocker| blocker.state == BlockerState::Held
                && blocker.event_id.as_ref().is_some_and(|id| draft.events.contains(id)))
            .cloned().collect::<Vec<_>>();
        let mut open = true;
        let (mut confirm, mut back) = (false, false);
        let title = if draft.cancel { "Release all BLOCKERs" } else { "Meeting scheduled" };
        egui::Window::new(language.text(title))
            .id(Id::new("release-blockers")).open(&mut open)
            .default_width(420.0).resizable(false).collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.strong(&proposal.topic);
                if !draft.cancel {
                    ui.label(language.text("The invitation was created and verified. Delete the BLOCKERs of this proposal?"));
                }
                if blockers.is_empty() {
                    ui.weak(language.text("No BLOCKERs of this proposal are in your calendar any more."));
                } else {
                    ui.colored_label(ui.visuals().warn_fg_color,
                        RichText::new(scheduling::release_notice(blockers.len(), language)).strong());
                    for blocker in &blockers {
                        ui.label(format!("{} · {}", blocker.title, Slot { start: blocker.start, end: blocker.end }.label(language)));
                    }
                }
                let ready = self.proposal_writes();
                if let Err(reason) = ready && !blockers.is_empty() {
                    ui.weak(language.text(reason));
                }
                ui.horizontal(|ui| {
                    let label = if blockers.is_empty() {
                        language.text(if draft.cancel { "Close the proposal" } else { "OK" }).into_owned()
                    } else {
                        scheduling::release_button(blockers.len(), language)
                    };
                    confirm = ui.add_enabled(blockers.is_empty() || ready.is_ok(), egui::Button::new(label)).clicked();
                    back = ui.button(language.text(if draft.cancel { "Back" } else { "Keep them" })).clicked();
                });
            });
        if confirm {
            if !blockers.is_empty() || draft.cancel {
                if let Some(account) = self.calendar_owner.clone() {
                    self.command(Command::ReleaseBlockers {
                        account, proposal: draft.proposal.clone(),
                        events: blockers.iter().filter_map(|blocker| blocker.event_id.clone()).collect(),
                        cancel: draft.cancel,
                    });
                }
                self.messages_open = !blockers.is_empty();
            }
        } else if open && !back {
            self.blocker_release = Some(draft);
        }
    }

    /// After a confirmed proposal time was created and verified, marks the proposal scheduled and
    /// offers to delete its BLOCKERs.
    pub(super) fn sync_proposal_confirmation(&mut self) {
        let Some((proposal, record)) = self.proposal_confirmation.clone() else { return };
        let Some(state) = self.activity.event_creations.get(&record).map(|record| record.state) else { return };
        match state {
            tomlook::event_creation::State::Creating => {}
            tomlook::event_creation::State::Created => {
                self.proposal_confirmation = None;
                if let Some(account) = self.calendar_owner.clone() {
                    self.command(Command::ScheduleProposal { account, proposal: proposal.clone(), record });
                }
                self.open_release(&proposal, false);
            }
            _ => self.proposal_confirmation = None,
        }
    }

    /// Saved proposals of the calendar owner for **Messages**.
    pub(super) fn proposal_messages(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let mut proposals = self.activity.proposals.values()
            .filter(|proposal| self.activity.task_account.as_deref() == Some(proposal.account.as_str()))
            .cloned().collect::<Vec<_>>();
        if proposals.is_empty() {
            return;
        }
        proposals.sort_by_key(|proposal| std::cmp::Reverse(proposal.updated_at));
        egui::CollapsingHeader::new(language.text("Scheduling proposals")).default_open(proposals.iter().any(Proposal::needs_review))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("scheduling-proposals").max_height(200.0).show(ui, |ui| {
                    for proposal in proposals.iter().take(20) {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(&proposal.topic);
                            ui.weak(language.text(proposal.status.label()));
                            if proposal.needs_review() {
                                ui.colored_label(ui.visuals().warn_fg_color, language.text("Needs a check"));
                            }
                        });
                        ui.weak(format!("{}: {}", language.text("Invitees"), proposal.attendees.iter()
                            .map(Attendee::label).collect::<Vec<_>>().join(", ")));
                        self.proposal_progress(ui, proposal);
                        if let Some(chat) = &proposal.chat {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(language.text(chat.state.label()));
                                if !chat.message.is_empty() { ui.weak(language.text(&chat.message)); }
                                if let Some(url) = &chat.web_url
                                    && ui.small_button(language.text("Open the chat in Teams")).clicked()
                                    && let Err(error) = tomlook::browser::open(url) {
                                    self.error = Some(error);
                                }
                            });
                        }
                        if proposal.status == ProposalStatus::Open && proposal.held().next().is_some()
                            && ui.small_button(language.text("Release all BLOCKERs")).clicked() {
                            self.open_release(&proposal.id, true);
                        }
                        ui.separator();
                    }
                });
            });
    }

    /// The `findtime` and `teams` demo scenes: fictional candidates, two selected; the Teams scene
    /// also shows the approval preview. Ready once the candidates (and Teams check) are shown.
    pub(super) fn demo_find_time_ready(&mut self, teams: bool) -> bool {
        if self.find_time.is_none() {
            let (topic, people) = if teams {
                ("Pilot retrospective", "Alex Rivers <alex@example.invalid>; Jamie Brooks <jamie@example.invalid>; Morgan Lee <morgan@example.invalid>")
            } else {
                ("Pilot retrospective", tomlook::showcase::FIND_TIME_ATTENDEES)
            };
            let mut list = attendees::parse_input(people).unwrap_or_default();
            for attendee in list.iter_mut().skip(2) {
                attendee.role = Role::Optional;
            }
            self.open_find_time(topic, list, 30);
            if let Some(find) = &mut self.find_time {
                find.focus_topic = false;
            }
        }
        let Some(find) = &mut self.find_time else { return false };
        let picks = match find.candidates() {
            Some(Ok(candidates)) => candidates.list.iter().filter(|candidate| candidate.all_required).take(2)
                .map(|candidate| candidate.slot()).collect::<Vec<_>>(),
            _ => return false,
        };
        if find.selected.is_empty() {
            for slot in picks {
                let _ = scheduling::toggle(&mut find.selected, slot);
            }
            if teams {
                find.step = Step::Chat;
            }
            return false;
        }
        !teams || (matches!(find.chat_check.result, Some(Ok(_))) && !find.message.is_empty())
    }
}
