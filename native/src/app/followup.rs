use super::*;
use tomlook::followup::{self as model, Saved};

pub(super) struct Running {
    pub(super) id: u64,
    pub(super) request: Box<model::Request>,
}

/// Follow-up e-mail of the selected ended meeting; the text stays local until the user copies it
/// or opens it in their own e-mail app.
#[derive(Default)]
pub(super) struct FollowUpUi {
    pub(super) request_id: u64,
    pub(super) running: Option<Running>,
    /// Meeting occurrence key and message of the last failure.
    pub(super) error: Option<(String, String)>,
    pub(super) editing: Option<Saved>,
    pub(super) dirty: bool,
    pub(super) discarded: Option<String>,
    /// Meeting occurrence key and message of the last Copy or Open result.
    pub(super) status: Option<(String, String)>,
    pub(super) focus: bool,
}

enum Action {
    Regenerate,
    /// Discards the saved draft with this occurrence key.
    Discard(String),
}

#[cfg(not(test))]
pub(super) fn open_mail_link(uri: &str) -> Result<(), String> {
    tomlook::browser::open_mailto(uri)
}

#[cfg(test)]
thread_local! {
    /// Links that UI tests "opened"; tests never start an e-mail app.
    pub(super) static OPENED_LINKS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(super) fn open_mail_link(uri: &str) -> Result<(), String> {
    tomlook::browser::validate_mailto(uri)?;
    OPENED_LINKS.with(|links| links.borrow_mut().push(uri.into()));
    Ok(())
}

const LONG_MESSAGE: &str = "The message is too long for an e-mail link. The body was copied to the clipboard; paste it into the new message.";

impl Tomlook {
    /// The signed-in account and the user's other mailboxes, never offered as recipients.
    pub(super) fn own_emails(&self) -> Vec<String> {
        let mut own = Vec::new();
        own.extend(self.calendar_owner.clone());
        if let Some(identity) = &self.activity.identity {
            own.push(identity.account.clone());
            own.extend(identity.mail.clone());
        }
        own.push(self.preferences.work_email.clone());
        own.retain(|email| !email.trim().is_empty());
        own
    }

    /// A pending follow-up recommendation for this meeting; the e-mail action here covers it.
    pub(super) fn follow_up_recommended(&self, event: &Event) -> bool {
        let now = Local::now().fixed_offset();
        self.activity.recommendations.values().any(|recommendation| {
            recommendation.event_id == event.id
                && recommendation.suggestion.kind == tomlook::recommendations::Kind::FollowUp
                && recommendation.visible(&self.calendar, self.calendar_owner.as_deref(), now)
        })
    }

    /// Opens a meeting's details and scrolls to its follow-up e-mail.
    pub(super) fn open_followup(&mut self, event: &Event) {
        self.open_recommended_meeting(event);
        self.insights_expanded = false;
        self.followup.focus = true;
    }

    /// Saves unsaved edits of the open follow-up e-mail, for example before another meeting opens.
    pub(super) fn flush_followup(&mut self) {
        if self.followup.dirty
            && let Some(draft) = self.followup.editing.as_mut()
            && draft.validate().is_ok()
        {
            draft.updated_at = chrono::Utc::now().max(draft.created_at);
            let draft = draft.clone();
            self.command(Command::SaveFollowUp(Box::new(draft)));
        }
        self.followup.dirty = false;
    }

    fn request_followup(&mut self, event: &Event, account: &str) {
        let key = tomlook::recaps::key(account, event);
        let Some(notes) = self.briefing.clone() else {
            return;
        };
        self.followup.request_id += 1;
        let id = self.followup.request_id;
        let sender = self
            .activity
            .identity
            .as_ref()
            .and_then(|identity| identity.display_name.clone());
        self.followup.status = None;
        match model::Request::new(
            id,
            account,
            event,
            &notes,
            self.preferences.language,
            &self.own_emails(),
            sender.as_deref(),
        ) {
            Ok(request) => {
                if self.ai_command(ai::Command::FollowUp(Box::new(request.clone()))) {
                    self.followup.running = Some(Running {
                        id,
                        request: Box::new(request),
                    });
                    self.followup.error = None;
                } else {
                    let error = self
                        .ai_error
                        .take()
                        .unwrap_or_else(|| "The AI worker is unavailable. Restart Tomlook.".into());
                    self.followup.error = Some((key, error));
                }
            }
            Err(error) => self.followup.error = Some((key, error)),
        }
    }

    pub(super) fn receive_followup(&mut self, id: u64, result: Result<model::Draft, String>) {
        let Some(running) = self
            .followup
            .running
            .take_if(|running| running.id == id)
        else {
            return;
        };
        let key = running.request.key.clone();
        match result {
            Ok(draft) => {
                let mut saved = Saved::new(&running.request, draft, chrono::Utc::now());
                let previous = self
                    .followup
                    .editing
                    .as_ref()
                    .filter(|previous| previous.event_id == saved.event_id && previous.account == saved.account)
                    .cloned();
                if let Some(previous) = &previous {
                    saved.excluded = previous
                        .excluded
                        .iter()
                        .filter(|email| saved.recipients.iter().any(|recipient| &recipient.email == *email))
                        .cloned()
                        .collect();
                }
                match saved.validate() {
                    Ok(()) => {
                        if let Some(previous) = previous.filter(|previous| previous.key != saved.key) {
                            self.command(Command::DiscardFollowUp { account: previous.account, key: previous.key });
                        }
                        self.command(Command::SaveFollowUp(Box::new(saved.clone())));
                        self.followup.editing = Some(saved);
                        self.followup.dirty = false;
                        self.followup.discarded = None;
                    }
                    Err(error) => self.followup.error = Some((key, error)),
                }
            }
            Err(error) => self.followup.error = Some((key, error)),
        }
    }

    pub(super) fn stop_followup(&mut self) {
        if let Some(running) = self.followup.running.take() {
            self.followup.error = Some((
                running.request.key,
                "AI stopped without a completed answer; provider outcome may be unknown".into(),
            ));
        }
    }

    fn cancel_followup(&mut self) {
        if let Some(running) = self.followup.running.take() {
            self.ai_command(ai::Command::Cancel(running.id | ai::FOLLOWUP_BIT));
            self.followup.error = Some((
                running.request.key,
                "Follow-up e-mail cancelled; nothing was saved.".into(),
            ));
        }
    }

    /// Follow-up e-mail of an ended meeting with saved notes, shown in its details.
    pub(super) fn followup_section(&mut self, ui: &mut egui::Ui, event: &Event) {
        let language = self.preferences.language;
        let Some(account) = self.feedback_account() else {
            return;
        };
        if self
            .briefing
            .as_ref()
            .and_then(|briefing| briefing.get("tomlook_recap"))
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            return;
        }
        let key = tomlook::recaps::key(&account, event);
        // The occurrence key changes when calendar metadata is enriched; the event identity remains.
        let mut saved = self
            .activity
            .followups
            .get(&key)
            .or_else(|| {
                self.activity
                    .followups
                    .values()
                    .find(|saved| saved.event_id == event.id)
            })
            .filter(|saved| saved.account == account)
            .cloned();
        match (&self.followup.discarded, &saved) {
            (Some(discarded), Some(found)) if found.key == *discarded => saved = None,
            (Some(_), _) => self.followup.discarded = None,
            _ => {}
        }
        match &self.followup.editing {
            Some(editing) if editing.event_id == event.id && editing.account == account => {
                if !self.followup.dirty
                    && saved.as_ref().is_some_and(|saved| saved != editing && saved.updated_at > editing.updated_at)
                {
                    self.followup.editing = saved;
                }
            }
            _ => {
                self.flush_followup();
                self.followup.editing = saved;
                self.followup.dirty = false;
            }
        }
        ui.add_space(14.0);
        ui.separator();
        let heading = ui.strong(language.text("Follow-up e-mail"));
        if self.followup.focus {
            heading.scroll_to_me(Some(egui::Align::Min));
            self.followup.focus = false;
        }
        if self.follow_up_recommended(event) {
            ui.label(
                RichText::new(language.text("Tomlook recommends a follow-up for this meeting."))
                    .color(self.accent()),
            );
        }
        ui.weak(language.text(
            "Local text only: Tomlook never sends e-mail or creates Outlook drafts.",
        ));
        let running = self
            .followup
            .running
            .as_ref()
            .is_some_and(|running| running.request.key == key);
        if running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(language.text("Preparing the follow-up e-mail..."));
                if ui.small_button(language.text("Stop")).clicked() {
                    self.cancel_followup();
                }
            });
        }
        if let Some((failed, message)) = &self.followup.error
            && *failed == key
        {
            ui.colored_label(ui.visuals().error_fg_color, language.text(message));
        }
        let ready = !self.preview && self.ai_ready;
        let unavailable = if self.preview {
            "Offline demo never sends AI requests."
        } else {
            "Connect GitHub Copilot first. No AI request was made."
        };
        if self.followup.editing.is_some() {
            match self.followup_editor(ui, &key, ready && !running, unavailable) {
                Some(Action::Regenerate) => self.request_followup(event, &account),
                Some(Action::Discard(saved_key)) => {
                    self.command(Command::DiscardFollowUp {
                        account: account.clone(),
                        key: saved_key.clone(),
                    });
                    self.followup.editing = None;
                    self.followup.dirty = false;
                    self.followup.status = None;
                    self.followup.discarded = Some(saved_key);
                }
                None => {}
            }
        } else if !running
            && ui
                .add_enabled(ready, egui::Button::new(language.text("Prepare follow-up e-mail")))
                .on_hover_text(language.text(
                    "Drafts the e-mail from these notes with one Quick AI request. Nothing is sent.",
                ))
                .on_disabled_hover_text(language.text(unavailable))
                .clicked()
        {
            self.request_followup(event, &account);
        }
    }

    fn followup_editor(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        can_regenerate: bool,
        unavailable: &str,
    ) -> Option<Action> {
        let language = self.preferences.language;
        let mut draft = self.followup.editing.take()?;
        let mut action = None;
        let mut save = false;
        let mut copy = false;
        let mut open = false;
        let accent = self.accent();
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(8)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(language.text(&format!(
                        "Recipients ({} of {})",
                        draft.selected().len(),
                        draft.recipients.len()
                    )))
                    .strong(),
                );
                if draft.recipients.is_empty() {
                    ui.weak(language.text(
                        "No participant has an e-mail address; add recipients in your e-mail app.",
                    ));
                }
                let mut toggled = Vec::new();
                let mut list = |ui: &mut egui::Ui| {
                    for recipient in &draft.recipients {
                        let mut include = !draft.excluded.contains(&recipient.email);
                        let mut label = if recipient.name.is_empty() {
                            recipient.email.clone()
                        } else {
                            format!("{} <{}>", recipient.name, recipient.email)
                        };
                        if recipient.organizer {
                            label.push_str(&format!(" · {}", language.text("Organizer")));
                        }
                        if recipient.external {
                            label.push_str(&format!(" · {}", language.text("External")));
                        }
                        if ui.checkbox(&mut include, label).changed() {
                            toggled.push((recipient.email.clone(), include));
                        }
                    }
                };
                // A nested scroll area inside the details panel is used only for long lists.
                if draft.recipients.len() > 12 {
                    egui::ScrollArea::vertical()
                        .id_salt(("followup-recipients", key))
                        .max_height(260.0)
                        .min_scrolled_height(260.0)
                        .show(ui, list);
                } else {
                    list(ui);
                }
                for (email, include) in toggled {
                    if include {
                        draft.excluded.remove(&email);
                    } else {
                        draft.excluded.insert(email);
                    }
                    save = true;
                }
                ui.add_space(6.0);
                let label = ui.label(language.text("Subject"));
                let subject = ui
                    .add(
                        egui::TextEdit::singleline(&mut draft.subject)
                            .id(Id::new(("followup-subject", key)))
                            .desired_width(f32::INFINITY)
                            .char_limit(model::SUBJECT_LIMIT),
                    )
                    .labelled_by(label.id);
                let label = ui.label(language.text("Message"));
                let body = ui
                    .add(
                        egui::TextEdit::multiline(&mut draft.body)
                            .id(Id::new(("followup-body", key)))
                            .desired_rows(12)
                            .desired_width(f32::INFINITY)
                            .char_limit(model::BODY_LIMIT),
                    )
                    .labelled_by(label.id);
                if subject.changed() || body.changed() {
                    self.followup.dirty = true;
                }
                let valid = draft.validate();
                if (subject.lost_focus() || body.lost_focus()) && self.followup.dirty {
                    save = true;
                }
                if let Err(error) = &valid {
                    ui.colored_label(ui.visuals().error_fg_color, language.text(error));
                }
                ui.horizontal_wrapped(|ui| {
                    copy = ui
                        .add_enabled(valid.is_ok(), egui::Button::new(language.text("Copy")))
                        .on_hover_text(language.text("Copies the subject and message to the clipboard"))
                        .clicked();
                    open = ui
                        .add_enabled(valid.is_ok(), egui::Button::new(language.text("Open in Outlook")))
                        .on_hover_text(language.text(
                            "Opens a new message in your default e-mail app. Nothing is sent automatically.",
                        ))
                        .clicked();
                    if ui
                        .add_enabled(can_regenerate, egui::Button::new(language.text("Regenerate")))
                        .on_hover_text(language.text("Replaces this text with a new AI draft"))
                        .on_disabled_hover_text(language.text(unavailable))
                        .clicked()
                    {
                        action = Some(Action::Regenerate);
                    }
                    if ui.button(language.text("Discard")).clicked() {
                        action = Some(Action::Discard(draft.key.clone()));
                    }
                });
                if let Some((shown, message)) = &self.followup.status
                    && shown == key
                {
                    ui.label(RichText::new(language.text(message)).color(accent));
                }
            });
        let valid = draft.validate().is_ok();
        if copy && valid {
            ui.ctx().copy_text(draft.clipboard());
            self.followup.status = Some((key.into(), "Copied to the clipboard.".into()));
            save |= self.followup.dirty;
        }
        if open && valid {
            match draft.mailto() {
                Ok(link) => {
                    if link.body_in_clipboard {
                        ui.ctx().copy_text(draft.body.clone());
                    }
                    match open_mail_link(&link.uri) {
                        Ok(()) => {
                            self.followup.status = Some((
                                key.into(),
                                if link.body_in_clipboard {
                                    LONG_MESSAGE
                                } else {
                                    "Opened in your e-mail app. Nothing was sent."
                                }
                                .into(),
                            ));
                        }
                        Err(error) => self.followup.status = Some((key.into(), error)),
                    }
                }
                Err(error) => self.followup.status = Some((key.into(), error)),
            }
            save |= self.followup.dirty;
        }
        if save && valid && action.is_none() {
            draft.updated_at = chrono::Utc::now().max(draft.created_at);
            self.command(Command::SaveFollowUp(Box::new(draft.clone())));
            self.followup.dirty = false;
        }
        if !matches!(action, Some(Action::Discard(_))) {
            self.followup.editing = Some(draft);
        }
        action
    }

    /// Opens an Ask e-mail draft in the user's e-mail app; recipients are plain addresses found
    /// in the draft's target.
    pub(super) fn open_email_draft(
        &mut self,
        proposal: &tomlook::proposals::Proposal,
        context: &egui::Context,
    ) {
        let addresses = model::addresses(&proposal.target);
        let to = addresses.iter().map(String::as_str).collect::<Vec<_>>();
        match model::mailto(&to, &proposal.title, &proposal.body) {
            Ok(link) => {
                if link.body_in_clipboard {
                    context.copy_text(proposal.body.clone());
                    self.ai_error = Some(LONG_MESSAGE.into());
                }
                if let Err(error) = open_mail_link(&link.uri) {
                    self.ai_error = Some(error);
                }
            }
            Err(error) => self.ai_error = Some(error),
        }
    }
}
