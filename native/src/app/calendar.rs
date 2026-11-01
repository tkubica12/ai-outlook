use super::*;

pub(super) const MIN_EVENT_LANE_WIDTH: f32 = 72.0;

fn focus_logo(
    ui: &mut egui::Ui,
    color: Color32,
    language: tomlook::i18n::Language,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let scale = rect.width() / 64.0;
        let point = |[x, y]: [f32; 2]| rect.min + Vec2::new(x, y) * scale;
        let painter = ui.painter();
        for corner in tomlook::brand::CORNERS {
            for segment in corner.windows(2) {
                painter.line_segment(
                    [point(segment[0]), point(segment[1])],
                    Stroke::new(6.0 * scale, color),
                );
            }
            for position in corner {
                painter.circle_filled(point(position), 3.0 * scale, color);
            }
        }
        for [left, top, right, bottom] in tomlook::brand::LETTER {
            painter.rect_filled(
                egui::Rect::from_min_max(point([left, top]), point([right, bottom])),
                0.0,
                color,
            );
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            ui.is_enabled(),
            language.text("Tomlook Focus logo"),
        )
    });
    response
}

pub(super) fn timeline_content_width(column_width: f32, slots: &[tomlook::calendar::Slot]) -> f32 {
    let lanes = slots
        .iter()
        .map(|slot| slot.lanes)
        .max()
        .unwrap_or(1)
        .max(1);
    column_width.max(lanes as f32 * MIN_EVENT_LANE_WIDTH)
}

impl Tomlook {
    pub(super) fn shortcuts(&mut self, context: &egui::Context) {
        if !context.input(|input| input.focused) {
            self.time_drag = None;
        }
        let consume = |modifiers, key| context.input_mut(|input| input.consume_key(modifiers, key));
        if consume(Modifiers::CTRL | Modifiers::SHIFT, Key::Q) {
            self.exit(context);
        }
        if consume(Modifiers::CTRL | Modifiers::SHIFT, Key::P) {
            self.open_palette();
        }
        if consume(Modifiers::CTRL, Key::F) {
            self.close_palette(context);
            self.focus_search = true;
        }
        if consume(Modifiers::CTRL, Key::Space) {
            self.toggle_assistant(context);
        }
        if consume(Modifiers::CTRL, Key::D) {
            self.close_palette(context);
            self.toggle_myday();
        }
        let slash_menu = self.assistant
            && !self.assistant_detached
            && !self.ask_menu_dismissed
            && !ask::matches(&self.question, self.preferences.language).is_empty()
            && context.memory(|memory| {
                memory.focused() == Some(Id::new("assistant-question"))
                    || memory.had_focus_last_frame(Id::new("assistant-question"))
            });
        if !slash_menu && consume(Modifiers::NONE, Key::Escape) {
            if self.event_draft.take().is_some()
                || self.time_drag.take().is_some()
                || self.meeting_action.take().is_some()
                || self.meeting_menu.take().is_some()
                || self.date_drag.take().is_some()
                || self.tasks.editing.take().is_some()
            {
                context.request_repaint();
            } else if self.palette {
                self.close_palette(context);
            } else if self.tasks_open {
                self.tasks_open = false;
            } else if self.weekly.open {
                self.weekly.open = false;
            } else if self.myday.open {
                self.myday.open = false;
            } else if self.insights_expanded {
                self.insights_expanded = false;
            } else if self.messages_open {
                self.messages_open = false;
            } else if self.preferences_open {
                self.preferences_open = false;
            } else if self.settings {
                self.settings = false;
            } else if self.connection_panel.is_some() {
                self.connection_panel = None;
                self.focus_search = true;
            } else if self.assistant {
                self.toggle_assistant(context);
            } else if self.selected.is_some() {
                self.selected = None;
                self.detail_revision += 1;
            } else {
                self.query.clear();
            }
        }
        let editing = context.memory(|memory| {
            memory.focused().is_some_and(|id| {
                [
                    "calendar-search",
                    "assistant-question",
                    "workiq-account",
                    "context-folders",
                    "command-search",
                    "task-quick-add",
                ]
                .into_iter()
                .any(|name| id == Id::new(name))
            })
        });
        let search_focused =
            context.memory(|memory| memory.focused() == Some(Id::new("calendar-search")));
        if (editing && !search_focused)
            || self.palette
            || self.settings
            || self.messages_open
            || self.tasks_open
            || self.myday.open
            || self.weekly.open
            || self.meeting_action.is_some()
            || self.event_draft.is_some()
            || self.meeting_menu.is_some()
            || self.connection_panel.is_some()
        {
            return;
        }
        if consume(Modifiers::SHIFT, Key::F10) {
            if let Some(index) = self.selected {
                self.meeting_menu = Some(self.calendar.events[index].id.clone());
            }
            return;
        }
        if consume(Modifiers::ALT, Key::ArrowLeft) {
            self.shift_period(-1);
        }
        if consume(Modifiers::ALT, Key::ArrowRight) {
            self.shift_period(1);
        }
        if search_focused
            && !context
                .input(|input| input.key_pressed(Key::ArrowUp) || input.key_pressed(Key::ArrowDown))
        {
            return;
        }
        let up = consume(Modifiers::NONE, Key::ArrowUp);
        let down = consume(Modifiers::NONE, Key::ArrowDown);
        if up || down {
            let days = self.visible_days();
            let query = tomlook::calendar::SearchQuery::new(&self.query);
            let visible = days
                .iter()
                .filter_map(|day| self.calendar.days.get(day))
                .flat_map(|day| day.events.iter().copied())
                .filter(|i| self.matches_event(*i, &query))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if !visible.is_empty() {
                let current = self
                    .selected
                    .and_then(|id| visible.iter().position(|i| *i == id));
                let next = match current {
                    None => 0,
                    Some(position) if up => position.saturating_sub(1),
                    Some(position) => (position + 1).min(visible.len() - 1),
                };
                self.select(visible[next]);
                self.focus_meeting = true;
                self.scroll_meeting = true;
            }
        }
    }

    pub(super) fn visible_days(&self) -> Vec<NaiveDate> {
        self.date_drag
            .map(|(first, last)| DateSelection::new(first, last))
            .or(self.date_selection)
            .map_or_else(
                || self.preferences.view.days(self.anchor),
                DateSelection::days,
            )
    }

    pub(super) fn mini_selected(&self, date: NaiveDate) -> bool {
        if let Some((first, last)) = self.date_drag {
            let range = DateSelection::new(first, last);
            return date >= range.first && date <= range.last;
        }
        if let Some(range) = self.date_selection {
            return date >= range.first && date <= range.last;
        }
        if self.preferences.view == View::Month {
            return date.month() == self.anchor.month() && date.year() == self.anchor.year();
        }
        let days = self.preferences.view.days(self.anchor);
        date >= days[0] && date <= *days.last().expect("nonempty view")
    }

    pub(super) fn matches_event(&self, index: usize, query: &tomlook::calendar::SearchQuery) -> bool {
        self.calendar.matches_query(index, query, &self.category)
            && !(self.preferences.hide_unconfirmed && self.calendar.events[index].unconfirmed())
    }

    pub(super) fn choose_dates(&mut self, start: NaiveDate, end: NaiveDate) {
        let selection = DateSelection::new(start, end);
        if (selection.last - selection.first).num_days() > 41 {
            self.error = Some("Select at most 42 consecutive days.".into());
            return;
        }
        self.anchor = selection.first;
        self.date_selection = (selection.first != selection.last).then_some(selection);
        self.preferences.view = View::Day;
        if self.selected.is_some_and(|index| {
            let day = self.calendar.events[index]
                .start
                .with_timezone(&Local)
                .date_naive();
            day < selection.first || day > selection.last
        }) {
            self.selected = None;
            self.detail_revision += 1;
            self.briefing = None;
        }
        self.save_preferences();
    }

    pub(super) fn choose_view(&mut self, view: View) {
        self.date_drag = None;
        self.date_selection = None;
        self.preferences.view = view;
        self.save_preferences();
    }

    pub(super) fn shift_period(&mut self, direction: i32) {
        self.mini_anchor = None;
        if let Some(selection) = self.date_selection {
            let shift = chrono::Duration::days(
                ((selection.last - selection.first).num_days() + 1) * i64::from(direction),
            );
            self.date_selection = Some(DateSelection {
                first: selection.first + shift,
                last: selection.last + shift,
            });
            self.anchor = selection.first + shift;
        } else {
            self.anchor = self.preferences.view.shift(self.anchor, direction);
        }
    }

    pub(super) fn go_today(&mut self) {
        self.mini_anchor = None;
        let today = Local::now().date_naive();
        if let Some(selection) = self.date_selection {
            self.date_selection = Some(DateSelection {
                first: today,
                last: today + (selection.last - selection.first),
            });
        }
        self.anchor = today;
    }

    pub(super) fn header(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        ui.horizontal(|ui| {
            focus_logo(ui, self.accent(), language);
            ui.label(RichText::new(language.text("Tomlook")).size(22.0).strong());
            if self.preview {
                ui.weak(language.text("Demo"));
            }
            ui.add_space(24.0);
            if ui.button(language.text("Today")).clicked() {
                self.go_today();
            }
            if ui.button(language.text("New event"))
                .on_hover_text(language.text("Click or drag an empty time in the calendar"))
                .clicked() {
                self.open_event_draft(self.anchor, 9 * 60, 9 * 60 + 30);
            }
            if ui
                .button(language.text("<"))
                .on_hover_text(language.text("Previous period - Alt+Left"))
                .clicked()
            {
                self.shift_period(-1);
            }
            if ui
                .button(language.text(">"))
                .on_hover_text(language.text("Next period - Alt+Right"))
                .clicked()
            {
                self.shift_period(1);
            }
            let days = self.visible_days();
            let label = if self.preferences.view == View::Month
                && self.date_selection.is_none()
                && self.date_drag.is_none()
            {
                language.month(self.anchor)
            } else if days.len() == 1 {
                language.date(self.anchor, true)
            } else {
                language.range(days[0], *days.last().expect("view is nonempty"))
            };
            ui.label(RichText::new(language.text(&(label))).size(17.0).strong());
            if matches!(self.preferences.view, View::Week | View::WorkWeek)
                && self.date_selection.is_none()
                && self.date_drag.is_none()
            {
                let week = tomlook::weekly::week_of(days[0]);
                if ui
                    .add_enabled(
                        week <= self.current_week(),
                        egui::Button::new(RichText::new(language.text("Weekly review")).small()),
                    )
                    .on_hover_text(language.text("Review the displayed week"))
                    .on_disabled_hover_text(language.text("Future weeks cannot be reviewed yet."))
                    .clicked()
                {
                    self.open_weekly(Some(week));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if header_icon(ui, "Commands", HeaderIcon::Commands, language, false).clicked() {
                    self.open_palette();
                }
                if header_icon(ui, "Settings", HeaderIcon::Settings, language, false).clicked() {
                    self.prompt_draft = self.preferences.prompts.clone();
                    self.preferences_open = true;
                }
                if header_icon(ui, "Ask", HeaderIcon::Ask, language,
                    self.active_request.is_some() || self.chat.running() || self.ask_unread)
                    .on_hover_text(language.text(if self.active_request.is_some() || self.chat.running() {
                        "Thinking..."
                    } else if self.ask_unread { "Answer ready" } else { "Ctrl+Space" })).clicked() {
                    self.toggle_assistant(ui.ctx());
                }
                let open = self.open_task_count();
                let tasks = header_icon(ui, "Tasks", HeaderIcon::Tasks, language, false);
                if open > 0 {
                    count_badge(ui, tasks.rect, open);
                }
                let tasks = if open > 0 {
                    tasks.on_hover_text(language.text(&format!("{open} open tasks")))
                } else {
                    tasks
                };
                if tasks.clicked() {
                    if self.tasks_open {
                        self.tasks_open = false;
                    } else {
                        self.open_tasks(false);
                    }
                }
                if header_icon(ui, "My day", HeaderIcon::MyDay, language, false)
                    .on_hover_text("Ctrl+D")
                    .clicked()
                {
                    self.toggle_myday();
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (view, label) in View::ALL {
                if ui
                    .selectable_label(
                        self.date_selection.is_none()
                            && self.date_drag.is_none()
                            && self.preferences.view == view,
                        language.text(label),
                    )
                    .clicked()
                {
                    self.choose_view(view);
                }
            }
            ui.add_space(16.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(Id::new("calendar-search"))
                    .hint_text(language.text("Search meetings"))
                    .desired_width(230.0),
            );
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }
            if !self.query.is_empty() && ui.button(language.text("Clear")).clicked() {
                self.query.clear();
            }
            if ui
                .checkbox(
                    &mut self.preferences.hide_unconfirmed,
                    language.text("Hide unconfirmed"),
                )
                .changed()
            {
                self.save_preferences();
            }
        });
    }

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        ui.add_space(8.0);
        let mut month = self.mini_anchor.unwrap_or(self.anchor);
        ui.horizontal(|ui| {
            let previous = ui.small_button("<").on_hover_text(language.text("Previous month"));
            previous.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, language.text("Previous month")));
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(Id::new("mini-previous-month"), previous.rect));
            if previous.clicked() {
                month = View::Month.shift(month, -1);
                self.mini_anchor = Some(month);
            }
            ui.label(RichText::new(language.month(month)).strong().color(self.accent()));
            let next = ui.small_button(">").on_hover_text(language.text("Next month"));
            next.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, language.text("Next month")));
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(Id::new("mini-next-month"), next.rect));
            if next.clicked() {
                month = View::Month.shift(month, 1);
                self.mini_anchor = Some(month);
            }
        });
        let dates = View::Month.days(month);
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let pressed_at = ui.input(|input| {
            input.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some(*pos),
                _ => None,
            })
        });
        let down = ui.input(|input| input.pointer.primary_down());
        let released =
            ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
        if !ui.input(|input| input.focused) && !released {
            self.date_drag = None;
        }
        let mut pressed_date = None;
        let mut pointer_date = None;
        ui.scope(|ui| {
            ui.spacing_mut().button_padding = Vec2::new(1.0, 2.0);
            ui.style_mut().override_font_id = Some(egui::FontId::proportional(11.0));
            egui::Grid::new("mini-calendar")
                .min_col_width(0.0)
                .spacing([3.0, 4.0])
                .show(ui, |ui| {
                    ui.weak(language.text(""));
                    for day in if language == tomlook::i18n::Language::Cs {
                        ["Po", "Út", "St", "Čt", "Pá", "So", "Ne"]
                    } else {
                        ["M", "T", "W", "T", "F", "S", "S"]
                    } {
                        ui.weak(language.text(day));
                    }
                    ui.end_row();
                    for (i, date) in dates.iter().enumerate() {
                        if i % 7 == 0 {
                            let response = ui
                                .add_sized([16.0, 25.0], egui::Button::new("›"))
                                .on_hover_text(language.text(&format!(
                                    "Show work week of {}",
                                    language.date(*date, true)
                                )));
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    language.text(&format!(
                                        "Show work week of {}",
                                        language.date(*date, true)
                                    )),
                                )
                            });
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(Id::new(("mini-week", *date)), response.rect)
                            });
                            if response.clicked() {
                                self.anchor = *date;
                                self.choose_view(View::WorkWeek);
                            }
                        }
                        let text = RichText::new(language.text(&(date.day().to_string()))).color(
                            if *date == Local::now().date_naive() {
                                self.accent()
                            } else if date.month() == month.month() {
                                ui.visuals().text_color()
                            } else {
                                ui.visuals().weak_text_color()
                            },
                        );
                        let response = ui
                            .add_sized(
                                [20.0, 25.0],
                                egui::Button::new(text)
                                    .selected(self.mini_selected(*date))
                                    .sense(egui::Sense::click_and_drag()),
                            )
                            .on_hover_text(language.date(*date, true));
                        #[cfg(test)]
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(Id::new(("mini-date", *date)), response.rect)
                        });
                        if pressed_at.is_some_and(|position| {
                            response.rect.contains(position)
                                && ui.clip_rect().contains(position)
                                && ui.ctx().layer_id_at(position) == Some(ui.layer_id())
                        }) && ui.is_enabled()
                        {
                            pressed_date = Some(*date);
                        }
                        if (down || released)
                            && pointer.is_some_and(|position| {
                                response.rect.contains(position)
                                    && ui.clip_rect().contains(position)
                            })
                        {
                            pointer_date = Some(*date);
                        }
                        if response.clicked() && self.date_drag.is_none() && pressed_at.is_none() {
                            let start = if ui.input(|input| input.modifiers.shift) {
                                self.anchor
                            } else {
                                *date
                            };
                            self.choose_dates(start, *date);
                            ui.ctx().request_repaint();
                        }
                        if i % 7 == 6 {
                            ui.end_row();
                        }
                    }
                });
            if let Some(date) = pressed_date {
                let start = if ui.input(|input| input.modifiers.shift) {
                    self.anchor
                } else {
                    date
                };
                self.date_drag = Some((start, date));
                ui.ctx().request_repaint();
            }
            if let Some(date) = pointer_date
                && let Some((_, end)) = &mut self.date_drag
                && *end != date
            {
                *end = date;
                ui.ctx().request_repaint();
            }
            if released && let Some((start, end)) = self.date_drag.take() {
                self.choose_dates(start, end);
                ui.ctx().request_repaint();
            }
        });
        ui.add_space(24.0);
        ui.weak(language.text("CATEGORIES"));
        if ui
            .selectable_label(self.category.is_empty(), language.text("All meetings"))
            .clicked()
        {
            self.category.clear();
        }
        let calendar = self.calendar.clone();
        for category in &calendar.categories {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    2.0,
                    category_color(category, self.preferences.dark),
                );
                if ui
                    .selectable_label(self.category == *category, language.text(category))
                    .clicked()
                {
                    self.category = category.clone();
                }
            });
        }
    }

    fn event_button(&mut self, ui: &mut egui::Ui, index: usize, date: NaiveDate, size: Vec2) {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.push_id(("meeting-day", date), |ui| self.event_card(ui, index, rect));
    }

    pub(super) fn event_card(&mut self, ui: &mut egui::Ui, index: usize, rect: egui::Rect) {
        let language = self.preferences.language;
        if !ui.is_rect_visible(rect) && self.selected != Some(index) {
            if let Some(profile) = &mut self.render_profile {
                profile.culled_cards += 1;
            }
            return;
        }
        let time_started = self
            .render_profile
            .as_ref()
            .map(|_| std::time::Instant::now());
        let event = &self.calendar.events[index];
        let ghost = event.unconfirmed();
        let rect = if ghost {
            egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), rect.height().min(38.0)))
        } else {
            rect
        };
        let time = if event.all_day_like() {
            language.text("All day").into_owned()
        } else {
            format!(
                "{} - {}",
                event.start.with_timezone(&Local).format("%H:%M"),
                event.end.with_timezone(&Local).format("%H:%M")
            )
        };
        let short_time = || {
            if event.all_day_like() {
                time.clone()
            } else {
                event
                    .start
                    .with_timezone(&Local)
                    .format("%H:%M")
                    .to_string()
            }
        };
        if let (Some(profile), Some(started)) = (&mut self.render_profile, time_started) {
            profile.cards += 1;
            profile.card_time_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        let widget_started = self
            .render_profile
            .as_ref()
            .map(|_| std::time::Instant::now());
        let response = ui
            .scope_builder(
                egui::UiBuilder::new().id(ui.id().with(("meeting-card", &event.id))),
                |ui| {
                    ui.put(
                        rect,
                        egui::Button::new(language.text(""))
                            .min_size(rect.size())
                            .fill(if ghost {
                                if self.preferences.dark { Color32::from_gray(31) } else { Color32::from_gray(247) }
                            } else {
                                ui.visuals().widgets.inactive.weak_bg_fill
                            })
                            .stroke(if ghost {
                                Stroke::NONE
                            } else {
                                ui.visuals().widgets.inactive.bg_stroke
                            })
                            .selected(self.selected == Some(index))
                            .truncate(),
                    )
                },
            )
            .inner;
        // The custom card paints its title once; accessibility does not need a hidden text galley.
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                ui.is_enabled(),
                self.selected == Some(index),
                format!(
                    "{time} {}{}",
                    event.title,
                    if ghost {
                        format!(" – {}", language.text(event.response_label()))
                    } else {
                        String::new()
                    }
                ),
            )
        });
        let response = response.on_hover_ui(|ui| {
            ui.strong(&event.title);
            ui.label(language.text(&time));
            if ghost {
                ui.weak(language.text(event.response_label()));
            }
            if !event.location.is_empty() {
                ui.label(&event.location);
            }
        });
        if let (Some(profile), Some(started)) = (&mut self.render_profile, widget_started) {
            profile.card_widget_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        let paint_started = self
            .render_profile
            .as_ref()
            .map(|_| std::time::Instant::now());
        if self.selected == Some(index) {
            if self.focus_meeting {
                response.request_focus();
                self.focus_meeting = false;
            }
            if self.scroll_meeting {
                response.scroll_to_me(Some(egui::Align::Center));
                self.scroll_meeting = false;
            }
        }
        if ui.is_rect_visible(rect) {
            if ghost {
                let painter = ui.painter().with_clip_rect(rect.shrink(1.0));
                let color = if self.preferences.dark { Color32::from_gray(39) } else { Color32::from_gray(237) };
                let mut x = rect.left() - rect.height();
                while x < rect.right() {
                    painter.line_segment([egui::pos2(x, rect.bottom()), egui::pos2(x + rect.height(), rect.top())],
                        Stroke::new(1.0, color));
                    x += 9.0;
                }
            }
            let notes_ready = self.calendar_owner.as_deref().and_then(|account| self.activity.recaps.record(account, event))
                .is_some_and(|record| record.payload.is_some());
            let prepared = !notes_ready && event.accepted() && event.end > Local::now().fixed_offset()
                && self.activity.queue.jobs.get(&event.id).is_some_and(|job| job.state == tomlook::scheduler::State::Completed);
            if notes_ready || prepared {
                let badge = egui::Rect::from_min_size(rect.right_top() + Vec2::new(-19.0, 3.0), Vec2::splat(15.0));
                insight_badge(ui, badge, notes_ready, self.accent(), language);
            }
            let content = rect.shrink2(Vec2::new(7.0_f32.min(rect.width() * 0.15), 4.0));
            let mut y = content.top();
            let wide_row = !ghost && content.width() >= 85.0 && content.height() < 31.0;
            let mut x = content.left();
            let mut width = (content.width() - if notes_ready || prepared { 15.0 } else { 0.0 }).max(1.0);
            if wide_row {
                let (label, space) = if content.width() >= 205.0 || event.all_day_like() {
                    (time.clone(), 83.0)
                } else {
                    (
                        event
                            .start
                            .with_timezone(&Local)
                            .format("%H:%M")
                            .to_string(),
                        36.0,
                    )
                };
                ui.painter().with_clip_rect(content).text(
                    egui::pos2(x, y),
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(10.0),
                    ui.visuals().weak_text_color(),
                );
                x += space;
                width -= space;
            } else if content.height() >= 31.0 || ghost && content.height() >= 28.0 {
                let label = if ghost {
                    format!(
                        "{} · {}",
                        short_time(),
                        language.text(event.response_label())
                    )
                } else if content.width() < 105.0 && !event.all_day_like() {
                    event
                        .start
                        .with_timezone(&Local)
                        .format("%H:%M")
                        .to_string()
                } else {
                    time.clone()
                };
                ui.painter().with_clip_rect(content).text(
                    egui::pos2(x, y),
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(10.0),
                    ui.visuals().weak_text_color(),
                );
                y += 14.0;
            }
            let rows = if ghost {
                1
            } else {
                (((content.bottom() - y) / 14.0).floor() as usize).clamp(1, 6)
            };
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width = width.max(1.0);
            job.wrap.max_rows = rows;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('\u{2026}');
            let title = if ghost && content.height() < 28.0 {
                std::borrow::Cow::Owned(format!(
                    "{} · {} · {}",
                    short_time(),
                    language.text(event.response_label()),
                    event.title
                ))
            } else {
                std::borrow::Cow::Borrowed(event.title.as_str())
            };
            job.append(
                &title,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(if content.width() < 85.0 {
                        10.0
                    } else {
                        12.0
                    }),
                    color: if ghost {
                        ui.visuals().weak_text_color()
                    } else {
                        ui.visuals().text_color()
                    },
                    ..Default::default()
                },
            );
            let layout_started = self
                .render_profile
                .as_ref()
                .map(|_| std::time::Instant::now());
            let galley = ui.painter().layout_job(job);
            if let (Some(profile), Some(started)) = (&mut self.render_profile, layout_started) {
                profile.title_layout_ms += started.elapsed().as_secs_f64() * 1000.0;
            }
            let title_height = galley.size().y;
            ui.painter().with_clip_rect(content).galley(
                egui::pos2(x, y),
                galley,
                ui.visuals().text_color(),
            );
            if !ghost && content.bottom() - y - title_height >= 20.0 {
                let summary = self
                    .briefing
                    .as_ref()
                    .filter(|_| self.selected == Some(index))
                    .and_then(|value| language.insight(value))
                    .and_then(|value| value.get("summary"))
                    .and_then(|value| value.as_str())
                    .unwrap_or(&event.location);
                if !summary.is_empty() {
                    let mut job = egui::text::LayoutJob::default();
                    job.wrap.max_width = width.max(1.0);
                    job.wrap.max_rows =
                        ((content.bottom() - y - title_height - 4.0) / 13.0).floor() as usize;
                    job.wrap.overflow_character = Some('\u{2026}');
                    job.append(
                        summary,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(10.0),
                            color: ui.visuals().weak_text_color(),
                            ..Default::default()
                        },
                    );
                    let galley = ui.painter().layout_job(job);
                    ui.painter().with_clip_rect(content).galley(
                        egui::pos2(x, y + title_height + 4.0),
                        galley,
                        ui.visuals().weak_text_color(),
                    );
                }
            }
            if !ghost {
                ui.painter().line_segment(
                    [rect.left_top(), rect.left_bottom()],
                    Stroke::new(
                        3.0,
                        event
                            .tags()
                            .first()
                            .map(|tag| category_color(tag, self.preferences.dark))
                            .unwrap_or_else(|| self.accent()),
                    ),
                );
            }
        }
        if let (Some(profile), Some(started)) = (&mut self.render_profile, paint_started) {
            profile.card_paint_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        if response.clicked() {
            self.select(index);
        }
        response.context_menu(|ui| self.meeting_actions(ui, index));
    }

    pub(super) fn month(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let dates = View::Month.days(self.anchor);
        let query = tomlook::calendar::SearchQuery::new(&self.query);
        let width = (ui.available_width() - 6.0 * 6.0) / 7.0;
        let height = ((ui.available_height() - 30.0 - 5.0 * 6.0) / 6.0).max(94.0);
        ui.horizontal(|ui| {
            for weekday in [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ] {
                ui.add_sized(
                    [width, 24.0],
                    egui::Label::new(RichText::new(language.text(weekday)).small()),
                );
            }
        });
        for week in dates.chunks(7) {
            ui.horizontal_top(|ui| {
                for date in week {
                    let frame = egui::Frame::new().inner_margin(6).stroke(Stroke::new(
                        1.0,
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                    ));
                    frame.show(ui, |ui| {
                        ui.set_min_size(Vec2::new((width - 12.0).max(40.0), height - 12.0));
                        ui.set_max_width((width - 12.0).max(40.0));
                        if ui
                            .button(
                                RichText::new(language.text(&(date.day().to_string()))).color(
                                    if *date == Local::now().date_naive() {
                                        self.accent()
                                    } else {
                                        ui.visuals().text_color()
                                    },
                                ),
                            )
                            .clicked()
                        {
                            self.choose_dates(*date, *date);
                        }
                        let indices = self
                            .calendar
                            .days
                            .get(date)
                            .map(|day| {
                                day.events
                                    .iter()
                                    .copied()
                                    .filter(|i| self.matches_event(*i, &query))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        for index in indices.iter().take(3) {
                            self.event_button(ui, *index, *date, Vec2::new(width - 12.0, 23.0));
                        }
                        if indices.len() > 3
                            && ui
                                .small_button(
                                    language.text(&(format!("+{} more", indices.len() - 3))),
                                )
                                .clicked()
                        {
                            self.choose_dates(*date, *date);
                        }
                        if !self.calendar.coverage.contains(date) {
                            ui.weak(language.text("Not synced"));
                        }
                    });
                }
            });
        }
    }

    pub(super) fn timeline(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let dates = self.visible_days();
        let query = tomlook::calendar::SearchQuery::new(&self.query);
        let day_width = ((ui.available_width() - 48.0) / dates.len() as f32).max(95.0);
        egui::ScrollArea::horizontal()
            .id_salt("timeline-horizontal")
            .scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, ..Default::default() })
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(48.0 + day_width * dates.len() as f32);
                if self.scroll_meeting
                    && let Some(selected) = self.selected
                    && let Some(column) = dates.iter().position(|date| {
                        self.calendar
                            .days
                            .get(date)
                            .is_some_and(|day| day.events.contains(&selected))
                    })
                {
                    let position = ui.cursor().min;
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(
                            egui::pos2(position.x + 48.0 + column as f32 * day_width, position.y),
                            Vec2::new(day_width, 1.0),
                        ),
                        None,
                    );
                }
                ui.horizontal(|ui| {
                    ui.add_space(48.0);
                    for date in &dates {
                        ui.add_sized(
                            [day_width - 8.0, 35.0],
                            egui::Label::new(
                                RichText::new(language.date(*date, false)).strong().color(
                                    if *date == Local::now().date_naive() {
                                        self.accent()
                                    } else {
                                        ui.visuals().text_color()
                                    },
                                ),
                            ),
                        );
                    }
                });
                let first = 0.0_f32;
                let last = 24.0_f32;
                let mut all_day_rows = 0;
                for date in &dates {
                    if let Some(day) = self.calendar.days.get(date) {
                        all_day_rows = all_day_rows.max(
                            day.all_day
                                .iter()
                                .filter(|index| self.matches_event(**index, &query))
                                .count()
                                .min(4),
                        );
                    }
                }
                if all_day_rows > 0 {
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            [40.0, 23.0],
                            egui::Label::new(language.text("All day")).wrap(),
                        );
                        for date in &dates {
                            ui.vertical(|ui| {
                                ui.set_width(day_width - 8.0);
                                let indices = self
                                    .calendar
                                    .days
                                    .get(date)
                                    .map(|day| day.all_day.clone())
                                    .unwrap_or_default();
                                egui::ScrollArea::vertical()
                                    .id_salt(("all-day", date))
                                    .max_height(140.0)
                                    .show(ui, |ui| {
                                        for index in indices {
                                            if self.matches_event(index, &query) {
                                                self.event_button(
                                                    ui,
                                                    index,
                                                    *date,
                                                    Vec2::new(day_width - 8.0, 23.0),
                                                );
                                            }
                                        }
                                    });
                            });
                        }
                    });
                }
                let period = (
                    dates[0],
                    *dates.last().expect("calendar period is nonempty"),
                    first as i32,
                );
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt("calendar-timeline")
                    .scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, ..Default::default() })
                    .auto_shrink([false, false]);
                if self.timeline_period != Some(period) {
                    scroll = scroll.vertical_scroll_offset((8.0 - first).max(0.0) * 58.0);
                    self.timeline_period = Some(period);
                }
                scroll.show(ui, |ui| {
                    let height = (last - first) * 58.0;
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(48.0 + day_width * dates.len() as f32, height),
                        egui::Sense::hover(),
                    );
                    let line = ui.visuals().widgets.noninteractive.bg_stroke;
                    for hour in first as i32..=last as i32 {
                        let y = rect.top() + (hour as f32 - first) * 58.0;
                        ui.painter().text(
                            egui::pos2(rect.left(), y + 5.0),
                            egui::Align2::LEFT_TOP,
                            format!("{hour:02}:00"),
                            egui::FontId::proportional(11.0),
                            ui.visuals().weak_text_color(),
                        );
                        ui.painter().line_segment(
                            [
                                egui::pos2(rect.left() + 48.0, y),
                                egui::pos2(rect.right(), y),
                            ],
                            line,
                        );
                    }
                    for (column, date) in dates.iter().enumerate() {
                        let x = rect.left() + 48.0 + column as f32 * day_width;
                        ui.painter().line_segment(
                            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                            line,
                        );
                        let mut day = self.calendar.days.get(date).cloned().unwrap_or_default();
                        if !self.query.is_empty()
                            || !self.category.is_empty()
                            || self.preferences.hide_unconfirmed
                        {
                            day.timed
                                .retain(|slot| self.matches_event(slot.event, &query));
                            Calendar::layout_slots(&mut day.timed);
                        }
                        if !self.calendar.coverage.contains(date) {
                            ui.painter().text(
                                egui::pos2(x + 8.0, rect.top() + 6.0),
                                egui::Align2::LEFT_TOP,
                                language.text("Not synced"),
                                egui::FontId::proportional(11.0),
                                ui.visuals().weak_text_color(),
                            );
                        }
                        let column_width = day_width - 8.0;
                        let content_width = timeline_content_width(column_width, &day.timed);
                        if self.scroll_meeting
                            && let Some(selected) = self.selected
                            && let Some(slot) = day.timed.iter().find(|slot| slot.event == selected)
                        {
                            // Save the vertical reveal outside the nested horizontal scroller,
                            // which consumes both axes of requests made inside it.
                            ui.scroll_to_rect(
                                egui::Rect::from_min_size(
                                    egui::pos2(
                                        x,
                                        rect.top() + (slot.start_minute / 60.0 - first) * 58.0,
                                    ),
                                    Vec2::new(
                                        column_width,
                                        ((slot.end_minute - slot.start_minute) / 60.0 * 58.0)
                                            .max(23.0),
                                    ),
                                ),
                                Some(egui::Align::Center),
                            );
                        }
                        let mut column_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt(("meeting-lanes", date))
                                .max_rect(egui::Rect::from_min_size(
                                    egui::pos2(x + 4.0, rect.top()),
                                    Vec2::new(column_width, height),
                                )),
                        );
                        egui::ScrollArea::horizontal()
                            .id_salt(("meeting-lanes-scroll", date))
                            .scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, ..Default::default() })
                            .auto_shrink([false, false])
                            .show(&mut column_ui, |ui| {
                                let (lane_rect, _) = ui.allocate_exact_size(
                                    Vec2::new(content_width, height),
                                    egui::Sense::hover(),
                                );
                                let mut cards = Vec::new();
                                for slot in day.timed {
                                    if !self.matches_event(slot.event, &query) {
                                        continue;
                                    }
                                    let width = content_width / slot.lanes.max(1) as f32;
                                    let y =
                                        lane_rect.top() + (slot.start_minute / 60.0 - first) * 58.0;
                                    let card = egui::Rect::from_min_size(
                                        egui::pos2(
                                            lane_rect.left() + slot.lane as f32 * width,
                                            y + 2.0,
                                        ),
                                        Vec2::new(
                                            width - 3.0,
                                            ((slot.end_minute - slot.start_minute) / 60.0 * 58.0
                                                - 4.0)
                                                .max(23.0),
                                        ),
                                    );
                                    cards.push(card);
                                    self.event_card(ui, slot.event, card);
                                }
                                self.select_event_time(ui, *date, lane_rect, &cards);
                            });
                    }
                });
            });
    }
}

#[derive(Clone, Copy)]
enum HeaderIcon { Settings, Ask, Commands, Tasks, MyDay }

/// Number of open tasks on a header icon; text uses the panel color for contrast with the accent.
fn count_badge(ui: &egui::Ui, rect: egui::Rect, count: usize) {
    let text = if count > 99 { "99+".to_owned() } else { count.to_string() };
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(9.5), ui.visuals().panel_fill);
    let size = Vec2::new((galley.size().x + 6.0).max(14.0), 14.0);
    let badge = egui::Rect::from_center_size(rect.right_top() + Vec2::new(-4.0, 5.0), size);
    ui.painter().rect_filled(badge, 7.0, ui.visuals().selection.stroke.color);
    ui.painter().galley(badge.center() - galley.size() / 2.0, galley, ui.visuals().panel_fill);
}

fn header_icon(ui: &mut egui::Ui, label: &str, icon: HeaderIcon,
    language: tomlook::i18n::Language, attention: bool) -> egui::Response {
    let response = ui.push_id(label, |ui| ui.add(egui::Button::new("").frame(false)
        .min_size(Vec2::splat(30.0)))).inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), language.text(label)));
    let rect = response.rect.shrink(5.0);
    let center = rect.center();
    let stroke = Stroke::new(1.5, ui.visuals().text_color());
    let point = |x, y| rect.min + Vec2::new(x * rect.width(), y * rect.height());
    match icon {
        HeaderIcon::Settings => {
            ui.painter().circle_stroke(center, 6.0, stroke);
            ui.painter().circle_stroke(center, 2.2, stroke);
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                let direction = Vec2::new(angle.cos(), angle.sin());
                ui.painter().line_segment([center + direction * 6.0, center + direction * 9.0], Stroke::new(2.5, stroke.color));
            }
        }
        HeaderIcon::Ask => {
            ui.painter().rect_stroke(egui::Rect::from_min_max(point(0.08, 0.1), point(0.92, 0.76)),
                4, stroke, egui::StrokeKind::Inside);
            ui.painter().line_segment([point(0.4, 0.76), point(0.2, 0.95)], stroke);
            for x in [0.3, 0.5, 0.7] { ui.painter().circle_filled(point(x, 0.43), 1.0, stroke.color); }
        }
        HeaderIcon::Commands => {
            ui.painter().rect_stroke(rect.shrink(1.0), 3, stroke, egui::StrokeKind::Inside);
            ui.painter().line_segment([point(0.22, 0.3), point(0.42, 0.5)], stroke);
            ui.painter().line_segment([point(0.42, 0.5), point(0.22, 0.7)], stroke);
            ui.painter().line_segment([point(0.55, 0.7), point(0.8, 0.7)], stroke);
        }
        HeaderIcon::Tasks => {
            ui.painter().rect_stroke(egui::Rect::from_min_max(point(0.08, 0.12), point(0.42, 0.46)),
                2, stroke, egui::StrokeKind::Inside);
            ui.painter().line_segment([point(0.14, 0.29), point(0.23, 0.38)], stroke);
            ui.painter().line_segment([point(0.23, 0.38), point(0.38, 0.18)], stroke);
            ui.painter().rect_stroke(egui::Rect::from_min_max(point(0.08, 0.58), point(0.42, 0.92)),
                2, stroke, egui::StrokeKind::Inside);
            for y in [0.29, 0.75] { ui.painter().line_segment([point(0.55, y), point(0.94, y)], stroke); }
        }
        HeaderIcon::MyDay => {
            // A sun rising over the horizon.
            let horizon = point(0.5, 0.74);
            let radius = rect.width() * 0.22;
            ui.painter().line_segment([point(0.04, 0.74), point(0.96, 0.74)], stroke);
            let arc = (0..=16).map(|step| {
                let angle = std::f32::consts::PI * (1.0 + step as f32 / 16.0);
                horizon + Vec2::new(angle.cos(), angle.sin()) * radius
            }).collect();
            ui.painter().add(egui::Shape::line(arc, stroke));
            for index in 0..5 {
                let angle = std::f32::consts::PI * (1.1 + index as f32 * 0.2);
                let direction = Vec2::new(angle.cos(), angle.sin());
                ui.painter().line_segment([horizon + direction * (radius + 2.5), horizon + direction * (radius + 5.5)], stroke);
            }
            ui.painter().line_segment([point(0.3, 0.9), point(0.7, 0.9)], stroke);
        }
    }
    if attention { ui.painter().circle_filled(response.rect.right_top() + Vec2::new(-3.0, 4.0), 3.0, ui.visuals().selection.stroke.color); }
    if response.has_focus() { ui.painter().rect_stroke(response.rect, 3, ui.visuals().selection.stroke, egui::StrokeKind::Inside); }
    response.on_hover_text(language.text(label))
}

fn insight_badge(ui: &mut egui::Ui, rect: egui::Rect, notes: bool, color: Color32, language: tomlook::i18n::Language) {
    let stroke = Stroke::new(1.2, color);
    ui.painter().rect_stroke(rect.shrink2(Vec2::new(2.0, 0.0)), 2, stroke, egui::StrokeKind::Inside);
    let p = |x, y| rect.min + Vec2::new(x, y);
    if notes {
        ui.painter().line_segment([p(4.0, 8.0), p(6.5, 10.5)], stroke);
        ui.painter().line_segment([p(6.5, 10.5), p(11.0, 4.5)], stroke);
    } else {
        for y in [4.0, 7.0, 10.0] { ui.painter().line_segment([p(5.0, y), p(10.0, y)], stroke); }
    }
    ui.interact(rect, ui.id().with(("insight-badge", rect.min.x.to_bits(), rect.min.y.to_bits())), egui::Sense::hover())
        .on_hover_text(language.text(if notes { "AI meeting notes ready" } else { "AI preparation ready" }));
}
