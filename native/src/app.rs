use chrono::{Datelike, Local, NaiveDate};
use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText, Stroke, Vec2};
use std::{collections::BTreeMap, sync::Arc};
use tomlook::{
    ai::{self, Engine, Question},
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

const ASSISTANT_CONTEXT_LIMIT: usize = 32;

#[derive(Default)]
struct AssistantState {
    question: String,
    public_topic: String,
    answer: Option<Arc<String>>,
    error: Option<String>,
    fingerprint: Option<String>,
}

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
    ai: Option<Engine>,
    connection: String,
    ai_ready: bool,
    assistant: bool,
    assistant_context: Option<Event>,
    assistant_states: BTreeMap<Option<String>, AssistantState>,
    context_error: Option<String>,
    focus_assistant: bool,
    question: String,
    public_topic: String,
    request_id: u64,
    active_request: Option<u64>,
    answer: Option<Arc<String>>,
    ai_error: Option<String>,
    activity: Arc<tomlook::worker::Activity>,
    tray: Option<crate::tray::Tray>,
    in_tray: bool,
    last_prepared: String,
}

impl Tomlook {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        options: Options,
        shutdown: Arc<std::sync::Mutex<crate::Lifecycle>>,
    ) -> Self {
        let (worker, mut error) = match Worker::start(options.clone(), creation.egui_ctx.clone()) {
            Ok(mut worker) => {
                if let Ok(mut handle) = shutdown.lock()
                    && let Some(done) = worker.done.take()
                {
                    handle.storage = Some(tomlook::worker::Shutdown {
                        commands: worker.commands.clone(),
                        stop: worker.stop.clone(),
                        done,
                    });
                }
                (Some(worker), None)
            }
            Err(error) => (None, Some(error)),
        };
        let ai = match Engine::start(
            options.state,
            options.demo.is_some(),
            creation.egui_ctx.clone(),
        ) {
            Ok(mut engine) => {
                match shutdown.lock() {
                    Ok(mut handle) => {
                        if let Some(done) = engine.done.take() {
                            handle.ai = Some(ai::Shutdown {
                                stop: engine.stop.clone(),
                                done,
                            });
                        }
                    }
                    Err(_) => error = Some("AI shutdown coordinator is unavailable".into()),
                }
                Some(engine)
            }
            Err(problem) => {
                error = Some(problem);
                None
            }
        };
        if let (Some(worker), Some(engine)) = (&worker, &ai)
            && let Err(problem) = worker.commands.try_send(Command::AttachAi {
                commands: engine.commands.clone(),
                ready: engine.ready.clone(),
                occupied: engine.occupied.clone(),
                terminated: engine.terminated.clone(),
                analysis_revision: engine.analysis_revision.clone(),
            })
        {
            error = Some(format!("Attach preparation engine: {problem}"));
        }
        let tray = match crate::tray::Tray::new(
            creation.egui_ctx.clone(),
            worker.as_ref().map(|worker| worker.commands.clone()),
        ) {
            Ok(tray) => Some(tray),
            Err(problem) => {
                error = Some(problem);
                None
            }
        };
        let mut app = Self::initial(worker, ai, error);
        app.tray = tray;
        app
    }

    fn initial(worker: Option<Worker>, ai: Option<Engine>, error: Option<String>) -> Self {
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
            ai,
            connection: "AI not connected - no runtime started".into(),
            ai_ready: false,
            assistant: false,
            assistant_context: None,
            assistant_states: BTreeMap::new(),
            context_error: None,
            focus_assistant: false,
            question: String::new(),
            public_topic: String::new(),
            request_id: 0,
            active_request: None,
            answer: None,
            ai_error: None,
            activity: Arc::new(tomlook::worker::Activity::default()),
            tray: None,
            in_tray: false,
            last_prepared: String::new(),
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
        if let Some(activity) = self
            .worker
            .as_ref()
            .and_then(|worker| worker.activity.try_lock().ok().map(|value| value.clone()))
            && activity.revision != self.activity.revision
        {
            self.preferences.prepare_enabled = activity.enabled;
            self.preferences.paused = activity.paused;
            if let Some(error) = &activity.error {
                self.error = Some(error.clone());
            } else if self.error == self.activity.error {
                self.error = None;
            }
            self.activity = activity;
            if let Some(index) = self.selected
                && let Some(job) = self
                    .activity
                    .queue
                    .jobs
                    .get(&self.calendar.events[index].id)
                && job.state == tomlook::scheduler::State::Completed
                && self.last_prepared != job.fingerprint
            {
                self.last_prepared = job.fingerprint.clone();
                self.detail_revision += 1;
                self.command(Command::Detail {
                    revision: self.detail_revision,
                    id: self.calendar.events[index].id.clone(),
                });
            }
        }
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
                    if let Some(context) = &self.assistant_context {
                        let updated = calendar.events.iter().find(|event| event.id == context.id);
                        if updated.is_none_or(|event| event.fingerprint() != context.fingerprint())
                        {
                            let updated = updated.cloned();
                            self.switch_assistant_context(updated);
                            self.ai_error = Some("The meeting changed or disappeared. The old result was invalidated and cancellation requested; review the updated context before asking again.".into());
                        }
                    }
                    let id = self.selected.map(|i| self.calendar.events[i].id.clone());
                    let next =
                        id.and_then(|id| calendar.events.iter().position(|event| event.id == id));
                    let changed = match (self.selected, next) {
                        (Some(old), Some(new)) => {
                            self.calendar.events[old].fingerprint()
                                != calendar.events[new].fingerprint()
                        }
                        (Some(_), None) => true,
                        _ => false,
                    };
                    self.selected = next;
                    if changed {
                        self.detail_revision += 1;
                        self.briefing = None;
                    }
                    self.calendar = calendar;
                    self.preferences = preferences;
                    self.loading = false;
                    if changed && let Some(index) = self.selected {
                        self.command(Command::Detail {
                            revision: self.detail_revision,
                            id: self.calendar.events[index].id.clone(),
                        });
                    }
                }
                Notice::Detail { revision, payload } if revision == self.detail_revision => {
                    self.briefing = payload;
                }
                Notice::Detail { .. } => {}
                Notice::Error(error) => {
                    self.error = Some(error);
                    self.loading = false;
                }
            }
        }
        for _ in 0..16 {
            let Some(notice) = self
                .ai
                .as_ref()
                .and_then(|engine| engine.notices.try_recv().ok())
            else {
                break;
            };
            match notice {
                ai::Notice::Connection { ready, message } => {
                    self.ai_ready = ready;
                    self.connection = message;
                    self.command(Command::Wake);
                }
                ai::Notice::Answer { id, result } if self.active_request == Some(id) => {
                    self.active_request = None;
                    match result {
                        Ok(answer) if answer.len() <= 64_000 => {
                            self.answer = Some(answer);
                            self.ai_error = None;
                        }
                        Ok(_) => {
                            self.ai_error = Some("AI answer exceeded the 64 KiB response limit; no answer was published.".into());
                            self.answer = None;
                        }
                        Err(error) => {
                            self.ai_error = Some(error);
                            self.answer = None;
                        }
                    }
                }
                ai::Notice::Answer { .. } => {}
                ai::Notice::Stopped => {
                    self.ai_ready = false;
                    self.connection = "AI stopped".into();
                    if self.active_request.take().is_some() {
                        self.answer = None;
                        self.ai_error = Some("AI stopped without a completed answer; provider outcome may be unknown".into());
                    }
                }
            }
        }
    }

    fn select(&mut self, index: usize) {
        if self.assistant_context.is_some() {
            self.switch_assistant_context(Some(self.calendar.events[index].clone()));
        }
        self.selected = Some(index);
        self.detail_revision += 1;
        self.briefing = None;
        self.command(Command::Detail {
            revision: self.detail_revision,
            id: self.calendar.events[index].id.clone(),
        });
        self.command(Command::Promote(self.calendar.events[index].id.clone()));
        self.last_prepared.clear();
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
            style.visuals.override_text_color =
                Some(Color32::from_gray(if self.preferences.dark {
                    225
                } else {
                    35
                }));
            style.visuals.weak_text_color = Some(Color32::from_gray(if self.preferences.dark {
                158
            } else {
                100
            }));
        });
    }

    fn shortcuts(&mut self, context: &egui::Context) {
        let consume = |modifiers, key| context.input_mut(|input| input.consume_key(modifiers, key));
        if consume(Modifiers::CTRL | Modifiers::SHIFT, Key::Q) {
            self.exit(context);
        }
        if consume(Modifiers::CTRL, Key::K) {
            self.palette = !self.palette;
        }
        if consume(Modifiers::CTRL, Key::F) {
            self.focus_search = true;
        }
        if consume(Modifiers::CTRL, Key::Space) {
            self.assistant = !self.assistant;
            self.focus_assistant = self.assistant;
            if !self.assistant {
                self.cancel_ai();
                self.focus_search = true;
            }
        }
        if consume(Modifiers::NONE, Key::Escape) {
            if self.palette {
                self.palette = false;
            } else if self.settings {
                self.settings = false;
            } else if self.assistant {
                self.assistant = false;
                self.cancel_ai();
                self.focus_search = true;
            } else if self.selected.is_some() {
                self.selected = None;
                self.detail_revision += 1;
            } else {
                self.query.clear();
            }
        }
        if context.memory(|memory| matches!(memory.focused(), Some(id) if id == Id::new("calendar-search") || id == Id::new("assistant-question") || id == Id::new("public-topic")))
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
                if ui.button("Ask").on_hover_text("Ctrl+Space").clicked() {
                    self.assistant = !self.assistant;
                    self.focus_assistant = self.assistant;
                    if !self.assistant {
                        self.cancel_ai();
                        self.focus_search = true;
                    }
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
        let calendar = self.calendar.clone();
        for category in &calendar.categories {
            if ui
                .selectable_label(self.category == *category, category)
                .clicked()
            {
                self.category = category.clone();
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
        egui::ScrollArea::horizontal()
            .id_salt("timeline-horizontal")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(48.0 + day_width * dates.len() as f32);
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
                                            if self.calendar.matches(index, &query, &self.category)
                                            {
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
                egui::ScrollArea::vertical()
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
        if ui.button("Ask about this meeting").clicked() {
            self.assistant = true;
            self.switch_assistant_context(Some(event.clone()));
            self.focus_assistant = true;
        }
        ui.add_space(16.0);
        ui.separator();
        ui.weak("PREPARATION");
        if !self.activity.enabled {
            ui.weak("Automatic preparation is disabled.");
        } else if self.activity.paused {
            ui.weak("Preparation is paused.");
        } else if !self.ai_ready {
            ui.weak("Waiting for the isolated SDK.");
        }
        if let Some(job) = self.activity.queue.jobs.get(&event.id).cloned() {
            ui.label(format!("{:?}: {}", job.state, job.message));
            if matches!(
                job.state,
                tomlook::scheduler::State::Failed | tomlook::scheduler::State::Interrupted
            ) && ui.button("Retry preparation (AI)").clicked()
            {
                self.command(Command::Retry(event.id.clone()));
            }
            if job.state != tomlook::scheduler::State::Completed && self.briefing.is_some() {
                ui.weak("Previous saved briefing; not current completion evidence.");
            }
        }
        if let Some(briefing) = &self.briefing {
            if let Some(evidence) = briefing
                .get("evidence_status")
                .and_then(|value| value.as_str())
            {
                ui.weak(evidence);
            }
            if let Some(summary) = briefing.get("summary").and_then(|v| v.as_str()) {
                ui.label(summary);
            }
            for (field, label) in [
                ("preparation", "Prepare"),
                ("talking_points", "Talking points"),
                ("open_questions", "Open questions"),
                ("risks", "Risks"),
                ("warnings", "Evidence gaps"),
                ("gaps", "Evidence gaps"),
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
            ui.weak("Questions work independently of a saved briefing.");
        }
    }

    fn ai_command(&mut self, command: ai::Command) -> bool {
        match self
            .ai
            .as_ref()
            .map(|engine| engine.commands.try_send(command))
        {
            Some(Ok(())) => return true,
            Some(Err(tokio::sync::mpsc::error::TrySendError::Full(_))) => {
                self.ai_error =
                    Some("AI command queue is full; nothing was submitted. Try again.".into())
            }
            _ => self.ai_error = Some("The AI worker is unavailable. Restart Tomlook.".into()),
        }
        false
    }

    fn cancel_ai(&mut self) {
        if let Some(id) = self.active_request.take() {
            self.ai_error = Some("Cancellation requested; partial output is not a completed answer. Provider outcome may be unknown.".into());
            if !self.ai_command(ai::Command::Cancel(id)) {
                self.error = Some("AI cancellation could not be submitted. Its result will be discarded, but the provider request may continue until timeout or application exit.".into());
            }
        }
    }

    fn switch_assistant_context(&mut self, next: Option<Event>) -> bool {
        let current_id = self
            .assistant_context
            .as_ref()
            .map(|event| event.id.clone());
        let next_id = next.as_ref().map(|event| event.id.clone());
        let fingerprint = next.as_ref().map(Event::fingerprint);
        if current_id == next_id {
            self.context_error = None;
            if self.assistant_context.as_ref().map(Event::fingerprint) != fingerprint {
                self.cancel_ai();
                self.answer = None;
                self.ai_error = Some("Meeting context changed. The previous answer was invalidated; review your draft before asking again.".into());
            }
            self.assistant_context = next;
            return true;
        }
        self.cancel_ai();
        if !self.assistant_states.contains_key(&next_id)
            && self.assistant_states.len() + 1 >= ASSISTANT_CONTEXT_LIMIT
        {
            self.context_error = Some("Assistant memory holds 32 contexts. Return to an existing context or restart Tomlook to clear memory. No new question can be submitted for the requested context.".into());
            return false;
        }
        let mut restored = self.assistant_states.remove(&next_id).unwrap_or_default();
        self.assistant_states.insert(
            current_id,
            AssistantState {
                question: std::mem::take(&mut self.question),
                public_topic: std::mem::take(&mut self.public_topic),
                answer: self.answer.take(),
                error: self.ai_error.take(),
                fingerprint: self.assistant_context.as_ref().map(Event::fingerprint),
            },
        );
        if restored.fingerprint != fingerprint && restored.answer.is_some() {
            restored.answer = None;
            restored.error = Some("Meeting context changed. The previous answer was invalidated; review your draft before asking again.".into());
        }
        self.question = restored.question;
        self.public_topic = restored.public_topic;
        self.answer = restored.answer;
        self.ai_error = restored.error;
        self.assistant_context = next;
        self.context_error = None;
        self.focus_assistant = true;
        true
    }

    fn ask_question(&mut self) {
        if self.active_request.is_some() {
            return;
        }
        if !self.ai_ready || self.context_error.is_some() {
            self.ai_error = Some(
                "Connect the isolated SDK and resolve the context warning before asking.".into(),
            );
            return;
        }
        if self.question.trim().is_empty()
            || self.question.len() > 8000
            || self.public_topic.len() > 2000
        {
            self.ai_error = Some("Use a nonempty question of at most 8,000 UTF-8 bytes and a public topic of at most 2,000 bytes.".into());
            return;
        }
        let Some(id) = self.request_id.checked_add(1).filter(|id| *id < 1 << 63) else {
            self.ai_error = Some("Assistant request identity limit reached. No question was submitted; restart Tomlook.".into());
            return;
        };
        self.request_id = id;
        self.ai_error = None;
        if self.ai_command(ai::Command::Ask(Box::new(Question {
            id,
            question: self.question.clone(),
            meeting: self.assistant_context.clone(),
            public_topic: (!self.public_topic.trim().is_empty())
                .then(|| self.public_topic.trim().to_owned()),
        }))) {
            self.active_request = Some(id);
            self.answer = None;
        }
    }

    fn assistant_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.weak("ASSISTANT");
            if ui.button("Close").on_hover_text("Esc").clicked() {
                self.assistant = false;
                self.cancel_ai();
                self.focus_search = true;
            }
        });
        ui.add_space(8.0);
        ui.label(&self.connection);
        ui.weak("Memory only - drafts and last answers are cleared on Exit.");
        if !self.ai_ready && ui.button("Connect isolated SDK").clicked() {
            self.ai_command(ai::Command::Connect);
        }
        ui.separator();
        if let Some(event) = &self.assistant_context {
            ui.label(RichText::new(&event.title).strong());
            ui.weak("Meeting snapshot; no briefing prerequisite");
            if ui.button("Switch to global question").clicked() {
                self.switch_assistant_context(None);
            }
        } else {
            ui.label(RichText::new("Your workspace").strong());
            ui.weak("Ask about mail, calendar or work context. Read-only.");
            if let Some(index) = self.selected
                && ui.button("Use selected meeting").clicked()
            {
                self.switch_assistant_context(Some(self.calendar.events[index].clone()));
            }
        }
        ui.add_space(12.0);
        let question = ui.add(
            egui::TextEdit::multiline(&mut self.question)
                .id(Id::new("assistant-question"))
                .hint_text("Ask a question...")
                .char_limit(8000)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        if self.focus_assistant {
            question.request_focus();
            self.focus_assistant = false;
        }
        ui.weak("Public web is off unless you supply a public topic.");
        let topic = ui.add(
            egui::TextEdit::singleline(&mut self.public_topic)
                .id(Id::new("public-topic"))
                .hint_text("Public topic (optional)")
                .char_limit(2000)
                .desired_width(f32::INFINITY),
        );
        let shortcut = (question.has_focus() || topic.has_focus())
            && ui.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter));
        let can_ask = self.ai_ready
            && !self.question.trim().is_empty()
            && self.active_request.is_none()
            && self.context_error.is_none()
            && self.question.len() <= 8000
            && self.public_topic.len() <= 2000;
        if self.question.len() > 8000 || self.public_topic.len() > 2000 {
            ui.label(
                "Question/public topic exceeds its UTF-8 byte limit. Shorten it before asking.",
            );
        }
        if let Some(error) = &self.context_error {
            ui.label(RichText::new(error).strong());
        }
        ui.horizontal(|ui| {
            let clicked = ui
                .add_enabled(
                    can_ask,
                    egui::Button::new("Ask").min_size([72.0, 30.0].into()),
                )
                .on_hover_text("Ctrl+Enter")
                .clicked();
            if (clicked || shortcut) && can_ask {
                self.ask_question();
            }
            if self.active_request.is_some() && ui.button("Cancel").clicked() {
                self.cancel_ai();
            }
        });
        if let Some(error) = &self.ai_error {
            ui.label(RichText::new(error).strong());
        }
        ui.separator();
        if let Some(answer) = &self.answer {
            ui.weak("Last answer - model output, not independently verified sources.");
            ui.label(answer.as_str());
        }
        if let Some(id) = self.active_request {
            ui.weak("Working asynchronously - the calendar remains available");
            let preview = self.ai.as_ref().and_then(|engine| {
                engine
                    .progress
                    .try_lock()
                    .ok()
                    .filter(|progress| progress.id == id)
                    .map(|progress| progress.text.clone())
            });
            if let Some(text) = preview {
                ui.label(text.as_str());
            }
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
                ui.separator();
                if ui.checkbox(&mut self.preferences.prepare_enabled, "Prepare upcoming meetings (read-only AI)").changed() { self.command(Command::EnablePreparation(self.preferences.prepare_enabled)); }
                if ui.checkbox(&mut self.preferences.paused, "Pause preparation").changed() { self.command(Command::PausePreparation(self.preferences.paused)); }
                ui.weak("Seven future days; 64 queued; at most one background job. Failed/interrupted jobs require explicit retry.");
                if let Some(error) = &self.activity.error {
                    ui.label(error);
                    if ui.add_enabled(self.activity.queue.flights.is_empty(), egui::Button::new("Resume other queued jobs")).clicked() {
                        self.command(Command::ResumeQueued);
                    }
                }
                ui.weak(if self.tray.is_some() {
                    "Closing or minimizing hides Tomlook in the tray. Use Exit Tomlook to stop all work."
                } else {
                    "Tray unavailable. Closing Tomlook exits instead of hiding it."
                });
                if ui.button("Exit Tomlook").clicked() { self.exit(context); }
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
                    if ui.button("Open assistant").clicked() {
                        self.assistant = true;
                        self.focus_assistant = true;
                        self.palette = false;
                    }
                    if ui
                        .add_enabled(self.tray.is_some(), egui::Button::new("Hide in tray"))
                        .clicked()
                    {
                        context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        self.cancel_ai();
                    }
                    if ui.button("Exit Tomlook").clicked() {
                        self.exit(context);
                    }
                });
            self.palette &= open;
        }
    }

    fn exit(&self, context: &egui::Context) {
        if let Some(tray) = &self.tray {
            tray.exit(context);
        } else {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for Tomlook {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        let viewport = context.input(|input| input.viewport().clone());
        if let Some(tray) = &self.tray {
            if viewport.close_requested()
                && !tray.exiting.load(std::sync::atomic::Ordering::Acquire)
            {
                context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.cancel_ai();
            } else if viewport.minimized == Some(true) && !self.in_tray {
                self.in_tray = true;
                context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.cancel_ai();
            } else if viewport.minimized == Some(false) {
                self.in_tray = false;
            }
        }
        if let Some(error) =
            context.data_mut(|data| data.remove_temp::<String>(Id::new("tray-error")))
        {
            self.error = Some(error);
        }
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
                        if self.ai_ready {
                            "Local calendar - isolated AI connected"
                        } else {
                            "Local calendar - AI not connected"
                        }
                    } else {
                        self.calendar.warnings.first().expect("nonempty")
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let queued = self.activity.queued;
                        let deferred = self.activity.deferred;
                        ui.weak(format!(
                            "Preparation {} | {queued} queued / {deferred} deferred | {} running",
                            if !self.activity.enabled {
                                "disabled"
                            } else if self.activity.paused {
                                "paused"
                            } else if !self.ai_ready {
                                "waiting for isolated AI"
                            } else {
                                "enabled"
                            },
                            self.activity.queue.flights.len()
                        ));
                    });
                });
            }
        });
        egui::Panel::left("navigation")
            .exact_size(192.0)
            .show(ui, |ui| self.sidebar(ui));
        if self.assistant {
            egui::Panel::right("assistant")
                .default_size(380.0)
                .size_range(300.0..=520.0)
                .resizable(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.assistant_panel(ui));
                });
        } else if let Some(event) = self.selected.map(|i| self.calendar.events[i].clone()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_fixture() -> (
        Tomlook,
        Arc<ai::Notices>,
        tokio::sync::mpsc::Receiver<ai::Command>,
    ) {
        let notices = Arc::new(ai::Notices::default());
        let (commands, input) = tokio::sync::mpsc::channel(64);
        let (stop, _stopped) = tokio::sync::watch::channel(false);
        let engine = Engine {
            commands,
            notices: notices.clone(),
            progress: Arc::new(std::sync::Mutex::new(ai::Progress::default())),
            done: None,
            ready: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            occupied: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            terminated: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::new()),
            stop,
        };
        let mut app = Tomlook::initial(None, Some(engine), None);
        app.ai_ready = true;
        (app, notices, input)
    }

    #[test]
    fn ai_notices_are_drained_without_storage_notices_and_stale_answers_are_ignored() {
        let output = Arc::new(ai::Notices::default());
        let notices = output.clone();
        let (commands, _input) = tokio::sync::mpsc::channel(1);
        let (stop, _stopped) = tokio::sync::watch::channel(false);
        let engine = Engine {
            commands,
            notices,
            progress: Arc::new(std::sync::Mutex::new(ai::Progress::default())),
            done: None,
            ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            occupied: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            terminated: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::new()),
            stop,
        };
        let mut app = Tomlook::initial(None, Some(engine), None);
        output
            .send(ai::Notice::Connection {
                ready: true,
                message: "Test connection".into(),
            })
            .unwrap();
        app.receive();
        assert!(app.ai_ready);
        assert_eq!(app.connection, "Test connection");
        app.active_request = Some(2);
        output
            .send(ai::Notice::Answer {
                id: 1,
                result: Ok(Arc::new("Old meeting".into())),
            })
            .unwrap();
        app.receive();
        assert!(app.answer.is_none());
        assert_eq!(app.active_request, Some(2));
        output
            .send(ai::Notice::Answer {
                id: 2,
                result: Ok(Arc::new("Current request".into())),
            })
            .unwrap();
        app.receive();
        assert_eq!(
            app.answer.as_deref().map(String::as_str),
            Some("Current request")
        );
        assert!(app.active_request.is_none());
    }

    #[test]
    fn global_and_meeting_drafts_topics_and_last_answers_do_not_leak() {
        let (mut app, _, _input) = assistant_fixture();
        let meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
        app.question = "Global question".into();
        app.public_topic = "Rust".into();
        app.answer = Some(Arc::new("Global answer".into()));
        assert!(app.switch_assistant_context(Some(meeting.clone())));
        assert!(app.question.is_empty());
        assert!(app.public_topic.is_empty());
        assert!(app.answer.is_none());
        app.question = "Meeting question".into();
        app.answer = Some(Arc::new("Meeting answer".into()));
        assert!(app.switch_assistant_context(None));
        assert_eq!(app.question, "Global question");
        assert_eq!(app.public_topic, "Rust");
        assert_eq!(app.answer.as_deref().unwrap(), "Global answer");
        assert!(app.switch_assistant_context(Some(meeting)));
        assert_eq!(app.question, "Meeting question");
        assert!(app.public_topic.is_empty());
        assert_eq!(app.answer.as_deref().unwrap(), "Meeting answer");
    }

    #[test]
    fn switched_context_cancels_its_request_and_cannot_receive_its_late_answer() {
        let (mut app, output, mut input) = assistant_fixture();
        app.question = "Global question".into();
        app.ask_question();
        let Some(ai::Command::Ask(question)) = input.try_recv().ok() else {
            panic!("question was not admitted");
        };
        let meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
        assert!(app.switch_assistant_context(Some(meeting)));
        assert!(matches!(input.try_recv().unwrap(), ai::Command::Cancel(id) if id == question.id));
        output
            .send(ai::Notice::Answer {
                id: question.id,
                result: Ok(Arc::new("Late global answer".into())),
            })
            .unwrap();
        app.receive();
        assert!(app.answer.is_none());
        assert!(app.active_request.is_none());
        assert!(app.switch_assistant_context(None));
        assert_eq!(app.question, "Global question");
        assert!(app.answer.is_none());
        assert!(
            app.ai_error
                .as_ref()
                .unwrap()
                .contains("Cancellation requested")
        );
    }

    #[test]
    fn same_context_reopen_preserves_request_but_changed_snapshot_invalidates_answer() {
        let (mut app, _, mut input) = assistant_fixture();
        let mut meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
        app.switch_assistant_context(Some(meeting.clone()));
        app.question = "Question".into();
        app.ask_question();
        assert!(matches!(input.try_recv().unwrap(), ai::Command::Ask(_)));
        let active = app.active_request;
        assert!(app.switch_assistant_context(Some(meeting.clone())));
        assert_eq!(app.active_request, active);
        assert!(input.try_recv().is_err());
        app.cancel_ai();
        app.answer = Some(Arc::new("Previous answer".into()));
        app.switch_assistant_context(None);
        meeting.title = "Updated meeting".into();
        app.switch_assistant_context(Some(meeting));
        assert!(app.answer.is_none());
        assert_eq!(app.question, "Question");
        assert!(app.ai_error.as_ref().unwrap().contains("invalidated"));
    }

    #[test]
    fn bounded_contexts_refuse_new_scope_without_evicting_drafts_or_allowing_a_question() {
        let (mut app, _, mut input) = assistant_fixture();
        let mut meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
        app.question = "Preserved global draft".into();
        for index in 0..ASSISTANT_CONTEXT_LIMIT - 1 {
            meeting.id = format!("context-{index}");
            assert!(app.switch_assistant_context(Some(meeting.clone())));
        }
        assert_eq!(app.assistant_states.len(), ASSISTANT_CONTEXT_LIMIT - 1);
        meeting.id = "overflow".into();
        assert!(!app.switch_assistant_context(Some(meeting)));
        app.question = "Must not use the old meeting".into();
        app.ask_question();
        assert!(app.context_error.is_some());
        assert!(app.active_request.is_none());
        assert!(input.try_recv().is_err());
        assert!(app.switch_assistant_context(None));
        assert_eq!(app.question, "Preserved global draft");
        assert!(app.context_error.is_none());
    }

    #[test]
    fn admission_is_single_flight_and_rejection_preserves_the_last_answer() {
        let (mut app, _, mut input) = assistant_fixture();
        app.question = "Question".into();
        app.ask_question();
        app.ask_question();
        assert!(matches!(input.try_recv().unwrap(), ai::Command::Ask(_)));
        assert!(input.try_recv().is_err());
        app.cancel_ai();
        assert!(matches!(input.try_recv().unwrap(), ai::Command::Cancel(_)));
        app.answer = Some(Arc::new("Last completed answer".into()));
        drop(input);
        app.ask_question();
        assert!(app.active_request.is_none());
        assert_eq!(app.answer.as_deref().unwrap(), "Last completed answer");
        assert!(app.ai_error.as_ref().unwrap().contains("unavailable"));
    }

    #[test]
    fn request_and_payload_limits_fail_before_submission_and_never_alias_preparation_ids() {
        let (mut app, output, mut input) = assistant_fixture();
        app.question = "\u{010D}".repeat(4001);
        app.ask_question();
        assert!(input.try_recv().is_err());
        assert_eq!(app.request_id, 0);
        app.question = "Question".into();
        app.request_id = (1 << 63) - 1;
        app.ask_question();
        assert!(input.try_recv().is_err());
        assert!(app.ai_error.as_ref().unwrap().contains("identity limit"));
        app.active_request = Some(1);
        output
            .send(ai::Notice::Answer {
                id: 1,
                result: Ok(Arc::new("x".repeat(64_001))),
            })
            .unwrap();
        app.receive();
        assert!(app.answer.is_none());
        assert!(app.active_request.is_none());
        assert!(app.ai_error.as_ref().unwrap().contains("response limit"));
    }

    #[test]
    fn opening_assistant_focuses_question_and_escape_returns_focus_to_search() {
        let (mut app, _, _input) = assistant_fixture();
        let context = egui::Context::default();
        app.assistant = true;
        app.focus_assistant = true;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.assistant_panel(ui);
        });
        output.textures_delta.clear();
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("assistant-question"))
        );
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output = context.run_ui(input, |ui| {
            app.shortcuts(ui.ctx());
            app.header(ui);
        });
        output.textures_delta.clear();
        assert!(!app.assistant);
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("calendar-search"))
        );
    }

    #[test]
    fn control_enter_from_public_topic_submits_once() {
        let (mut app, _, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        app.question = "Synthetic question".into();
        app.public_topic = "Rust".into();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.assistant_panel(ui);
        });
        output.textures_delta.clear();
        context.memory_mut(|memory| memory.request_focus(Id::new("public-topic")));
        for _ in 0..2 {
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::CTRL,
                }],
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                app.assistant_panel(ui);
            });
            output.textures_delta.clear();
        }
        let ai::Command::Ask(question) = commands.try_recv().unwrap() else {
            panic!("expected one question");
        };
        assert_eq!(question.public_topic.as_deref(), Some("Rust"));
        assert!(commands.try_recv().is_err());
    }
}
