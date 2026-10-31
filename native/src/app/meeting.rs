use super::*;
use super::feedback::Panel;
use super::settings::insight_state;

/// `feedback(ui, true)` adds header controls; `feedback(ui, false)` adds content below the header.
fn insight_section(
    ui: &mut egui::Ui,
    label: &str,
    items: &[serde_json::Value],
    accent: Color32,
    mut feedback: impl FnMut(&mut egui::Ui, bool),
) {
    let language = tomlook::i18n::Language::En;
    if items.is_empty() {
        return;
    }
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 18.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 1.0, accent);
                ui.strong(language.text(label));
                ui.weak(language.text(&(items.len().to_string())));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    feedback(ui, true)
                });
            });
            feedback(ui, false);
            for (index, item) in items.iter().enumerate() {
                let Some(text) = item.as_str() else { continue };
                ui.add_space(9.0);
                ui.horizontal_top(|ui| {
                    ui.label(
                        RichText::new(language.text(&(format!("{:02}", index + 1))))
                            .small()
                            .color(accent),
                    );
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        if let Some((heading, body)) = text
                            .split_once(':')
                            .filter(|(heading, _)| heading.chars().count() <= 80)
                        {
                            ui.strong(language.text(heading));
                            ui.label(language.text(body.trim()));
                        } else {
                            ui.label(text);
                        }
                    });
                });
            }
        });
}

pub(super) struct MeetingActionDraft {
    pub(super) event: Event,
    account: String,
    pub(super) change: tomlook::recommendations::Change,
    calendar_snapshot: Arc<Calendar>,
    unchanged: bool,
}

impl Tomlook {
    pub(super) fn choose_meeting_action(&mut self, event: Event, change: tomlook::recommendations::Change) {
        let Some(account) = self.calendar_owner.clone() else {
            self.error = Some("Connect WorkIQ before confirming a calendar change".into());
            return;
        };
        self.meeting_menu = None;
        if matches!(
            change,
            tomlook::recommendations::Change::Category(_)
                | tomlook::recommendations::Change::ClearCategory
                | tomlook::recommendations::Change::Follow(_)
        ) {
            self.meeting_action = None;
            self.command(Command::ConfirmMeetingAction {
                event: Box::new(event),
                account,
                change,
            });
            return;
        }
        self.meeting_action = Some(MeetingActionDraft {
            event,
            account,
            change,
            calendar_snapshot: self.calendar.clone(),
            unchanged: true,
        });
    }

    pub(super) fn meeting_actions(&mut self, ui: &mut egui::Ui, index: usize) {
        use tomlook::recommendations::{Change, MeetingResponse, validate_manual_change};
        let language = self.preferences.language;
        let calendar = self.calendar.clone();
        let event = &calendar.events[index];
        ui.strong(event.title.chars().take(100).collect::<String>());
        ui.weak(meeting_date_label(event, language));
        ui.separator();
        if ui.button(language.text("Open Insights")).clicked() {
            self.select(index);
            self.insights_expanded = true;
            self.meeting_menu = None;
            ui.close();
        }
        if let Some(url) = &event.source_url
            && tomlook::calendar::safe_url(url)
            && ui.button(language.text("Open original meeting")).clicked()
        {
            if let Err(error) = tomlook::browser::open(url) {
                self.error = Some(error);
            }
            self.meeting_menu = None;
            ui.close();
        }
        ui.separator();
        let ready = self.workiq_ready && self.ai.is_some() && self.calendar_owner.is_some();
        let follow = Change::Follow(!event.following());
        let follow_validation = validate_manual_change(event, &follow);
        let follow_hint = follow_validation.as_ref().err().map(String::as_str)
            .unwrap_or("Connect WorkIQ before confirming a calendar change");
        let follow_response = ui.add_enabled(ready && follow_validation.is_ok(),
            egui::Button::new(language.text(if event.following() { "Stop following" } else { "Follow" })))
            .on_disabled_hover_text(language.text(follow_hint))
            .on_hover_text(language.text(if event.following() {
                "Removes Follow only. Availability stays unchanged; the organizer is not notified."
            } else {
                "Adds Follow, keeps other categories and marks this occurrence as free. Your invitation response is unchanged; the organizer is not notified."
            }));
        if follow_response.clicked() {
            self.choose_meeting_action(event.clone(), follow);
            ui.close();
        }
        let current_categories = tomlook::recommendations::real_categories(event).into_iter()
            .filter(|category| !category.eq_ignore_ascii_case("follow")).collect::<Vec<_>>();
        ui.menu_button(language.text("Change category"), |ui| {
            for category in tomlook::recommendations::known_categories(&self.calendar).into_iter()
                .filter(|category| !category.eq_ignore_ascii_case("follow")) {
                let change = Change::Category(category.clone());
                let selected = current_categories.len() == 1 && current_categories[0] == category;
                let allowed = ready && !selected && validate_manual_change(event, &change).is_ok();
                if ui
                    .add_enabled(allowed, egui::Button::new(category).selected(selected))
                    .clicked()
                {
                    self.choose_meeting_action(event.clone(), change);
                    ui.close();
                }
            }
            ui.separator();
            if ui
                .add_enabled(
                    ready
                        && !current_categories.is_empty()
                        && validate_manual_change(event, &Change::ClearCategory).is_ok(),
                    egui::Button::new(language.text("Remove category")),
                )
                .clicked()
            {
                self.choose_meeting_action(event.clone(), Change::ClearCategory);
                ui.close();
            }
        });
        if event.extra.get("isOrganizer").and_then(serde_json::Value::as_bool) == Some(true) {
            let validation = tomlook::event_creation::validate_travel_parent(event);
            let response = ui.add_enabled(ready && validation.is_ok(),
                egui::Button::new(language.text("Add travel blocks before and after")));
            if response.clicked() {
                self.open_travel_draft(event.clone());
                self.meeting_menu = None;
                ui.close();
            } else if let Err(error) = validation {
                response.on_hover_text(language.text(&error));
            }
        }
        for response in [
            MeetingResponse::Accept,
            MeetingResponse::Tentative,
            MeetingResponse::Decline,
            MeetingResponse::Cancel,
        ] {
            let change = Change::Respond {
                response,
                comment: String::new(),
                send_response: response != MeetingResponse::Decline,
            };
            if validate_manual_change(event, &change).is_ok()
                && ui
                    .add_enabled(ready, egui::Button::new(language.text(response.label())))
                    .clicked()
            {
                self.choose_meeting_action(event.clone(), change);
                ui.close();
            }
        }
        if validate_manual_change(event, &Change::Delete).is_ok()
            && ui
                .add_enabled(
                    ready,
                    egui::Button::new(language.text("Delete own appointment")),
                )
                .clicked()
        {
            self.choose_meeting_action(event.clone(), Change::Delete);
            ui.close();
        }
        if !ready {
            ui.weak(language.text("Connect WorkIQ before confirming a calendar change"));
        } else if validate_manual_change(event, &Change::ClearCategory).is_err() {
            ui.weak(language.text("Refresh the calendar first; actions require one verified occurrence, not a whole series"));
        }
        ui.weak(language.text("Shift+F10: meeting actions"));
    }

    pub(super) fn meeting_action_dialog(&mut self, context: &egui::Context) {
        use tomlook::recommendations::{Change, MeetingResponse};
        let language = self.preferences.language;
        if let Some(id) = self.meeting_menu.clone() {
            let mut open = true;
            egui::Window::new(language.text("Meeting actions"))
                .id(Id::new("meeting-actions"))
                .open(&mut open)
                .resizable(false)
                .show(context, |ui| {
                    if let Some(index) = self.calendar.event_index(&id) {
                        self.meeting_actions(ui, index);
                    } else {
                        ui.label(language.text("Meeting is no longer in the current calendar."));
                    }
                });
            if !open {
                self.meeting_menu = None;
            }
        }
        let Some(mut draft) = self.meeting_action.take() else {
            return;
        };
        if !Arc::ptr_eq(&draft.calendar_snapshot, &self.calendar) {
            draft.unchanged = self
                .calendar
                .event_index(&draft.event.id)
                .is_some_and(|index| {
                    tomlook::scheduler::fingerprint(&self.calendar.events[index])
                        == tomlook::scheduler::fingerprint(&draft.event)
                });
            draft.calendar_snapshot = self.calendar.clone();
        }
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        let mut confirmed_change = draft.change.clone();
        let action = match &draft.change {
            Change::Respond { response, .. } => response.label(),
            Change::Delete => "Delete event",
            _ => "Confirm calendar change",
        };
        egui::Window::new(format!("{}?", language.text(action)))
            .id(Id::new("confirm-meeting-action")).open(&mut open)
            .default_width(360.0).resizable(false).collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(context, |ui| {
                ui.strong(&draft.event.title);
                ui.weak(meeting_date_label(&draft.event, language));
                if matches!(draft.event.extra.get("type").and_then(serde_json::Value::as_str),
                    Some("occurrence" | "exception")) {
                    ui.weak(language.text("This occurrence only"));
                }
                ui.add_space(8.0);
                match &mut draft.change {
                    Change::Document(_) | Change::Text(_) => {
                        ui.label(language.text("Review AI-proposed edits in Messages."));
                        return;
                    }
                    Change::Category(category) => {
                        let old = tomlook::recommendations::real_categories(&draft.event);
                        ui.label(format!("{}: {} -> {category}", language.text("Category"),
                            if old.is_empty() { language.text("None").into_owned() } else { old.join(", ") }));
                    }
                    Change::ClearCategory => {
                        ui.label(language.text("Remove category"));
                    }
                    Change::Follow(enabled) => {
                        ui.label(language.text(if *enabled {
                            "Adds Follow, keeps other categories and marks this occurrence as free. Your invitation response is unchanged; the organizer is not notified."
                        } else {
                            "Removes Follow only. Availability stays unchanged; the organizer is not notified."
                        }));
                    }
                    Change::Respond { response, comment, send_response } => {
                        if *response == MeetingResponse::Cancel {
                            ui.label(language.text("Notifies all attendees."));
                            *send_response = true;
                        } else {
                            ui.checkbox(send_response, language.text("Send response to organizer"));
                        }
                        if *send_response {
                            let label = ui.label(language.text("Comment (optional)"));
                            ui.add(egui::TextEdit::multiline(comment)
                                .id(Id::new("meeting-action-comment")).desired_rows(2)
                                .desired_width(f32::INFINITY).char_limit(2000)).labelled_by(label.id);
                        }
                    }
                    Change::Delete => {}
                    Change::Buffer { .. } => {}
                }
                let unchanged = self.calendar_owner.as_deref() == Some(draft.account.as_str())
                    && draft.unchanged;
                confirmed_change = draft.change.clone();
                if let Change::Respond { comment, send_response: false, .. } = &mut confirmed_change {
                    comment.clear();
                }
                let validation = tomlook::recommendations::validate_manual_change(&draft.event, &confirmed_change);
                if !unchanged { ui.label(language.text("Meeting or account changed; close this dialog and review the current meeting.")); }
                if let Err(error) = &validation { ui.label(language.text(error)); }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    confirm = ui.add_enabled(unchanged && validation.is_ok() && self.workiq_ready && self.ai.is_some(),
                        egui::Button::new(language.text(if matches!(draft.change, Change::Delete) { "Delete" } else { action }))).clicked();
                    cancel = ui.button(language.text("Back")).clicked();
                });
            });
        if confirm {
            self.command(Command::ConfirmMeetingAction {
                event: Box::new(draft.event),
                account: draft.account,
                change: confirmed_change,
            });
            self.meeting_action = None;
        } else if !open || cancel {
            self.meeting_action = None;
        } else {
            self.meeting_action = Some(draft);
        }
    }

    pub(super) fn details(&mut self, ui: &mut egui::Ui, event: Event) {
        let language = self.preferences.language;
        let ended = event.end <= Local::now().fixed_offset();
        self.photos_context = Some(ui.ctx().clone());
        ui.horizontal(|ui| {
            ui.weak(language.text("MEETING"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(language.text("Close"))
                    .on_hover_text(language.text("Esc"))
                    .clicked()
                {
                    self.selected = None;
                    self.detail_revision += 1;
                }
            });
        });
        ui.add_space(12.0);
        ui.heading(&event.title);
        if ended { ui.strong(language.text("Meeting ended")); }
        if event.extra.get("sensitivity").and_then(serde_json::Value::as_str) == Some("private") {
            ui.weak(language.text("Private event"));
        }
        if !ended && let Some(url) = event.join_url()
            && ui.button(language.text("Join Teams meeting")).clicked()
            && let Err(error) = tomlook::browser::open(url) {
                self.error = Some(error);
        }
        if ui
            .button(language.text("Meeting actions"))
            .on_hover_text(language.text("Shift+F10: meeting actions"))
            .clicked()
        {
            self.meeting_menu = Some(event.id.clone());
        }
        let (first, last) = event.calendar_dates();
        ui.label(if first == last {
            language.date(first, true)
        } else {
            format!(
                "{} – {}",
                language.date(first, true),
                language.date(last, true)
            )
        });
        if event.unconfirmed() {
            ui.weak(language.text(event.response_label()));
        }
        ui.label(if event.is_all_day {
            language.text("All day").into_owned()
        } else {
            format!(
                "{} - {}",
                event.start.with_timezone(&Local).format("%H:%M"),
                event.end.with_timezone(&Local).format("%H:%M")
            )
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if !event.location.is_empty() {
                ui.weak(&event.location);
            }
            for tag in event.tags() {
                ui.colored_label(
                    category_color(tag, self.preferences.dark),
                    language.text(tag),
                );
            }
        });
        let mut requested_photos = Vec::new();
        let mut requested_companies = Vec::new();
        let mut companies = BTreeMap::<(bool, String), Vec<&String>>::new();
        for person in &event.attendees {
            let company = person_email(person)
                .and_then(|email| email.split_once('@').map(|(_, domain)| domain.to_owned()))
                .unwrap_or_else(|| "Guests".into());
            let microsoft = company.eq_ignore_ascii_case("microsoft.com");
            companies
                .entry((
                    !microsoft,
                    if microsoft {
                        "Microsoft".into()
                    } else {
                        company
                    },
                ))
                .or_default()
                .push(person);
        }
        if !event.organizer.is_empty() {
            ui.label(format!("{}: {}", language.text("Organizer"),
                event.organizer.split('<').next().unwrap_or(&event.organizer).trim())).on_hover_text(&event.organizer);
        }
        ui.horizontal_wrapped(|ui| {
        for ((external, company), people) in companies.iter().take(8) {
            let domain = if *external {
                company.as_str()
            } else {
                "microsoft.com"
            };
            let icon = self.companies.get(domain);
            let tooltip = format!("{company}\n{}", people.iter().map(|person| person.as_str()).collect::<Vec<_>>().join("\n"));
            ui.horizontal(|ui| {
                if let Some(image) = icon.and_then(|icon| icon.image.as_ref()) {
                    egui::Frame::new()
                        .fill(Color32::WHITE)
                        .corner_radius(4)
                        .inner_margin(3)
                        .show(ui, |ui| {
                            ui.add(egui::Image::new((image.id(), Vec2::splat(20.0))))
                                .on_hover_text(&tooltip);
                        });
                } else {
                let label = ui.label(
                    RichText::new(language.text(company))
                        .small()
                        .strong()
                        .color(if *external {
                            ui.visuals().text_color()
                        } else {
                            self.accent()
                        }),
                );
                label.on_hover_text(if let Some(error) = icon.and_then(|icon| icon.error.as_ref()) {
                    format!("{tooltip}\n{}", language.text(error))
                } else { tooltip });
                }
            });
            if !self.companies_busy
                && self.companies.len() + requested_companies.len() < 128
                && !self.companies.contains_key(domain)
                && tomlook::company::valid_domain(domain)
            {
                requested_companies.push(domain.to_owned());
            }
        }
        });
        if companies.len() > 8 {
            ui.weak(language.text(&(format!("+{} companies in People", companies.len() - 8))));
        }
        if !requested_companies.is_empty()
            && self.ai_command(ai::Command::Companies(requested_companies))
        {
            self.companies_busy = true;
        }
        let people_label = if event.attendee_count() > event.attendees.len() {
            format!(
                "People ({} of {})",
                event.attendees.len(),
                event.attendee_count()
            )
        } else {
            format!("People ({})", event.attendees.len())
        };
        egui::CollapsingHeader::new(language.text(&(people_label)))
            .id_salt(("attendees", &event.id))
            .default_open(false)
            .show(ui, |ui| {
                if event.attendee_count() > event.attendees.len() {
                    ui.weak(language.text("Participant list is limited to 500. Open the original meeting for the full list."));
                }
                egui::ScrollArea::vertical()
                    .id_salt("attendee-list")
                    .max_height(180.0)
                    .show_rows(ui, 36.0, event.attendees.len(), |ui, rows| {
                        for row in rows {
                            let attendee = &event.attendees[row];
                            let email = person_email(attendee);
                            let photo = email.as_ref().and_then(|email| self.photos.get(email));
                            ui.horizontal(|ui| {
                                if let Some(image) = photo.and_then(|photo| photo.image.as_ref()) {
                                    ui.add(
                                        egui::Image::new((image.id(), Vec2::splat(30.0)))
                                            .corner_radius(15),
                                    );
                                }
                                let role = email.as_ref().and_then(|email| event.extra.get("tomlook_attendee_roles")?.get(email)?.as_str()).unwrap_or("unknown");
                                let role_label = match role { "required" => "Required", "optional" => "Optional", "resource" => "Resource", _ => "Role unavailable" };
                                let (marker, response) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                                if role == "required" { ui.painter().circle_filled(marker.center(), 3.0, self.accent()); }
                                else { ui.painter().circle_stroke(marker.center(), 3.0, Stroke::new(1.0, ui.visuals().weak_text_color())); }
                                response.on_hover_text(language.text(role_label));
                                let name = attendee.split('<').next().unwrap_or(attendee).trim();
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(name).on_hover_text(photo.and_then(|photo| photo.error.as_deref()).unwrap_or(attendee));
                                        ui.weak(language.text(role_label));
                                    });
                                    if let Some(email) = &email { ui.label(RichText::new(email).small().weak()); }
                                });
                            });
                            if self.workiq_ready
                                && !self.photos_busy
                                && let Some(email) = email
                                && !self.photos.contains_key(&email)
                                && !self.photos_pending.contains(&email)
                                && requested_photos.len() < 8
                            {
                                requested_photos.push(email);
                            }
                        }
                    });
            });
        if !requested_photos.is_empty() {
            match self.ai.as_ref().map(|engine| {
                engine
                    .commands
                    .try_send(ai::Command::Photos(requested_photos.clone()))
            }) {
                Some(Ok(())) => {
                    self.photos_busy = true;
                    self.photos_pending.extend(requested_photos);
                }
                _ => self.ai_error = Some("Photo lookup could not be admitted".into()),
            }
        }
        if let Some(url) = event.source_url.as_ref().filter(|url| safe_url(url)) {
            ui.hyperlink_to(language.text("Open original meeting"), url);
        }
        if ui.button(language.text("Ask about this meeting")).clicked() {
            self.assistant = true;
            self.switch_assistant_context(Some(event.clone()));
            self.focus_assistant = true;
        }
        let recap_record = self.calendar_owner.as_deref().and_then(|account| self.activity.recaps.record(account, &event)).cloned();
        let running = recap_record.as_ref().is_some_and(|record| record.checking) || self
            .activity
            .queue
            .flights
            .values()
            .any(|flight| flight.event_id == event.id);
        if !self.preview
            && ui
                .add_enabled(
                    !running && (!ended || tomlook::recaps::eligible(&event, Local::now().fixed_offset())
                        && recap_record.as_ref().is_none_or(|record| record.payload.is_none())),
                    egui::Button::new(language.text(if running {
                        "Researching..."
                    } else if ended {
                        "Check transcript"
                    } else {
                        "Research now"
                    })),
                )
                .clicked()
        {
            self.request_insights(event.id.clone());
        }
        ui.add_space(16.0);
        ui.separator();
        let document_feedback = self.document_feedback();
        ui.horizontal(|ui| {
            ui.strong(language.text(if ended { "Meeting notes" } else { "Insights" }));
            if self.briefing.is_some() && ui.button(language.text("Expand")).clicked() {
                self.insights_expanded = true;
            }
            if let Some((output, target)) = &document_feedback {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.feedback_thumbs(ui, output, target, Panel::Details);
                });
            }
        });
        if let Some((output, target)) = &document_feedback {
            self.feedback_reasons(ui, output, target, Panel::Details);
        }
        if self.preview {
            ui.weak(language.text("Offline demo - fictional calendar"));
        } else if !self.activity.enabled {
            ui.weak(language.text("Automatic Insights are disabled."));
        } else if self.activity.paused {
            ui.weak(language.text("Automatic Insights are paused."));
        } else if !self.ai_ready {
            ui.weak(language.text("AI is not connected yet; Research now queues this meeting."));
        }
        if ended {
            if let Some(record) = &recap_record { ui.weak(language.text(&record.message)); }
            else { ui.weak(language.text(if tomlook::recaps::eligible(&event, Local::now().fixed_offset()) {
                "Waiting for a transcript (up to 7 days after the meeting)"
            } else { "Automatic notes cover accepted or followed online meetings from the last 7 days." })); }
        }
        if !ended && !self.preview
            && let Some(job) = self.activity.queue.jobs.get(&event.id).cloned()
        {
            ui.label(format!(
                "{}: {}",
                language.text(insight_state(job.state)),
                language.text(&job.message)
            ));
            if matches!(
                job.state,
                tomlook::scheduler::State::Failed | tomlook::scheduler::State::Interrupted
            ) && ui.button(language.text("Retry Insights")).clicked()
            {
                self.command(Command::Retry(event.id.clone()));
            }
            if job.state != tomlook::scheduler::State::Completed && self.briefing.is_some() {
                ui.weak(language.text("Previous saved Insights; research is not complete yet."));
            }
        }
        self.render_insights(ui, false);
    }

    pub(super) fn render_insights(&mut self, ui: &mut egui::Ui, expanded: bool) {
        // Rendering borrows the document while feedback controls update `self`.
        let Some(briefing) = self.briefing.take() else {
            let language = self.preferences.language;
            let ended = self.selected.and_then(|index| self.calendar.events.get(index))
                .is_some_and(|event| event.end <= Local::now().fixed_offset());
            ui.label(language.text(if ended { "No AI meeting notes yet." } else {
                "No saved Insights yet. Research now prioritizes this meeting."
            }));
            return;
        };
        self.render_briefing(ui, &briefing, expanded);
        self.briefing = Some(briefing);
    }

    fn render_briefing(&mut self, ui: &mut egui::Ui, briefing: &serde_json::Value, expanded: bool) {
        let language = self.preferences.language;
        let output = if expanded { self.feedback_output(briefing) } else { None };
        {
            let notes = briefing.get("tomlook_recap").and_then(serde_json::Value::as_bool) == Some(true);
            let narrative = language.insight(briefing);
            if tomlook::changes::approved_context(briefing).is_some() {
                ui.weak(language.text("Approved sections are preserved during automatic updates."));
            }
            if briefing
                .get("translation_only")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                ui.weak(language.text("Translated saved Insights; no new research was performed."));
            }
            if let Some(evidence) = briefing
                .get("evidence_status")
                .and_then(|value| value.as_str())
            {
                ui.weak(language.text(evidence));
            }
            if let Some(summary) = narrative
                .and_then(|value| value.get("summary"))
                .and_then(|v| v.as_str())
            {
                let target = output.as_ref().and_then(|output| output.target(briefing, "summary"));
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(8)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(language.text("Overview"));
                            if let (Some(output), Some(target)) = (&output, &target) {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    self.feedback_thumbs(ui, output, target, Panel::Expanded);
                                });
                            }
                        });
                        if let (Some(output), Some(target)) = (&output, &target) {
                            self.feedback_reasons(ui, output, target, Panel::Expanded);
                        }
                        ui.add_space(6.0);
                        ui.label(summary);
                    });
            } else {
                ui.weak(language.text(
                    "Czech version is not saved yet. Research now can create both languages.",
                ));
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for (field, label) in [
                    ("preparation", if notes { "My tasks" } else { "Next steps" }),
                    ("open_questions", if notes { "Mentions of me" } else { "Questions" }),
                    ("sources", "Sources"),
                ] {
                    let count = briefing
                        .get(field)
                        .and_then(|value| value.as_array())
                        .map_or(0, Vec::len);
                    egui::Frame::new()
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(6)
                        .inner_margin(7)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(language.text(&(count.to_string())))
                                    .strong()
                                    .color(self.accent()),
                            );
                            ui.weak(language.text(label));
                        });
                }
            });
            if !expanded {
                if notes {
                    for (field, label) in [("preparation", "My tasks"), ("open_questions", "Mentions of me")] {
                        if let Some(items) = narrative.and_then(|value| value[field].as_array()).filter(|items| !items.is_empty()) {
                            insight_section(ui, &language.text(label), items, self.accent(), |_, _| {});
                        }
                    }
                }
                return;
            }
            ui.add_space(12.0);
            let sections = [
                ("preparation", if notes { "My tasks" } else { "Prepare" }),
                ("talking_points", if notes { "Technical takeaways and materials" } else { "Talking points" }),
                ("open_questions", if notes { "Mentions of me" } else { "Open questions" }),
                ("risks", if notes { "Decisions and unresolved issues" } else { "Risks" }),
                ("warnings", "Evidence gaps"),
                ("gaps", "Evidence gaps"),
            ];
            let columns = if ui.available_width() >= 640.0 { 2 } else { 1 };
            let accent = self.accent();
            ui.columns(columns, |uis| {
                for (index, (field, label)) in sections.iter().enumerate() {
                    if let Some(items) = narrative
                        .and_then(|value| value.get(field))
                        .and_then(|value| value.as_array())
                        .filter(|items| !items.is_empty())
                    {
                        let ui = &mut uis[index % columns];
                        let target = output.as_ref().and_then(|output| output.target(briefing, field));
                        insight_section(ui, &language.text(label), items, accent, |ui, header| {
                            if let (Some(output), Some(target)) = (&output, &target) {
                                if header {
                                    self.feedback_thumbs(ui, output, target, Panel::Expanded);
                                } else {
                                    self.feedback_reasons(ui, output, target, Panel::Expanded);
                                }
                            }
                        });
                        ui.add_space(10.0);
                    }
                }
            });
            if let Some(sources) = briefing.get("sources").and_then(|v| v.as_array()) {
                ui.add_space(10.0);
                ui.weak(language.text("SOURCES"));
                for source in sources {
                    let title = source
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Untitled source");
                    if let Some(url) = source
                        .get("url")
                        .and_then(|v| v.as_str())
                        .filter(|url| tomlook::evidence::safe_source_url(url))
                    {
                        ui.hyperlink_to(title, url);
                    } else {
                        ui.label(title);
                    }
                    if notes { ui.weak(language.text("Original meeting source")); continue; }
                    match source
                        .get("retrieved_records")
                        .and_then(|value| value.as_array())
                    {
                        Some(records) if !records.is_empty() => {
                            ui.weak(language.text("Referenced in captured results"))
                                .on_hover_text(language.text(
                                    "A matching URL is not verification of the source's claims.",
                                ));
                        }
                        Some(_) => {
                            ui.weak(language.text("Not found in captured results"))
                                .on_hover_text(language.text(
                                "Capture is bounded. This does not mean the source does not exist.",
                            ));
                        }
                        None => {
                            ui.weak(language.text("Unverified citation"));
                        }
                    }
                }
                if let Some(records) = briefing
                    .get("retrieved_evidence")
                    .and_then(|evidence| evidence.get("records"))
                    .and_then(|records| records.as_array())
                {
                    ui.add_space(12.0);
                    ui.strong(language.text("Retrieved context"));
                    if let Some(omitted) = briefing
                        .get("retrieved_evidence")
                        .and_then(|evidence| evidence.get("omitted"))
                        .and_then(|value| value.as_u64())
                        .filter(|count| *count > 0)
                    {
                        ui.weak(
                            language.text(
                                &(format!("{omitted} additional tool results not retained.")),
                            ),
                        );
                    }
                    for record in records {
                        let tool = record
                            .get("tool")
                            .and_then(|value| value.as_str())
                            .unwrap_or("Source");
                        let resource = record.get("resource").and_then(|value| value.as_str());
                        let label = tomlook::evidence::source_label(tool, resource);
                        let ordinal = record.get("ordinal").and_then(|value| value.as_u64());
                        let heading = ordinal.map_or_else(
                            || label.to_owned(),
                            |ordinal| format!("R{ordinal}: {label}"),
                        );
                        ui.collapsing(language.text(&(heading)), |ui| {
                            if let Some(observed) =
                                record.get("observed_at").and_then(|value| value.as_str())
                            {
                                ui.weak(language.text(&(format!("Retrieved {observed}"))));
                            }
                            if record.get("succeeded").and_then(|value| value.as_bool())
                                != Some(true)
                            {
                                ui.weak(language.text("Retrieval failed"));
                            }
                            if let Some(resource) = resource {
                                ui.label(resource);
                            }
                            if let Some(excerpt) =
                                record.get("excerpt").and_then(|value| value.as_str())
                            {
                                ui.label(excerpt);
                            }
                            if record.get("truncated").and_then(|value| value.as_bool())
                                == Some(true)
                            {
                                ui.weak(language.text("Excerpt only, not the complete source."));
                            }
                            if let Some(links) =
                                record.get("links").and_then(|value| value.as_array())
                            {
                                for url in links
                                    .iter()
                                    .filter_map(|value| value.as_str())
                                    .filter(|url| tomlook::evidence::safe_source_url(url))
                                {
                                    ui.hyperlink_to(language.text("Open cited source"), url);
                                }
                            }
                        });
                    }
                }
            }
        }
    }

    pub(super) fn request_insights(&mut self, id: String) {
        if !self.ai_ready {
            self.ai_command(ai::Command::Connect);
        }
        if !self.workiq_ready && !self.hosted_workiq_busy {
            if self.hosted_workiq_account.is_empty() {
                self.connection_panel = Some(ConnectionPanel::Workiq);
            } else {
                self.ai_command(ai::Command::RestoreWorkiq(
                    self.hosted_workiq_account.clone(),
                ));
            }
        }
        self.command(Command::PrepareNow(id));
    }
}
