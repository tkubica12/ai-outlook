use chrono::{Datelike, Local, NaiveDate};
use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText, Stroke, Vec2};
use std::sync::Arc;
use tomlook::{
    calendar::{Calendar, Event, View, safe_url},
    storage::Preferences,
    worker::{Command, Notice, Options, Worker},
};

const ACCENTS: [(&str, [u8; 3], [u8; 3]); 4] = [
    ("Blue", [0, 105, 161], [50, 170, 240]),
    ("Red", [177, 54, 41], [246, 115, 99]),
    ("Green", [43, 112, 54], [112, 195, 124]),
    ("Yellow", [131, 96, 0], [242, 197, 68]),
];

pub struct Tomlook {
    worker: Option<Worker>,
    calendar: Arc<Calendar>,
    preferences: Preferences,
    anchor: NaiveDate,
    selected: Option<usize>,
    detail_revision: u64,
    briefing: Option<serde_json::Value>,
    query: String,
    category: String,
    settings: bool,
    palette: bool,
    focus_search: bool,
    loading: bool,
    error: Option<String>,
}

impl Tomlook {
    pub fn new(creation: &eframe::CreationContext<'_>, options: Options) -> Self {
        let (worker, error) = match Worker::start(options, creation.egui_ctx.clone()) {
            Ok(worker) => (Some(worker), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            worker,
            error,
            calendar: Arc::new(Calendar::default()),
            preferences: Preferences::default(),
            anchor: Local::now().date_naive(),
            selected: None,
            detail_revision: 0,
            briefing: None,
            query: String::new(),
            category: String::new(),
            settings: false,
            palette: false,
            focus_search: false,
            loading: true,
        }
    }

    fn command(&mut self, command: Command) {
        match self
            .worker
            .as_ref()
            .map(|worker| worker.commands.try_send(command))
        {
            Some(Ok(())) => {}
            Some(Err(std::sync::mpsc::TrySendError::Full(_))) => {
                self.error = Some("Local queue is full. Nothing was submitted; try again.".into());
            }
            _ => self.error = Some("The storage worker is unavailable. Restart Tomlook.".into()),
        }
    }

    fn receive(&mut self) {
        for _ in 0..16 {
            let Some(notice) = self
                .worker
                .as_ref()
                .and_then(|worker| worker.notices.try_recv().ok())
            else {
                break;
            };
            match notice {
                Notice::Loaded {
                    calendar,
                    preferences,
                } => {
                    let id = self.selected.map(|i| self.calendar.events[i].id.clone());
                    self.selected =
                        id.and_then(|id| calendar.events.iter().position(|event| event.id == id));
                    self.calendar = calendar;
                    self.preferences = preferences;
                    self.loading = false;
                }
                Notice::Detail { revision, payload } if revision == self.detail_revision => {
                    match payload.map(|json| serde_json::from_str(&json)).transpose() {
                        Ok(briefing) => self.briefing = briefing,
                        Err(error) => {
                            self.error = Some(format!("Saved briefing is invalid: {error}"))
                        }
                    }
                }
                Notice::Detail { .. } => {}
                Notice::Error(error) => {
                    self.error = Some(error);
                    self.loading = false;
                }
            }
        }
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.detail_revision += 1;
        self.briefing = None;
        self.command(Command::Detail {
            revision: self.detail_revision,
            id: self.calendar.events[index].id.clone(),
        });
    }

    fn accent(&self) -> Color32 {
        let (_, light, dark) = ACCENTS[self.preferences.accent.min(3)];
        let [r, g, b] = if self.preferences.dark { dark } else { light };
        Color32::from_rgb(r, g, b)
    }

    fn style(&self, context: &egui::Context) {
        let theme = if self.preferences.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        context.set_theme(theme);
        let accent = self.accent();
        context.style_mut_of(theme, |style| {
            style.spacing.item_spacing = Vec2::new(8.0, 8.0);
            style.spacing.button_padding = Vec2::new(10.0, 6.0);
            style.visuals.selection.bg_fill = accent.gamma_multiply(0.25);
            style.visuals.selection.stroke = Stroke::new(1.5, accent);
            style.visuals.hyperlink_color = accent;
            style.visuals.widgets.active.fg_stroke = Stroke::new(1.5, accent);
            style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
            style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(
                1.0,
                Color32::from_gray(if self.preferences.dark { 51 } else { 215 }),
            );
            style.visuals.panel_fill =
                Color32::from_gray(if self.preferences.dark { 23 } else { 250 });
            style.visuals.window_fill =
                Color32::from_gray(if self.preferences.dark { 29 } else { 255 });
        });
    }

    fn shortcuts(&mut self, context: &egui::Context) {
        let consume = |modifiers, key| context.input_mut(|input| input.consume_key(modifiers, key));
        if consume(Modifiers::CTRL, Key::K) {
            self.palette = !self.palette;
        }
        if consume(Modifiers::CTRL, Key::F) {
            self.focus_search = true;
        }
        if consume(Modifiers::NONE, Key::Escape) {
            if self.palette {
                self.palette = false;
            } else if self.settings {
                self.settings = false;
            } else if self.selected.is_some() {
                self.selected = None;
                self.detail_revision += 1;
            } else {
                self.query.clear();
            }
        }
        if context.memory(|memory| memory.focused() == Some(Id::new("calendar-search")))
            || self.palette
            || self.settings
        {
            return;
        }
        if consume(Modifiers::ALT, Key::ArrowLeft) {
            self.anchor = self.preferences.view.shift(self.anchor, -1);
        }
        if consume(Modifiers::ALT, Key::ArrowRight) {
            self.anchor = self.preferences.view.shift(self.anchor, 1);
        }
        if consume(Modifiers::NONE, Key::T) {
            self.anchor = Local::now().date_naive();
        }
        for (key, view) in [
            (Key::Num1, View::Day),
            (Key::Num2, View::WorkWeek),
            (Key::Num3, View::Week),
            (Key::Num4, View::Month),
        ] {
            if consume(Modifiers::NONE, key) {
                self.preferences.view = view;
                self.save_preferences();
            }
        }
        let up = consume(Modifiers::NONE, Key::ArrowUp);
        let down = consume(Modifiers::NONE, Key::ArrowDown);
        if up || down {
            let days = self.preferences.view.days(self.anchor);
            let query = self.query.to_lowercase();
            let visible = days
                .iter()
                .filter_map(|day| self.calendar.days.get(day))
                .flat_map(|day| day.events.iter().copied())
                .filter(|i| self.calendar.matches(*i, &query, &self.category))
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
            }
        }
    }

    fn save_preferences(&mut self) {
        self.command(Command::Preferences(self.preferences.clone()));
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Tomlook").size(22.0).strong());
            ui.add_space(24.0);
            if ui.button("Today").on_hover_text("T").clicked() {
                self.anchor = Local::now().date_naive();
            }
            if ui
                .button("<")
                .on_hover_text("Previous period - Alt+Left")
                .clicked()
            {
                self.anchor = self.preferences.view.shift(self.anchor, -1);
            }
            if ui
                .button(">")
                .on_hover_text("Next period - Alt+Right")
                .clicked()
            {
                self.anchor = self.preferences.view.shift(self.anchor, 1);
            }
            let days = self.preferences.view.days(self.anchor);
            let label = if self.preferences.view == View::Month {
                self.anchor.format("%B %Y").to_string()
            } else if self.preferences.view == View::Day {
                self.anchor.format("%A, %e %B %Y").to_string()
            } else {
                format!(
                    "{} - {}",
                    days[0].format("%e %b"),
                    days.last().expect("view is nonempty").format("%e %b %Y")
                )
            };
            ui.label(RichText::new(label).size(17.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Commands").on_hover_text("Ctrl+K").clicked() {
                    self.palette = true;
                }
                if ui.button("Settings").clicked() {
                    self.settings = !self.settings;
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (view, label) in View::ALL {
                if ui
                    .selectable_label(self.preferences.view == view, label)
                    .clicked()
                {
                    self.preferences.view = view;
                    self.save_preferences();
                }
            }
            ui.add_space(16.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(Id::new("calendar-search"))
                    .hint_text("Search meetings")
                    .desired_width(230.0),
            );
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }
            if !self.query.is_empty() && ui.button("Clear").clicked() {
                self.query.clear();
            }
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(RichText::new(self.anchor.format("%B %Y").to_string()).strong());
        let dates = View::Month.days(self.anchor);
        ui.scope(|ui| {
            ui.spacing_mut().button_padding = Vec2::new(1.0, 2.0);
            ui.style_mut().override_font_id = Some(egui::FontId::proportional(11.0));
            egui::Grid::new("mini-calendar")
                .min_col_width(0.0)
                .spacing([3.0, 4.0])
                .show(ui, |ui| {
                    for day in ["M", "T", "W", "T", "F", "S", "S"] {
                        ui.weak(day);
                    }
                    ui.end_row();
                    for (i, date) in dates.iter().enumerate() {
                        let text = RichText::new(date.day().to_string()).color(
                            if *date == Local::now().date_naive() {
                                self.accent()
                            } else if date.month() == self.anchor.month() {
                                ui.visuals().text_color()
                            } else {
                                ui.visuals().weak_text_color()
                            },
                        );
                        if ui
                            .add_sized(
                                [22.0, 25.0],
                                egui::Button::new(text).selected(*date == self.anchor),
                            )
                            .on_hover_text(date.format("%A %e %B").to_string())
                            .clicked()
                        {
                            self.anchor = *date;
                        }
                        if i % 7 == 6 {
                            ui.end_row();
                        }
                    }
                });
        });
        ui.add_space(24.0);
        ui.weak("WORKSPACE");
        ui.label(format!("{} cached meetings", self.calendar.events.len()));
        ui.add_space(12.0);
        ui.weak("CATEGORIES");
        if ui
            .selectable_label(self.category.is_empty(), "All meetings")
            .clicked()
        {
            self.category.clear();
        }
        let categories = self
            .calendar
            .events
            .iter()
            .map(|e| e.category.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for category in categories {
            if category.is_empty() {
                continue;
            }
            if ui
                .selectable_label(self.category == category, &category)
                .clicked()
            {
                self.category = category;
            }
        }
        ui.add_space(24.0);
        ui.weak("KEYBOARD");
        for hint in [
            "Ctrl+F   Search",
            "Ctrl+K   Commands",
            "1 / 2 / 3 / 4   View",
            "Alt+Left / Right   Period",
            "Up / Down   Meeting",
            "Esc   Close",
            "Tab / Shift+Tab   Focus",
        ] {
            ui.weak(hint);
        }
        ui.add_space(24.0);
        if ui.button("Reload local cache").clicked() {
            self.command(Command::Reload);
        }
    }

    fn event_button(&mut self, ui: &mut egui::Ui, index: usize, size: Vec2) {
        let event = &self.calendar.events[index];
        let time = if event.all_day_like() {
            "All day".into()
        } else {
            event
                .start
                .with_timezone(&Local)
                .format("%H:%M")
                .to_string()
        };
        let text = format!("{time}  {}", event.title);
        if ui
            .add_sized(
                size,
                egui::Button::new(text)
                    .selected(self.selected == Some(index))
                    .truncate(),
            )
            .on_hover_text(format!(
                "{}\n{} - {}\n{}",
                event.title,
                event.start.with_timezone(&Local).format("%H:%M"),
                event.end.with_timezone(&Local).format("%H:%M"),
                event.location
            ))
            .clicked()
        {
            self.select(index);
        }
    }

    fn month(&mut self, ui: &mut egui::Ui) {
        let dates = View::Month.days(self.anchor);
        let query = self.query.to_lowercase();
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
                    egui::Label::new(RichText::new(weekday).small()),
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
                            .button(RichText::new(date.day().to_string()).color(
                                if *date == Local::now().date_naive() {
                                    self.accent()
                                } else {
                                    ui.visuals().text_color()
                                },
                            ))
                            .clicked()
                        {
                            self.anchor = *date;
                            self.preferences.view = View::Day;
                            self.save_preferences();
                        }
                        let indices = self
                            .calendar
                            .days
                            .get(date)
                            .map(|day| {
                                day.events
                                    .iter()
                                    .copied()
                                    .filter(|i| self.calendar.matches(*i, &query, &self.category))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        for index in indices.iter().take(3) {
                            self.event_button(ui, *index, Vec2::new(width - 12.0, 23.0));
                        }
                        if indices.len() > 3
                            && ui
                                .small_button(format!("+{} more", indices.len() - 3))
                                .clicked()
                        {
                            self.anchor = *date;
                            self.preferences.view = View::Day;
                            self.save_preferences();
                        }
                        if !self.calendar.coverage.contains(date) {
                            ui.weak("Not synced");
                        }
                    });
                }
            });
        }
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let dates = self.preferences.view.days(self.anchor);
        let query = self.query.to_lowercase();
        let day_width = ((ui.available_width() - 48.0) / dates.len() as f32).max(95.0);
        ui.horizontal(|ui| {
            ui.add_space(48.0);
            for date in &dates {
                ui.add_sized(
                    [day_width - 8.0, 35.0],
                    egui::Label::new(
                        RichText::new(date.format("%a %e").to_string())
                            .strong()
                            .color(if *date == Local::now().date_naive() {
                                self.accent()
                            } else {
                                ui.visuals().text_color()
                            }),
                    ),
                );
            }
        });
        let mut first = 8.0_f32;
        let mut last = 18.0_f32;
        let mut all_day_rows = 0;
        for date in &dates {
            if let Some(day) = self.calendar.days.get(date) {
                all_day_rows = all_day_rows.max(day.all_day.len().min(4));
                for slot in &day.timed {
                    if self.calendar.matches(slot.event, &query, &self.category) {
                        first = first.min((slot.start_minute / 60.0).floor());
                        last = last.max((slot.end_minute / 60.0).ceil());
                    }
                }
            }
        }
        if all_day_rows > 0 {
            ui.horizontal_top(|ui| {
                ui.add_sized([40.0, 23.0], egui::Label::new("All day").wrap());
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
                                    if self.calendar.matches(index, &query, &self.category) {
                                        self.event_button(
                                            ui,
                                            index,
                                            Vec2::new(day_width - 8.0, 23.0),
                                        );
                                    }
                                }
                            });
                    });
                }
            });
        }
        egui::ScrollArea::both()
            .id_salt("calendar-timeline")
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
                    let day = self.calendar.days.get(date).cloned().unwrap_or_default();
                    if !self.calendar.coverage.contains(date) {
                        ui.painter().text(
                            egui::pos2(x + 8.0, rect.top() + 6.0),
                            egui::Align2::LEFT_TOP,
                            "Not synced",
                            egui::FontId::proportional(11.0),
                            ui.visuals().weak_text_color(),
                        );
                    }
                    for slot in day.timed {
                        if !self.calendar.matches(slot.event, &query, &self.category) {
                            continue;
                        }
                        let width = (day_width - 8.0) / slot.lanes.max(1) as f32;
                        let y = rect.top() + (slot.start_minute / 60.0 - first) * 58.0;
                        let card = egui::Rect::from_min_size(
                            egui::pos2(x + 4.0 + slot.lane as f32 * width, y + 2.0),
                            Vec2::new(
                                (width - 3.0).max(12.0),
                                ((slot.end_minute - slot.start_minute) / 60.0 * 58.0 - 4.0)
                                    .max(23.0),
                            ),
                        );
                        let event = &self.calendar.events[slot.event];
                        let text = format!(
                            "{}  {}",
                            event.start.with_timezone(&Local).format("%H:%M"),
                            event.title
                        );
                        let button = egui::Button::new(RichText::new(text).size(12.0))
                            .selected(self.selected == Some(slot.event))
                            .truncate();
                        if ui
                            .put(card, button)
                            .on_hover_text(format!(
                                "{}\n{}\n{}",
                                event.title, event.location, event.category
                            ))
                            .clicked()
                        {
                            self.select(slot.event);
                        }
                        ui.painter().line_segment(
                            [card.left_top(), card.left_bottom()],
                            Stroke::new(2.0, self.accent()),
                        );
                    }
                }
            });
    }

    fn details(&mut self, ui: &mut egui::Ui, event: Event) {
        ui.horizontal(|ui| {
            ui.weak("MEETING");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Esc").clicked() {
                    self.selected = None;
                    self.detail_revision += 1;
                }
            });
        });
        ui.add_space(12.0);
        ui.heading(&event.title);
        ui.label(
            event
                .start
                .with_timezone(&Local)
                .format("%A, %e %B %Y")
                .to_string(),
        );
        ui.label(format!(
            "{} - {}",
            event.start.with_timezone(&Local).format("%H:%M"),
            event.end.with_timezone(&Local).format("%H:%M")
        ));
        ui.separator();
        for (label, value) in [
            ("Organizer", event.organizer.as_str()),
            ("Location", event.location.as_str()),
            ("Response", event.status.as_str()),
            ("Category", event.category.as_str()),
        ] {
            ui.weak(label);
            ui.label(if value.is_empty() {
                "Not provided"
            } else {
                value
            });
        }
        ui.weak("Attendees");
        for attendee in &event.attendees {
            ui.label(attendee);
        }
        if let Some(url) = event.source_url.as_ref().filter(|url| safe_url(url)) {
            ui.hyperlink_to("Open original meeting", url);
        }
        ui.add_space(16.0);
        ui.separator();
        ui.weak("PREPARATION");
        if let Some(briefing) = &self.briefing {
            if let Some(summary) = briefing.get("summary").and_then(|v| v.as_str()) {
                ui.label(summary);
            }
            for (field, label) in [
                ("preparation", "Prepare"),
                ("talking_points", "Talking points"),
                ("open_questions", "Open questions"),
                ("risks", "Risks"),
                ("warnings", "Evidence gaps"),
            ] {
                if let Some(items) = briefing
                    .get(field)
                    .and_then(|v| v.as_array())
                    .filter(|items| !items.is_empty())
                {
                    ui.add_space(10.0);
                    ui.label(RichText::new(label).strong());
                    for item in items {
                        if let Some(text) = item.as_str() {
                            ui.label(text);
                        }
                    }
                }
            }
            if let Some(sources) = briefing.get("sources").and_then(|v| v.as_array()) {
                ui.add_space(10.0);
                ui.weak("SOURCES");
                for source in sources {
                    let title = source
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Untitled source");
                    if let Some(url) = source
                        .get("url")
                        .and_then(|v| v.as_str())
                        .filter(|url| safe_url(url))
                    {
                        ui.hyperlink_to(title, url);
                    } else {
                        ui.label(title);
                    }
                }
            }
        } else {
            ui.label("No saved briefing. Meeting details are always available.");
            ui.weak("The isolated SDK connection is added in the next migration milestone.");
        }
    }

    fn overlays(&mut self, context: &egui::Context) {
        if self.settings {
            let mut open = true;
            egui::Window::new("Settings").open(&mut open).collapsible(false).resizable(false).show(context, |ui| {
                if ui.checkbox(&mut self.preferences.dark, "Dark appearance").changed() { self.save_preferences(); }
                ui.add_space(8.0); ui.label("One accent");
                for (i, (label, _, _)) in ACCENTS.iter().enumerate() {
                    if ui.selectable_label(self.preferences.accent == i, *label).clicked() { self.preferences.accent = i; self.save_preferences(); }
                }
                ui.separator(); ui.weak("Calendar data stays local. Existing prototype files are never overwritten.");
            });
            self.settings = open;
        }
        if self.palette {
            let mut open = true;
            egui::Window::new("Commands")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 110.0])
                .show(context, |ui| {
                    for (view, label) in View::ALL {
                        if ui.button(format!("Show {label}")).clicked() {
                            self.preferences.view = view;
                            self.save_preferences();
                            self.palette = false;
                        }
                    }
                    if ui.button("Go to today").clicked() {
                        self.anchor = Local::now().date_naive();
                        self.palette = false;
                    }
                    if ui.button("Search meetings").clicked() {
                        self.focus_search = true;
                        self.palette = false;
                    }
                    if ui.button("Appearance settings").clicked() {
                        self.settings = true;
                        self.palette = false;
                    }
                });
            self.palette &= open;
        }
    }
}

impl eframe::App for Tomlook {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.receive();
        self.style(&context);
        self.shortcuts(&context);
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            if let Some(error) = self.error.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(error).strong());
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            } else if self.loading {
                ui.label("Loading local calendar on a background worker...");
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(if self.calendar.warnings.is_empty() {
                        "Local calendar - AI disconnected"
                    } else {
                        self.calendar.warnings.first().expect("nonempty")
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak("Native Rust / no web server");
                    });
                });
            }
        });
        egui::Panel::left("navigation")
            .exact_size(192.0)
            .show(ui, |ui| self.sidebar(ui));
        if let Some(event) = self.selected.map(|i| self.calendar.events[i].clone()) {
            egui::Panel::right("meeting-details")
                .default_size(345.0)
                .size_range(285.0..=500.0)
                .resizable(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.details(ui, event));
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if self.preferences.view == View::Month {
                self.month(ui);
            } else {
                self.timeline(ui);
            }
        });
        self.overlays(&context);
    }
}
