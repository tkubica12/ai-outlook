use super::*;

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
    focus_title: bool,
}

fn selected_times(start: u16, end: u16) -> (u16, u16) {
    let first = start.min(end).min(1425);
    let last = start.max(end).min(1440);
    (first, if last > first { last } else { (first + 30).min(1440) })
}

impl EventDraft {
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
        };
        draft.validate()?;
        Ok(draft)
    }
}

impl Tomlook {
    pub(super) fn open_event_draft(&mut self, day: NaiveDate, start: u16, end: u16) {
        self.event_draft = Some(EventDraft {
            parent: None,
            account: self.calendar_owner.clone(), day, start, end,
            title: String::new(), description: String::new(), category: None,
            private: false, teams: false, travel: false, travel_minutes: 30, focus_title: true,
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

    pub(super) fn event_creation_dialog(&mut self, context: &egui::Context) {
        let Some(mut draft) = self.event_draft.take() else { return };
        let language = self.preferences.language;
        let categories = tomlook::recommendations::known_categories(&self.calendar).into_iter()
            .filter(|category| !category.eq_ignore_ascii_case("follow")).collect::<Vec<_>>();
        let mut open = true;
        let mut cancel = false;
        let mut confirm = false;
        let travel_only = draft.parent.is_some();
        egui::Window::new(language.text(if travel_only { "Add travel blocks before and after" } else { "New event" }))
            .id(Id::new("create-calendar-event")).open(&mut open)
            .default_width(460.0).resizable(false).collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
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
                let time = |minutes: u16| format!("{:02}:{:02}", minutes / 60, minutes % 60);
                ui.horizontal(|ui| {
                    ui.label(language.text("Start"));
                    ui.strong(time(draft.start));
                });
                if ui.add(egui::Slider::new(&mut draft.start, 0..=1425).step_by(15.0).show_value(false)
                    .text(language.text("Start"))).changed() && draft.end <= draft.start {
                    draft.end = (draft.start + 30).min(1440);
                }
                ui.horizontal(|ui| {
                    ui.label(language.text("End"));
                    ui.strong(time(draft.end));
                });
                ui.add(egui::Slider::new(&mut draft.end, 15..=1440).step_by(15.0).show_value(false)
                    .text(language.text("End")));
                let label = ui.label(language.text("Description (optional)"));
                ui.add(egui::TextEdit::multiline(&mut draft.description).desired_rows(2)
                    .desired_width(f32::INFINITY).char_limit(8000)).labelled_by(label.id);
                ui.label(language.text("Category"));
                egui::Grid::new("new-event-categories").num_columns(3).spacing([6.0, 6.0]).show(ui, |ui| {
                    for (index, category) in std::iter::once(None).chain(categories.iter().map(Some)).enumerate() {
                        let label = category.map_or_else(|| language.text("None").into_owned(), Clone::clone);
                        let selected = draft.category.as_ref() == category;
                        let fill = category.map(|category| category_color(category, self.preferences.dark).gamma_multiply(if self.preferences.dark { 0.3 } else { 0.15 }))
                            .unwrap_or(ui.visuals().faint_bg_color);
                        let button = egui::Button::new(label).fill(fill)
                            .stroke(if selected { Stroke::new(2.0, self.accent()) } else { Stroke::NONE }).truncate();
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
                let unchanged = draft.account.is_some() && draft.account == self.calendar_owner
                    && draft.parent.as_ref().is_none_or(|event| {
                        self.calendar.event_index(&event.id).is_some_and(|index|
                            tomlook::scheduler::fingerprint(&self.calendar.events[index]) == tomlook::scheduler::fingerprint(event))
                    });
                if !unchanged && travel_only {
                    ui.weak(language.text("Meeting or account changed; close this dialog and review the current meeting."));
                }
                let ready = unchanged && self.workiq_ready && self.ai.is_some() && !self.preview
                    && !self.activity.event_creations.values().any(|record| record.state == tomlook::event_creation::State::Creating);
                if !ready { ui.weak(language.text("Connect WorkIQ before confirming a calendar change")); }
                if let Err(error) = &value && (travel_only || !draft.title.trim().is_empty()) {
                    ui.colored_label(ui.visuals().error_fg_color, language.text(error));
                }
                ui.horizontal(|ui| {
                    confirm = ui.add_enabled(ready && value.is_ok(), egui::Button::new(language.text(if travel_only { "Create travel blocks" } else { "Create event" }))).clicked();
                    cancel = ui.button(language.text("Back")).clicked();
                });
            });
        if confirm {
            if let (Some(account), Ok(value)) = (draft.account.clone(), draft.value(language)) {
                let submitted = self.worker.as_ref().is_some_and(|worker| worker.commands.try_send(Command::CreateEvents {
                    account, draft: Box::new(value), parent: draft.parent.clone().map(Box::new),
                }).is_ok());
                if submitted {
                    self.messages_open = true;
                } else {
                    self.error = Some("Event creation could not be queued; nothing was submitted".into());
                    self.event_draft = Some(draft);
                }
            }
        } else if open && !cancel {
            self.event_draft = Some(draft);
        }
    }

    pub(super) fn select_event_time(&mut self, ui: &mut egui::Ui, day: NaiveDate, rect: egui::Rect, cards: &[egui::Rect]) {
        if self.event_draft.is_some() || self.meeting_action.is_some() || self.meeting_menu.is_some()
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
