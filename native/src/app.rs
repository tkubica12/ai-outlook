use chrono::{Datelike, Local, NaiveDate};
use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText, Stroke, Vec2};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, mpsc},
};
use tomlook::{
    ai::{self, Engine, Question},
    ask,
    calendar::{Calendar, Event, View, person_email, safe_url},
    proposals,
    storage::Preferences,
    worker::{ActivityFilter, Command, Notice, Options, Worker},
};

const ACCENTS: [(&str, [u8; 3], [u8; 3]); 4] = [
    ("Blue", [0, 105, 161], [50, 170, 240]),
    ("Red", [177, 54, 41], [246, 115, 99]),
    ("Green", [43, 112, 54], [112, 195, 124]),
    ("Yellow", [131, 96, 0], [242, 197, 68]),
];

const ASSISTANT_CONTEXT_LIMIT: usize = 32;
const MIN_EVENT_LANE_WIDTH: f32 = 72.0;
const NAVIGATION_WIDTH: f32 = 208.0;

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

fn insight_section(ui: &mut egui::Ui, label: &str, items: &[serde_json::Value], accent: Color32) {
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
            });
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

fn category_color(category: &str, dark: bool) -> Color32 {
    let normalized = category.trim().to_lowercase();
    let colors = match normalized.as_str() {
        "call" => Some(([142, 97, 0], [236, 186, 73])),
        "mimo office" => Some(([132, 92, 66], [199, 150, 117])),
        "mimo prahu" => Some(([100, 65, 187], [170, 142, 242])),
        "nejisté" => Some(([50, 117, 20], [139, 199, 91])),
        "office" => Some(([89, 99, 109], [167, 179, 192])),
        "osobní" => Some(([180, 47, 42], [242, 121, 113])),
        _ => None,
    };
    if let Some((light, night)) = colors {
        let [r, g, b] = if dark { night } else { light };
        return Color32::from_rgb(r, g, b);
    }
    let colors = [
        [65, 151, 210],
        [81, 167, 110],
        [204, 112, 73],
        [191, 160, 55],
        [155, 117, 201],
    ];
    let hash = normalized.bytes().fold(2166136261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16777619)
    });
    let [r, g, b] = colors[hash as usize % colors.len()];
    Color32::from_rgb(r, g, b)
}

fn insight_state(state: tomlook::scheduler::State) -> &'static str {
    use tomlook::scheduler::State;
    match state {
        State::Deferred => "Later",
        State::Queued => "Waiting",
        State::Running => "Researching",
        State::Completed => "Ready",
        State::Failed => "Failed",
        State::Interrupted => "Needs retry",
        State::Cancelled => "Cancelled",
    }
}

const ACTIVITY_ROW_HEIGHT: f32 = 34.0;

fn activity_row(
    ui: &mut egui::Ui,
    job: &tomlook::scheduler::Job,
    title: &str,
    enabled: bool,
    language: tomlook::i18n::Language,
) -> egui::Response {
    ui.scope_builder(
        egui::UiBuilder::new().id(Id::new(("insights-job", &job.event_id))),
        |ui| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [78.0, ACTIVITY_ROW_HEIGHT],
                    egui::Label::new(
                        RichText::new(language.text(insight_state(job.state))).strong(),
                    )
                    .truncate(),
                );
                ui.add_sized(
                    [94.0, ACTIVITY_ROW_HEIGHT],
                    egui::Label::new(
                        RichText::new(format!(
                            "{} {}",
                            language.date(job.start.with_timezone(&Local).date_naive(), false),
                            job.start.with_timezone(&Local).format("%H:%M")
                        ))
                        .weak(),
                    )
                    .truncate(),
                );
                let width = ui.available_width();
                ui.add_enabled_ui(enabled, |ui| {
                    ui.add_sized(
                        [width, ACTIVITY_ROW_HEIGHT],
                        egui::Button::new(title).wrap_mode(egui::TextWrapMode::Truncate),
                    )
                })
                .inner
                .on_hover_text(format!("{title}\n{}", language.text(&job.message)))
            })
            .inner
        },
    )
    .inner
}

fn timeline_content_width(column_width: f32, slots: &[tomlook::calendar::Slot]) -> f32 {
    let lanes = slots
        .iter()
        .map(|slot| slot.lanes)
        .max()
        .unwrap_or(1)
        .max(1);
    column_width.max(lanes as f32 * MIN_EVENT_LANE_WIDTH)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DateSelection {
    first: NaiveDate,
    last: NaiveDate,
}

impl DateSelection {
    fn new(start: NaiveDate, end: NaiveDate) -> Self {
        Self {
            first: start.min(end),
            last: start.max(end),
        }
    }

    fn days(self) -> Vec<NaiveDate> {
        (0..=(self.last - self.first).num_days())
            .map(|day| self.first + chrono::Duration::days(day))
            .collect()
    }
}

struct PersonPhoto {
    image: Option<egui::TextureHandle>,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionPanel {
    Ai,
    Workiq,
    External(tomlook::connectors::Kind),
}

#[derive(Default)]
struct ExternalStatus {
    busy: bool,
    ready: bool,
    forgetting: bool,
    message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaletteCommand {
    View(View),
    Today,
    Search,
    Ask,
    Prepare,
    Settings,
    Theme,
    Refresh,
    Exit,
}

impl PaletteCommand {
    fn label(self) -> &'static str {
        match self {
            Self::View(View::Day) => "Show day",
            Self::View(View::WorkWeek) => "Show work week",
            Self::View(View::Week) => "Show week",
            Self::View(View::Month) => "Show month",
            Self::Today => "Go to today",
            Self::Search => "Search meetings",
            Self::Ask => "Ask a question",
            Self::Prepare => "Research selected meeting now",
            Self::Settings => "Insights activity",
            Self::Theme => "Toggle dark / light",
            Self::Refresh => "Refresh calendar",
            Self::Exit => "Exit Tomlook",
        }
    }
}

#[cfg(test)]
fn command_matches(query: &str) -> Vec<PaletteCommand> {
    command_matches_in(query, tomlook::i18n::Language::En)
}

fn command_matches_in(query: &str, language: tomlook::i18n::Language) -> Vec<PaletteCommand> {
    let query = query.trim().to_lowercase();
    let mut matches: Vec<_> = [
        PaletteCommand::View(View::Day),
        PaletteCommand::View(View::WorkWeek),
        PaletteCommand::View(View::Week),
        PaletteCommand::View(View::Month),
        PaletteCommand::Today,
        PaletteCommand::Search,
        PaletteCommand::Ask,
        PaletteCommand::Prepare,
        PaletteCommand::Settings,
        PaletteCommand::Theme,
        PaletteCommand::Refresh,
        PaletteCommand::Exit,
    ]
    .into_iter()
    .filter(|command| {
        query.split_whitespace().all(|word| {
            language.text(command.label()).to_lowercase().contains(word)
                || command.label().to_lowercase().contains(word)
        })
    })
    .collect();
    matches.sort_by_key(|command| language.text(command.label()).to_lowercase() != query);
    matches
}

#[derive(Default)]
struct AssistantState {
    question: String,
    answer: Option<ai::Answer>,
    error: Option<String>,
    fingerprint: Option<String>,
    chat: Arc<tomlook::chat::Conversation>,
}

#[derive(Default)]
struct DetachedAsk {
    view: ask::View,
    synced_question: String,
    synced_mode: tomlook::chat::Mode,
    failure: Option<String>,
}

fn ask_viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("tomlook-ask")
}

struct MeetingActionDraft {
    event: Event,
    account: String,
    change: tomlook::recommendations::Change,
    calendar_snapshot: Arc<Calendar>,
    unchanged: bool,
}

struct EventDraft {
    parent: Option<Event>,
    account: Option<String>,
    day: NaiveDate,
    start: u16,
    end: u16,
    title: String,
    description: String,
    category: Option<String>,
    private: bool,
    teams: bool,
    travel: bool,
    travel_minutes: u16,
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

fn meeting_date_label(event: &Event, language: tomlook::i18n::Language) -> String {
    let (first, last) = event.calendar_dates();
    let dates = if first == last {
        language.date(first, true)
    } else {
        language.range(first, last)
    };
    if event.is_all_day {
        format!("{} · {}", dates, language.text("All day"))
    } else {
        format!(
            "{} · {} - {}",
            dates,
            event.start.with_timezone(&Local).format("%H:%M"),
            event.end.with_timezone(&Local).format("%H:%M")
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PreferencesSection {
    Display,
    Prompts,
}

pub struct Tomlook {
    worker: Option<Worker>,
    calendar: Arc<Calendar>,
    preferences: Preferences,
    anchor: NaiveDate,
    mini_anchor: Option<NaiveDate>,
    date_selection: Option<DateSelection>,
    date_drag: Option<(NaiveDate, NaiveDate)>,
    timeline_period: Option<(NaiveDate, NaiveDate, i32)>,
    selected: Option<usize>,
    detail_revision: u64,
    briefing: Option<serde_json::Value>,
    query: String,
    category: String,
    settings: bool,
    preferences_open: bool,
    preferences_section: PreferencesSection,
    prompt_draft: tomlook::prompts::Prompts,
    prompt_kind: tomlook::prompts::Kind,
    messages_open: bool,
    recommendations_tab: bool,
    recommendation_cursor: usize,
    recommendation_selected: Option<String>,
    recommendation_conflict_page: (String, usize),
    meeting_menu: Option<String>,
    meeting_action: Option<MeetingActionDraft>,
    event_draft: Option<EventDraft>,
    time_drag: Option<(NaiveDate, u16, u16)>,
    activity_filter: ActivityFilter,
    insights_expanded: bool,
    preview: bool,
    demo_screenshot: Option<std::path::PathBuf>,
    screenshot_requested: bool,
    palette: bool,
    palette_query: String,
    palette_cursor: usize,
    palette_focus: bool,
    focus_meeting: bool,
    scroll_meeting: bool,
    folder_editor: String,
    repository_editor: String,
    cloud_folder_editor: String,
    source_status: String,
    photos: BTreeMap<String, PersonPhoto>,
    photos_pending: BTreeSet<String>,
    photos_busy: bool,
    photos_context: Option<egui::Context>,
    companies: BTreeMap<String, PersonPhoto>,
    companies_busy: bool,
    focus_search: bool,
    loading: bool,
    error: Option<String>,
    ai: Option<Engine>,
    connection: String,
    workiq_status: String,
    hosted_workiq_account: String,
    hosted_workiq_status: String,
    hosted_workiq_busy: bool,
    workiq_ready: bool,
    calendar_owner: Option<String>,
    workiq_requested_range: Option<tomlook::workiq::connection::Range>,
    connection_panel: Option<ConnectionPanel>,
    ai_ready: bool,
    ai_usage: Option<tomlook::telemetry::Summary>,
    ai_login_busy: bool,
    ai_login_status: Option<String>,
    auto_status: tomlook::decision::Status,
    external_status: BTreeMap<tomlook::connectors::Kind, ExternalStatus>,
    brand_color: Option<Color32>,
    brand_language: Option<tomlook::i18n::Language>,
    webiq_key: String,
    github_client_id: String,
    forget_connector: Option<tomlook::connectors::Kind>,
    assistant: bool,
    assistant_detached: bool,
    ask_unread: bool,
    ask_cursor: usize,
    ask_menu_dismissed: bool,
    detached_ask: Arc<Mutex<DetachedAsk>>,
    ask_input: mpsc::Receiver<ask::Input>,
    ask_output: mpsc::SyncSender<ask::Input>,
    assistant_context: Option<Event>,
    assistant_revision: Option<String>,
    assistant_states: BTreeMap<Option<String>, AssistantState>,
    context_error: Option<String>,
    focus_assistant: bool,
    question: String,
    request_id: u64,
    active_request: Option<u64>,
    chat: Arc<tomlook::chat::Conversation>,
    ask_mode: tomlook::chat::Mode,
    research_recoveries: Arc<Vec<tomlook::chat::Checkpoint>>,
    recovery_account: Option<String>,
    answer: Option<ai::Answer>,
    ai_error: Option<String>,
    activity: Arc<tomlook::worker::Activity>,
    tray: Option<crate::tray::Tray>,
    in_tray: bool,
    last_prepared: Option<(String, Option<chrono::DateTime<chrono::FixedOffset>>)>,
    startup: Option<crate::StartupTrace>,
    startup_built: bool,
    lifecycle: Option<Arc<std::sync::Mutex<crate::Lifecycle>>>,
    interaction_trace: Option<crate::interaction_trace::Trace>,
    render_profile: Option<crate::interaction_trace::RenderProfile>,
}

impl Tomlook {
    fn open_palette(&mut self) {
        self.palette = true;
        self.palette_query.clear();
        self.palette_cursor = 0;
        self.palette_focus = true;
    }

    fn close_palette(&mut self, context: &egui::Context) {
        self.palette = false;
        context.memory_mut(|memory| memory.surrender_focus(Id::new("command-search")));
    }

    fn run_palette_command(&mut self, command: PaletteCommand, context: &egui::Context) {
        self.close_palette(context);
        match command {
            PaletteCommand::View(view) => {
                self.choose_view(view);
            }
            PaletteCommand::Today => self.go_today(),
            PaletteCommand::Search => {
                self.focus_search = true;
                context.request_repaint();
            }
            PaletteCommand::Ask => {
                self.assistant = true;
                self.focus_assistant = true;
            }
            PaletteCommand::Prepare => {
                if let Some(index) = self.selected {
                    self.request_insights(self.calendar.events[index].id.clone());
                } else {
                    self.error = Some("Select a meeting first.".into());
                }
            }
            PaletteCommand::Settings => self.settings = true,
            PaletteCommand::Theme => {
                self.preferences.dark = !self.preferences.dark;
                self.save_preferences();
            }
            PaletteCommand::Refresh => self.refresh_workiq(),
            PaletteCommand::Exit => self.exit(context),
        }
        context.request_repaint();
    }

    pub fn new(
        creation: &eframe::CreationContext<'_>,
        options: Options,
        shutdown: Arc<std::sync::Mutex<crate::Lifecycle>>,
        startup: Option<crate::StartupTrace>,
        preload: Option<tomlook::worker::Preload>,
        interaction_trace: Option<crate::interaction_trace::Trace>,
        demo_screenshot: Option<std::path::PathBuf>,
    ) -> Self {
        let preview = options.demo.is_some();
        let mut startup = startup;
        if let Some(trace) = &mut startup {
            trace.mark("window_and_gl_ready");
        }
        let (worker, mut error) =
            match Worker::start_with(options.clone(), creation.egui_ctx.clone(), preload) {
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
            options.demo.is_some() || startup.is_some() || interaction_trace.is_some(),
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
                workiq_ready: engine.workiq_ready.clone(),
            })
        {
            error = Some(format!("Attach preparation engine: {problem}"));
        }
        if let Some(trace) = &mut startup {
            trace.mark("workers_started");
        }
        let tray = if interaction_trace.is_some() {
            None
        } else {
            match crate::tray::Tray::new(
                creation.egui_ctx.clone(),
                worker.as_ref().map(|worker| worker.commands.clone()),
            ) {
                Ok(tray) => Some(tray),
                Err(problem) => {
                    error = Some(problem);
                    None
                }
            }
        };
        let mut app = Self::initial(worker, ai, error);
        app.preview = preview;
        app.demo_screenshot = demo_screenshot;
        app.tray = tray;
        if startup.is_some() || interaction_trace.is_some() {
            app.lifecycle = Some(shutdown);
        }
        if let Some(trace) = &mut startup {
            trace.mark("constructor_complete");
        }
        app.startup = startup;
        app.interaction_trace = interaction_trace;
        app
    }

    fn initial(worker: Option<Worker>, ai: Option<Engine>, error: Option<String>) -> Self {
        let (ask_output, ask_input) = mpsc::sync_channel(16);
        Self {
            worker,
            error,
            calendar: Arc::new(Calendar::default()),
            preferences: Preferences::default(),
            anchor: Local::now().date_naive(),
            mini_anchor: None,
            date_selection: None,
            date_drag: None,
            timeline_period: None,
            selected: None,
            detail_revision: 0,
            briefing: None,
            query: String::new(),
            category: String::new(),
            settings: false,
            preferences_open: false,
            preferences_section: PreferencesSection::Display,
            prompt_draft: Default::default(),
            prompt_kind: tomlook::prompts::Kind::Recap,
            messages_open: false,
            recommendations_tab: true,
            recommendation_cursor: 0,
            recommendation_selected: None,
            recommendation_conflict_page: Default::default(),
            meeting_menu: None,
            meeting_action: None,
            event_draft: None,
            time_drag: None,
            activity_filter: ActivityFilter::All,
            insights_expanded: false,
            preview: false,
            demo_screenshot: None,
            screenshot_requested: false,
            palette: false,
            palette_query: String::new(),
            palette_cursor: 0,
            palette_focus: false,
            focus_meeting: false,
            scroll_meeting: false,
            folder_editor: String::new(),
            repository_editor: String::new(),
            cloud_folder_editor: String::new(),
            source_status: "Sources not indexed yet".into(),
            ai_usage: None,
            photos: BTreeMap::new(),
            photos_pending: BTreeSet::new(),
            photos_busy: false,
            photos_context: None,
            companies: BTreeMap::new(),
            companies_busy: false,
            focus_search: false,
            loading: true,
            ai,
            connection: "Not connected".into(),
            workiq_status: "WorkIQ authentication has not been tested".into(),
            hosted_workiq_account: String::new(),
            hosted_workiq_status: "Not connected".into(),
            hosted_workiq_busy: false,
            workiq_ready: false,
            calendar_owner: None,
            workiq_requested_range: None,
            connection_panel: None,
            ai_ready: false,
            ai_login_busy: false,
            ai_login_status: None,
            auto_status: Default::default(),
            external_status: BTreeMap::new(),
            brand_color: None,
            brand_language: None,
            webiq_key: String::new(),
            github_client_id: String::new(),
            forget_connector: None,
            assistant: false,
            assistant_detached: false,
            ask_unread: false,
            ask_cursor: 0,
            ask_menu_dismissed: false,
            detached_ask: Arc::new(Mutex::new(DetachedAsk::default())),
            ask_input,
            ask_output,
            assistant_context: None,
            assistant_revision: None,
            assistant_states: BTreeMap::new(),
            context_error: None,
            focus_assistant: false,
            question: String::new(),
            request_id: 0,
            active_request: None,
            chat: Default::default(),
            ask_mode: Default::default(),
            research_recoveries: Default::default(),
            recovery_account: None,
            answer: None,
            ai_error: None,
            activity: Arc::new(tomlook::worker::Activity::default()),
            tray: None,
            in_tray: false,
            last_prepared: None,
            startup: None,
            startup_built: false,
            lifecycle: None,
            interaction_trace: None,
            render_profile: None,
        }
    }

    fn command(&mut self, command: Command) {
        let Some(worker) = &self.worker else {
            self.error = Some("The storage worker is unavailable. Restart Tomlook.".into());
            return;
        };
        match worker.commands.try_send(command) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                self.error = Some("Local queue is full. Nothing was submitted; try again.".into());
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.error = Some("The storage worker is unavailable. Restart Tomlook.".into());
            }
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
            if let Some(error) = &activity.error
                && activity.error != self.activity.error
            {
                self.error = Some(error.clone());
            } else if activity.error.is_none() && self.error == self.activity.error {
                self.error = None;
            }
            let recaps_changed = !Arc::ptr_eq(&self.activity.recaps, &activity.recaps);
            self.activity = activity;
            if recaps_changed && let Some(index) = self.selected
                && self.calendar.events[index].end <= Local::now().fixed_offset() {
                self.detail_revision += 1;
                self.command(Command::Detail { revision: self.detail_revision, id: self.calendar.events[index].id.clone() });
            }
            if let Some(index) = self.selected
                && let Some(job) = self
                    .activity
                    .queue
                    .jobs
                    .get(&self.calendar.events[index].id)
                && job.state == tomlook::scheduler::State::Completed
                && self
                    .last_prepared
                    .as_ref()
                    .is_none_or(|(fingerprint, prepared)| {
                        fingerprint != &job.fingerprint || *prepared != job.prepared_at
                    })
            {
                self.last_prepared = Some((job.fingerprint.clone(), job.prepared_at));
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
                    calendar_owner,
                } => {
                    if let Some(context) = &self.assistant_context {
                        let updated = calendar.events.iter().find(|event| event.id == context.id);
                        if updated.is_none_or(|event| event.fingerprint() != context.fingerprint())
                        {
                            let updated = updated.cloned();
                            self.cancel_ai();
                            self.answer = None;
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
                    if self.calendar_owner != calendar_owner {
                        self.invalidate_connection_answers();
                        self.research_recoveries = Default::default();
                    }
                    self.calendar_owner = calendar_owner;
                    if self.recovery_account != self.calendar_owner {
                        self.recovery_account = self.calendar_owner.clone();
                        if let Some(account) = self.calendar_owner.clone() {
                            self.ai_command(ai::Command::RecoverChat(account));
                        }
                    }
                    if self.hosted_workiq_account.is_empty() {
                        self.hosted_workiq_account = if preferences.work_email.is_empty() {
                            self.calendar_owner.clone().unwrap_or_default()
                        } else {
                            preferences.work_email.clone()
                        };
                    }
                    self.folder_editor = preferences
                        .context_folders
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.repository_editor = preferences
                        .repository_collections
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.cloud_folder_editor = preferences.cloud_context_folder.clone();
                    if self.loading && self.github_client_id.is_empty() {
                        self.github_client_id = preferences.github_client_id.clone();
                    }
                    self.preferences = *preferences;
                    self.loading = false;
                    if let Some(trace) = &mut self.startup
                        && !self.startup_built
                    {
                        trace.mark("calendar_notice_received");
                    }
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
                Notice::LocalAnswer { id, result } => {
                    if self
                        .chat
                        .turns
                        .iter()
                        .any(|turn| turn.id == id && turn.running)
                    {
                        if self.active_request == Some(id) {
                            self.active_request = None;
                        }
                        match result {
                            Ok(result) => {
                                if let Some(turn) = Arc::make_mut(&mut self.chat)
                                    .turns
                                    .iter_mut()
                                    .find(|turn| turn.id == id)
                                {
                                    turn.meetings = result.meetings;
                                }
                                self.answer = Some(result.answer.clone());
                                Arc::make_mut(&mut self.chat).finish(id, Ok(result.answer));
                            }
                            Err(error) => Arc::make_mut(&mut self.chat).finish(id, Err(error)),
                        }
                        self.ask_unread = !self.assistant;
                    }
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
                ai::Notice::Auto(status) => self.auto_status = status,
                ai::Notice::External {
                    kind,
                    busy,
                    ready,
                    message,
                    client_id,
                } => {
                    let previous = self.external_status.entry(kind).or_default();
                    let forgetting = busy && previous.forgetting;
                    if !busy && (previous.ready != ready || ready) {
                        self.invalidate_connection_answers();
                    }
                    self.external_status.insert(
                        kind,
                        ExternalStatus {
                            busy,
                            ready,
                            forgetting,
                            message,
                        },
                    );
                    if let Some(client_id) = client_id {
                        self.github_client_id = client_id.clone();
                        if self.preferences.github_client_id != client_id {
                            self.preferences.github_client_id = client_id;
                            self.save_preferences();
                        }
                    }
                }
                ai::Notice::Sources(message) => self.source_status = message,
                ai::Notice::Usage(summary) => self.ai_usage = Some(summary),
                ai::Notice::Diagnostic(message) => self.error = Some(message),
                ai::Notice::Companies(results) => {
                    self.companies_busy = false;
                    for (domain, result) in results.into_iter().take(4) {
                        let icon = match result {
                            Ok(Some(image)) if image.size[0] <= 32 && image.size[1] <= 32 => {
                                PersonPhoto {
                                    image: self.photos_context.as_ref().map(|context| {
                                        context.load_texture(
                                            format!("company-{domain}"),
                                            image,
                                            egui::TextureOptions::LINEAR,
                                        )
                                    }),
                                    error: None,
                                }
                            }
                            Ok(Some(_)) => PersonPhoto {
                                image: None,
                                error: Some("Company icon exceeds its display limit".into()),
                            },
                            Ok(None) => PersonPhoto {
                                image: None,
                                error: Some("No icon found on the company's own website".into()),
                            },
                            Err(error) => PersonPhoto {
                                image: None,
                                error: Some(error),
                            },
                        };
                        self.companies.insert(domain, icon);
                    }
                }
                ai::Notice::AiLogin { busy, message } => {
                    self.ai_login_busy = busy;
                    if busy {
                        self.connection = message.clone();
                    }
                    self.ai_login_status = Some(message);
                }
                ai::Notice::HostedWorkiq {
                    busy,
                    ready,
                    message,
                } => {
                    self.hosted_workiq_busy = busy;
                    if self.workiq_ready && !ready {
                        self.invalidate_connection_answers();
                    }
                    self.workiq_ready = ready;
                    if !ready {
                        self.workiq_requested_range = None;
                        self.photos.clear();
                        self.photos_pending.clear();
                        self.photos_busy = false;
                    }
                    self.hosted_workiq_status = message;
                }
                ai::Notice::Workiq { message } => self.workiq_status = message,
                ai::Notice::Photos(results) => {
                    self.photos_busy = false;
                    self.photos_pending.clear();
                    if self.workiq_ready {
                        if self.photos.len() + results.len() > 128 {
                            self.photos.clear();
                        }
                        for (email, result) in results.into_iter().take(8) {
                            let photo = match result {
                                Ok(Some(image)) if image.size[0] <= 48 && image.size[1] <= 48 => {
                                    PersonPhoto {
                                        image: self.photos_context.as_ref().map(|context| {
                                            context.load_texture(
                                                format!("person-{email}"),
                                                image,
                                                egui::TextureOptions::LINEAR,
                                            )
                                        }),
                                        error: None,
                                    }
                                }
                                Ok(Some(_)) => PersonPhoto {
                                    image: None,
                                    error: Some("Photo exceeds its display limit".into()),
                                },
                                Ok(None) => PersonPhoto {
                                    image: None,
                                    error: None,
                                },
                                Err(error) => PersonPhoto {
                                    image: None,
                                    error: Some(error),
                                },
                            };
                            self.photos.insert(email, photo);
                        }
                    }
                }
                ai::Notice::Connection { ready, message } => {
                    self.ai_ready = ready;
                    self.connection = message;
                    self.command(Command::Wake);
                }
                ai::Notice::ChatProgress { id, progress } => {
                    if let Some(turn) = Arc::make_mut(&mut self.chat)
                        .turns
                        .iter_mut()
                        .find(|turn| turn.id == id && turn.running)
                    {
                        if progress.mode == tomlook::chat::Mode::Deep
                            && self.active_request == Some(id)
                        {
                            self.active_request = None;
                        }
                        turn.progress = progress;
                        Arc::make_mut(&mut self.chat).revision += 1;
                    }
                }
                ai::Notice::ChatRecovered { account, result } => {
                    if self.calendar_owner.as_deref() == Some(account.as_str()) {
                        match result {
                            Ok(saved) => {
                                self.research_recoveries = Arc::new(
                                    saved
                                        .into_iter()
                                        .filter(|saved| {
                                            !self.chat.turns.iter().any(|turn| {
                                                turn.running
                                                    && turn.progress.research_key.as_deref()
                                                        == Some(&saved.key)
                                            })
                                        })
                                        .collect(),
                                )
                            }
                            Err(error) => self.ai_error = Some(error),
                        }
                    }
                }
                ai::Notice::Answer { id, result }
                    if self.active_request == Some(id)
                        || self
                            .chat
                            .turns
                            .iter()
                            .any(|turn| turn.id == id && turn.running) =>
                {
                    if self.active_request == Some(id) {
                        self.active_request = None;
                    }
                    self.ask_unread = !self.assistant;
                    let result=result.and_then(|answer| {
                        if answer.text.len()>64_000 { return Err("AI answer exceeded the 64 KiB response limit; no answer was published.".into()); }
                        answer.evidence.validate()?;
                        if answer.proposals.len()>proposals::PROPOSAL_LIMIT { return Err("AI response exceeded the local-proposal limit".into()); }
                        for proposal in answer.proposals.iter() { proposal.validate()?; }
                        Ok(answer)
                    });
                    self.answer = result.as_ref().ok().cloned();
                    self.ai_error = result.as_ref().err().cloned();
                    Arc::make_mut(&mut self.chat).finish(id, result);
                }
                ai::Notice::Answer { .. } => {}
                ai::Notice::Stopped => {
                    if !self.ai_ready && self.connection != "Not connected" {
                        self.error = Some(format!("AI stopped: {}", self.connection));
                    }
                    self.ai_login_busy = false;
                    self.hosted_workiq_busy = false;
                    self.workiq_ready = false;
                    self.ai_ready = false;
                    if self.connection == "Connected" {
                        self.connection = "AI stopped".into();
                    }
                    if self.active_request.take().is_some() {
                        self.answer = None;
                        self.ask_unread = !self.assistant;
                        self.ai_error = Some("AI stopped without a completed answer; provider outcome may be unknown".into());
                    }
                    let ids = self
                        .chat
                        .turns
                        .iter()
                        .filter(|turn| turn.running)
                        .map(|turn| turn.id)
                        .collect::<Vec<_>>();
                    for id in ids {
                        Arc::make_mut(&mut self.chat).finish(id,Err("AI stopped without a completed answer; provider outcome may be unknown".into()));
                    }
                }
            }
        }
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.insights_expanded = false;
        self.detail_revision += 1;
        self.briefing = None;
        self.command(Command::Detail {
            revision: self.detail_revision,
            id: self.calendar.events[index].id.clone(),
        });
        self.last_prepared = None;
    }

    fn accent(&self) -> Color32 {
        let (_, light, dark) = ACCENTS[self.preferences.accent.min(3)];
        let [r, g, b] = if self.preferences.dark { dark } else { light };
        Color32::from_rgb(r, g, b)
    }

    fn sync_brand(&mut self, context: &egui::Context) {
        let language = self.preferences.language;
        if self.brand_language != Some(language) {
            if let Some(tray) = &self.tray
                && let Err(error) = tray.set_language(language)
            {
                self.error = Some(error);
            }
            self.brand_language = Some(language);
        }
        let color = self.accent();
        if self.brand_color == Some(color) {
            return;
        }
        self.brand_color = Some(color);
        let rgb = [color.r(), color.g(), color.b()];
        context.send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(
            egui::IconData {
                rgba: tomlook::brand::rgba_with_color(64, rgb),
                width: 64,
                height: 64,
            },
        ))));
        if let Some(tray) = &self.tray
            && let Err(error) = tray.set_color(rgb)
        {
            eprintln!("Tomlook: {error}");
            self.error = Some(error);
        }
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
            {
                context.request_repaint();
            } else if self.palette {
                self.close_palette(context);
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

    fn save_preferences(&mut self) {
        self.command(Command::Preferences(self.preferences.clone()));
    }

    fn visible_days(&self) -> Vec<NaiveDate> {
        self.date_drag
            .map(|(first, last)| DateSelection::new(first, last))
            .or(self.date_selection)
            .map_or_else(
                || self.preferences.view.days(self.anchor),
                DateSelection::days,
            )
    }

    fn mini_selected(&self, date: NaiveDate) -> bool {
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

    fn matches_event(&self, index: usize, query: &tomlook::calendar::SearchQuery) -> bool {
        self.calendar.matches_query(index, query, &self.category)
            && !(self.preferences.hide_unconfirmed && self.calendar.events[index].unconfirmed())
    }

    fn choose_dates(&mut self, start: NaiveDate, end: NaiveDate) {
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

    fn choose_view(&mut self, view: View) {
        self.date_drag = None;
        self.date_selection = None;
        self.preferences.view = view;
        self.save_preferences();
    }

    fn shift_period(&mut self, direction: i32) {
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

    fn go_today(&mut self) {
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

    fn header(&mut self, ui: &mut egui::Ui) {
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

    fn sidebar(&mut self, ui: &mut egui::Ui) {
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

    fn event_card(&mut self, ui: &mut egui::Ui, index: usize, rect: egui::Rect) {
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

    fn choose_meeting_action(&mut self, event: Event, change: tomlook::recommendations::Change) {
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

    fn meeting_actions(&mut self, ui: &mut egui::Ui, index: usize) {
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

    fn open_event_draft(&mut self, day: NaiveDate, start: u16, end: u16) {
        self.event_draft = Some(EventDraft {
            parent: None,
            account: self.calendar_owner.clone(), day, start, end,
            title: String::new(), description: String::new(), category: None,
            private: false, teams: false, travel: false, travel_minutes: 30, focus_title: true,
        });
    }

    fn open_travel_draft(&mut self, event: Event) {
        self.open_event_draft(event.start.with_timezone(&Local).date_naive(), 0, 15);
        if let Some(draft) = &mut self.event_draft {
            draft.parent = Some(event);
            draft.travel = true;
            draft.focus_title = false;
        }
    }

    fn event_creation_dialog(&mut self, context: &egui::Context) {
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

    fn select_event_time(&mut self, ui: &mut egui::Ui, day: NaiveDate, rect: egui::Rect, cards: &[egui::Rect]) {
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

    fn meeting_action_dialog(&mut self, context: &egui::Context) {
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

    fn month(&mut self, ui: &mut egui::Ui) {
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

    fn timeline(&mut self, ui: &mut egui::Ui) {
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

    fn details(&mut self, ui: &mut egui::Ui, event: Event) {
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
        if !ended && let Some(url) = event.extra.get("onlineMeeting")
            .and_then(|meeting| meeting["joinUrl"].as_str()).filter(|url| safe_url(url))
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
        ui.horizontal(|ui| {
            ui.strong(language.text(if ended { "Meeting notes" } else { "Insights" }));
            if self.briefing.is_some() && ui.button(language.text("Expand")).clicked() {
                self.insights_expanded = true;
            }
        });
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

    fn render_insights(&self, ui: &mut egui::Ui, expanded: bool) {
        let language = self.preferences.language;
        if let Some(briefing) = &self.briefing {
            let notes = briefing.get("tomlook_recap").and_then(serde_json::Value::as_bool) == Some(true);
            let narrative = language.insight(briefing);
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
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(8)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.strong(language.text("Overview"));
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
                            insight_section(ui, &language.text(label), items, self.accent());
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
            ui.columns(columns, |uis| {
                for (index, (field, label)) in sections.iter().enumerate() {
                    if let Some(items) = narrative
                        .and_then(|value| value.get(field))
                        .and_then(|value| value.as_array())
                        .filter(|items| !items.is_empty())
                    {
                        let ui = &mut uis[index % columns];
                        insight_section(ui, &language.text(label), items, self.accent());
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
        } else {
            let ended = self.selected.and_then(|index| self.calendar.events.get(index))
                .is_some_and(|event| event.end <= Local::now().fixed_offset());
            ui.label(language.text(if ended { "No AI meeting notes yet." } else {
                "No saved Insights yet. Research now prioritizes this meeting."
            }));
        }
    }

    fn request_insights(&mut self, id: String) {
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
        let mut ids = self
            .chat
            .turns
            .iter()
            .filter(|turn| turn.running)
            .map(|turn| turn.id)
            .collect::<std::collections::BTreeSet<_>>();
        ids.extend(self.active_request);
        for id in ids {
            self.cancel_chat_request(id);
        }
    }

    fn cancel_chat_request(&mut self, id: u64) {
        let local = self
            .chat
            .turns
            .iter()
            .any(|turn| turn.id == id && turn.progress.stage == "Local calendar");
        if self.active_request == Some(id) {
            self.active_request = None;
        }
        self.ask_unread = !self.assistant;
        let message = "Cancellation requested; partial output is not a completed answer. Provider outcome may be unknown.";
        self.ai_error = Some(message.into());
        Arc::make_mut(&mut self.chat).finish(id, Err(message.into()));
        if !local && !self.ai_command(ai::Command::Cancel(id)) {
            self.error=Some("AI cancellation could not be submitted. Its result will be discarded, but the provider request may continue until timeout or application exit.".into());
        }
    }

    fn invalidate_connection_answers(&mut self) {
        self.ai_error = None;
        self.cancel_ai();
        self.answer = None;
        self.chat = Default::default();
        for state in self.assistant_states.values_mut() {
            state.answer = None;
            state.error = None;
            state.chat = Default::default();
        }
    }

    fn switch_assistant_context(&mut self, next: Option<Event>) -> bool {
        if self.assistant_detached {
            let shared = match self.detached_ask.try_lock() {
                Ok(shared) => shared,
                Err(std::sync::TryLockError::WouldBlock) => {
                    self.context_error =
                        Some("Ask is updating. Try changing context again.".into());
                    return false;
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    self.context_error =
                        Some("The Ask window state is unavailable. Restart Tomlook.".into());
                    return false;
                }
            };
            if shared.view.revision == self.assistant_revision
                && shared.view.context.as_ref().map(|event| event.id.as_str())
                    == self
                        .assistant_context
                        .as_ref()
                        .map(|event| event.id.as_str())
            {
                self.question = shared.view.question.clone();
            }
        }
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
                self.chat = Default::default();
                self.ai_error = Some("Meeting context changed. The previous answer was invalidated; review your draft before asking again.".into());
            }
            self.assistant_context = next;
            self.assistant_revision = fingerprint;
            return true;
        }
        if self.active_request.is_some() || self.chat.running() {
            self.ai_error =
                Some("Stop the current question before changing its meeting context.".into());
            return false;
        }
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
                answer: self.answer.take(),
                error: self.ai_error.take(),
                fingerprint: self.assistant_context.as_ref().map(Event::fingerprint),
                chat: std::mem::take(&mut self.chat),
            },
        );
        if restored.fingerprint != fingerprint
            && (restored.answer.is_some() || !restored.chat.turns.is_empty())
        {
            restored.answer = None;
            restored.chat = Default::default();
            restored.error = Some("Meeting context changed. The previous answer was invalidated; review your draft before asking again.".into());
        }
        self.question = restored.question;
        self.answer = restored.answer;
        self.ai_error = restored.error;
        self.chat = restored.chat;
        self.assistant_context = next;
        self.assistant_revision = fingerprint;
        self.context_error = None;
        self.focus_assistant = true;
        self.ask_cursor = 0;
        self.ask_menu_dismissed = false;
        true
    }

    fn ask_question(&mut self) {
        if self.context_error.is_some() {
            self.ai_error = Some("Connect AI before asking.".into());
            return;
        }
        if self.chat.turns.len() >= tomlook::chat::TURN_LIMIT {
            self.ai_error =
                Some("This chat holds 32 questions; start a new chat to continue".into());
            return;
        }
        if self.question.trim().is_empty() || self.question.len() > 8000 {
            self.ai_error = Some("Use a nonempty question of at most 8,000 bytes.".into());
            return;
        }
        let submission = match ask::expand(
            &self.question,
            self.assistant_context.as_ref(),
            self.preferences.language,
        ) {
            Ok(submission) => submission,
            Err(error) => {
                self.ai_error = Some(error);
                return;
            }
        };
        if matches!(submission, ask::Submission::Question(_))
            && self.chat.turns.iter().any(|turn| {
                turn.running
                    && turn.progress.stage != "Local calendar"
                    && (turn.progress.mode == tomlook::chat::Mode::Deep)
                        == (self.ask_mode == tomlook::chat::Mode::Deep)
            })
        {
            self.ai_error = Some(
                "This answer lane is busy; choose the other mode, wait or stop its question".into(),
            );
            return;
        }
        if matches!(submission, ask::Submission::Question(_))
            && self.active_request.is_some()
            && self.chat.turns.is_empty()
            && self.ask_mode != tomlook::chat::Mode::Deep
        {
            return;
        }
        if matches!(submission, ask::Submission::Local(_))
            && self
                .chat
                .turns
                .iter()
                .any(|turn| turn.running && turn.progress.stage == "Local calendar")
        {
            self.ai_error = Some("A local command is already running; wait for its result".into());
            return;
        }
        let Some(id) = self.request_id.checked_add(1).filter(|id| *id < 1 << 63) else {
            self.ai_error = Some("Assistant request identity limit reached. No question was submitted; restart Tomlook.".into());
            return;
        };
        self.request_id = id;
        self.ai_error = None;
        let raw = self.question.clone();
        let question = match submission {
            ask::Submission::Question(question) => question,
            ask::Submission::Review(kind) => {
                if !self.ai_ready {
                    self.ai_error = Some("Connect AI before asking.".into());
                    return;
                }
                if self.request_recommendation_review(kind) {
                    if let Err(error) =
                        Arc::make_mut(&mut self.chat).admit(id, raw, tomlook::chat::Mode::Fast)
                    {
                        self.ai_error = Some(error);
                        return;
                    }
                    if let Some(answer) = self.answer.clone() {
                        Arc::make_mut(&mut self.chat).finish(id, Ok(answer));
                    }
                    self.question.clear();
                    self.focus_assistant = true;
                }
                return;
            }
            ask::Submission::Local(query) => {
                let command = Command::AskLocal {
                    id,
                    query,
                    account: self.calendar_owner.clone(),
                    meeting: self.assistant_context.clone(),
                    language: self.preferences.language,
                };
                let sent = self.worker.as_ref().map(|worker| {
                    worker
                        .commands
                        .try_send(command)
                        .map_err(|error| matches!(error, mpsc::TrySendError::Full(_)))
                });
                match sent {
                    Some(Ok(())) => {
                        if let Err(error) =
                            Arc::make_mut(&mut self.chat).admit(id, raw, tomlook::chat::Mode::Fast)
                        {
                            self.ai_error = Some(error);
                            return;
                        }
                        if let Some(turn) = Arc::make_mut(&mut self.chat).turns.back_mut() {
                            turn.progress.stage = "Local calendar".into();
                        }
                        self.question.clear();
                        self.focus_assistant = true;
                        self.ask_unread = false;
                    }
                    Some(Err(true)) => {
                        self.ai_error =
                            Some("Local queue is full. Nothing was submitted; try again.".into())
                    }
                    _ => {
                        self.ai_error =
                            Some("The storage worker is unavailable. Restart Tomlook.".into())
                    }
                }
                return;
            }
        };
        if !self.ai_ready {
            self.ai_error = Some("Connect AI before asking.".into());
            return;
        }
        let request = tomlook::chat::Request {
            question: Question {
                id,
                question,
                meeting: self.assistant_context.clone(),
                account: self.calendar_owner.clone(),
                public_topic: None,
                suggest_drafts: false,
                language: self.preferences.language,
            },
            mode: self.ask_mode,
            history: self.chat.history(),
            resume: None,
        };
        if self.ai_command(ai::Command::Chat(Box::new(request))) {
            if let Err(error) = Arc::make_mut(&mut self.chat).admit(id, raw, self.ask_mode) {
                self.ai_error = Some(error);
                return;
            }
            if self.ask_mode != tomlook::chat::Mode::Deep {
                self.active_request = Some(id);
            }
            self.answer = None;
            self.ask_unread = false;
            self.question.clear();
            self.focus_assistant = true;
        }
    }

    fn request_recommendation_review(&mut self, kind: tomlook::recommendations::Review) -> bool {
        let Some(account) = self
            .calendar_owner
            .clone()
            .filter(|account| !account.is_empty())
        else {
            self.ai_error = Some("Connect AI and WorkIQ before starting a calendar review.".into());
            return false;
        };
        let command = Command::ReviewRecommendations { kind, account };
        let Some(worker) = &self.worker else {
            self.ai_error = Some("The storage worker is unavailable. Restart Tomlook.".into());
            return false;
        };
        match worker.commands.try_send(command) {
            Ok(()) => {
                let message = match kind {
                    tomlook::recommendations::Review::Travel => {
                        "Travel review queued for the next 14 days. Suggestions will appear in Messages; calendar changes still need approval."
                    }
                    tomlook::recommendations::Review::Categories => {
                        "Category review queued for the next 14 days. Suggestions will appear in Messages; calendar changes still need approval."
                    }
                };
                self.answer = Some(ai::Answer::new(
                    self.preferences.language.text(message).into_owned(),
                    Arc::new(Default::default()),
                    Arc::new(Vec::new()),
                ));
                self.ask_unread = false;
                true
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.ai_error =
                    Some("Local queue is full. Nothing was submitted; try again.".into());
                false
            }
            _ => {
                self.ai_error = Some("The storage worker is unavailable. Restart Tomlook.".into());
                false
            }
        }
    }

    fn resume_research(&mut self, key: String) {
        let Some(saved) = self
            .research_recoveries
            .iter()
            .find(|saved| saved.key == key)
            .cloned()
        else {
            self.ai_error = Some("This saved research task is unavailable".into());
            return;
        };
        let meeting = if let Some(event) = &saved.meeting {
            match self
                .calendar
                .events
                .iter()
                .find(|candidate| {
                    candidate.id == event.id && candidate.fingerprint() == event.fingerprint
                })
                .cloned()
            {
                Some(current) => Some(current),
                None => {
                    self.ai_error =
                        Some("The saved research meeting changed; start a new question".into());
                    return;
                }
            }
        } else {
            None
        };
        if !self.ai_ready || !self.switch_assistant_context(meeting.clone()) {
            return;
        }
        if self.chat.turns.len() >= tomlook::chat::TURN_LIMIT {
            self.ai_error =
                Some("This chat holds 32 questions; start a new chat to continue".into());
            return;
        }
        let Some(id) = self.request_id.checked_add(1).filter(|id| *id < 1 << 63) else {
            self.ai_error=Some("Assistant request identity limit reached. No question was submitted; restart Tomlook.".into());
            return;
        };
        let request = tomlook::chat::Request {
            question: Question {
                id,
                question: saved.question.clone(),
                meeting,
                account: self.calendar_owner.clone(),
                public_topic: None,
                suggest_drafts: false,
                language: saved.language,
            },
            mode: tomlook::chat::Mode::Deep,
            history: saved.history,
            resume: Some(key.clone()),
        };
        if self.ai_command(ai::Command::Chat(Box::new(request))) {
            self.request_id = id;
            if let Err(error) =
                Arc::make_mut(&mut self.chat).admit(id, saved.question, tomlook::chat::Mode::Deep)
            {
                self.ai_error = Some(error);
                return;
            }
            self.research_recoveries = Arc::new(
                self.research_recoveries
                    .iter()
                    .filter(|saved| saved.key != key)
                    .cloned()
                    .collect(),
            );
            self.ai_error = None;
            self.ask_unread = false;
        }
    }

    fn retry_chat_question(&mut self, id: u64) {
        let Some(turn) = self
            .chat
            .turns
            .iter()
            .find(|turn| turn.id == id && !turn.running && turn.error.is_some())
            .cloned()
        else {
            return;
        };
        let draft = std::mem::replace(&mut self.question, turn.question);
        let mode = std::mem::replace(&mut self.ask_mode, turn.progress.mode);
        self.ask_question();
        if !draft.is_empty() {
            self.question = draft;
        }
        self.ask_mode = mode;
    }

    fn prepare_follow_up(&mut self, recommendation: &tomlook::recommendations::Recommendation) {
        let now = chrono::Local::now().fixed_offset();
        if recommendation.suggestion.kind != tomlook::recommendations::Kind::FollowUp
            || !recommendation.visible(&self.calendar, self.calendar_owner.as_deref(), now)
        {
            self.error = Some("Recommendation changed or is no longer available; refresh Insights before preparing a draft.".into());
            return;
        }
        let Some(event) = recommendation
            .current(&self.calendar, self.calendar_owner.as_deref(), now)
            .cloned()
        else {
            self.error = Some("Recommendation changed or is no longer available; refresh Insights before preparing a draft.".into());
            return;
        };
        self.assistant = true;
        if !self.switch_assistant_context(Some(event)) {
            return;
        }
        if self.question.trim().is_empty() {
            self.question = self.preferences.language.text("Prepare a local follow-up draft for this meeting using saved Insights and retrieved evidence. Separate verified outcomes from pre-meeting plans; use placeholders for unknown decisions, owners and dates. Do not send any message, invite attendees or modify the calendar.").into_owned();
        } else {
            self.ai_error = Some("An unsent question is already saved for this meeting; it was preserved. Review it before asking for a follow-up draft.".into());
        }
        self.focus_assistant = true;
        self.messages_open = false;
    }

    fn ask_view(&self) -> ask::View {
        ask::View {
            question: self.question.clone(),
            answer: self.answer.clone(),
            error: self.context_error.clone().or_else(|| {
                self.ai_error
                    .as_ref()
                    .filter(|error| {
                        !self
                            .chat
                            .turns
                            .iter()
                            .any(|turn| turn.error.as_ref() == Some(*error))
                    })
                    .cloned()
            }),
            context: self.assistant_context.clone(),
            revision: self.assistant_revision.clone(),
            selected: self.selected.is_some(),
            ready: self.ai_ready && self.context_error.is_none(),
            busy: self.active_request.is_some(),
            cursor: self.ask_cursor,
            menu_dismissed: self.ask_menu_dismissed,
            focus: self.focus_assistant,
            language: self.preferences.language,
            accent: self.accent(),
            chat: self.chat.clone(),
            recoveries: self.research_recoveries.clone(),
            mode: self.ask_mode,
            auto_status: self.auto_status.clone(),
            ..Default::default()
        }
    }

    fn toggle_assistant(&mut self, context: &egui::Context) {
        if self.assistant {
            self.apply_ask_input(self.ask_view().input(ask::Action::Hide), context);
        } else {
            self.assistant = true;
            self.ask_unread = false;
            self.focus_assistant = true;
            if self.assistant_detached {
                context.send_viewport_cmd_to(ask_viewport(), egui::ViewportCommand::Visible(true));
                context.send_viewport_cmd_to(ask_viewport(), egui::ViewportCommand::Focus);
            }
        }
    }

    fn apply_ask_input(&mut self, input: ask::Input, context: &egui::Context) {
        let same_context = input.context_id.as_deref()
            == self
                .assistant_context
                .as_ref()
                .map(|event| event.id.as_str())
            && input.revision == self.assistant_revision;
        if same_context {
            self.question = input.question;
            self.ask_cursor = input.cursor;
            self.ask_menu_dismissed = input.menu_dismissed;
            self.ask_mode = input.mode;
        }
        match input.action {
            ask::Action::Submit if !same_context => {
                self.ai_error = Some(
                    "Meeting context changed. Review the question before submitting it.".into(),
                );
            }
            ask::Action::Submit => self.ask_question(),
            ask::Action::Stop => self.cancel_ai(),
            ask::Action::StopRequest(id) => self.cancel_chat_request(id),
            ask::Action::ModeChanged => {}
            ask::Action::NewChat => {
                if !self.chat.running() && self.active_request.is_none() {
                    self.chat = Default::default();
                    self.answer = None;
                    self.ai_error = None;
                    self.focus_assistant = true;
                }
            }
            ask::Action::OpenMeeting(id) => {
                if let Some(index) = self.calendar.events.iter().position(|event| event.id == id) {
                    let event = self.calendar.events[index].clone();
                    self.open_recommended_meeting(&event);
                } else {
                    self.ai_error = Some(
                        "Meeting is no longer in the local calendar; refresh and search again"
                            .into(),
                    );
                }
            }
            ask::Action::Resume(key) => self.resume_research(key),
            ask::Action::DiscardResearch(key) => {
                if let Some(account) = self.calendar_owner.clone()
                    && self
                        .research_recoveries
                        .iter()
                        .any(|saved| saved.key == key && saved.account == account)
                {
                    self.ai_command(ai::Command::DiscardChat { account, key });
                }
            }
            ask::Action::Retry(id) => self.retry_chat_question(id),
            ask::Action::ConfigureAuto => {
                self.open_ai_connection();
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
            ask::Action::Hide => {
                self.assistant = false;
                self.focus_assistant = false;
                self.focus_search = true;
                context.memory_mut(|memory| memory.surrender_focus(Id::new("assistant-question")));
                context.send_viewport_cmd_to(ask_viewport(), egui::ViewportCommand::Visible(false));
            }
            ask::Action::Detach => {
                self.assistant_detached = true;
                self.focus_assistant = true;
                context.memory_mut(|memory| memory.surrender_focus(Id::new("assistant-question")));
            }
            ask::Action::Dock => {
                self.assistant_detached = false;
                self.focus_assistant = true;
                context.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Visible(true),
                );
                context.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Minimized(false),
                );
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                context.send_viewport_cmd_to(ask_viewport(), egui::ViewportCommand::Visible(false));
            }
            ask::Action::UseSelected => {
                if let Some(event) = self
                    .selected
                    .map(|index| self.calendar.events[index].clone())
                {
                    self.switch_assistant_context(Some(event));
                } else {
                    self.ai_error = Some("Select a meeting first.".into());
                }
            }
            ask::Action::ClearContext => {
                self.switch_assistant_context(None);
            }
            ask::Action::Exit => self.exit(context),
        }
        context.request_repaint();
    }

    fn receive_ask_inputs(&mut self, context: &egui::Context) {
        for _ in 0..16 {
            match self.ask_input.try_recv() {
                Ok(input) => self.apply_ask_input(input, context),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.ai_error = Some("The Ask window is unavailable. Restart Tomlook.".into());
                    break;
                }
            }
        }
    }

    fn sync_detached_ask(&mut self, context: &egui::Context) {
        if !self.assistant_detached {
            return;
        }
        let desired = self.ask_view();
        let mut shared = match self.detached_ask.try_lock() {
            Ok(shared) => shared,
            Err(std::sync::TryLockError::WouldBlock) => {
                context.request_repaint_after(std::time::Duration::from_millis(16));
                return;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                self.ai_error =
                    Some("The Ask window state is unavailable. Restart Tomlook.".into());
                return;
            }
        };
        if let Some(error) = shared.failure.take() {
            self.ai_error = Some(error.clone());
            shared.view.error = Some(error);
            context.request_repaint();
            context.request_repaint_of(ask_viewport());
            return;
        }
        let answer_changed = match (&shared.view.answer, &desired.answer) {
            (Some(old), Some(new)) => !Arc::ptr_eq(&old.text, &new.text),
            (None, None) => false,
            _ => true,
        };
        let context_changed = shared.view.revision != desired.revision
            || shared.view.context.as_ref().map(|event| event.id.as_str())
                != desired.context.as_ref().map(|event| event.id.as_str());
        let changed = shared.synced_question != desired.question
            || !Arc::ptr_eq(&shared.view.chat, &desired.chat)
            || !Arc::ptr_eq(&shared.view.recoveries, &desired.recoveries)
            || shared.synced_mode != desired.mode
            || answer_changed
            || shared.view.error != desired.error
            || context_changed
            || shared.view.selected != desired.selected
            || shared.view.ready != desired.ready
            || shared.view.busy != desired.busy
            || shared.view.language != desired.language
            || shared.view.accent != desired.accent
            || desired.focus;
        if shared.synced_question != desired.question || context_changed {
            shared.view.question = desired.question.clone();
            shared.synced_question = desired.question;
            shared.view.cursor = desired.cursor;
            shared.view.menu_dismissed = desired.menu_dismissed;
        }
        shared.view.answer = desired.answer;
        shared.view.chat = desired.chat;
        shared.view.recoveries = desired.recoveries;
        if shared.synced_mode != desired.mode {
            shared.synced_mode = desired.mode;
            shared.view.mode = desired.mode;
        }
        shared.view.error = desired.error;
        shared.view.context = desired.context;
        shared.view.revision = desired.revision;
        shared.view.selected = desired.selected;
        shared.view.ready = desired.ready;
        shared.view.busy = desired.busy;
        shared.view.language = desired.language;
        shared.view.accent = desired.accent;
        shared.view.focus |= desired.focus;
        self.focus_assistant = false;
        if changed {
            context.request_repaint_of(ask_viewport());
        }
    }

    fn assistant_panel(&mut self, ui: &mut egui::Ui) {
        let mut view = self.ask_view();
        let action = ask::draw(ui, &mut view, false);
        self.question = view.question.clone();
        self.ask_cursor = view.cursor;
        self.ask_menu_dismissed = view.menu_dismissed;
        self.ask_mode = view.mode;
        self.focus_assistant = view.focus;
        self.ask_unread = false;
        if let Some(action) = action {
            self.apply_ask_input(view.input(action), ui.ctx());
        }
    }

    fn assistant_dialog(&mut self, context: &egui::Context) {
        if !self.assistant || !self.assistant_detached {
            return;
        }
        self.sync_detached_ask(context);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!(
                "Tomlook — {}",
                self.preferences.language.text("Ask AI")
            ))
            .with_taskbar(true)
            .with_inner_size([560.0, 740.0])
            .with_min_inner_size([340.0, 420.0]);
        if let Ok(shared) = self.detached_ask.try_lock() {
            builder = builder.with_inner_size(shared.view.size);
            if let Some(position) = shared.view.position {
                builder = builder.with_position(position);
            }
        }
        let shared = self.detached_ask.clone();
        let output = self.ask_output.clone();
        let language = self.preferences.language;
        context.show_viewport_deferred(ask_viewport(), builder, move |ui, class| {
            let mut shared = match shared.try_lock() {
                Ok(shared) => shared,
                Err(std::sync::TryLockError::WouldBlock) => {
                    ui.label(language.text("Ask is updating..."));
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(16));
                    return;
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    ui.label(
                        language.text("The Ask window state is unavailable. Restart Tomlook."),
                    );
                    return;
                }
            };
            let action = if class == egui::ViewportClass::EmbeddedWindow {
                shared.failure = Some(
                    "Separate Ask windows are not supported by this renderer. Ask remains docked."
                        .into(),
                );
                Some(ask::Action::Dock)
            } else {
                let viewport = ui.ctx().input(|input| input.viewport().clone());
                if let Some(rect) = viewport.outer_rect {
                    shared.view.position = Some(rect.min);
                }
                if let Some(rect) = viewport.inner_rect {
                    shared.view.size = rect.size();
                }
                if viewport.close_requested() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    Some(ask::Action::Hide)
                } else {
                    egui::CentralPanel::default()
                        .show(ui, |ui| ask::draw(ui, &mut shared.view, true))
                        .inner
                }
            };
            if let Some(action) = action {
                match output.try_send(shared.view.input(action)) {
                    Ok(()) => {
                        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    }
                    Err(error) => {
                        let message = match error {
                            mpsc::TrySendError::Full(_) => {
                                "The Ask action queue is full. Nothing was submitted; try again."
                            }
                            mpsc::TrySendError::Disconnected(_) => {
                                "The Ask window is unavailable. Restart Tomlook."
                            }
                        };
                        shared.failure = Some(message.into());
                        shared.view.error = Some(message.into());
                        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    }
                }
            }
        });
    }

    fn visible_workiq_range(&self) -> Result<tomlook::workiq::connection::Range, String> {
        let days = self.visible_days();
        tomlook::workiq::connection::Range::new(
            *days.first().ok_or("Calendar view has no first day")?,
            *days.last().ok_or("Calendar view has no last day")?,
        )
    }

    fn refresh_workiq(&mut self) {
        if self.date_drag.is_some() {
            return;
        }
        match self.visible_workiq_range() {
            Ok(range) if self.ai_command(ai::Command::RefreshWorkiq(range)) => {
                self.workiq_requested_range = Some(range);
                self.hosted_workiq_busy = true;
                self.hosted_workiq_status = "Calendar refresh queued...".into();
            }
            Ok(_) => {}
            Err(error) => self.hosted_workiq_status = error,
        }
    }

    fn remember_github_client_id(&mut self) -> bool {
        let client_id = self.github_client_id.trim();
        if let Err(error) = tomlook::connectors::validate_client_id(client_id) {
            self.ai_error = Some(error);
            return false;
        }
        if self.preferences.github_client_id != client_id {
            self.preferences.github_client_id = client_id.into();
            self.save_preferences();
        }
        true
    }

    fn github_connection(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let kind = tomlook::connectors::Kind::Github;
        let status = self.external_status.entry(kind).or_default();
        let (busy, ready) = (status.busy, status.ready);
        ui.label(language.text(if busy {
            if status.message.is_empty() {
                "Connecting..."
            } else {
                &status.message
            }
        } else if ready {
            "Connected"
        } else {
            "Not connected"
        }))
        .on_hover_text(language.text(&status.message));
        let label = ui.label(language.text("GitHub App Client ID"));
        let editor = ui
            .add_enabled(
                !busy && !ready,
                egui::TextEdit::singleline(&mut self.github_client_id)
                    .id(Id::new(("connector-credential", kind, false)))
                    .char_limit(100)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
        if editor.lost_focus() && !self.github_client_id.trim().is_empty() {
            self.remember_github_client_id();
        }
        if busy {
            if ui.button(language.text("Cancel")).clicked() {
                self.ai_command(ai::Command::CancelExternal(kind));
            }
        } else if ui
            .add_enabled(
                !ready && !self.github_client_id.trim().is_empty(),
                egui::Button::new(language.text("Sign in")),
            )
            .clicked()
            && self.remember_github_client_id()
            && self.ai_command(ai::Command::ConnectExternal {
                kind,
                input: tomlook::connectors::Input::GithubApp(self.github_client_id.trim().into()),
            })
        {
            self.ai_error = None;
            self.external_status.insert(
                kind,
                ExternalStatus {
                    busy: true,
                    ready: false,
                    forgetting: false,
                    message: "Connecting...".into(),
                },
            );
        }
    }

    fn open_ai_connection(&mut self) {
        self.connection_panel = Some(ConnectionPanel::Ai);
        if !self.auto_status.busy() && self.ai_command(ai::Command::CheckAuto) {
            self.auto_status = tomlook::decision::Status::new(tomlook::decision::Phase::Checking, "Checking Foundry Azure sign-in...");
        }
    }

    fn connection_dialog(&mut self, context: &egui::Context) {
        let language = self.preferences.language;
        let Some(panel) = self.connection_panel else {
            self.webiq_key.clear();
            self.forget_connector = None;
            return;
        };
        if panel != ConnectionPanel::External(tomlook::connectors::Kind::Webiq) {
            self.webiq_key.clear();
        }
        let mut open = true;
        egui::Window::new(language.text(match panel {
            ConnectionPanel::Ai => "AI connection",
            ConnectionPanel::Workiq => "WorkIQ connection",
            ConnectionPanel::External(tomlook::connectors::Kind::Webiq) => "WebIQ connection",
            ConnectionPanel::External(tomlook::connectors::Kind::Github) => "GitHub connection",
        }))
        .id(Id::new("connection-dialog"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(440.0)
        .show(context, |ui| {
            if panel == ConnectionPanel::Ai {
                ui.strong("GitHub Copilot");
                ui.label(language.text(&self.connection));
                if !self.ai_login_busy
                    && !self.ai_ready
                    && let Some(message) = &self.ai_login_status
                {
                    ui.weak(language.text(message));
                }
                if self.ai_login_busy {
                    if ui.button(language.text("Cancel AI sign-in")).clicked() {
                        self.ai_command(ai::Command::CancelAiLogin);
                    }
                } else if !self.ai_ready
                    && ui.button(language.text("Sign in")).clicked()
                    && self.ai_command(ai::Command::SignInAi)
                {
                    self.ai_login_busy = true;
                    self.ai_login_status = None;
                    self.connection = "AI browser sign-in queued...".into();
                }
                ui.add_space(10.0);
                ui.separator();
                ui.strong("Foundry · Microsoft-Decision-1");
                ui.label(language.text(if self.auto_status.message.is_empty() {
                    self.auto_status.label()
                } else { &self.auto_status.message }));
                ui.weak(language.text("GitHub Copilot generates answers and analyses. Foundry selects Quick or Thorough in Auto mode."));
                if let Some(code) = &self.auto_status.device_code {
                    ui.horizontal(|ui| {
                        ui.monospace(code);
                        if ui.button(language.text("Copy code")).clicked() { ui.ctx().copy_text(code.clone()); }
                    });
                }
                if self.auto_status.phase == tomlook::decision::Phase::SigningIn {
                    if self.auto_status.device_code.is_none() {
                        ui.spinner();
                    }
                    ui.hyperlink_to(language.text("Open Microsoft device sign-in"), tomlook::decision::DEVICE_LOGIN_URL);
                    if ui.button(language.text("Cancel Azure sign-in")).clicked() {
                        self.ai_command(ai::Command::CancelAutoLogin);
                    }
                } else if !self.auto_status.busy() {
                    ui.weak(language.text("Uses your Azure CLI profile; its default account may change during sign-in."));
                    if self.auto_status.phase != tomlook::decision::Phase::Unconfigured
                        && ui.button(language.text("Sign in to Azure (device code)")).clicked()
                        && self.ai_command(ai::Command::SignInAuto) {
                        self.auto_status = tomlook::decision::Status::new(tomlook::decision::Phase::SigningIn, "Requesting an Azure device code (up to 60 seconds)...");
                    }
                    if ui.button(language.text("Check Foundry connection")).clicked()
                        && self.ai_command(ai::Command::CheckAuto) {
                        self.auto_status = tomlook::decision::Status::new(tomlook::decision::Phase::Checking, "Checking Foundry Azure sign-in...");
                    }
                }
            } else if panel == ConnectionPanel::External(tomlook::connectors::Kind::Github) {
                self.github_connection(ui);
            } else if let ConnectionPanel::External(kind) = panel {
                let status = self.external_status.entry(kind).or_default();
                let (busy, ready) = (status.busy, status.ready);
                let forgetting = status.forgetting;
                ui.label(language.text(if status.message.is_empty() { "Not connected" } else { &status.message }));
                let label = ui.label(language.text("API key"));
                ui.add_enabled(!busy && !ready, egui::TextEdit::singleline(&mut self.webiq_key)
                    .id(Id::new(("connector-credential", kind, false))).password(true)
                    .char_limit(8000).desired_width(f32::INFINITY)).labelled_by(label.id);
                ui.weak(language.text("The key is encrypted for your Windows account. Public web queries only."));
                if busy {
                    if !forgetting && ui.button(language.text("Cancel")).clicked() { self.ai_command(ai::Command::CancelExternal(kind)); }
                } else if !ready && ui.button(language.text("Connect and save key")).clicked() {
                    let input = tomlook::connectors::Input::WebiqKey(std::mem::take(&mut self.webiq_key));
                    if self.ai_command(ai::Command::ConnectExternal { kind, input }) {
                        self.external_status.insert(kind, ExternalStatus { busy: true, ready: false, forgetting: false, message: "Connecting...".into() });
                    }
                }
                if !busy && ui.button(language.text("Disconnect and forget")).clicked() { self.forget_connector = Some(kind); }
                if self.forget_connector == Some(kind) {
                    ui.strong(language.text("Remove this saved sign-in from Tomlook? Your provider account is not deleted."));
                    ui.horizontal(|ui| {
                        if ui.button(language.text("Remove saved sign-in")).clicked() && self.ai_command(ai::Command::ForgetExternal(kind)) {
                            self.forget_connector = None;
                            self.invalidate_connection_answers();
                            self.external_status.insert(kind, ExternalStatus { busy: true, ready: false, forgetting: true, message: "Removing saved sign-in...".into() });
                        }
                        if ui.button(language.text("Keep sign-in")).clicked() { self.forget_connector = None; }
                    });
                }
            } else {
                ui.label(language.text(&self.hosted_workiq_status));
                let account = ui.label(language.text("Work email"));
                ui.add_enabled(
                    !self.hosted_workiq_busy && !self.workiq_ready,
                    egui::TextEdit::singleline(&mut self.hosted_workiq_account)
                        .id(Id::new("workiq-account"))
                        .char_limit(254)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(account.id);
                if self.hosted_workiq_busy {
                    if ui.button(language.text("Cancel")).clicked() {
                        self.ai_command(ai::Command::CancelWorkiq);
                    }
                } else if !self.workiq_ready
                    && ui
                        .add_enabled(
                            !self.hosted_workiq_account.trim().is_empty(),
                            egui::Button::new(language.text("Sign in")),
                        )
                        .clicked()
                    && self.ai_command(ai::Command::ConnectWorkiq(
                        self.hosted_workiq_account.trim().into(),
                    ))
                {
                    self.preferences.work_email = self.hosted_workiq_account.trim().to_owned();
                    self.save_preferences();
                    if self.calendar_owner.as_deref().is_some_and(|owner| {
                        !owner.eq_ignore_ascii_case(self.hosted_workiq_account.trim())
                    }) {
                        self.invalidate_connection_answers();
                    }
                    self.hosted_workiq_busy = true;
                    self.hosted_workiq_status = "WorkIQ connection queued...".into();
                }
                if self.workiq_ready
                    && ui
                        .add_enabled(!self.hosted_workiq_busy, egui::Button::new(language.text("Refresh")))
                        .clicked()
                {
                    self.refresh_workiq();
                }
                if !self.hosted_workiq_busy
                    && !self.hosted_workiq_account.is_empty()
                    && ui
                        .button(language.text(if self.workiq_ready {
                            "Disconnect and forget sign-in"
                        } else {
                            "Forget saved sign-in"
                        }))
                        .clicked()
                    && self.ai_command(ai::Command::DisconnectWorkiq(
                        self.hosted_workiq_account.clone(),
                    ))
                {
                    self.invalidate_connection_answers();
                    self.workiq_ready = false;
                    self.hosted_workiq_busy = true;
                    self.workiq_requested_range = None;
                    self.hosted_workiq_status = "Removing saved WorkIQ sign-in...".into();
                }
            }
            if let Some(error) = self.ai_error.as_ref().filter(|error| tomlook::decision::routing_error(error).is_none()) {
                ui.label(RichText::new(language.text(error)).strong());
            }
        });
        if !open {
            self.connection_panel = None;
            self.webiq_key.clear();
            self.forget_connector = None;
        }
    }

    fn open_recommended_meeting(&mut self, event: &Event) {
        if let Some(index) = self.calendar.event_index(&event.id) {
            self.anchor = event.calendar_dates().0;
            if !self
                .preferences
                .view
                .days(self.anchor)
                .contains(&self.anchor)
            {
                self.preferences.view = View::Week;
                self.save_preferences();
            }
            self.date_selection = None;
            self.date_drag = None;
            self.query.clear();
            self.category.clear();
            self.select(index);
            self.scroll_meeting = true;
            self.insights_expanded = true;
            self.messages_open = false;
        } else {
            self.error = Some("Meeting is no longer in the current calendar.".into());
        }
    }

    fn recommendation_meeting(&mut self, ui: &mut egui::Ui, event: &Event, actions: bool) {
        use tomlook::recommendations::{Change, MeetingResponse, validate_manual_change};
        let language = self.preferences.language;
        if ui
            .add(
                egui::Button::new(RichText::new(&event.title).strong().color(self.accent()))
                    .frame(false)
                    .wrap_mode(egui::TextWrapMode::Wrap),
            )
            .clicked()
        {
            self.open_recommended_meeting(event);
        }
        ui.weak(meeting_date_label(event, language));
        if !event.location.is_empty() {
            ui.weak(event.location.chars().take(256).collect::<String>());
        }
        if actions {
            let ready = self.workiq_ready && self.ai.is_some() && self.calendar_owner.is_some();
            ui.push_id((&event.id, "conflict-actions"), |ui| {
                ui.horizontal_wrapped(|ui| {
                    for change in [
                        Change::Respond {
                            response: MeetingResponse::Tentative,
                            comment: String::new(),
                            send_response: true,
                        },
                        Change::Respond {
                            response: MeetingResponse::Decline,
                            comment: String::new(),
                            send_response: false,
                        },
                        Change::Respond {
                            response: MeetingResponse::Cancel,
                            comment: String::new(),
                            send_response: true,
                        },
                        Change::Delete,
                    ] {
                        if validate_manual_change(event, &change).is_err() {
                            continue;
                        }
                        let label = match &change {
                            Change::Respond { response, .. } => response.label(),
                            _ => "Delete own appointment",
                        };
                        if ui
                            .add_enabled(ready, egui::Button::new(language.text(label)))
                            .clicked()
                        {
                            self.choose_meeting_action(event.clone(), change);
                            self.messages_open = false;
                        }
                    }
                });
            });
        }
    }

    fn recommendation_card(
        &mut self,
        ui: &mut egui::Ui,
        recommendation: &tomlook::recommendations::Recommendation,
        calendar: &Calendar,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) {
        use tomlook::recommendations::{Change, Kind as RecommendationKind, State};
        let language = self.preferences.language;
        let index = calendar.event_index(&recommendation.event_id);
        let event = index.map(|index| &calendar.events[index]);
        let change = tomlook::recommendations::proposed_change(
            recommendation,
            calendar,
            self.calendar_owner.as_deref(),
            now,
            language,
        );
        let heading = match recommendation.suggestion.kind {
            RecommendationKind::Category => "Change category",
            RecommendationKind::Preparation => "Make time to prepare",
            RecommendationKind::PreparationAdvance => "Prepare in advance",
            RecommendationKind::Travel => "Reserve travel before meeting",
            RecommendationKind::TravelAfter => "Reserve travel after meeting",
            kind => kind.label(),
        };
        let accent = self.accent();
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(10)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (marker, _) = ui.allocate_exact_size(Vec2::new(3.0, 20.0), egui::Sense::hover());
                    ui.painter().rect_filled(marker, 1, accent);
                    ui.label(RichText::new(language.text(heading)).strong().size(17.0));
                    if recommendation.state == State::Review {
                        ui.weak(language.text("Needs review"));
                    } else if recommendation.state == State::Dispatching {
                        ui.spinner();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if let Some(event) = event
                        && ui.add(egui::Button::new(language.text("Open Insights")).frame(false)).clicked() {
                        self.open_recommended_meeting(event);
                    }
                    if recommendation.state != State::Dispatching
                        && ui.add(egui::Button::new(language.text("Skip recommendation")).frame(false)).clicked() {
                        self.command(Command::DismissRecommendation(recommendation.id.clone()));
                    }
                });
                ui.add_space(8.0);
                if let Ok(change) = &change {
                    match change {
                        Change::Category(category) => {
                            let previous = event.map(tomlook::recommendations::real_categories).unwrap_or_default();
                            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 32.0),
                                egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.label(RichText::new(if previous.is_empty() { language.text("None").into_owned() } else { previous.join(", ") }).size(16.0).weak());
                                let (arrow, response) = ui.allocate_exact_size(Vec2::new(28.0, 24.0), egui::Sense::hover());
                                let from = egui::pos2(arrow.left() + 3.0, arrow.center().y);
                                let to = egui::pos2(arrow.right() - 3.0, arrow.center().y);
                                let stroke = Stroke::new(1.6, ui.visuals().weak_text_color());
                                for segment in [
                                    [from, to],
                                    [to - Vec2::new(6.0, 5.0), to],
                                    [to - Vec2::new(6.0, -5.0), to],
                                ] { ui.painter().line_segment(segment, stroke); }
                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, language.text("To category")));
                                let color = category_color(category, self.preferences.dark);
                                egui::Frame::new().fill(color.gamma_multiply(0.12)).corner_radius(5).inner_margin(6)
                                    .show(ui, |ui| { ui.label(RichText::new(category).strong().size(16.0).color(color)); });
                            });
                        }
                        Change::Buffer { start, end, .. } => {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(format!("{} – {}",
                                    start.with_timezone(&Local).format("%H:%M"), end.with_timezone(&Local).format("%H:%M")))
                                    .strong().size(20.0));
                                ui.weak(format!("{} min", (*end - *start).num_minutes()));
                                ui.weak(language.date(start.with_timezone(&Local).date_naive(), false));
                            });
                        }
                        Change::ClearCategory => { ui.strong(language.text("Remove category")); }
                        Change::Follow(enabled) => { ui.strong(language.text(if *enabled { "Follow" } else { "Stop following" })); }
                        Change::Respond { response, .. } => { ui.strong(language.text(response.label())); }
                        Change::Delete => { ui.strong(language.text("Delete own appointment")); }
                    }
                } else if recommendation.suggestion.kind.is_travel()
                    || recommendation.suggestion.kind.is_preparation() {
                    ui.weak(language.text("No free interval fits the proposed buffer; adjust the calendar first"));
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(RichText::new(language.text("Meeting")).small().weak());
                if let Some(event) = event {
                    self.recommendation_meeting(ui, event, recommendation.suggestion.kind == RecommendationKind::PriorityConflict);
                } else if !recommendation.event_title.is_empty() {
                    ui.strong(&recommendation.event_title);
                } else {
                    ui.weak(language.text("Meeting is no longer in the current calendar."));
                }
                ui.add_space(10.0);
                let reason = match language {
                    tomlook::i18n::Language::En => &recommendation.suggestion.reason,
                    tomlook::i18n::Language::Cs => &recommendation.suggestion.reason_cs,
                };
                if recommendation.suggestion.kind == RecommendationKind::PriorityConflict {
                    ui.label(RichText::new(language.text("Conflicts with")).small().weak());
                    let conflicts = if let Some(entry) = self.activity.recommendation_view.iter().find(|entry| entry.id == recommendation.id) {
                        entry.conflicts.iter().filter_map(|id| calendar.event_index(id).map(|index| &calendar.events[index])).collect()
                    } else {
                        event.map(|event| tomlook::recommendations::conflicts(calendar, event)).unwrap_or_default()
                    };
                    if let Some(event) = event {
                        if self.recommendation_conflict_page.0 != recommendation.id {
                            self.recommendation_conflict_page = (recommendation.id.clone(), 0);
                        }
                        let page = self.recommendation_conflict_page.1.min(conflicts.len().saturating_sub(1) / 6);
                        self.recommendation_conflict_page.1 = page;
                        for other in conflicts.iter().skip(page * 6).take(6) {
                            ui.add_space(8.0);
                            self.recommendation_meeting(ui, other, true);
                            if !other.is_all_day {
                                ui.weak(format!("{}: {} – {}",
                                    language.text("Overlap"),
                                    other.start.max(event.start).with_timezone(&Local).format("%H:%M"),
                                    other.end.min(event.end).with_timezone(&Local).format("%H:%M")));
                            }
                        }
                        if conflicts.len() > 6 {
                            ui.horizontal(|ui| {
                                if ui.add_enabled(page > 0, egui::Button::new(language.text("Previous"))).clicked() {
                                    self.recommendation_conflict_page.1 -= 1;
                                }
                                ui.weak(format!("{}–{} / {}", page * 6 + 1, ((page + 1) * 6).min(conflicts.len()), conflicts.len()));
                                if ui.add_enabled((page + 1) * 6 < conflicts.len(), egui::Button::new(language.text("Next"))).clicked() {
                                    self.recommendation_conflict_page.1 += 1;
                                }
                            });
                        }
                    }
                    if conflicts.is_empty() {
                        ui.weak(language.text("No overlap in the current confirmed calendar."));
                    }
                } else {
                    ui.label(RichText::new(language.text("Why?")).small().weak());
                    let short = reason.chars().take(320).collect::<String>();
                    ui.label(if reason.chars().count() > 320 { format!("{short}…") } else { short });
                }
                if recommendation.suggestion.kind == RecommendationKind::MissingMaterials
                    && let Some(event) = event {
                    let contacts = tomlook::recommendations::material_contacts(event, self.calendar_owner.as_deref().unwrap_or(&self.preferences.work_email), self.activity.identity.as_deref());
                    if !contacts.is_empty() {
                        ui.add_space(10.0);
                        ui.label(RichText::new(language.text("Who to ask")).strong());
                        ui.weak(language.text("Ask for the agenda, materials and your expected role."));
                        for contact in contacts {
                            ui.label(&contact.person);
                            let role = if contact.organizer { "Organizer" } else if contact.same_domain { "Same email domain" } else { "Participant" };
                            ui.weak(language.text(role));
                        }
                    }
                }
                if !recommendation.message.is_empty() {
                    ui.add_space(6.0);
                    if recommendation.state == State::NotSent { ui.weak(language.text("Previous attempt; no calendar change was sent.")); }
                    ui.label(language.text(&recommendation.message));
                }
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if recommendation.state == State::Review
                        && matches!(recommendation.suggestion.kind, RecommendationKind::Manual | RecommendationKind::Category)
                        && ui.add_enabled(self.ai.is_some(),
                            egui::Button::new(language.text("Check calendar result"))).clicked() {
                        self.ai_command(ai::Command::RefreshCalendarAvailability);
                        self.command(Command::RecheckCalendarResult(recommendation.id.clone()));
                    }
                    if matches!(recommendation.state, State::Pending | State::NotSent)
                        && let Ok(change) = &change {
                        let label = if recommendation.suggestion.kind == RecommendationKind::Manual {
                            "Retry calendar change"
                        } else if recommendation.suggestion.kind == RecommendationKind::Category {
                            "Use category"
                        } else { "Add to calendar" };
                        if ui.add_enabled(self.ai.is_some(),
                            egui::Button::new(language.text(label))).clicked() {
                            self.ai_command(ai::Command::RefreshCalendarAvailability);
                            self.command(Command::ConfirmRecommendation { id: recommendation.id.clone(), change: change.clone() });
                        }
                    }
                    if recommendation.suggestion.kind == RecommendationKind::FollowUp
                        && ui.button(language.text("Draft follow-up")).clicked() {
                        self.prepare_follow_up(recommendation);
                    }
                });
                if reason.chars().count() > 320 || change.is_ok() || recommendation.source_url.is_some()
                    || recommendation.suggestion.kind == RecommendationKind::PriorityConflict {
                    ui.collapsing(language.text("Details and checks"), |ui| {
                        if reason.chars().count() > 320 || recommendation.suggestion.kind == RecommendationKind::PriorityConflict { ui.label(reason); }
                        match &change {
                            Ok(Change::Category(_)) => { ui.weak(language.text("Replaces all previous categories with this single category.")); }
                            Ok(Change::Buffer { category, .. }) => {
                                ui.weak(language.text("Suggested slots use weekdays, 08:00-18:00 local time."));
                                if let Some(category) = category { ui.weak(format!("{}: {category}", language.text("Category"))); }
                                ui.weak(language.text("Free in the confirmed calendar; actual availability is not verified."));
                                if recommendation.suggestion.kind.is_travel() {
                                    ui.weak(language.text("Travel duration is an estimate; live traffic has not been verified."));
                                }
                            }
                            _ => {}
                        }
                        if let Some(url) = recommendation.source_url.as_deref()
                            && tomlook::calendar::safe_url(url)
                            && ui.add(egui::Button::new(language.text("Open original meeting")).frame(false)).clicked()
                            && let Err(error) = tomlook::browser::open(url) {
                            self.error = Some(error);
                        }
                    });
                }
            });
    }

    fn preferences_dialog(&mut self, context: &egui::Context) {
        if !self.preferences_open { return; }
        let language = self.preferences.language;
        let mut open = true;
        egui::Window::new(language.text("Settings")).id(Id::new("preferences"))
            .open(&mut open).default_size(Vec2::new(940.0, 620.0))
            .min_size(Vec2::new(760.0, 480.0)).resizable(true).collapsible(false).show(context, |ui| {
                egui::Panel::left("preferences-navigation").resizable(false).exact_size(184.0)
                    .frame(egui::Frame::new().fill(ui.visuals().panel_fill)
                        .inner_margin(egui::Margin::symmetric(12, 16)))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        for (section, label) in [
                            (PreferencesSection::Display, "Display"),
                            (PreferencesSection::Prompts, "AI prompts"),
                        ] {
                            if ui.add_sized([ui.available_width(), 36.0],
                                egui::Button::new(language.text(label))
                                    .selected(self.preferences_section == section)).clicked() {
                                self.preferences_section = section;
                            }
                        }
                        ui.separator();
                        if ui.button(language.text("Insights activity")).clicked() { self.settings = true; }
                    });
                egui::CentralPanel::default().frame(egui::Frame::NONE
                    .inner_margin(egui::Margin {left:24,right:24,top:18,bottom:20})).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt(("preferences-content", self.preferences_section))
                        .auto_shrink([false, false]).show(ui, |ui| {
                        match self.preferences_section {
                            PreferencesSection::Display => {
                                ui.heading(language.text("Display"));
                                ui.add_space(16.0);
                                ui.strong(language.text("Language"));
                                ui.horizontal(|ui| {
                                    for (value, label) in [
                                        (tomlook::i18n::Language::En, "English"),
                                        (tomlook::i18n::Language::Cs, "Čeština"),
                                    ] {
                                        if ui.selectable_value(&mut self.preferences.language, value, label).changed() {
                                            self.save_preferences();
                                        }
                                    }
                                });
                                ui.add_space(20.0);
                                ui.strong(language.text("Theme"));
                                if ui.checkbox(&mut self.preferences.dark, language.text("Dark")).changed() {
                                    self.save_preferences();
                                }
                                ui.add_space(20.0);
                                ui.strong(language.text("Accent"));
                                ui.horizontal_wrapped(|ui| {
                                    for (index, (name, light, dark)) in ACCENTS.iter().enumerate() {
                                        let [red, green, blue] = if self.preferences.dark { *dark } else { *light };
                                        let color = Color32::from_rgb(red, green, blue);
                                        if ui.add(egui::Button::new(RichText::new(language.text(name)).color(color))
                                            .selected(self.preferences.accent == index)).clicked() {
                                            self.preferences.accent = index;
                                            self.save_preferences();
                                        }
                                    }
                                });
                            }
                            PreferencesSection::Prompts => {
                                ui.heading(language.text("AI prompts"));
                                ui.weak(language.text("Personal guidance only; evidence, privacy and write-safety rules stay enforced."));
                                ui.add_space(12.0);
                                ui.horizontal_wrapped(|ui| {
                                    for kind in tomlook::prompts::Kind::ALL {
                                        ui.selectable_value(&mut self.prompt_kind, kind, language.text(kind.label()));
                                    }
                                });
                                ui.add(egui::TextEdit::multiline(self.prompt_draft.edit(self.prompt_kind))
                                    .id(Id::new(("prompt-editor", self.prompt_kind.label())))
                                    .desired_width(f32::INFINITY).desired_rows(16).char_limit(4000));
                                let validation = self.prompt_draft.validate();
                                if let Err(error) = &validation { ui.colored_label(ui.visuals().error_fg_color, language.text(error)); }
                                ui.horizontal(|ui| {
                                    if ui.add_enabled(validation.is_ok() && self.prompt_draft != self.preferences.prompts,
                                        egui::Button::new(language.text("Save prompts"))).clicked() {
                                        self.preferences.prompts = self.prompt_draft.clone();
                                        self.save_preferences();
                                    }
                                    if ui.button(language.text("Restore default")).clicked() {
                                        *self.prompt_draft.edit(self.prompt_kind) = self.prompt_kind.default_text().into();
                                    }
                                });
                                ui.weak(language.text("Saved notes are not reprocessed when prompts change."));
                            }
                        }
                    });
                });
            });
        self.preferences_open = open;
    }

    fn overlays(&mut self, context: &egui::Context) {
        let language = self.preferences.language;
        self.preferences_dialog(context);
        self.meeting_action_dialog(context);
        self.event_creation_dialog(context);
        self.assistant_dialog(context);
        if self.messages_open {
            let calendar = self.calendar.clone();
            let mut open = true;
            let mut review = false;
            egui::Window::new(language.text("Messages"))
                .id(Id::new("status-messages-window-v3"))
                .open(&mut open)
                .default_width(940.0)
                .min_width(740.0)
                .default_height(660.0)
                .min_height(520.0)
                .resizable(true)
                .show(context, |ui| {
                    let creations = self.activity.event_creations.clone();
                    let mut records = creations.values().filter(|record| self.calendar_owner.as_deref() == Some(record.account.as_str())).collect::<Vec<_>>();
                    records.sort_by_key(|record| record.created_at);
                    if !records.is_empty() {
                        egui::CollapsingHeader::new(language.text("Event creation"))
                            .default_open(true).show(ui, |ui| {
                                egui::ScrollArea::vertical().id_salt("event-creation-results").max_height(160.0).show(ui, |ui| {
                                    for record in records.iter().rev() {
                                        ui.strong(format!("{}: {}", language.text(record.label()), record.draft.title));
                                        ui.weak(language.text(&record.message));
                                        ui.weak(format!("{} - {}", record.draft.start.with_timezone(&Local).format("%d.%m. %H:%M"),
                                            record.draft.end.with_timezone(&Local).format("%d.%m. %H:%M")));
                                    }
                                });
                            });
                    }
                    let inbox = self.activity.recommendations.clone();
                    let now = chrono::Local::now().fixed_offset();
                    let recommendations = self
                        .activity
                        .recommendation_view
                        .iter()
                        .filter_map(|entry| inbox.get(&entry.id))
                        .filter(|record| {
                            record.visible(&calendar, self.calendar_owner.as_deref(), now)
                        })
                        .collect::<Vec<_>>();
                    if !recommendations.is_empty() {
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut self.recommendations_tab,
                                true,
                                format!(
                                    "{} ({})",
                                    language.text("Recommendations"),
                                    recommendations.len()
                                ),
                            );
                            ui.selectable_value(
                                &mut self.recommendations_tab,
                                false,
                                language.text("Diagnostics"),
                            );
                        });
                    }
                    if self.recommendations_tab && !recommendations.is_empty() {
                        self.recommendation_cursor = self
                            .recommendation_selected
                            .as_ref()
                            .and_then(|id| {
                                recommendations.iter().position(|record| &record.id == id)
                            })
                            .unwrap_or(self.recommendation_cursor)
                            .min(recommendations.len() - 1);
                        let previous_cursor = self.recommendation_cursor;
                        let up =
                            ui.input_mut(|input| input.consume_key(Modifiers::ALT, Key::ArrowUp));
                        let down =
                            ui.input_mut(|input| input.consume_key(Modifiers::ALT, Key::ArrowDown));
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    self.recommendation_cursor > 0,
                                    egui::Button::new(language.text("Previous")),
                                )
                                .clicked()
                                || up
                            {
                                self.recommendation_cursor =
                                    self.recommendation_cursor.saturating_sub(1);
                            }
                            ui.label(format!(
                                "{} / {}",
                                self.recommendation_cursor + 1,
                                recommendations.len()
                            ));
                            if ui
                                .add_enabled(
                                    self.recommendation_cursor + 1 < recommendations.len(),
                                    egui::Button::new(language.text("Next")),
                                )
                                .clicked()
                                || down
                            {
                                self.recommendation_cursor =
                                    (self.recommendation_cursor + 1).min(recommendations.len() - 1);
                            }
                            ui.weak(language.text("Alt+Up / Alt+Down"));
                        });
                        self.recommendation_selected =
                            Some(recommendations[self.recommendation_cursor].id.clone());
                        ui.add_space(8.0);
                        let height = ui.available_height().max(400.0);
                        let list_width = 260.0;
                        let detail_width = (ui.available_width() - list_width - 24.0).max(360.0);
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(
                                Vec2::new(list_width, height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    let mut scroll = egui::ScrollArea::vertical()
                                        .id_salt("recommendation-list")
                                        .max_height(height);
                                    if previous_cursor != self.recommendation_cursor {
                                        scroll = scroll.vertical_scroll_offset(
                                            (self.recommendation_cursor as f32
                                                * (72.0 + ui.spacing().item_spacing.y)
                                                - height * 0.5)
                                                .max(0.0),
                                        );
                                    }
                                    scroll.show_rows(
                                        ui,
                                        72.0,
                                        recommendations.len(),
                                        |ui, rows| {
                                            for index in rows {
                                                let record = recommendations[index];
                                                let event = calendar
                                                    .event_index(&record.event_id)
                                                    .map(|index| &calendar.events[index]);
                                                let date = event
                                                    .map(|event| {
                                                        event
                                                            .start
                                                            .with_timezone(&Local)
                                                            .format("%d.%m. %H:%M")
                                                            .to_string()
                                                    })
                                                    .unwrap_or_default();
                                                let title = event
                                                    .map(|event| event.title.as_str())
                                                    .unwrap_or(&record.event_title);
                                                let label = format!(
                                                    "{}\n{}\n{date}",
                                                    language.text(record.suggestion.kind.label()),
                                                    title.chars().take(50).collect::<String>()
                                                );
                                                let response = ui
                                                    .push_id(&record.id, |ui| {
                                                        ui.add_sized(
                                                            Vec2::new(list_width - 12.0, 72.0),
                                                            egui::Button::new(label)
                                                                .selected(
                                                                    index
                                                                        == self
                                                                            .recommendation_cursor,
                                                                )
                                                                .truncate(),
                                                        )
                                                    })
                                                    .inner;
                                                if (up || down)
                                                    && index == self.recommendation_cursor
                                                {
                                                    response
                                                        .scroll_to_me(Some(egui::Align::Center));
                                                }
                                                if response.clicked() {
                                                    self.recommendation_cursor = index;
                                                    self.recommendation_selected =
                                                        Some(record.id.clone());
                                                }
                                            }
                                        },
                                    );
                                },
                            );
                            ui.separator();
                            ui.allocate_ui_with_layout(
                                Vec2::new(detail_width, height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    egui::ScrollArea::vertical()
                                        .id_salt((
                                            "recommendation-detail",
                                            &self.recommendation_selected,
                                        ))
                                        .max_height(height)
                                        .show(ui, |ui| {
                                            let record =
                                                recommendations[self.recommendation_cursor];
                                            ui.push_id(&record.id, |ui| {
                                                self.recommendation_card(ui, record, &calendar, now)
                                            });
                                        });
                                },
                            );
                        });
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                let mut messages = Vec::<&str>::new();
                                for message in [
                                    self.error.as_deref(),
                                    self.activity.error.as_deref(),
                                    self.activity.sync.error.as_deref(),
                                ]
                                .into_iter()
                                .flatten()
                                {
                                    if !messages.contains(&message) {
                                        messages.push(message);
                                    }
                                }
                                for status in self.external_status.values() {
                                    if !status.ready
                                        && !status.busy
                                        && !status.message.is_empty()
                                        && status.message != "Not connected"
                                        && !messages.contains(&status.message.as_str())
                                    {
                                        messages.push(&status.message);
                                    }
                                }
                                if messages.is_empty() {
                                    ui.weak(language.text("No messages."));
                                }
                                for message in messages {
                                    ui.label(language.text(message));
                                    ui.separator();
                                }
                            });
                        if self.error.is_some() && ui.button(language.text("Dismiss")).clicked() {
                            self.error = None;
                        }
                        if self.activity.index.len(ActivityFilter::Problems) > 0
                            && ui
                                .button(language.text("Review interrupted jobs"))
                                .clicked()
                        {
                            review = true;
                        }
                        if self.activity.error.is_some()
                            && ui
                                .add_enabled(
                                    self.activity.queue.flights.is_empty(),
                                    egui::Button::new(language.text("Resume other queued jobs")),
                                )
                                .clicked()
                        {
                            self.command(Command::ResumeQueued);
                        }
                    }
                });
            self.messages_open &= open && !review;
            if review {
                self.settings = true;
                self.activity_filter = ActivityFilter::Problems;
            }
        }
        if self.settings {
            let mut open = true;
            let mut selected_job = None;
            let calendar = self.calendar.clone();
            let activity = self.activity.clone();
            egui::Window::new(language.text("Insights activity"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(440.0)
                .show(context, |ui| {
                    if ui
                        .checkbox(&mut self.preferences.prepare_enabled, language.text("Automatic Insights"))
                        .changed()
                    {
                        self.command(Command::EnablePreparation(self.preferences.prepare_enabled));
                    }
                    if ui
                        .checkbox(&mut self.preferences.paused, language.text("Pause Insights"))
                        .changed()
                    {
                        self.command(Command::PausePreparation(self.preferences.paused));
                    }
                    ui.collapsing(language.text("Meeting notes"), |ui| {
                        let records = activity.recaps.records.values()
                            .filter(|record| self.calendar_owner.as_deref() == Some(record.account.as_str())).collect::<Vec<_>>();
                        if records.is_empty() { ui.weak(language.text("No AI meeting notes yet.")); }
                        egui::ScrollArea::vertical().id_salt("recap-activity").max_height(180.0)
                            .show_rows(ui, 42.0, records.len(), |ui, rows| {
                                for row in rows {
                                    let record = records[row];
                                    let title = calendar.event_index(&record.event_id).map(|index| calendar.events[index].title.as_str())
                                        .unwrap_or(&record.event_id);
                                    ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
                                    ui.add(egui::Label::new(language.text(&record.message)).truncate()).on_hover_text(language.text(&record.message));
                                }
                            });
                    });
                    ui.collapsing(language.text("AI usage"), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(language.text("Daily AI request limit"));
                            let limit = ui.add(egui::DragValue::new(&mut self.preferences.daily_ai_limit).range(1..=100_000));
                            if limit.drag_stopped() || limit.lost_focus() { self.save_preferences(); }
                        });
                        if let Some(summary) = &self.ai_usage {
                            ui.label(format!("{} · {}: {}/{}", summary.day, language.text("Requests"), summary.requests, self.preferences.daily_ai_limit));
                            for (label, value) in [
                                ("Input tokens", summary.tokens.input), ("Cache read tokens", summary.tokens.cache_read),
                                ("Cache write tokens", summary.tokens.cache_write), ("Output tokens", summary.tokens.output),
                            ] {
                                ui.label(format!("{}: {}", language.text(label),
                                    value.map(|value| value.to_string()).unwrap_or_else(|| language.text("Unavailable").into_owned())));
                            }
                            ui.weak(format!("{}: {}/{}", language.text("Requests with reported usage"),
                                summary.requests_with_usage, summary.requests));
                            ui.weak(language.text("Measured SDK tokens only; missing usage is not zero or a currency estimate."));
                        } else {
                            ui.weak(language.text("Usage accounting is loading."));
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for filter in [
                            ActivityFilter::All,
                            ActivityFilter::Working,
                            ActivityFilter::Waiting,
                            ActivityFilter::Problems,
                            ActivityFilter::Ready,
                        ] {
                            ui.selectable_value(&mut self.activity_filter, filter, language.text(filter.label()));
                        }
                    });
                    let job_count = activity.index.len(self.activity_filter);
                    if job_count == 0 {
                        ui.weak(language.text("No matching jobs."));
                    }
                    egui::ScrollArea::vertical()
                        .id_salt(("insights-activity", self.activity_filter))
                        .max_height(240.0).show_rows(
                        ui,
                        ACTIVITY_ROW_HEIGHT,
                        job_count,
                        |ui, rows| {
                            for row in rows {
                                let job = activity.index.job(&activity.queue, self.activity_filter, row);
                                let index = calendar.event_index(&job.event_id);
                                let title = index
                                    .map(|index| calendar.events[index].title.as_str())
                                    .unwrap_or(job.event_id.as_str());
                                if activity_row(ui, job, title, index.is_some(), language).clicked() {
                                    selected_job = index;
                                }
                            }
                        },
                    );
                    ui.label(language.text(&self.source_status));
                    ui.collapsing(language.text("Sources"), |ui| {
                        ui.label(language.text("Repository collections (one per line)"));
                        ui.add(egui::TextEdit::multiline(&mut self.repository_editor)
                            .id(Id::new("repository-collections")).desired_rows(2).desired_width(380.0));
                        ui.label(language.text("Local document folders"));
                        ui.add(egui::TextEdit::multiline(&mut self.folder_editor)
                            .id(Id::new("context-folders")).desired_rows(2).desired_width(380.0));
                        ui.label(language.text("Customer folder in OneDrive (WorkIQ)"));
                        ui.text_edit_singleline(&mut self.cloud_folder_editor);
                        if ui.button(language.text("Save sources")).clicked() {
                            let paths = |text: &str| text.lines().map(str::trim).filter(|line| !line.is_empty())
                                .map(std::path::PathBuf::from).collect::<Vec<_>>();
                            let folders = paths(&self.folder_editor);
                            let repositories = paths(&self.repository_editor);
                            if folders.len() > 8 || repositories.len() > 4 || folders.iter().any(|path| !path.is_absolute()
                                || path.parent().is_none_or(|parent| parent.as_os_str().is_empty()))
                                || repositories.iter().any(|path| !path.is_absolute())
                            {
                                self.error = Some("Use up to eight specific document folders and four absolute repository collection paths.".into());
                            } else {
                                self.preferences.context_folders = folders;
                                self.preferences.repository_collections = repositories;
                                self.preferences.cloud_context_folder = self.cloud_folder_editor.clone();
                                self.save_preferences();
                            }
                        }
                    });
                    if let Some(error) = &self.activity.error {
                        ui.label(language.text(error));
                        if ui
                            .add_enabled(
                                self.activity.queue.flights.is_empty(),
                                egui::Button::new(language.text("Resume other queued jobs")),
                            )
                            .clicked()
                        {
                            self.command(Command::ResumeQueued);
                        }
                    }
                    if ui.button(language.text("Exit Tomlook")).clicked() {
                        self.exit(context);
                    }
                });
            self.settings = open;
            if let Some(index) = selected_job {
                self.select(index);
                self.settings = false;
                self.scroll_meeting = true;
            }
        }
        if self.insights_expanded {
            let mut open = true;
            egui::Window::new(language.text("Meeting Insights"))
                .open(&mut open)
                .default_width(900.0)
                .default_height(640.0)
                .resizable(true)
                .show(context, |ui| {
                    if let Some(index) = self.selected {
                        ui.heading(language.text(&self.calendar.events[index].title));
                        ui.separator();
                    }
                    egui::ScrollArea::vertical().show(ui, |ui| self.render_insights(ui, true));
                });
            self.insights_expanded = open;
        }
        if self.palette {
            let mut open = true;
            let commands = command_matches_in(&self.palette_query, language);
            self.palette_cursor = self.palette_cursor.min(commands.len().saturating_sub(1));
            let up = context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
            let down =
                context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
            let enter = context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
            if up {
                self.palette_cursor = self.palette_cursor.saturating_sub(1);
            }
            if down {
                self.palette_cursor =
                    (self.palette_cursor + 1).min(commands.len().saturating_sub(1));
            }
            let mut chosen = None;
            egui::Window::new(language.text("Commands"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 110.0])
                .show(context, |ui| {
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.palette_query)
                            .id(Id::new("command-search"))
                            .hint_text(language.text("Search commands..."))
                            .desired_width(360.0),
                    );
                    if self.palette_focus {
                        search.request_focus();
                        self.palette_focus = false;
                    }
                    if search.changed() {
                        self.palette_cursor = 0;
                    }
                    for (row, command) in command_matches_in(&self.palette_query, language)
                        .iter()
                        .enumerate()
                    {
                        if ui
                            .selectable_label(
                                row == self.palette_cursor,
                                language.text(command.label()),
                            )
                            .clicked()
                        {
                            chosen = Some(*command);
                        }
                    }
                    if commands.is_empty() {
                        ui.weak(language.text("No matching commands"));
                    }
                });
            if !open {
                self.close_palette(context);
            }
            if chosen.is_none() && enter {
                chosen = command_matches_in(&self.palette_query, language)
                    .get(self.palette_cursor)
                    .copied();
            }
            if let Some(command) = chosen {
                self.run_palette_command(command, context);
            }
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
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive();
        self.receive_ask_inputs(context);
        self.sync_detached_ask(context);
        if let Some(trace) = &self.startup
            && trace.idle
        {
            let remaining =
                std::time::Duration::from_secs(160).saturating_sub(trace.launched.elapsed());
            if remaining.is_zero() {
                if let Some(lifecycle) = &self.lifecycle
                    && let Ok(mut lifecycle) = lifecycle.lock()
                {
                    lifecycle.startup = Some(serde_json::json!({
                        "schema": "tomlook-idle-lifecycle-trace-v1",
                        "events": self.calendar.events.len(),
                        "loading": self.loading,
                        "error": self.error,
                        "in_tray": self.in_tray,
                        "landmark": "Bounded idle fixture process; resource samples must be recorded by an external observer",
                    }));
                }
                self.exit(context);
            } else {
                context.request_repaint_after(remaining);
            }
        }
        if let Some(trace) = &mut self.interaction_trace {
            if trace.deadline_reached() {
                if let Some(lifecycle) = &self.lifecycle
                    && let Ok(mut lifecycle) = lifecycle.lock()
                {
                    lifecycle.startup = Some(trace.report());
                }
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                context.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }

    fn raw_input_hook(&mut self, context: &egui::Context, input: &mut egui::RawInput) {
        if self.interaction_trace.is_some() {
            let state = self.interaction_state(context);
            if let Some(trace) = &mut self.interaction_trace {
                trace.input(input, &state);
            }
            self.render_profile = Some(Default::default());
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let language = self.preferences.language;
        let interaction_started = self
            .interaction_trace
            .as_ref()
            .map(|_| std::time::Instant::now());
        if let Some(trace) = &mut self.interaction_trace {
            trace.previous_frame_cpu(frame.info().cpu_usage);
        }
        let context = ui.ctx().clone();
        if self.startup_built
            && self.startup.as_ref().is_some_and(|trace| !trace.idle)
            && let Some(mut trace) = self.startup.take()
        {
            trace.mark("next_frame_started");
            let marks: serde_json::Map<String, serde_json::Value> = trace
                .marks
                .iter()
                .map(|(name, ms)| ((*name).to_string(), (*ms).into()))
                .collect();
            let record = serde_json::json!({
                "landmark": "Start of the frame following the first loaded-calendar frame; the loaded frame was already handed to presentation",
                "marks_ms": marks,
                "first_calendar_frame_presented_ms": trace.launched.elapsed().as_secs_f64() * 1000.0,
                "first_calendar_frame_presented_from_process_creation_ms": trace.entry_after_creation_ms + trace.launched.elapsed().as_secs_f64() * 1000.0,
                "process_created_unix_ms": trace.process_created_unix_ms,
                "entry_after_process_creation_ms": trace.entry_after_creation_ms,
                "clock_alignment": "Windows GetProcessTimes UTC creation aligned with the main-entry SystemTime/Instant pair; loader time is included, UTC clock discontinuities are not inferred away",
                "first_calendar_frame_presented_unix_ms": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64() * 1000.0)
                    .unwrap_or(f64::NAN),
                "events": self.calendar.events.len(),
                "coverage_days": self.calendar.coverage.len(),
                "error": self.error,
                "trace_path": trace.path,
            });
            if let Some(lifecycle) = &self.lifecycle
                && let Ok(mut lifecycle) = lifecycle.lock()
            {
                lifecycle.startup = Some(record);
            }
            self.exit(&context);
        }
        let viewport = context.input(|input| input.viewport().clone());
        if let Some(tray) = &self.tray {
            if viewport.close_requested()
                && !tray.exiting.load(std::sync::atomic::Ordering::Acquire)
            {
                context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else if viewport.minimized == Some(true) && !self.in_tray {
                self.in_tray = true;
                context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else if viewport.minimized == Some(false) {
                self.in_tray = false;
            }
        }
        if let Some(error) =
            context.data_mut(|data| data.remove_temp::<String>(Id::new("tray-error")))
        {
            self.error = Some(error);
        }
        if self.preview && self.demo_screenshot.is_some() {
            if self.screenshot_requested {
                let image = context.input(|input| {
                    input.events.iter().find_map(|event| {
                        if let egui::Event::Screenshot { image, .. } = event {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                });
                if let Some(image) = image
                    && let Some(path) = self.demo_screenshot.take()
                {
                    self.command(Command::DemoScreenshot { path, image });
                }
            } else if !self.loading {
                if self.selected.is_none() {
                    if let Some(index) = self
                        .calendar
                        .events
                        .iter()
                        .position(|event| event.title == "Northstar / Architecture workshop")
                    {
                        self.select(index);
                    }
                } else if self.briefing.is_some() {
                    context
                        .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                    self.screenshot_requested = true;
                }
            }
        }
        if self.workiq_ready
            && !self.hosted_workiq_busy
            && self
                .visible_workiq_range()
                .is_ok_and(|range| self.workiq_requested_range != Some(range))
        {
            self.refresh_workiq();
        }
        self.style(&context);
        self.sync_brand(&context);
        self.shortcuts(&context);
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if self.preview {
                    ui.weak(language.text("Offline demo - fictional calendar"));
                } else {
                if status_text(ui, "status-ai", language.text(if self.ai_login_busy {
                        "GitHub Copilot: signing in..."
                    } else if self.ai_ready {
                        "GitHub Copilot: connected"
                    } else {
                        "GitHub Copilot: disconnected"
                    }))
                    .on_hover_text(language.text(&self.connection))
                    .clicked()
                {
                    self.open_ai_connection();
                }
                if status_text(ui, "status-auto", language.text(self.auto_status.label()))
                    .on_hover_text(language.text(&self.auto_status.message)).clicked() {
                    self.open_ai_connection();
                }
                let label = if self.hosted_workiq_busy {
                    "WorkIQ: working..."
                } else if self.workiq_ready {
                    "WorkIQ: connected"
                } else {
                    "WorkIQ: disconnected"
                };
                if status_text(ui, "status-workiq", language.text(label))
                    .on_hover_text(language.text(&self.hosted_workiq_status))
                    .clicked()
                {
                    self.connection_panel = Some(ConnectionPanel::Workiq);
                }
                for kind in [tomlook::connectors::Kind::Webiq, tomlook::connectors::Kind::Github] {
                    let status = self.external_status.entry(kind).or_default();
                    let state = if status.forgetting { "disconnecting..." } else if status.busy { "connecting..." } else if status.ready { "connected" } else { "disconnected" };
                    if status_text(ui, kind.server(), language.text(&(format!("{}: {state}", kind.label())))).on_hover_text(language.text(&status.message)).clicked() {
                        self.connection_panel = Some(ConnectionPanel::External(kind));
                    }
                }
                }
                ui.weak(language.text(&(format!("{} cached meetings", self.calendar.events.len())))).on_hover_text(
                    language.text(&(self.calendar_owner.as_deref().map(|owner| format!("Locally saved calendar owned by {owner}; retained when disconnected.")).unwrap_or_else(|| "Local calendar cache; no connected account assigned.".into())))
                );
                if self.activity.sync.total > 0 {
                    let sync = &self.activity.sync;
                    let label = if !self.workiq_ready {
                        format!("Sync {}/{} · offline", sync.fresh, sync.total)
                    } else if sync.error.is_some() {
                        format!("Sync {}/{} · retrying", sync.fresh, sync.total)
                    } else {
                        format!("Sync {}/{}", sync.fresh, sync.total)
                    };
                    ui.weak(language.text(&(label))).on_hover_text(language.text(&(format!(
                        "One month back, six months ahead through {}.\n{}{}",
                        sync.through.map(|day| day.to_string()).unwrap_or_default(),
                        sync.updated.map(|time| format!("Last saved refresh: {}.", time.with_timezone(&Local).format("%H:%M"))).unwrap_or_else(|| "No synchronized days yet.".into()),
                        sync.error.as_ref().map(|error| format!("\n{error}")).unwrap_or_default()
                    ))));
                }
                if self.loading { ui.spinner(); }
                if !self.preview {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let completed = self.activity.index.len(ActivityFilter::Ready);
                        let problems = self.activity.index.len(ActivityFilter::Problems);
                        if status_text(ui, "status-activity", language.text(&(format!("{} ready | {} researching | {} waiting | {} to review", completed, self.activity.queue.flights.len(), self.activity.queued + self.activity.deferred, problems)))).on_hover_text(language.text("Show meetings, queue and local context")).clicked() { self.settings = true; }
                        let has_messages = self.error.is_some() || self.activity.error.is_some() || self.activity.sync.error.is_some() || problems > 0
                            || self.activity.event_creations.values().any(|record| self.calendar_owner.as_deref() == Some(record.account.as_str()))
                            || self.activity.recommendations.values().any(|recommendation|
                                recommendation.state == tomlook::recommendations::State::Review
                                    && self.calendar_owner.as_deref() == Some(recommendation.account.as_str()))
                            || self.external_status.values().any(|status| !status.ready && !status.busy && !status.message.is_empty() && status.message != "Not connected");
                        let now = chrono::Local::now().fixed_offset();
                        let recommendations = self.activity.recommendation_view.iter().filter_map(|entry| self.activity.recommendations.get(&entry.id)).filter(|recommendation|
                            recommendation.visible(&self.calendar, self.calendar_owner.as_deref(), now)).count();
                        if (has_messages || recommendations > 0) && inbox_button(ui, language, has_messages, recommendations).clicked() {
                            self.messages_open = true;
                            if self.ai.is_some() { self.ai_command(ai::Command::RefreshCalendarAvailability); }
                        }
                    });
                }
            });
        });
        egui::Panel::left("navigation")
            .exact_size(NAVIGATION_WIDTH)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("navigation-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.sidebar(ui));
            });
        if self.assistant && !self.assistant_detached {
            egui::Panel::right("ask-panel")
                .default_size(520.0)
                .size_range(340.0..=680.0)
                .resizable(true)
                .show(ui, |ui| self.assistant_panel(ui));
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
            if self.preferences.view == View::Month
                && self.date_selection.is_none()
                && self.date_drag.is_none()
            {
                self.month(ui);
            } else {
                self.timeline(ui);
            }
        });
        self.overlays(&context);
        self.connection_dialog(&context);
        if self.interaction_trace.is_some() {
            if let (Some(profile), Some(started)) = (&mut self.render_profile, interaction_started)
            {
                profile.ui_passes += 1;
                profile.ui_total_ms += started.elapsed().as_secs_f64() * 1000.0;
            }
            let mut state = self.interaction_state(&context);
            state["ui_callback_ms"] = interaction_started
                .map(|started| started.elapsed().as_secs_f64() * 1000.0)
                .into();
            if let Some(trace) = &mut self.interaction_trace {
                trace.rendered(state);
                if trace.finished() {
                    if let Some(lifecycle) = &self.lifecycle
                        && let Ok(mut lifecycle) = lifecycle.lock()
                    {
                        lifecycle.startup = Some(trace.report());
                    }
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    context.request_repaint();
                }
            }
        }
        if self.startup.as_ref().is_some_and(|trace| !trace.idle)
            && !self.startup_built
            && !self.loading
        {
            self.startup_built = true;
            if let Some(trace) = &mut self.startup {
                trace.mark("loaded_frame_built");
            }
            context.request_repaint();
        }
    }
}

#[derive(Clone, Copy)]
enum HeaderIcon { Settings, Ask, Commands }

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

fn status_text(
    ui: &mut egui::Ui,
    identity: &str,
    label: impl Into<egui::WidgetText>,
) -> egui::Response {
    let response = ui
        .push_id(identity, |ui| {
            ui.add(
                egui::Button::new(label)
                    .frame(false)
                    .min_size(Vec2::new(0.0, 24.0)),
            )
        })
        .inner;
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            2,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        let rect = response.rect.shrink(3.0);
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            ui.visuals().widgets.hovered.fg_stroke,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
fn messages_button(ui: &mut egui::Ui, language: tomlook::i18n::Language) -> egui::Response {
    inbox_button(ui, language, true, 0)
}

fn inbox_button(
    ui: &mut egui::Ui,
    language: tomlook::i18n::Language,
    warning: bool,
    recommendations: usize,
) -> egui::Response {
    let response = ui
        .push_id("status-messages", |ui| {
            ui.add(egui::Button::new("").frame(false).min_size(Vec2::new(
                if recommendations > 0 { 36.0 } else { 28.0 },
                28.0,
            )))
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            if recommendations > 0 {
                format!("{}: {recommendations}", language.text("Recommendations"))
            } else {
                language.text("Messages").into_owned()
            },
        )
    });
    let rect =
        egui::Rect::from_min_size(response.rect.min + Vec2::new(3.0, 3.0), Vec2::splat(22.0));
    let color = ui.visuals().weak_text_color();
    let point = |x, y| rect.min + Vec2::new(x * rect.width(), y * rect.height());
    let stroke = Stroke::new(1.4, color);
    ui.painter().add(egui::epaint::Shape::closed_line(
        vec![
            point(0.5, 0.18),
            point(0.35, 0.2),
            point(0.25, 0.32),
            point(0.25, 0.58),
            point(0.2, 0.7),
            point(0.14, 0.77),
            point(0.86, 0.77),
            point(0.8, 0.7),
            point(0.75, 0.58),
            point(0.75, 0.32),
            point(0.65, 0.2),
        ],
        stroke,
    ));
    ui.painter()
        .line_segment([point(0.5, 0.08), point(0.5, 0.18)], stroke);
    ui.painter().circle_filled(point(0.5, 0.89), 1.6, color);
    if recommendations > 0 {
        let badge = response.rect.right_top() + Vec2::new(-7.0, 8.0);
        ui.painter()
            .circle_filled(badge, 7.0, ui.visuals().faint_bg_color);
        ui.painter().text(
            badge,
            egui::Align2::CENTER_CENTER,
            recommendations.min(99).to_string(),
            egui::FontId::proportional(10.0),
            ui.visuals().text_color(),
        );
    } else if warning {
        ui.painter()
            .circle_filled(response.rect.right_top() + Vec2::new(-3.0, 4.0), 2.5, color);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            3,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(if recommendations > 0 {
        format!("{}: {recommendations}", language.text("Recommendations"))
    } else {
        language.text("Messages").into_owned()
    })
}

impl Tomlook {
    fn interaction_state(&self, context: &egui::Context) -> serde_json::Value {
        let view = match self.preferences.view {
            View::Day => "day",
            View::WorkWeek => "work-week",
            View::Week => "week",
            View::Month => "month",
        };
        serde_json::json!({
            "loading": self.loading,
            "error": self.error,
            "events": self.calendar.events.len(),
            "view": view,
            "anchor": self.anchor.to_string(),
            "today": Local::now().date_naive().to_string(),
            "previous": self.preferences.view.shift(self.anchor, -1).to_string(),
            "next": self.preferences.view.shift(self.anchor, 1).to_string(),
            "palette": self.palette,
            "palette_query": self.palette_query,
            "query": self.query,
            "dark": self.preferences.dark,
            "selected": self.selected.is_some(),
            "assistant": self.assistant,
            "render_profile": self.render_profile,
            "search_focused": context.memory(|memory| memory.focused() == Some(Id::new("calendar-search"))),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn settings_content_has_clear_padding_from_navigation_in_both_sections_and_themes() {
        for dark in [true, false] {
            for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
                for section in [PreferencesSection::Display, PreferencesSection::Prompts] {
                    let mut app = Tomlook::initial(None, None, None);
                    app.preferences.dark = dark;
                    app.preferences.language = language;
                    app.preferences_open = true;
                    app.preferences_section = section;
                    let context = egui::Context::default();
                    context.set_visuals(if dark {egui::Visuals::dark()} else {egui::Visuals::light()});
                    let mut output = None;
                    for _ in 0..3 {
                        let mut frame = context.run_ui(egui::RawInput {
                            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                            ..Default::default()
                        }, |ui| app.preferences_dialog(ui.ctx()));
                        frame.textures_delta.clear();
                        output = Some(frame);
                    }
                    let output = output.unwrap();
                    let divider = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::LineSegment {points,..}
                            if (points[0].x - points[1].x).abs() < 0.1
                                && (points[0].y - points[1].y).abs() > 300.0 => Some(points[0].x),
                        _ => None,
                    }).expect("Settings navigation must have a vertical divider");
                    let heading = language.text(match section {
                        PreferencesSection::Display => "Display",
                        PreferencesSection::Prompts => "AI prompts",
                    });
                    let content = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text)
                            if text.galley.job.text == heading && text.pos.x > divider => Some(text.pos),
                        _ => None,
                    }).expect("Settings content heading must be visible");
                    assert!(content.x - divider >= 23.0,
                        "{language:?} / {section:?} / dark={dark}: content is only {}px from the divider", content.x - divider);
                    let window = context.memory(|memory| memory.area_rect(Id::new("preferences")).unwrap());
                    assert!(window.right() - content.x > 450.0, "Padding must preserve usable content width");
                }
            }
        }
    }

    #[test]
    fn settings_have_left_navigation_and_language_is_only_in_display_preferences() {
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            let (mut app, _, mut ai_commands) = assistant_fixture();
            let (sender, receiver) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender, notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()), done: None,
            });
            app.preferences.language = language;
            app.preferences_open = true;
            app.prompt_draft.recap.push_str(" Keep this unsaved edit.");
            let draft = app.prompt_draft.clone();
            let context = egui::Context::default();
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1400.0, 900.0))),
                    events, ..Default::default()
                }, |ui| app.preferences_dialog(ui.ctx()));
                output.textures_delta.clear();
                output
            };
            let click = |app: &mut Tomlook, label: &str| {
                draw(app, vec![]);
                let output = draw(app, vec![]);
                let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                        Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                }).unwrap_or_else(|| panic!("Missing settings control: {label}"));
                for pressed in [true, false] {
                    draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                        pos,button:egui::PointerButton::Primary,pressed,modifiers:Modifiers::NONE,
                    }]);
                }
            };
            draw(&mut app, vec![]);
            draw(&mut app, vec![]);
            let rect = context.memory(|memory| memory.area_rect(Id::new("preferences")).unwrap());
            assert!(rect.width() >= 900.0 && rect.height() >= 580.0, "{rect:?}");
            click(&mut app, language.text("AI prompts").as_ref());
            assert!(app.preferences_section == PreferencesSection::Prompts);
            click(&mut app, language.text("Display").as_ref());
            assert!(app.preferences_section == PreferencesSection::Display);
            assert_eq!(app.prompt_draft, draft);
            click(&mut app, if language == tomlook::i18n::Language::En { "Čeština" } else { "English" });
            assert_eq!(app.preferences.language, language.other());
            assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences)
                if preferences.language == language.other()));
            let mut output = context.run_ui(egui::RawInput::default(), |ui| app.header(ui));
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::epaint::Shape::Text(text) if matches!(text.galley.job.text.as_str(), "EN" | "CZ"))));
            output.textures_delta.clear();
            assert!(receiver.try_recv().is_err());
            assert!(ai_commands.try_recv().is_err());
        }
    }

    #[test]
    fn follow_menu_submits_directly_and_decline_defaults_to_no_organizer_response() {
        use tomlook::recommendations::{Change, MeetingResponse};
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            let (mut app, _, mut ai_commands) = assistant_fixture();
            let (sender, receiver) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender, notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()), done: None,
            });
            app.preferences.language = language;
            app.calendar_owner = Some("owner".into());
            app.workiq_ready = true;
            let mut event = tomlook::calendar::fixture(1).remove(0);
            event.organizer = "organizer@example.test".into();
            event.extra.insert("isOrganizer".into(), false.into());
            event.extra.insert("type".into(), "occurrence".into());
            let context = egui::Context::default();
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(egui::RawInput {events, ..Default::default()},
                    |ui| app.meeting_actions(ui, 0));
                output.textures_delta.clear();
                output
            };
            let click = |app: &mut Tomlook, label: &str| {
                draw(app, vec![]);
                let output = draw(app, vec![]);
                let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                        Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                }).unwrap_or_else(|| panic!("Missing meeting action: {label}"));
                for pressed in [true, false] {
                    draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                        pos,button:egui::PointerButton::Primary,pressed,modifiers:Modifiers::NONE,
                    }]);
                }
            };
            for enabled in [true, false] {
                event.extra.insert("categories".into(),
                    if enabled { serde_json::json!(["Call"]) } else { serde_json::json!(["Call", "Follow"]) });
                app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
                click(&mut app, language.text(if enabled { "Follow" } else { "Stop following" }).as_ref());
                assert!(app.meeting_action.is_none(), "Follow must not open a confirmation");
                assert!(matches!(receiver.try_recv().unwrap(), Command::ConfirmMeetingAction {event: submitted,account,change}
                    if submitted.id == event.id && account == "owner" && change == Change::Follow(enabled)));
                assert!(receiver.try_recv().is_err());
            }
            click(&mut app, language.text("Decline invitation").as_ref());
            assert!(matches!(app.meeting_action.as_ref().unwrap().change,
                Change::Respond {response: MeetingResponse::Decline,send_response: false,..}));
            assert!(receiver.try_recv().is_err(), "Decline still requires its existing confirmation");
            app.meeting_action = None;
            event.extra.insert("categories".into(), serde_json::json!(["Call"]));
            for unavailable in ["organizer", "metadata", "connection"] {
                let mut unavailable_event = event.clone();
                if unavailable == "organizer" {
                    unavailable_event.extra.insert("isOrganizer".into(), true.into());
                } else if unavailable == "metadata" {
                    unavailable_event.extra.remove("type");
                }
                app.workiq_ready = unavailable != "connection";
                app.calendar = Arc::new(Calendar::build(vec![unavailable_event], Default::default(), vec![]).unwrap());
                click(&mut app, language.text("Follow").as_ref());
                assert!(app.meeting_action.is_none(), "Unavailable Follow must stay visible without submitting");
                assert!(receiver.try_recv().is_err());
            }
            assert!(ai_commands.try_recv().is_err());
        }
    }

    #[test]
    fn mini_month_arrows_browse_without_changing_main_view_or_starting_ai() {
        let (mut app, _, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        let anchor = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        for view in [View::Day, View::WorkWeek, View::Month] {
            app.anchor = anchor;
            app.mini_anchor = None;
            app.preferences.view = view;
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(egui::RawInput {events, ..Default::default()}, |ui| app.sidebar(ui));
                output.textures_delta.clear();
            };
            draw(&mut app, vec![]);
            let pos = context.data(|data| data.get_temp::<egui::Rect>(Id::new("mini-next-month")).unwrap()).center();
            for pressed in [true, false] {
                draw(&mut app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos,button:egui::PointerButton::Primary,pressed,modifiers:Modifiers::NONE,
                }]);
            }
            assert_eq!(app.anchor, anchor);
            assert_eq!(app.preferences.view, view);
            assert_eq!(app.mini_anchor.unwrap().month(), 11);
        }
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn tentative_hatching_is_neutral_in_both_themes_and_notes_show_personal_evidence() {
        let mut app = Tomlook::initial(None, None, None);
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.status = "tentativelyAccepted".into();
        app.calendar = Arc::new(Calendar::build(vec![event], Default::default(), vec![]).unwrap());
        let context = egui::Context::default();
        for dark in [true, false] {
            app.preferences.dark = dark;
            context.set_visuals(if dark {egui::Visuals::dark()} else {egui::Visuals::light()});
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(300.0, 38.0));
                app.event_card(ui, 0, rect);
            });
            let fill = Color32::from_gray(if dark {31} else {247});
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.fill == fill)));
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::LineSegment {points,stroke}
                if stroke.width == 1.0 && points[0].x != points[1].x && points[0].y != points[1].y)));
            output.textures_delta.clear();
        }
        app.briefing = Some(serde_json::json!({"tomlook_recap":true,"summary":"A completed meeting",
            "preparation":["My assignment: deliver a demo"],"open_questions":["My mention: discussed the owner"],
            "talking_points":[],"risks":[],"gaps":[],"sources":[]}));
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.render_insights(ui, false));
        let mut text = String::new();
        for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
        assert!(text.contains("My tasks"));
        assert!(text.contains("Mentions of me"));
        assert!(text.contains("deliver a demo"));
        assert!(text.contains("discussed the owner"));
        output.textures_delta.clear();
    }

    #[test]
    fn right_click_owned_meeting_opens_paired_travel_confirmation_and_never_recreates_the_meeting() {
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            let (mut app, _, mut ai_commands) = assistant_fixture();
            let (sender, receiver) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender, notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()), done: None,
            });
            let mut event = tomlook::calendar::fixture(1).remove(0);
            event.id = "owned-meeting".into();
            event.title = "Existing owned meeting".into();
            event.is_all_day = false;
            event.extra.insert("type".into(), serde_json::json!("singleInstance"));
            event.extra.insert("isOrganizer".into(), serde_json::json!(true));
            event.extra.insert("sensitivity".into(), serde_json::json!("private"));
            app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap());
            app.calendar_owner = Some("owner".into());
            app.workiq_ready = true;
            app.preferences.language = language;
            let context = egui::Context::default();
            let rect = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), Vec2::new(300.0, 90.0));
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1366.0, 1000.0))),
                    events, ..Default::default()
                }, |ui| {
                    app.event_card(ui, 0, rect);
                    app.event_creation_dialog(ui.ctx());
                });
                output.textures_delta.clear();
                output
            };
            draw(&mut app, Vec::new());
            draw(&mut app, vec![egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE }]);
            draw(&mut app, vec![egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE }]);
            let click = |app: &mut Tomlook, label: &str| {
                draw(app, Vec::new());
                let output = draw(app, Vec::new());
                let point = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                }).unwrap_or_else(|| panic!("Missing travel action: {label}"));
                draw(app, vec![egui::Event::PointerMoved(point),
                    egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                    egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
            };
            click(&mut app, language.text("Add travel blocks before and after").as_ref());
            let draft = app.event_draft.as_mut().unwrap();
            assert_eq!(draft.parent.as_ref().unwrap().id, event.id);
            assert_eq!(draft.travel_minutes, 30);
            draft.travel_minutes = 45;
            assert!(receiver.try_recv().is_err(), "Opening the menu/dialog must not write");
            let output = draw(&mut app, Vec::new());
            let mut rendered = String::new();
            for shape in &output.shapes { append_shape_text(&shape.shape, &mut rendered); }
            assert!(!rendered.contains(language.text("Event title").as_ref()));
            assert!(rendered.contains(language.text("The existing meeting stays unchanged; only two travel blocks are created.").as_ref()));
            click(&mut app, language.text("Create travel blocks").as_ref());
            let Command::CreateEvents { account, draft, parent: Some(parent) } = receiver.try_recv().unwrap() else { panic!("Travel-only approval required") };
            assert_eq!(account, "owner");
            assert_eq!(parent.id, event.id);
            assert_eq!(draft.start, event.start);
            assert_eq!(draft.end, event.end);
            assert_eq!(draft.travel_minutes, Some(45));
            assert!(draft.private && !draft.teams);
            assert!(draft.category.is_none());
            assert!(app.event_draft.is_none());
            assert!(receiver.try_recv().is_err());
            assert!(ai_commands.try_recv().is_err());
        }
    }

    #[test]
    fn travel_context_action_is_not_offered_for_attendee_meetings_and_escape_never_confirms() {
        let (mut app, _, mut commands) = assistant_fixture();
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.is_all_day = false;
        event.extra.insert("type".into(), serde_json::json!("singleInstance"));
        event.extra.insert("isOrganizer".into(), serde_json::json!(false));
        event.extra.insert("sensitivity".into(), serde_json::json!("normal"));
        app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap());
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.meeting_actions(ui, 0));
        let mut text = String::new();
        for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
        output.textures_delta.clear();
        assert!(!text.contains("Add travel blocks before and after"));
        event.extra.insert("isOrganizer".into(), serde_json::json!(true));
        app.open_travel_draft(event);
        let mut output = context.run_ui(egui::RawInput {
            events: vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
            ..Default::default()
        }, |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(app.event_draft.is_none());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn timeline_real_empty_lane_drag_does_not_scroll_and_opens_exact_interval() {
        let (mut app, _, mut commands) = assistant_fixture();
        let day = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        app.anchor = day;
        app.preferences.view = View::Day;
        app.calendar = Arc::new(Calendar::build(Vec::new(), [day].into(), Vec::new()).unwrap());
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                events, ..Default::default()
            }, |ui| app.timeline(ui));
            output.textures_delta.clear();
            output
        };
        draw(&mut app, Vec::new());
        let output = draw(&mut app, Vec::new());
        let nine = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == "09:00" =>
                Some(egui::pos2(text.pos.x + 200.0, text.pos.y - 5.0)),
            _ => None,
        }).expect("Nine o'clock is initially visible");
        let ten = nine + Vec2::new(0.0, 58.0);
        draw(&mut app, vec![egui::Event::PointerMoved(nine)]);
        draw(&mut app, vec![egui::Event::PointerButton {
            pos: nine, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE,
        }]);
        assert!(app.time_drag.is_some());
        draw(&mut app, vec![egui::Event::PointerMoved(ten)]);
        assert!(app.event_draft.is_none());
        draw(&mut app, vec![egui::Event::PointerButton {
            pos: ten, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE,
        }]);
        let draft = app.event_draft.as_ref().unwrap();
        assert_eq!((draft.start, draft.end), (540, 600));
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn empty_time_click_drag_reverse_midnight_and_cancel_preserve_exact_selection_without_writes() {
        for (start, end, expected) in [
            (540, 600, (540, 600)),
            (600, 540, (540, 600)),
            (540, 540, (540, 570)),
            (1425, 1440, (1425, 1440)),
        ] {
            let (mut app, _, mut commands) = assistant_fixture();
            let context = egui::Context::default();
            let day = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
            let mut rect = egui::Rect::NOTHING;
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(500.0, 1600.0))),
                ..Default::default()
            }, |ui| {
                rect = ui.allocate_exact_size(Vec2::new(400.0, 24.0 * 58.0), egui::Sense::hover()).0;
            });
            output.textures_delta.clear();
            let point = |minutes: u16| egui::pos2(rect.left() + 30.0, rect.top() + f32::from(minutes) / 60.0 * 58.0);
            let pointer = |minutes, pressed| egui::Event::PointerButton {
                pos: point(minutes), button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
            };
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(500.0, 1600.0))),
                events: vec![egui::Event::PointerMoved(point(start)), pointer(start, true),
                    egui::Event::PointerMoved(point(end)), pointer(end, false)],
                ..Default::default()
            }, |ui| app.select_event_time(ui, day, rect, &[]));
            output.textures_delta.clear();
            let draft = app.event_draft.as_ref().expect("Release opens the editor");
            assert_eq!((draft.start, draft.end), expected);
            assert_eq!(draft.day, day);
            assert_eq!(draft.travel_minutes, 30);
            assert!(!draft.travel && !draft.private && !draft.teams);
            assert!(commands.try_recv().is_err());
            let mut output = context.run_ui(egui::RawInput {
                events: vec![egui::Event::Key { key: Key::Escape, physical_key: None,
                    pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                ..Default::default()
            }, |ui| app.shortcuts(ui.ctx()));
            output.textures_delta.clear();
            assert!(app.event_draft.is_none());
            assert!(commands.try_recv().is_err());
        }
    }

    #[test]
    fn event_time_selection_ignores_existing_cards_and_lost_focus() {
        let (mut app, _, _) = assistant_fixture();
        let context = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::new(200.0, 600.0));
        let day = Local::now().date_naive();
        for focused in [true, false] {
            let mut output = context.run_ui(egui::RawInput {
                focused,
                events: vec![egui::Event::PointerMoved(rect.center()), egui::Event::PointerButton {
                    pos: rect.center(), button: egui::PointerButton::Primary, pressed: true,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            }, |ui| app.select_event_time(ui, day, rect, &[rect]));
            output.textures_delta.clear();
            assert!(app.time_drag.is_none());
            assert!(app.event_draft.is_none());
        }
        app.time_drag = Some((day, 540, 600));
        let mut output = context.run_ui(egui::RawInput { focused: false, ..Default::default() },
            |ui| app.select_event_time(ui, day, rect, &[]));
        output.textures_delta.clear();
        assert!(app.time_drag.is_none());
    }

    #[test]
    fn event_editor_selects_personal_privacy_and_submits_one_exact_explicit_creation_in_both_languages() {
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            let (mut app, _, mut ai_commands) = assistant_fixture();
            let (sender, receiver) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender, notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()), done: None,
            });
            let mut event = tomlook::calendar::fixture(1).remove(0);
            event.category = "osobní".into();
            event.extra.insert("categories".into(), serde_json::json!(["osobní", "office"]));
            app.calendar = Arc::new(Calendar::build(vec![event], Default::default(), Vec::new()).unwrap());
            app.calendar_owner = Some("owner".into());
            app.workiq_ready = true;
            app.preferences.language = language;
            app.open_event_draft(Local::now().date_naive(), 540, 600);
            app.event_draft.as_mut().unwrap().title = "Synthetic private appointment".into();
            let context = egui::Context::default();
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1366.0, 1000.0))),
                    events, ..Default::default()
                }, |ui| app.event_creation_dialog(ui.ctx()));
                output.textures_delta.clear();
                output
            };
            let click = |app: &mut Tomlook, label: &str| {
                draw(app, Vec::new());
                let output = draw(app, Vec::new());
                let position = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                        Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                }).unwrap_or_else(|| panic!("Missing creation control: {label}"));
                draw(app, vec![egui::Event::PointerMoved(position),
                    egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                    egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
            };
            click(&mut app, language.text("Private event").as_ref());
            assert!(app.event_draft.as_ref().unwrap().private);
            assert_eq!(app.event_draft.as_ref().unwrap().category.as_deref(), Some("osobní"));
            click(&mut app, language.text("Teams meeting").as_ref());
            click(&mut app, language.text("Add travel blocks before and after").as_ref());
            assert!(receiver.try_recv().is_err(), "Editing must never submit writes");
            click(&mut app, language.text("Create event").as_ref());
            let Command::CreateEvents { account, draft, parent } = receiver.try_recv().unwrap() else { panic!("Expected exact creation approval") };
            assert!(parent.is_none());
            assert_eq!(account, "owner");
            assert_eq!(draft.title, "Synthetic private appointment");
            assert!(draft.private && draft.teams);
            assert_eq!(draft.category.as_deref(), Some("osobní"));
            assert_eq!(draft.travel_minutes, Some(30));
            assert!(app.event_draft.is_none());
            assert!(receiver.try_recv().is_err());
            assert!(ai_commands.try_recv().is_err(), "No model call for event creation");
        }
    }

    use super::*;
    use tomlook::proposals::{Kind, Proposal};

    #[test]
    fn recommendation_cards_separate_action_meeting_reason_and_adjacent_travel_sides() {
        use tomlook::recommendations::{Kind as RecommendationKind, Recommendation, Suggestion};
        let (mut app, _, mut commands) = assistant_fixture();
        let now = Local::now().fixed_offset();
        let monday = tomlook::calendar::monday(now.date_naive()) + chrono::Duration::days(7);
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.start = monday
            .and_hms_opt(14, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
            .fixed_offset();
        event.end = event.start + chrono::Duration::hours(1);
        event.is_all_day = false;
        event.title = "Client workshop".into();
        event.location = "Client office".into();
        event.status = "accepted".into();
        event.category = "mimo office".into();
        event
            .extra
            .insert("categories".into(), serde_json::json!(["mimo office"]));
        app.calendar =
            Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
        app.calendar_owner = Some("owner".into());
        let context = egui::Context::default();
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            for (kind, heading, time) in [
                (
                    RecommendationKind::Preparation,
                    "Make time to prepare",
                    "13:30",
                ),
                (
                    RecommendationKind::Travel,
                    "Reserve travel before meeting",
                    "13:30",
                ),
                (
                    RecommendationKind::TravelAfter,
                    "Reserve travel after meeting",
                    "15:00",
                ),
            ] {
                let record = Recommendation::from_insights(
                    &event,
                    "owner",
                    Suggestion {
                        kind,
                        reason: "Explain the architecture and demonstrate the prototype.".into(),
                        reason_cs: "Vysvetli architekturu a ukaz prototyp.".into(),
                        category: None,
                        minutes: Some(30),
                    },
                    now,
                )
                .unwrap();
                let mut text = String::new();
                for _ in 0..3 {
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(800.0, 900.0),
                            )),
                            ..Default::default()
                        },
                        |ui| app.recommendation_card(ui, &record, &app.calendar.clone(), now),
                    );
                    for shape in &output.shapes {
                        append_shape_text(&shape.shape, &mut text);
                    }
                    output.textures_delta.clear();
                }
                for label in [
                    heading,
                    "Meeting",
                    "Why?",
                    "Add to calendar",
                    "Details and checks",
                ] {
                    assert!(
                        text.contains(language.text(label).as_ref()),
                        "{language:?}, {kind:?}: {text}"
                    );
                }
                assert!(text.contains("Client workshop"));
                assert!(text.contains(time));
                assert!(
                    !text.contains(
                        language
                            .text("Suggested slots use weekdays, 08:00-18:00 local time.")
                            .as_ref()
                    )
                );
            }
        }
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn neutral_bell_replaces_the_triangle_and_remains_keyboard_accessible() {
        let context = egui::Context::default();
        let mut identity = None;
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            for warning in [false, true] {
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    let response = inbox_button(ui, language, warning, 3);
                    identity = Some(response.id);
                });
                assert!(!output.shapes.iter().any(|shape|
                    matches!(&shape.shape, egui::epaint::Shape::Path(path) if path.points.len() == 3)));
                let mut text = String::new();
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                assert!(!text.contains('!'));
                output.textures_delta.clear();
            }
        }
        context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
        let mut clicked = false;
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                clicked = inbox_button(ui, tomlook::i18n::Language::Cs, true, 3).clicked();
            },
        );
        output.textures_delta.clear();
        assert!(clicked);
    }

    #[test]
    fn recommendation_navigation_renders_one_detail_and_keyboard_does_not_submit() {
        use tomlook::recommendations::{Kind, Recommendation, Suggestion};
        let (mut app, _, mut commands) = assistant_fixture();
        let now = Local::now().fixed_offset();
        let mut events = Vec::new();
        let mut inbox = BTreeMap::new();
        for index in 0..12 {
            let mut event = tomlook::calendar::fixture(1).remove(0);
            event.id = format!("meeting-{index}");
            event.title = format!("Synthetic meeting {index}");
            event.start = now + chrono::Duration::hours(2 + index);
            event.end = event.start + chrono::Duration::minutes(30);
            event.is_all_day = false;
            event.status = "accepted".into();
            let record = Recommendation::from_insights(
                &event,
                "owner",
                Suggestion {
                    kind: Kind::MissingMaterials,
                    reason: format!("EXPLANATION_{index}_ONLY"),
                    reason_cs: format!("VYSVETLENI_{index}_JEN"),
                    category: None,
                    minutes: None,
                },
                now,
            )
            .unwrap();
            inbox.insert(record.id.clone(), record);
            events.push(event);
        }
        app.calendar = Arc::new(Calendar::build(events, Default::default(), vec![]).unwrap());
        app.calendar_owner = Some("owner".into());
        let mut activity = (*app.activity).clone();
        activity.error = Some("Saved diagnostic issue.".into());
        activity.recommendations = Arc::new(inbox);
        activity.recommendation_view = Arc::new(tomlook::recommendations::inbox_view(
            &app.calendar,
            &activity.recommendations,
            app.calendar_owner.as_deref(),
            now,
            &BTreeSet::new(),
        ));
        app.activity = Arc::new(activity);
        app.messages_open = true;
        let context = egui::Context::default();
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            app.recommendation_selected = None;
            app.recommendation_cursor = 0;
            let mut final_text = String::new();
            for _ in 0..30 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(1366.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| app.overlays(ui.ctx()),
                );
                final_text.clear();
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut final_text);
                }
                output.textures_delta.clear();
            }
            let explanation = if language == tomlook::i18n::Language::En {
                "EXPLANATION"
            } else {
                "VYSVETLENI"
            };
            assert!(final_text.contains(&format!("{explanation}_0_")));
            assert!(!final_text.contains(&format!("{explanation}_1_")));
            assert!(!final_text.contains(language.text("Resume other queued jobs").as_ref()));
            let rect = context.memory(|memory| {
                memory
                    .area_rect(Id::new("status-messages-window-v3"))
                    .unwrap()
            });
            assert!(
                rect.height() <= 710.0,
                "Recommendation window grew: {rect:?}"
            );
            for cursor in 1..12 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(1366.0, 900.0),
                        )),
                        events: vec![egui::Event::Key {
                            key: Key::ArrowDown,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Modifiers::ALT,
                        }],
                        ..Default::default()
                    },
                    |ui| app.overlays(ui.ctx()),
                );
                assert_eq!(app.recommendation_cursor, cursor);
                let mut text = String::new();
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                assert!(text.contains(&format!("{explanation}_{cursor}_")), "{text}");
                output.textures_delta.clear();
            }
            assert_eq!(
                app.recommendation_selected.as_deref(),
                app.activity
                    .recommendations
                    .values()
                    .find(|record| record.event_title == "Synthetic meeting 11")
                    .map(|record| record.id.as_str())
            );
        }
        app.recommendations_tab = false;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.overlays(ui.ctx()));
        output.textures_delta.clear();
        let mut text = String::new();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        assert!(
            text.contains(
                app.preferences
                    .language
                    .text("Resume other queued jobs")
                    .as_ref()
            )
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn conflict_cards_name_counterparts_and_render_the_local_overlap_not_raw_utc_reason() {
        use tomlook::recommendations::{Kind, Recommendation, Suggestion};
        let (mut app, _, mut commands) = assistant_fixture();
        let now = Local::now().fixed_offset();
        let day = tomlook::calendar::monday(now.date_naive()) + chrono::Duration::days(7);
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.start = day
            .and_hms_opt(17, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
            .fixed_offset();
        event.end = event.start + chrono::Duration::hours(3);
        event.is_all_day = false;
        event.status = "accepted".into();
        let mut overlap = event.clone();
        overlap.id = "transfer".into();
        overlap.title = "Synthetic transfer reserve".into();
        overlap.start = (event.start + chrono::Duration::minutes(15))
            .with_timezone(&chrono::FixedOffset::east_opt(0).unwrap());
        overlap.end = event.start + chrono::Duration::hours(4);
        app.calendar = Arc::new(
            Calendar::build(vec![event.clone(), overlap], Default::default(), vec![]).unwrap(),
        );
        app.calendar_owner = Some("owner".into());
        let record = Recommendation::from_insights(
            &event,
            "owner",
            Suggestion {
                kind: Kind::PriorityConflict,
                reason: "Overlap from 15:15 UTC.".into(),
                reason_cs: "Prekryv od 15:15 UTC.".into(),
                category: None,
                minutes: None,
            },
            now,
        )
        .unwrap();
        let context = egui::Context::default();
        let mut text = String::new();
        for _ in 0..3 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(800.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| app.recommendation_card(ui, &record, &app.calendar.clone(), now),
            );
            text.clear();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        assert!(text.contains("Synthetic transfer reserve"));
        assert!(text.contains("Overlap: 17:15"));
        assert!(text.contains("17:15 - 21:00"));
        assert!(
            text.contains("20:00"),
            "The intersection must remain secondary to the full counterpart time"
        );
        assert!(!text.contains("15:15 UTC"));
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn recommendation_meeting_links_and_conflict_actions_are_navigable_and_require_confirmation() {
        use tomlook::recommendations::{Change, MeetingResponse};
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = std::sync::mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender,
            notices: Arc::new(Default::default()),
            activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()),
            done: None,
        });
        let now = Local::now().fixed_offset();
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.title = "Synthetic meeting link".into();
        event.start = now + chrono::Duration::days(7);
        event.end = event.start + chrono::Duration::hours(1);
        event.is_all_day = false;
        event.status = "accepted".into();
        event.organizer = "organizer@example.test".into();
        event
            .extra
            .insert("isOrganizer".into(), serde_json::json!(false));
        event
            .extra
            .insert("type".into(), serde_json::json!("occurrence"));
        app.calendar =
            Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
        app.calendar_owner = Some("owner".into());
        app.workiq_ready = true;
        app.preferences.view = View::Week;
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            let context = egui::Context::default();
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(800.0, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| app.recommendation_meeting(ui, &event, true),
                );
                output.textures_delta.clear();
                output
            };
            let click = |app: &mut Tomlook, label: &str| {
                draw(app, vec![]);
                let output = draw(app, vec![]);
                let position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                            Some(text.pos + text.galley.size() * 0.5)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("Missing button: {label}"));
                draw(
                    app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        },
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                );
            };
            let output = draw(&mut app, vec![]);
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            assert!(text.contains(&meeting_date_label(&event, language)));
            for response in [MeetingResponse::Tentative, MeetingResponse::Decline] {
                app.messages_open = true;
                click(&mut app, language.text(response.label()).as_ref());
                let draft = app.meeting_action.take().unwrap();
                assert_eq!(draft.event.id, event.id);
                assert!(
                    matches!(draft.change, Change::Respond { response: selected, send_response, .. }
                        if selected == response && send_response == (response != MeetingResponse::Decline))
                );
                assert!(!app.messages_open);
                assert!(
                    receiver.try_recv().is_err(),
                    "Opening confirmation must not submit a write"
                );
            }
            app.messages_open = true;
            app.query = "hidden".into();
            app.category = "other".into();
            click(&mut app, &event.title);
            assert_eq!(app.selected, app.calendar.event_index(&event.id));
            assert_eq!(app.anchor, event.calendar_dates().0);
            assert!(app.query.is_empty() && app.category.is_empty());
            assert!(app.insights_expanded && !app.messages_open && app.scroll_meeting);
            assert!(
                matches!(receiver.try_recv().unwrap(), Command::Detail { id, .. } if id == event.id)
            );
            assert!(receiver.try_recv().is_err());
            assert!(ai_commands.try_recv().is_err());
        }
    }

    #[test]
    fn missing_material_cards_show_actual_contacts_in_both_languages_without_outreach() {
        use tomlook::recommendations::{Kind, Recommendation, Suggestion};
        let (mut app, _, mut commands) = assistant_fixture();
        let now = Local::now().fixed_offset();
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.start = now + chrono::Duration::hours(2);
        event.end = event.start + chrono::Duration::hours(1);
        event.is_all_day = false;
        event.organizer = "Synthetic organizer <agenda@partner.example>".into();
        event.attendees = vec![
            "Synthetic colleague <colleague@home.example>".into(),
            "Own alias <alias@home.example>".into(),
        ];
        app.preferences.work_email = "next-account@tenant.example".into();
        app.calendar_owner = Some("me@home.example".into());
        Arc::make_mut(&mut app.activity).identity =
            Some(Arc::new(tomlook::workiq::connection::Identity {
                account: "me@home.example".into(),
                mail: Some("alias@home.example".into()),
                display_name: None,
            }));
        app.calendar =
            Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
        let record = Recommendation::from_insights(
            &event,
            "me@home.example",
            Suggestion {
                kind: Kind::MissingMaterials,
                reason: "Ask for the workshop agenda.".into(),
                reason_cs: "Pozadej o agendu workshopu.".into(),
                category: None,
                minutes: None,
            },
            now,
        )
        .unwrap();
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            let context = egui::Context::default();
            let mut text = String::new();
            for _ in 0..3 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(800.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| app.recommendation_card(ui, &record, &app.calendar.clone(), now),
                );
                text.clear();
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                output.textures_delta.clear();
            }
            assert!(text.contains(language.text("Who to ask").as_ref()));
            assert!(text.contains("agenda@partner.example"));
            assert!(text.contains("colleague@home.example"));
            assert!(!text.contains("alias@home.example"));
            assert!(text.contains(language.text("Same email domain").as_ref()));
            assert!(!text.contains("Send"));
            assert!(commands.try_recv().is_err());
        }
    }

    #[test]
    fn category_transition_has_a_shared_baseline_and_a_vector_arrow_without_a_glyph_box() {
        use tomlook::recommendations::{Kind, Recommendation, Suggestion};
        let (mut app, _, mut commands) = assistant_fixture();
        let now = Local::now().fixed_offset();
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.start = now + chrono::Duration::hours(2);
        event.end = event.start + chrono::Duration::hours(1);
        event.is_all_day = false;
        event.status = "accepted".into();
        event.category.clear();
        event
            .extra
            .insert("categories".into(), serde_json::json!([]));
        let mut reference = event.clone();
        reference.id = "category-reference".into();
        reference.status = "cancelled".into();
        reference.category = "nejist\u{e9}".into();
        reference
            .extra
            .insert("categories".into(), serde_json::json!(["nejist\u{e9}"]));
        app.calendar = Arc::new(
            Calendar::build(vec![event.clone(), reference], Default::default(), vec![]).unwrap(),
        );
        app.calendar_owner = Some("owner".into());
        let record = Recommendation::from_insights(
            &event,
            "owner",
            Suggestion {
                kind: Kind::Category,
                reason: "Explicitly tentative.".into(),
                reason_cs: "Nejista udalost.".into(),
                category: Some("nejist\u{e9}".into()),
                minutes: None,
            },
            now,
        )
        .unwrap();
        let context = egui::Context::default();
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            let mut centers = (None, None);
            let mut arrow_segments = 0;
            for _ in 0..3 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(800.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| app.recommendation_card(ui, &record, &app.calendar.clone(), now),
                );
                centers = (None, None);
                arrow_segments = 0;
                for shape in &output.shapes {
                    match &shape.shape {
                        egui::epaint::Shape::Text(text) => {
                            let value = &text.galley.job.text;
                            assert!(!value.contains('\u{2192}'));
                            let center = text.pos.y + text.galley.rect.center().y;
                            if value == language.text("None").as_ref() {
                                centers.0 = Some(center);
                            }
                            if value == "nejist\u{e9}" {
                                centers.1 = Some(center);
                            }
                        }
                        egui::epaint::Shape::LineSegment { stroke, .. } if stroke.width == 1.6 => {
                            arrow_segments += 1
                        }
                        _ => {}
                    }
                }
                output.textures_delta.clear();
            }
            assert_eq!(arrow_segments, 3);
            assert!(
                (centers.0.unwrap() - centers.1.unwrap()).abs() < 1.0,
                "{centers:?}"
            );
        }
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn inbox_is_bilingual_quiet_and_unknown_outcomes_have_no_confirmation_action() {
        let (mut app, _, mut commands) = assistant_fixture();
        let now = Local::now().fixed_offset();
        let mut events = tomlook::calendar::fixture(2);
        events[0].start = now + chrono::Duration::hours(2);
        events[0].end = events[0].start + chrono::Duration::hours(1);
        events[0].is_all_day = false;
        events[0].category = "Call".into();
        events[0]
            .extra
            .insert("categories".into(), serde_json::json!(["Call"]));
        events[1].category = "Office".into();
        events[1].start = now + chrono::Duration::hours(4);
        events[1].end = events[1].start + chrono::Duration::hours(1);
        events[1]
            .extra
            .insert("categories".into(), serde_json::json!(["Office"]));
        app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
        app.calendar_owner = Some("owner".into());
        app.workiq_ready = true;
        let mut recommendation = tomlook::recommendations::Recommendation::from_insights(
            &app.calendar.events[0],
            "owner",
            tomlook::recommendations::Suggestion {
                kind: tomlook::recommendations::Kind::Category,
                reason: "Office visit.".into(),
                reason_cs: "Osobni navsteva kancelare.".into(),
                category: Some("Office".into()),
                minutes: None,
            },
            now,
        )
        .unwrap();
        let context = egui::Context::default();
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            for state in [
                tomlook::recommendations::State::Pending,
                tomlook::recommendations::State::Review,
            ] {
                recommendation.state = state;
                let mut activity = (*app.activity).clone();
                activity.recommendations = Arc::new(std::collections::BTreeMap::from([(
                    recommendation.id.clone(),
                    recommendation.clone(),
                )]));
                activity.recommendation_view = Arc::new(tomlook::recommendations::inbox_view(
                    &app.calendar,
                    &activity.recommendations,
                    app.calendar_owner.as_deref(),
                    now,
                    &BTreeSet::new(),
                ));
                app.activity = Arc::new(activity);
                for open in [false, true] {
                    app.messages_open = open;
                    let mut text = String::new();
                    for _ in 0..3 {
                        let mut output = context.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    Vec2::new(1366.0, 768.0),
                                )),
                                ..Default::default()
                            },
                            |ui| app.overlays(ui.ctx()),
                        );
                        for shape in &output.shapes {
                            append_shape_text(&shape.shape, &mut text);
                        }
                        output.textures_delta.clear();
                    }
                    assert_eq!(
                        text.contains(language.text("Recommendations").as_ref()),
                        open,
                        "Inbox visibility: {language:?}, {state:?}, open={open}: {text}"
                    );
                    assert_eq!(
                        text.contains(language.text("Use category").as_ref()),
                        open && state == tomlook::recommendations::State::Pending,
                        "Confirmation visibility: {language:?}, {state:?}, open={open}: {text}"
                    );
                }
            }
        }
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn follow_up_opens_local_draft_request_without_submitting_or_erasing_questions() {
        let (mut app, _, mut commands) = assistant_fixture();
        let now = chrono::Local::now().fixed_offset();
        let mut events = tomlook::calendar::fixture(1);
        events[0].start = now - chrono::Duration::hours(2);
        events[0].end = now - chrono::Duration::hours(1);
        app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
        app.calendar_owner = Some("owner".into());
        let recommendation = tomlook::recommendations::Recommendation::from_insights(
            &app.calendar.events[0],
            "owner",
            tomlook::recommendations::Suggestion {
                kind: tomlook::recommendations::Kind::FollowUp,
                reason: "Review the meeting outcomes.".into(),
                reason_cs: "Zkontroluj zavery schuzky.".into(),
                category: None,
                minutes: None,
            },
            now,
        )
        .unwrap();
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            app.question.clear();
            app.messages_open = true;
            app.prepare_follow_up(&recommendation);
            assert!(app.assistant && app.focus_assistant);
            assert!(!app.messages_open);
            assert_eq!(
                app.assistant_context.as_ref().unwrap().id,
                recommendation.event_id
            );
            assert!(
                app.question
                    .contains(if language == tomlook::i18n::Language::En {
                        "placeholders"
                    } else {
                        "zástupná místa"
                    })
            );
            app.question = "Keep my unsent question".into();
            app.prepare_follow_up(&recommendation);
            assert_eq!(app.question, "Keep my unsent question");
            assert!(app.ai_error.as_ref().unwrap().contains("was preserved"));
        }
        app.calendar_owner = Some("other-owner".into());
        app.prepare_follow_up(&recommendation);
        assert_eq!(app.question, "Keep my unsent question");
        assert!(app.error.as_ref().unwrap().contains("no longer available"));
        assert!(app.active_request.is_none());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn right_clicking_a_meeting_opens_actions_without_selecting_or_submitting() {
        let (mut app, _, mut commands) = assistant_fixture();
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event
            .extra
            .insert("type".into(), serde_json::json!("singleInstance"));
        app.calendar =
            Arc::new(Calendar::build(vec![event], Default::default(), Vec::new()).unwrap());
        app.calendar_owner = Some("owner".into());
        let context = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), Vec2::new(300.0, 90.0));
        let point = rect.center();
        let mut text = String::new();
        for events in [
            Vec::new(),
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            Vec::new(),
            Vec::new(),
        ] {
            let mut output = context.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1366.0, 768.0),
                    )),
                    ..Default::default()
                },
                |ui| app.event_card(ui, 0, rect),
            );
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        assert!(
            text.contains(app.preferences.language.text("Change category").as_ref()),
            "{text}"
        );
        assert!(app.selected.is_none());
        assert!(app.meeting_action.is_none());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn manual_menu_and_dialog_are_bilingual_local_and_escape_never_submits() {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = std::sync::mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender,
            notices: Arc::new(Default::default()),
            activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()),
            done: None,
        });
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event
            .extra
            .insert("type".into(), serde_json::json!("occurrence"));
        event
            .extra
            .insert("isOrganizer".into(), serde_json::json!(false));
        event.organizer = "organizer@example.test".into();
        app.calendar =
            Arc::new(Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap());
        app.calendar_owner = Some("owner".into());
        app.selected = Some(0);
        app.workiq_ready = true;
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            let context = egui::Context::default();
            let key = |key, modifiers| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            let mut output = context.run_ui(
                egui::RawInput {
                    events: vec![key(Key::F10, Modifiers::SHIFT)],
                    ..Default::default()
                },
                |ui| app.shortcuts(ui.ctx()),
            );
            output.textures_delta.clear();
            assert_eq!(app.meeting_menu.as_deref(), Some(event.id.as_str()));
            let mut text = String::new();
            for _ in 0..3 {
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    app.meeting_action_dialog(ui.ctx())
                });
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                output.textures_delta.clear();
            }
            assert!(
                text.contains(language.text("Accept invitation").as_ref()),
                "{text}"
            );
            assert!(
                text.contains(language.text("Decline invitation").as_ref()),
                "{text}"
            );
            assert!(
                text.contains(&meeting_date_label(&event, language)),
                "{text}"
            );
            assert!(!text.contains(language.text("Cancel organized meeting").as_ref()));
            assert!(receiver.try_recv().is_err());
            assert!(ai_commands.try_recv().is_err());
            let context = egui::Context::default();
            app.choose_meeting_action(
                event.clone(),
                tomlook::recommendations::Change::Respond {
                    response: tomlook::recommendations::MeetingResponse::Decline,
                    comment: "Unsent draft comment.".into(),
                    send_response: false,
                },
            );
            let mut text = String::new();
            for _ in 0..3 {
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    app.meeting_action_dialog(ui.ctx())
                });
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                output.textures_delta.clear();
            }
            assert!(
                text.contains(language.text("Decline invitation").as_ref()),
                "{text}"
            );
            assert!(
                text.contains(language.text("This occurrence only").as_ref()),
                "{text}"
            );
            assert!(!text.contains("organizer@example.test"), "{text}");
            assert!(
                !text.contains(language.text("Comment (optional)").as_ref()),
                "{text}"
            );
            assert!(!text.contains(language.text("Account").as_ref()), "{text}");
            assert!(
                !text.contains(
                    language
                        .text("Confirmation sends this response and comment.")
                        .as_ref()
                ),
                "{text}"
            );
            assert_eq!(
                app.meeting_action.as_ref().unwrap().change,
                tomlook::recommendations::Change::Respond {
                    response: tomlook::recommendations::MeetingResponse::Decline,
                    comment: "Unsent draft comment.".into(),
                    send_response: false
                }
            );
            assert!(receiver.try_recv().is_err());
            let mut output = context.run_ui(
                egui::RawInput {
                    events: vec![key(Key::Escape, Modifiers::NONE)],
                    ..Default::default()
                },
                |ui| app.shortcuts(ui.ctx()),
            );
            output.textures_delta.clear();
            assert!(app.meeting_action.is_none());
            assert!(receiver.try_recv().is_err());
            assert!(ai_commands.try_recv().is_err());
        }
    }

    #[test]
    fn category_menu_submits_once_without_confirmations_or_opening_messages() {
        use tomlook::recommendations::Change;
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            for clear in [false, true] {
                let (mut app, _, mut ai_commands) = assistant_fixture();
                let (sender, receiver) = std::sync::mpsc::sync_channel(16);
                app.worker = Some(Worker {
                    commands: sender,
                    notices: Arc::new(Default::default()),
                    activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
                    stop: Arc::new(Default::default()),
                    done: None,
                });
                let mut event = tomlook::calendar::fixture(1).remove(0);
                event
                    .extra
                    .insert("type".into(), serde_json::json!("occurrence"));
                event
                    .extra
                    .insert("categories".into(), serde_json::json!(["call", "office"]));
                app.calendar = Arc::new(
                    Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap(),
                );
                app.calendar_owner = Some("owner".into());
                app.preferences.language = language;
                app.workiq_ready = true;
                app.meeting_menu = Some(event.id.clone());
                let context = egui::Context::default();
                let draw = |app: &mut Tomlook, events| {
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(1366.0, 768.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ui| app.meeting_action_dialog(ui.ctx()),
                    );
                    output.textures_delta.clear();
                    output
                };
                let click = |app: &mut Tomlook, label: &str| {
                    draw(app, Vec::new());
                    let output = draw(app, Vec::new());
                    let position = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                                Some(text.pos + text.galley.size() * 0.5)
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| panic!("Missing menu item: {label}"));
                    draw(
                        app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: Modifiers::NONE,
                            },
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                modifiers: Modifiers::NONE,
                            },
                        ],
                    );
                };
                click(&mut app, language.text("Change category").as_ref());
                assert!(
                    receiver.try_recv().is_err(),
                    "Opening the submenu must not write"
                );
                let category = if clear {
                    language.text("Remove category")
                } else {
                    "office".into()
                };
                click(&mut app, category.as_ref());
                match receiver.try_recv().unwrap() {
                    Command::ConfirmMeetingAction {
                        event: selected,
                        account,
                        change,
                    } => {
                        assert_eq!(
                            serde_json::to_value(*selected).unwrap(),
                            serde_json::to_value(&event).unwrap()
                        );
                        assert_eq!(account, "owner");
                        assert_eq!(
                            change,
                            if clear {
                                Change::ClearCategory
                            } else {
                                Change::Category("office".into())
                            }
                        );
                    }
                    _ => panic!("Category selection must dispatch the exact native change"),
                }
                for _ in 0..3 {
                    draw(&mut app, Vec::new());
                }
                assert!(app.meeting_action.is_none());
                assert!(app.meeting_menu.is_none());
                assert!(!app.messages_open);
                assert!(
                    receiver.try_recv().is_err(),
                    "Selection must submit only once"
                );
                assert!(
                    ai_commands.try_recv().is_err(),
                    "No model request belongs in a manual category change"
                );
            }
        }
    }

    #[test]
    fn compact_meeting_confirmation_submits_once_and_stays_quiet() {
        use tomlook::recommendations::{Change, MeetingResponse};
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            for response in [
                Some(MeetingResponse::Accept),
                Some(MeetingResponse::Tentative),
                Some(MeetingResponse::Decline),
                Some(MeetingResponse::Cancel),
                None,
            ] {
                for send_response in [false, true] {
                    let (mut app, _, mut ai_commands) = assistant_fixture();
                    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
                    app.worker = Some(Worker {
                        commands: sender,
                        notices: Arc::new(Default::default()),
                        activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
                        stop: Arc::new(Default::default()),
                        done: None,
                    });
                    let mut event = tomlook::calendar::fixture(1).remove(0);
                    event
                        .extra
                        .insert("type".into(), serde_json::json!("occurrence"));
                    let organizer = response.is_none() || response == Some(MeetingResponse::Cancel);
                    event
                        .extra
                        .insert("isOrganizer".into(), serde_json::json!(organizer));
                    event.organizer = "organizer@example.test".into();
                    if response.is_none() {
                        event.attendees.clear();
                    }
                    app.calendar = Arc::new(
                        Calendar::build(vec![event.clone()], Default::default(), Vec::new())
                            .unwrap(),
                    );
                    app.calendar_owner = Some("owner".into());
                    app.preferences.language = language;
                    app.workiq_ready = true;
                    let action = response.map(MeetingResponse::label).unwrap_or("Delete");
                    let comment = "Preview comment.";
                    app.choose_meeting_action(
                        event,
                        response
                            .map(|response| Change::Respond {
                                response,
                                comment: comment.into(),
                                send_response,
                            })
                            .unwrap_or(Change::Delete),
                    );
                    let context = egui::Context::default();
                    let draw = |app: &mut Tomlook, events| {
                        let mut output = context.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    Vec2::new(1366.0, 768.0),
                                )),
                                events,
                                ..Default::default()
                            },
                            |ui| app.meeting_action_dialog(ui.ctx()),
                        );
                        output.textures_delta.clear();
                        output
                    };
                    draw(&mut app, Vec::new());
                    draw(&mut app, Vec::new());
                    let output = draw(&mut app, Vec::new());
                    let mut text = String::new();
                    for shape in &output.shapes {
                        append_shape_text(&shape.shape, &mut text);
                    }
                    assert!(
                        text.lines().count() <= 12,
                        "Confirmation has too much text: {text}"
                    );
                    assert!(!text.contains("organizer@example.test"), "{text}");
                    assert!(!text.contains(language.text("Account").as_ref()), "{text}");
                    assert_eq!(
                        text.contains(language.text("Notifies all attendees.").as_ref()),
                        response == Some(MeetingResponse::Cancel)
                    );
                    assert!(
                        receiver.try_recv().is_err(),
                        "Opening the confirmation must not write"
                    );
                    let position = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::epaint::Shape::Text(text)
                                if text.galley.job.text == language.text(action) =>
                            {
                                Some(text.pos + text.galley.size() * 0.5)
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| panic!("Missing confirm button: {text}"));
                    draw(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: Modifiers::NONE,
                            },
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                modifiers: Modifiers::NONE,
                            },
                        ],
                    );
                    match receiver.try_recv().unwrap() {
                        Command::ConfirmMeetingAction { change, .. } => assert_eq!(
                            change,
                            response
                                .map(|response| Change::Respond {
                                    response,
                                    comment: if send_response || response == MeetingResponse::Cancel
                                    {
                                        comment.into()
                                    } else {
                                        String::new()
                                    },
                                    send_response: send_response
                                        || response == MeetingResponse::Cancel,
                                })
                                .unwrap_or(Change::Delete)
                        ),
                        _ => panic!("Confirmation must dispatch the exact native action"),
                    }
                    for _ in 0..3 {
                        draw(&mut app, Vec::new());
                    }
                    assert!(app.meeting_action.is_none());
                    assert!(!app.messages_open);
                    assert!(receiver.try_recv().is_err());
                    assert!(ai_commands.try_recv().is_err());
                }
            }
        }
    }

    #[test]
    fn warnings_only_render_after_opening_messages_and_never_submit_research() {
        let (mut app, _, mut commands) = assistant_fixture();
        app.error = Some("AI executor stopped before an outcome was recorded; inspect saved evidence before explicit retry".into());
        let context = egui::Context::default();
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            for opened in [false, true] {
                app.messages_open = opened;
                let mut text = String::new();
                for _ in 0..2 {
                    let mut output =
                        context.run_ui(egui::RawInput::default(), |ui| app.overlays(ui.ctx()));
                    for shape in &output.shapes {
                        append_shape_text(&shape.shape, &mut text);
                    }
                    output.textures_delta.clear();
                }
                assert_eq!(
                    text.contains(language.text(app.error.as_ref().unwrap()).as_ref()),
                    opened
                );
            }
            app.shortcuts(&context);
            assert!(commands.try_recv().is_err());
        }
    }

    #[test]
    fn status_text_has_no_idle_box_and_keeps_keyboard_activation_across_languages() {
        let context = egui::Context::default();
        let mut identity = None;
        let mut rect = egui::Rect::NOTHING;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let response = status_text(ui, "status-fixture", "WorkIQ: connected");
            identity = Some(response.id);
            rect = response.rect;
        });
        assert!(!output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::epaint::Shape::Rect(shape) if shape.rect == rect && shape.fill != Color32::TRANSPARENT)
        }));
        output.textures_delta.clear();
        context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
        let mut clicked = false;
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                let response = status_text(
                    ui,
                    "status-fixture",
                    tomlook::i18n::Language::Cs.text("WorkIQ: connected"),
                );
                assert_eq!(response.id, identity.unwrap());
                clicked = response.clicked();
            },
        );
        output.textures_delta.clear();
        assert!(clicked);
    }

    #[test]
    fn github_dialog_only_shows_status_client_id_and_one_action() {
        let (mut app, _, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        app.connection_panel = Some(ConnectionPanel::External(tomlook::connectors::Kind::Github));
        app.github_client_id = "Iv23fixture".into();
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            for ready in [false, true] {
                app.external_status.insert(
                    tomlook::connectors::Kind::Github,
                    ExternalStatus {
                        ready,
                        message: "Fixture provider diagnostic belongs in Messages".into(),
                        ..Default::default()
                    },
                );
                let mut text = String::new();
                for _ in 0..2 {
                    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                        app.connection_dialog(ui.ctx())
                    });
                    for shape in &output.shapes {
                        append_shape_text(&shape.shape, &mut text);
                    }
                    output.textures_delta.clear();
                }
                assert!(
                    text.contains(
                        language
                            .text(if ready { "Connected" } else { "Not connected" })
                            .as_ref()
                    )
                );
                assert!(text.contains("Iv23fixture"));
                assert!(text.contains(language.text("Sign in").as_ref()));
                for removed in [
                    "Browser sign-in",
                    "Personal token",
                    "GitHub App setup",
                    "Disconnect and forget",
                ] {
                    assert!(
                        !text.contains(language.text(removed).as_ref()),
                        "Unexpected control: {removed}"
                    );
                }
                assert!(!text.contains("Fixture provider diagnostic"));
                assert!(commands.try_recv().is_err());
            }
        }
        app.external_status.insert(
            tomlook::connectors::Kind::Github,
            ExternalStatus {
                busy: true,
                message: "Open https://github.com/login/device and enter ABCD-EF12".into(),
                ..Default::default()
            },
        );
        let mut text = String::new();
        for _ in 0..2 {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                app.connection_dialog(ui.ctx())
            });
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        assert!(
            text.contains("ABCD-EF12"),
            "Device authorization instructions must remain visible"
        );
        assert!(text.contains(app.preferences.language.text("Cancel").as_ref()));
    }

    #[test]
    fn messages_indicator_is_small_stable_and_keyboard_accessible() {
        let context = egui::Context::default();
        let mut identity = None;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let response = messages_button(ui, tomlook::i18n::Language::En);
            identity = Some(response.id);
            assert!(response.rect.width() <= 32.0);
        });
        output.textures_delta.clear();
        context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
        let mut clicked = false;
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                let response = messages_button(ui, tomlook::i18n::Language::Cs);
                assert_eq!(response.id, identity.unwrap());
                clicked = response.clicked();
            },
        );
        output.textures_delta.clear();
        assert!(clicked);
    }

    fn append_shape_text(shape: &egui::epaint::Shape, output: &mut String) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    append_shape_text(shape, output);
                }
            }
            egui::epaint::Shape::Text(shape) => {
                output.push_str(shape.galley.text());
                output.push('\n');
            }
            _ => {}
        }
    }

    fn activity_fixture_job(id: &str, state: tomlook::scheduler::State) -> tomlook::scheduler::Job {
        tomlook::scheduler::Job {
            event_id: id.into(),
            fingerprint: "0".repeat(64),
            start: Local::now().fixed_offset(),
            foreground: false,
            manual: false,
            review: None,
            on_demand: false,
            promotion: 0,
            verified: true,
            state,
            message: "Fixture original-source read failed; explicitly review before retry.".into(),
            retries: 0,
            retry_at: None,
            prepared_at: None,
        }
    }

    #[test]
    fn activity_filters_find_problems_beyond_thousands_of_waiting_jobs_without_loss() {
        use tomlook::scheduler::State;
        let mut queue = tomlook::scheduler::Queue::default();
        for index in 0..5000 {
            let state = match index {
                4994 => State::Deferred,
                4995 => State::Running,
                4996 => State::Completed,
                4997 => State::Cancelled,
                4998 => State::Failed,
                4999 => State::Interrupted,
                _ => State::Queued,
            };
            let job = activity_fixture_job(&format!("fixture-{index:04}"), state);
            queue.jobs.insert(job.event_id.clone(), job);
        }
        let index = tomlook::worker::ActivityIndex::new(&queue);
        assert_eq!(index.len(ActivityFilter::All), 5000);
        assert_eq!(index.len(ActivityFilter::Waiting), 4995);
        assert_eq!(index.len(ActivityFilter::Working), 1);
        assert_eq!(index.len(ActivityFilter::Ready), 1);
        let problems = (0..index.len(ActivityFilter::Problems))
            .map(|row| index.job(&queue, ActivityFilter::Problems, row))
            .collect::<Vec<_>>();
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0].event_id, "fixture-4998");
        assert_eq!(problems[1].event_id, "fixture-4999");
        assert!(
            problems
                .iter()
                .all(|job| job.message.contains("explicitly review"))
        );
        assert_eq!(queue.jobs.len(), 5000);
        assert!(ActivityFilter::All.matches(State::Cancelled));
        assert!(!ActivityFilter::Problems.matches(State::Cancelled));
    }

    #[test]
    fn activity_rows_keep_fixed_height_and_stable_identity_with_long_multiline_titles() {
        let context = egui::Context::default();
        let job = activity_fixture_job("fixture-row", tomlook::scheduler::State::Failed);
        let other = activity_fixture_job("fixture-other", tomlook::scheduler::State::Queued);
        let title = format!(
            "{} \n {}",
            "A very long meeting title ".repeat(100),
            "\u{010d}esk\u{00e9} ".repeat(100)
        );
        let mut identity = None;
        for (index, width) in [280.0, 360.0, 440.0, 720.0].into_iter().enumerate() {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                if index % 2 == 1 {
                    activity_row(ui, &other, "Other row", true, tomlook::i18n::Language::En);
                }
                let row = ui.allocate_ui(Vec2::new(width, ACTIVITY_ROW_HEIGHT), |ui| {
                    activity_row(ui, &job, &title, true, tomlook::i18n::Language::En)
                });
                assert!(
                    row.response.rect.height() <= ACTIVITY_ROW_HEIGHT + 0.01,
                    "{:?}",
                    row.response.rect
                );
                assert!(
                    row.response.rect.width() <= width + 0.01,
                    "{:?}",
                    row.response.rect
                );
                assert!(row.inner.rect.height() <= ACTIVITY_ROW_HEIGHT + 0.01);
                if let Some(id) = identity {
                    assert_eq!(row.inner.id, id);
                }
                identity = Some(row.inner.id);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn opening_a_problem_from_keyboard_reads_detail_without_retrying_or_submitting_ai() {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (commands, storage_commands) = std::sync::mpsc::sync_channel(64);
        app.worker = Some(Worker {
            commands,
            notices: Arc::new(tomlook::worker::Notices::default()),
            activity: Arc::new(std::sync::Mutex::new(Arc::new(
                tomlook::worker::Activity::default(),
            ))),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            done: None,
        });
        app.calendar = Arc::new(
            Calendar::build(
                tomlook::calendar::fixture(1),
                tomlook::calendar::fixture_coverage(),
                Vec::new(),
            )
            .unwrap(),
        );
        let event = &app.calendar.events[0];
        let job = activity_fixture_job(&event.id, tomlook::scheduler::State::Failed);
        let title = event.title.clone();
        let context = egui::Context::default();
        let mut identity = None;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            identity = Some(activity_row(ui, &job, &title, true, tomlook::i18n::Language::En).id);
        });
        output.textures_delta.clear();
        context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                if activity_row(ui, &job, &title, true, tomlook::i18n::Language::En).clicked() {
                    app.select(0);
                }
            },
        );
        output.textures_delta.clear();
        assert_eq!(app.selected, Some(0));
        let Command::Detail { id, .. } = storage_commands.try_recv().unwrap() else {
            panic!("Only saved detail may be requested");
        };
        assert_eq!(id, job.event_id);
        assert!(storage_commands.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err());
    }

    #[test]
    fn problems_filter_renders_all_failed_targets_and_an_honest_empty_working_view() {
        use tomlook::scheduler::State;
        let (mut app, _, mut commands) = assistant_fixture();
        let mut events = tomlook::calendar::fixture(2);
        events[0].title = "Failed target fixture".into();
        events[1].title = "Interrupted target fixture".into();
        let ids = [events[0].id.clone(), events[1].id.clone()];
        app.calendar = Arc::new(
            Calendar::build(events, tomlook::calendar::fixture_coverage(), Vec::new()).unwrap(),
        );
        let mut queue = tomlook::scheduler::Queue::default();
        for index in 0..4998 {
            let job = activity_fixture_job(&format!("queued-{index}"), State::Queued);
            queue.jobs.insert(job.event_id.clone(), job);
        }
        for (id, state) in ids.iter().zip([State::Failed, State::Interrupted]) {
            queue
                .jobs
                .insert(id.clone(), activity_fixture_job(id, state));
        }
        app.activity = Arc::new(tomlook::worker::Activity {
            index: Arc::new(tomlook::worker::ActivityIndex::new(&queue)),
            queue,
            ..Default::default()
        });
        app.settings = true;
        app.activity_filter = ActivityFilter::Problems;
        let context = egui::Context::default();
        for (filter, expected) in [
            (ActivityFilter::Problems, "Failed target fixture"),
            (ActivityFilter::Working, "No matching jobs."),
        ] {
            app.activity_filter = filter;
            let mut sizing = context.run_ui(egui::RawInput::default(), |ui| app.overlays(ui.ctx()));
            sizing.textures_delta.clear();
            let mut output = context.run_ui(egui::RawInput::default(), |ui| app.overlays(ui.ctx()));
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            output.textures_delta.clear();
            assert!(text.contains(expected), "Missing {expected}: {text}");
            if filter == ActivityFilter::Problems {
                assert!(text.contains("Interrupted target fixture"));
                assert!(!text.contains("queued-"));
            }
        }
        assert_eq!(app.activity.queue.jobs.len(), 5000);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn insights_show_unmatched_citations_and_distinguish_search_from_original_reads() {
        let (mut app, _, _) = assistant_fixture();
        app.briefing = Some(serde_json::json!({
            "summary": "Fixture summary",
            "evidence_status": "1/3 cited links in captured results. Claims remain unverified.",
            "sources": [
                {"title":"Captured source", "url":"https://example.com/actual", "retrieved_records":[1]},
                {"title":"Uncaptured source", "url":"https://example.com/uncaptured", "retrieved_records":[]},
                {"title":"Legacy source", "url":"https://example.com/legacy"}
            ],
            "retrieved_evidence": {
                "omitted": 2,
                "records": [
                    {"ordinal":1, "tool":tomlook::workiq::connection::CONTEXT_TOOL, "succeeded":true},
                    {"ordinal":2, "tool":tomlook::workiq::connection::SDK_TOOL, "resource":"/me/messages/actual", "succeeded":true}
                ]
            }
        }));
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.render_insights(ui, true)
        });
        let mut rendered = String::new();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut rendered);
        }
        output.textures_delta.clear();
        for expected in [
            "Fixture summary",
            "Claims remain unverified.",
            "Referenced in captured results",
            "Not found in captured results",
            "Unverified citation",
            "R1: Workplace search summary",
            "R2: Message read",
            "2 additional tool results not retained.",
        ] {
            assert!(
                rendered.contains(expected),
                "Missing rendered text: {expected}; {rendered}"
            );
        }
        assert!(!rendered.contains("Open original source"));
        let mut compact = context.run_ui(egui::RawInput::default(), |ui| {
            app.render_insights(ui, false)
        });
        let mut rendered = String::new();
        for shape in &compact.shapes {
            append_shape_text(&shape.shape, &mut rendered);
        }
        compact.textures_delta.clear();
        assert!(rendered.contains("Fixture summary"));
        assert!(rendered.contains("Claims remain unverified."));
        assert!(!rendered.contains("Uncaptured source"));
    }

    #[test]
    fn escape_closes_expanded_insights_without_losing_selected_meeting() {
        let (mut app, _, _) = assistant_fixture();
        app.selected = Some(0);
        app.insights_expanded = true;
        let context = egui::Context::default();
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
        let mut output = context.run_ui(input, |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.insights_expanded);
        assert_eq!(app.selected, Some(0));
    }

    #[test]
    fn exact_palette_command_precedes_partial_word_matches() {
        let matches = command_matches(" Show WEEK ");
        assert!(matches!(matches[0], PaletteCommand::View(View::Week)));
        assert!(matches!(matches[1], PaletteCommand::View(View::WorkWeek)));
        assert!(matches!(
            command_matches("week")[0],
            PaletteCommand::View(View::WorkWeek)
        ));
    }

    #[test]
    fn all_theme_and_accent_choices_meet_text_and_focus_contrast_limits() {
        let luminance = |color: Color32| {
            let linear = |channel: u8| {
                let value = f64::from(channel) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        };
        let contrast = |foreground: Color32, background: Color32| {
            let foreground = luminance(foreground);
            let background = luminance(background);
            (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
        };
        let context = egui::Context::default();
        let mut app = Tomlook::initial(None, None, None);
        for dark in [false, true] {
            for accent in 0..ACCENTS.len() {
                app.preferences.dark = dark;
                app.preferences.accent = accent;
                app.style(&context);
                let style = context.style_of(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                let visuals = &style.visuals;
                for background in [visuals.panel_fill, visuals.window_fill] {
                    for category in [
                        "call",
                        "mimo office",
                        "mimo Prahu",
                        "nejisté",
                        "office",
                        "osobní",
                    ] {
                        assert!(
                            contrast(category_color(category, dark), background) >= 4.5,
                            "Category contrast: {category}, dark={dark}"
                        );
                        assert_eq!(
                            category_color(category, dark),
                            category_color(&category.to_uppercase(), dark)
                        );
                    }
                    assert!(
                        contrast(visuals.text_color(), background) >= 4.5,
                        "Body contrast: dark={dark}, accent={accent}"
                    );
                    assert!(
                        contrast(visuals.weak_text_color(), background) >= 4.5,
                        "Secondary text contrast: dark={dark}, accent={accent}"
                    );
                    assert!(
                        contrast(visuals.hyperlink_color, background) >= 4.5,
                        "Link contrast: dark={dark}, accent={accent}"
                    );
                    assert!(
                        contrast(visuals.selection.stroke.color, background) >= 3.0,
                        "Focus contrast: dark={dark}, accent={accent}"
                    );
                }
            }
        }
    }

    #[test]
    fn focus_window_icon_tracks_theme_and_accent_without_rebuilding_on_unchanged_frames() {
        let context = egui::Context::default();
        let mut app = Tomlook::initial(None, None, None);
        for dark in [false, true] {
            for accent in 0..ACCENTS.len() {
                app.preferences.dark = dark;
                app.preferences.accent = accent;
                let color = app.accent();
                let mut output = context.run_ui(Default::default(), |ui| {
                    app.sync_brand(ui.ctx());
                    app.header(ui);
                });
                output.textures_delta.clear();
                let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
                let icons: Vec<_> = commands
                    .iter()
                    .filter_map(|command| {
                        if let egui::ViewportCommand::Icon(Some(icon)) = command {
                            Some(icon)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(icons.len(), 1);
                assert_eq!((icons[0].width, icons[0].height), (64, 64));
                assert_eq!(
                    icons[0].rgba,
                    tomlook::brand::rgba_with_color(64, [color.r(), color.g(), color.b()])
                );
                let mut text = String::new();
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                assert!(text.contains("Tomlook"));
                assert!(!text.contains("O^O"));
                let mut output = context.run_ui(Default::default(), |ui| app.sync_brand(ui.ctx()));
                output.textures_delta.clear();
                assert!(
                    output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .iter()
                        .all(|command| !matches!(command, egui::ViewportCommand::Icon(_)))
                );
            }
        }
    }

    #[test]
    fn dense_lanes_remain_readable_and_keyboard_selection_reveals_hidden_cards() {
        let context = egui::Context::default();
        let mut app = Tomlook::initial(None, None, None);
        let mut events = tomlook::calendar::fixture(5000);
        for event in &mut events {
            event.title = format!("{} {}", event.id, event.title);
        }
        app.calendar = Arc::new(
            Calendar::build(events, tomlook::calendar::fixture_coverage(), Vec::new()).unwrap(),
        );
        app.anchor = tomlook::calendar::fixture_anchor();
        app.preferences.view = View::Day;
        let slots = &app.calendar.days[&app.anchor].timed;
        let width = timeline_content_width(240.0, slots);
        assert!(width > 240.0);
        assert!(
            slots
                .iter()
                .all(|slot| width / slot.lanes as f32 >= MIN_EVENT_LANE_WIDTH)
        );
        let selected = slots.iter().max_by_key(|slot| slot.lane).unwrap().event;
        let title = app.calendar.events[selected].title.clone();
        let draw = |app: &mut Tomlook, frame| {
            context.run_ui(
                egui::RawInput {
                    time: Some(frame as f64 * 0.1),
                    ..Default::default()
                },
                |ui| {
                    ui.set_max_size(Vec2::new(288.0, 640.0));
                    ui.set_clip_rect(egui::Rect::from_min_size(
                        ui.cursor().min,
                        Vec2::new(288.0, 640.0),
                    ));
                    app.timeline(ui);
                },
            )
        };
        let visible = |output: &egui::FullOutput| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::epaint::Shape::Text(text)
                if text.galley.job.text == title && shape.clip_rect.contains(text.pos))
            })
        };
        let mut output = draw(&mut app, 0);
        output.textures_delta.clear();
        assert!(
            !visible(&output),
            "high lane should initially require horizontal scrolling"
        );
        app.selected = Some(selected);
        app.focus_meeting = true;
        app.scroll_meeting = true;
        for frame in 1..10 {
            output = draw(&mut app, frame);
            output.textures_delta.clear();
        }
        assert!(
            visible(&output),
            "keyboard selection must reveal the high lane"
        );
        assert!(!app.focus_meeting);
        assert!(!app.scroll_meeting);
        assert!(context.memory(|memory| memory.focused()).is_some());
        app.query = title.clone();
        for frame in 10..20 {
            output = draw(&mut app, frame);
            output.textures_delta.clear();
        }
        assert!(
            visible(&output),
            "filtered lane must repack into the viewport"
        );
        assert_eq!(app.calendar.events.len(), 5000);
    }

    #[test]
    fn offscreen_meeting_cards_skip_widgets_but_keep_keyboard_reveal() {
        let context = egui::Context::default();
        let mut app = Tomlook::initial(None, None, None);
        app.calendar = Arc::new(
            Calendar::build(
                tomlook::calendar::fixture(200),
                tomlook::calendar::fixture_coverage(),
                Vec::new(),
            )
            .unwrap(),
        );
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 100_000.0), Vec2::new(180.0, 70.0));
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.event_card(ui, 0, rect);
        });
        output.textures_delta.clear();
        assert!(output.shapes.is_empty());
        app.selected = Some(0);
        app.focus_meeting = true;
        app.scroll_meeting = true;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.event_card(ui, 0, rect);
        });
        output.textures_delta.clear();
        assert!(!app.focus_meeting);
        assert!(!app.scroll_meeting);
        assert!(context.memory(|memory| memory.focused()).is_some());
    }

    #[test]
    fn custom_meeting_cards_do_not_layout_a_second_invisible_title() {
        let context = egui::Context::default();
        let mut app = Tomlook::initial(None, None, None);
        app.calendar = Arc::new(
            Calendar::build(
                tomlook::calendar::fixture(200),
                tomlook::calendar::fixture_coverage(),
                Vec::new(),
            )
            .unwrap(),
        );
        let title = app.calendar.events[0].title.clone();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(180.0, 70.0));
            app.event_card(ui, 0, rect);
        });
        output.textures_delta.clear();
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.iter().filter(|text| **text == title).count(), 1);
        assert!(
            !texts
                .iter()
                .any(|text| **text != title && text.contains(&title))
        );
    }

    #[test]
    fn ordinary_calendar_cards_show_only_their_time_range_not_a_conflict_caption() {
        let context = egui::Context::default();
        let mut app = Tomlook::initial(None, None, None);
        let mut events = tomlook::calendar::fixture(1);
        events[0].is_all_day = false;
        events[0].end = events[0].start + chrono::Duration::hours(1);
        events[0].status = "accepted".into();
        let expected = format!(
            "{} - {}",
            events[0].start.with_timezone(&Local).format("%H:%M"),
            events[0].end.with_timezone(&Local).format("%H:%M")
        );
        app.calendar = Arc::new(Calendar::build(events, Default::default(), vec![]).unwrap());
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            app.preferences.language = language;
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(320.0, 140.0));
                app.event_card(ui, 0, rect);
            });
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            assert!(text.contains(&expected), "{text}");
            assert!(!text.contains(language.text("Overlap").as_ref()), "{text}");
        }
    }

    #[test]
    fn closing_palette_releases_its_editor_focus() {
        let context = egui::Context::default();
        let mut app = Tomlook::initial(None, None, None);
        app.palette = true;
        context.memory_mut(|memory| memory.request_focus(Id::new("command-search")));
        app.close_palette(&context);
        assert!(!app.palette);
        assert_ne!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("command-search"))
        );
    }

    #[test]
    fn calendar_keyboard_commands_still_work_when_a_non_editor_control_has_focus() {
        let (mut app, _, _) = assistant_fixture();
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(Id::new("fixture-button")));
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::P,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::CTRL | Modifiers::SHIFT,
            }],
            ..Default::default()
        };
        let mut output = context.run_ui(input, |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(app.palette);
    }

    fn fixture_proposal(title: &str) -> Proposal {
        Proposal {
            kind: Kind::Email,
            target: "Unverified fixture message".into(),
            title: title.into(),
            body: "Fixture text, never sent".into(),
        }
    }

    #[test]
    fn invalid_typed_proposals_do_not_reach_the_native_panel() {
        let (mut app, output, _) = assistant_fixture();
        let mut answer = fixture_answer("Fixture");
        answer.proposals = Arc::new(vec![Proposal::default()]);
        app.active_request = Some(9);
        output
            .send(ai::Notice::Answer {
                id: 9,
                result: Ok(answer),
            })
            .unwrap();
        app.receive();
        assert!(app.answer.is_none());
        assert!(app.ai_error.as_ref().unwrap().contains("Title"));
    }

    fn fixture_answer(text: impl Into<String>) -> ai::Answer {
        ai::Answer::new(
            text.into(),
            Arc::new(tomlook::evidence::Snapshot::default()),
            Arc::new(Vec::new()),
        )
    }

    fn fixture_source_answer(scope: &str) -> ai::Answer {
        let recorder =
            tomlook::evidence::Recorder::new(Arc::new(tomlook::evidence::Redactor::default()));
        recorder.record(
            "WorkIQ-Mail-GetMessage",
            &serde_json::json!({
                "url":"https://example.com/fixture", "scope":scope
            }),
            true,
        );
        ai::Answer::new(
            scope.into(),
            recorder.snapshot().unwrap(),
            Arc::new(vec![fixture_proposal(scope)]),
        )
    }

    #[test]
    fn captured_evidence_stays_atomic_with_its_answer_across_scope_changes_and_late_results() {
        let (mut app, output, mut input) = assistant_fixture();
        let mut meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
        app.answer = Some(fixture_source_answer("global-fixture"));
        assert!(app.switch_assistant_context(Some(meeting.clone())));
        assert!(app.answer.is_none());
        app.answer = Some(fixture_source_answer("meeting-fixture"));
        assert!(app.switch_assistant_context(None));
        assert!(
            app.answer.as_ref().unwrap().evidence.records[0]
                .excerpt
                .contains("global-fixture")
        );
        app.question = "New global question".into();
        app.ask_question();
        let ai::Command::Chat(request) = input.try_recv().unwrap() else {
            panic!("Question not admitted")
        };
        let question = request.question;
        assert!(!app.switch_assistant_context(Some(meeting.clone())));
        app.cancel_ai();
        assert!(matches!(input.try_recv(), Ok(ai::Command::Cancel(id)) if id == question.id));
        assert!(app.switch_assistant_context(Some(meeting.clone())));
        output
            .send(ai::Notice::Answer {
                id: question.id,
                result: Ok(fixture_source_answer("late-global-fixture")),
            })
            .unwrap();
        app.receive();
        let answer = app.answer.as_ref().unwrap();
        assert_eq!(answer.text.as_str(), "meeting-fixture");
        assert_eq!(answer.proposals[0].title, "meeting-fixture");
        assert!(
            answer.evidence.records[0]
                .excerpt
                .contains("meeting-fixture")
        );
        assert!(
            !answer.evidence.records[0]
                .excerpt
                .contains("late-global-fixture")
        );
        meeting.title = "Changed fixture snapshot".into();
        assert!(app.switch_assistant_context(Some(meeting)));
        assert!(app.answer.is_none());
    }

    #[test]
    fn unsafe_captured_source_payload_cannot_reach_the_answer_inspector() {
        let (mut app, output, _) = assistant_fixture();
        let mut answer = fixture_source_answer("unsafe-fixture");
        Arc::make_mut(&mut answer.evidence).records[0]
            .links
            .push("javascript:alert(1)".into());
        app.active_request = Some(9);
        output
            .send(ai::Notice::Answer {
                id: 9,
                result: Ok(answer),
            })
            .unwrap();
        app.receive();
        assert!(app.answer.is_none());
        assert!(app.ai_error.as_ref().unwrap().contains("unsafe"));
    }

    #[test]
    fn workiq_diagnostics_do_not_change_copilot_connection_state() {
        let (mut app, output, _) = assistant_fixture();
        let connection = app.connection.clone();
        output
            .send(ai::Notice::Workiq {
                message: "Sign-in blocked: issuer mismatch".into(),
            })
            .unwrap();
        app.receive();
        assert!(app.workiq_status.contains("issuer mismatch"));
        assert!(app.ai_ready);
        assert_eq!(app.connection, connection);
    }

    #[test]
    fn hosted_workiq_progress_is_independent_of_assistant_and_calendar() {
        let (mut app, output, _) = assistant_fixture();
        let connection = app.connection.clone();
        let calendar = app.calendar.clone();
        output
            .send(ai::Notice::HostedWorkiq {
                busy: true,
                ready: false,
                message: "Waiting for browser sign-in".into(),
            })
            .unwrap();
        app.receive();
        assert!(app.hosted_workiq_busy);
        assert_eq!(app.hosted_workiq_status, "Waiting for browser sign-in");
        output
            .send(ai::Notice::HostedWorkiq {
                busy: false,
                ready: true,
                message: "WorkIQ read verified; tokens discarded".into(),
            })
            .unwrap();
        app.receive();
        assert!(!app.hosted_workiq_busy);
        assert!(app.ai_ready);
        assert_eq!(app.connection, connection);
        assert!(Arc::ptr_eq(&calendar, &app.calendar));
    }

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
            workiq_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stop,
        };
        let mut app = Tomlook::initial(None, Some(engine), None);
        app.ai_ready = true;
        (app, notices, input)
    }

    #[test]
    fn auto_authentication_has_separate_status_and_explicit_device_login_controls() {
        use tomlook::decision::{Phase, Status};
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            let (mut app, notices, mut commands) = assistant_fixture();
            app.preferences.language = language;
            app.connection = "Connected".into();
            let error = "Microsoft-Decision-1 needs a valid Azure CLI sign-in for the configured tenant and subscription; choose a mode explicitly";
            app.ai_error = Some(error.into());
            Arc::make_mut(&mut app.chat).admit(7, "Synthetic failed Auto question".into(), tomlook::chat::Mode::Auto).unwrap();
            Arc::make_mut(&mut app.chat).finish(7, Err(error.into()));
            notices.send(ai::Notice::Auto(Status::error(error.into()))).unwrap();
            app.receive();
            assert!(app.ai_ready, "Auto auth failure must not disconnect Copilot");
            assert_eq!(app.connection, "Connected");
            let context = egui::Context::default();
            app.apply_ask_input(app.ask_view().input(ask::Action::ConfigureAuto), &context);
            assert!(matches!(commands.try_recv().unwrap(), ai::Command::CheckAuto));
            assert!(commands.try_recv().is_err(), "Opening the dialog must not start login or repeat a question");
            notices.send(ai::Notice::Auto(Status::error(error.into()))).unwrap();
            app.receive();
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 1000.0))),
                    events, ..Default::default()
                }, |ui| app.connection_dialog(ui.ctx()));
                output.textures_delta.clear();
                output
            };
            let click = |app: &mut Tomlook, label: &str| {
                draw(app, vec![]);
                let output = draw(app, vec![]);
                let position = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                        Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                }).unwrap_or_else(|| panic!("Missing Auto connection control: {label}"));
                for pressed in [true, false] {
                    draw(app, vec![egui::Event::PointerMoved(position), egui::Event::PointerButton {
                        pos: position, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                    }]);
                }
            };
            draw(&mut app, vec![]);
            let output = draw(&mut app, vec![]);
            let mut text = String::new();
            for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
            assert!(text.contains("GitHub Copilot") && text.contains("Foundry · Microsoft-Decision-1"));
            assert_eq!(text.matches(language.text(error).as_ref()).count(), 1, "Do not repeat the routing error as a Copilot failure: {text}");
            click(&mut app, language.text("Sign in to Azure (device code)").as_ref());
            assert!(matches!(commands.try_recv().unwrap(), ai::Command::SignInAuto));
            assert!(commands.try_recv().is_err());
            let output = draw(&mut app, vec![]);
            text.clear();
            for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
            assert!(text.contains(language.text("Requesting an Azure device code (up to 60 seconds)...").as_ref()));
            assert!(text.contains(language.text("Open Microsoft device sign-in").as_ref()));
            assert!(text.contains(language.text("Cancel Azure sign-in").as_ref()));
            assert!(output.platform_output.commands.is_empty());
            let timeout = "Azure CLI did not provide a device code within 60 seconds. Check connectivity and try signing in again.";
            notices.send(ai::Notice::Auto(Status::error(timeout.into()))).unwrap();
            app.receive();
            let output = draw(&mut app, vec![]);
            text.clear();
            for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
            assert!(text.contains(language.text(timeout).as_ref()));
            assert!(app.ai_ready && !app.auto_status.busy());
            click(&mut app, language.text("Sign in to Azure (device code)").as_ref());
            assert!(matches!(commands.try_recv().unwrap(), ai::Command::SignInAuto));
            notices.send(ai::Notice::Auto(Status {
                phase: Phase::SigningIn,
                message: "Enter this code on Microsoft's device sign-in page.".into(),
                device_code: Some("ABCD-EF12".into()),
            })).unwrap();
            app.receive();
            let output = draw(&mut app, vec![]);
            text.clear();
            for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
            assert!(text.contains("ABCD-EF12"));
            assert!(text.contains(language.text("Open Microsoft device sign-in").as_ref()));
            assert!(output.platform_output.commands.is_empty(), "Rendering a code must not open a browser or copy it automatically");
            click(&mut app, language.text("Cancel Azure sign-in").as_ref());
            assert!(matches!(commands.try_recv().unwrap(), ai::Command::CancelAutoLogin));
            notices.send(ai::Notice::Auto(Status::signed_in())).unwrap();
            app.receive();
            assert!(app.ai_ready && app.auto_status.phase == Phase::SignedIn);
            assert_eq!(app.chat.turns[0].error.as_deref(), Some(error));
            assert!(commands.try_recv().is_err(), "Successful login must not repeat a failed question");
        }
    }

    #[test]
    fn concurrent_chat_results_keep_their_turn_and_complete_history() {
        let (mut app, notices, mut commands) = assistant_fixture();
        app.ask_mode = tomlook::chat::Mode::Deep;
        app.question = "Research synthetic preparation".into();
        app.ask_question();
        let ai::Command::Chat(deep) = commands.try_recv().unwrap() else {
            panic!("Missing deep request");
        };
        assert!(app.question.is_empty());
        notices
            .send(ai::Notice::ChatProgress {
                id: deep.question.id,
                progress: tomlook::chat::Progress {
                    mode: tomlook::chat::Mode::Deep,
                    stage: "Searching local materials".into(),
                    ..Default::default()
                },
            })
            .unwrap();
        app.receive();
        app.ask_mode = tomlook::chat::Mode::Fast;
        app.question = "A quick synthetic question".into();
        app.ask_question();
        let ai::Command::Chat(fast) = commands.try_recv().unwrap() else {
            panic!("Missing quick request");
        };
        assert!(
            fast.history.is_empty(),
            "Unfinished research is not completed conversation"
        );
        notices
            .send(ai::Notice::Answer {
                id: fast.question.id,
                result: Ok(fixture_answer("Quick answer")),
            })
            .unwrap();
        app.receive();
        assert!(app.chat.turns[0].running);
        assert_eq!(
            app.chat.turns[1].answer.as_ref().unwrap().text.as_str(),
            "Quick answer"
        );
        notices
            .send(ai::Notice::Answer {
                id: deep.question.id,
                result: Ok(fixture_answer("Deep findings")),
            })
            .unwrap();
        app.receive();
        assert!(!app.chat.running());
        app.question = "Use those findings".into();
        app.ask_question();
        let ai::Command::Chat(follow_up) = commands.try_recv().unwrap() else {
            panic!("Missing follow-up");
        };
        assert_eq!(follow_up.history.len(), 2);
        assert_eq!(follow_up.history[0].answer, "Deep findings");
        assert_eq!(follow_up.history[1].answer, "Quick answer");
    }

    #[test]
    fn chat_meeting_links_open_insights_and_preserve_drafts_and_running_work() {
        let (mut app, _, mut commands) = assistant_fixture();
        let event = tomlook::calendar::fixture(1).remove(0);
        app.calendar =
            Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
        app.preferences.view = View::Week;
        app.assistant = true;
        app.focus_assistant = true;
        app.question = "Unsent follow-up".into();
        Arc::make_mut(&mut app.chat)
            .admit(100, "Running research".into(), tomlook::chat::Mode::Deep)
            .unwrap();
        let chat = app.chat.clone();
        let context = egui::Context::default();
        let input = app
            .ask_view()
            .input(ask::Action::OpenMeeting(event.id.clone()));
        app.apply_ask_input(input, &context);
        assert_eq!(app.selected, Some(0));
        assert!(app.assistant);
        assert!(app.insights_expanded);
        assert_eq!(app.question, "Unsent follow-up");
        assert!(Arc::ptr_eq(&chat, &app.chat));
        assert!(app.chat.running());
        assert!(commands.try_recv().is_err());

        app.assistant = true;
        app.assistant_detached = true;
        let input = app.ask_view().input(ask::Action::OpenMeeting(event.id));
        app.apply_ask_input(input, &context);
        assert!(app.assistant, "A separate Ask window stays open");
        assert_eq!(app.question, "Unsent follow-up");
        assert!(app.chat.running());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn local_slash_is_admitted_offline_during_research_and_preserves_rejected_drafts() {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (commands, worker_commands) = mpsc::sync_channel(1);
        let notices = Arc::new(tomlook::worker::Notices::default());
        app.worker = Some(Worker {
            commands,
            notices: notices.clone(),
            activity: Default::default(),
            stop: Default::default(),
            done: None,
        });
        app.ai_ready = false;
        Arc::make_mut(&mut app.chat)
            .admit(
                90,
                "Running synthetic research".into(),
                tomlook::chat::Mode::Deep,
            )
            .unwrap();
        app.question = "/search Northstar".into();
        app.ask_question();
        let Command::AskLocal { id, query, .. } = worker_commands.try_recv().unwrap() else {
            panic!("Not a local query");
        };
        assert_eq!(
            query,
            tomlook::chat_cache::LocalQuery::Search("Northstar".into())
        );
        assert!(app.question.is_empty());
        assert!(ai_commands.try_recv().is_err());
        Arc::make_mut(&mut app.chat).finish(id, Ok(fixture_answer("Saved match")));
        assert!(app.chat.turns[0].running);
        assert!(!app.chat.turns[1].running);
        app.worker
            .as_ref()
            .unwrap()
            .commands
            .try_send(Command::Wake)
            .unwrap();
        app.question = "/today".into();
        app.ask_question();
        assert_eq!(
            app.question, "/today",
            "Rejected local draft must not disappear"
        );
        assert_eq!(app.chat.turns.len(), 2);
        assert!(app.ai_error.as_ref().unwrap().contains("queue is full"));
    }

    #[test]
    fn review_slashes_enqueue_typed_background_work_without_an_ask_or_calendar_write() {
        use tomlook::recommendations::Review;
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (commands, worker_commands) = mpsc::sync_channel(1);
        app.worker = Some(Worker {
            commands,
            notices: Arc::new(Default::default()),
            activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            done: None,
        });
        app.calendar_owner = Some("owner".into());
        for (question, expected) in [
            ("/travel", Review::Travel),
            ("/categories", Review::Categories),
        ] {
            app.question = question.into();
            app.ask_question();
            let Command::ReviewRecommendations { kind, account } =
                worker_commands.try_recv().unwrap()
            else {
                panic!("Review did not enter the storage queue");
            };
            assert_eq!(kind, expected);
            assert_eq!(account, "owner");
            assert!(app.active_request.is_none());
            assert!(app.answer.as_ref().unwrap().text.contains("14"));
            assert!(ai_commands.try_recv().is_err());
        }
        app.question = "/travel".into();
        app.ask_question();
        app.question = "/travel".into();
        app.ask_question();
        assert!(app.ai_error.as_ref().unwrap().contains("queue is full"));
        assert!(ai_commands.try_recv().is_err());
    }

    #[test]
    fn external_connection_dialog_masks_keys_dispatches_once_and_requires_forget_confirmation() {
        let (mut app, notices, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1366.0, 768.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.shortcuts(ui.ctx());
                    app.connection_dialog(ui.ctx());
                },
            );
            output.textures_delta.clear();
            output
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, Vec::new());
            let output = draw(app, Vec::new());
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing button: {label}"));
            draw(
                app,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        };
        app.connection_panel = Some(ConnectionPanel::External(tomlook::connectors::Kind::Webiq));
        app.webiq_key = "synthetic-never-rendered-key".into();
        draw(&mut app, Vec::new());
        let output = draw(&mut app, Vec::new());
        let mut text = String::new();
        for shape in output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        assert!(!text.contains("synthetic-never-rendered-key"));
        click(&mut app, "Connect and save key");
        assert!(
            matches!(commands.try_recv(), Ok(ai::Command::ConnectExternal { kind: tomlook::connectors::Kind::Webiq, input: tomlook::connectors::Input::WebiqKey(input) }) if input == "synthetic-never-rendered-key")
        );
        assert!(app.webiq_key.is_empty());
        assert!(app.external_status[&tomlook::connectors::Kind::Webiq].busy);
        assert!(commands.try_recv().is_err());
        notices
            .send(ai::Notice::External {
                kind: tomlook::connectors::Kind::Webiq,
                busy: false,
                ready: true,
                message: "Connected".into(),
                client_id: None,
            })
            .unwrap();
        app.receive();
        assert!(app.external_status[&tomlook::connectors::Kind::Webiq].ready);
        app.question = "Keep my unsent question".into();
        app.answer = Some(fixture_answer("Previous connected answer"));
        app.assistant_states.insert(
            Some("cached-meeting".into()),
            AssistantState {
                question: "Keep the cached question".into(),
                answer: Some(fixture_answer("Cached connected answer")),
                ..Default::default()
            },
        );
        click(&mut app, "Disconnect and forget");
        assert_eq!(app.forget_connector, Some(tomlook::connectors::Kind::Webiq));
        assert!(
            commands.try_recv().is_err(),
            "First click must not delete a saved sign-in"
        );
        assert!(
            app.answer.is_some(),
            "The first confirmation must not clear answers"
        );
        click(&mut app, "Remove saved sign-in");
        assert!(matches!(
            commands.try_recv(),
            Ok(ai::Command::ForgetExternal(
                tomlook::connectors::Kind::Webiq
            ))
        ));
        assert!(app.forget_connector.is_none());
        assert!(app.answer.is_none());
        assert!(
            app.assistant_states
                .values()
                .all(|state| state.answer.is_none())
        );
        assert_eq!(app.question, "Keep my unsent question");
        assert_eq!(
            app.assistant_states[&Some("cached-meeting".into())].question,
            "Keep the cached question"
        );
        app.connection_panel = Some(ConnectionPanel::External(tomlook::connectors::Kind::Github));
        app.github_client_id = " Iv23fixture ".into();
        click(&mut app, "Sign in");
        assert!(
            matches!(commands.try_recv(), Ok(ai::Command::ConnectExternal { kind: tomlook::connectors::Kind::Github, input: tomlook::connectors::Input::GithubApp(input) }) if input == "Iv23fixture")
        );
        assert!(commands.try_recv().is_err());
        assert_eq!(app.preferences.github_client_id, "Iv23fixture");
        click(&mut app, "Cancel");
        assert!(matches!(
            commands.try_recv(),
            Ok(ai::Command::CancelExternal(
                tomlook::connectors::Kind::Github
            ))
        ));
        app.webiq_key = "clear-on-dialog-close".into();
        draw(
            &mut app,
            vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert!(app.connection_panel.is_none());
        assert!(app.webiq_key.is_empty());
        assert_eq!(app.github_client_id.trim(), "Iv23fixture");
    }

    #[test]
    fn external_reconnection_invalidates_answers_but_preserves_unsent_questions() {
        let (mut app, notices, mut commands) = assistant_fixture();
        app.external_status.insert(
            tomlook::connectors::Kind::Github,
            ExternalStatus {
                ready: true,
                ..Default::default()
            },
        );
        app.active_request = Some(17);
        app.question = "Keep my draft".into();
        app.answer = Some(fixture_answer("Previous account answer"));
        app.assistant_states.insert(
            None,
            AssistantState {
                question: "Global draft".into(),
                answer: Some(fixture_answer("Old global answer")),
                ..Default::default()
            },
        );
        notices
            .send(ai::Notice::External {
                kind: tomlook::connectors::Kind::Github,
                busy: false,
                ready: true,
                message: "Connected as new-fixture-owner".into(),
                client_id: Some("Iv23fixture".into()),
            })
            .unwrap();
        app.receive();
        assert!(matches!(commands.try_recv(), Ok(ai::Command::Cancel(17))));
        assert!(app.active_request.is_none());
        assert!(app.answer.is_none());
        assert!(app.assistant_states[&None].answer.is_none());
        assert_eq!(app.question, "Keep my draft");
        assert_eq!(app.assistant_states[&None].question, "Global draft");
    }

    #[test]
    fn slow_progress_and_burst_notifications_leave_text_navigation_and_cancel_responsive() {
        let (mut app, output, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        let (storage_commands, _storage_input) = std::sync::mpsc::sync_channel(64);
        app.worker = Some(Worker {
            commands: storage_commands,
            notices: Arc::new(tomlook::worker::Notices::default()),
            activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            done: None,
        });
        app.calendar = Arc::new(
            Calendar::build(
                tomlook::calendar::fixture(5000),
                tomlook::calendar::fixture_coverage(),
                Vec::new(),
            )
            .unwrap(),
        );
        app.anchor = tomlook::calendar::fixture_anchor();
        app.loading = false;
        app.assistant = true;
        app.focus_assistant = true;
        app.active_request = Some(7);
        let progress = app.ai.as_ref().unwrap().progress.clone();
        app.ai
            .as_ref()
            .unwrap()
            .occupied
            .store(2, std::sync::atomic::Ordering::Release);
        let (locked, lock_ready) = std::sync::mpsc::sync_channel(1);
        let (release, released) = std::sync::mpsc::sync_channel(1);
        let (finished, worker_done) = std::sync::mpsc::sync_channel(1);
        let provider = std::thread::spawn(move || {
            let mut value = progress.lock().unwrap();
            value.id = 7;
            value.text = Arc::new("Delayed synthetic provider progress".into());
            locked.send(()).unwrap();
            let released_early = released
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok();
            drop(value);
            finished.send(released_early).unwrap();
        });
        lock_ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        for index in 0..200 {
            output
                .send(ai::Notice::Sources(format!("Synthetic update {index}")))
                .unwrap();
        }
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1366.0, 768.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.receive();
                    app.style(ui.ctx());
                    app.shortcuts(ui.ctx());
                    egui::Panel::top("test-header").show(ui, |ui| app.header(ui));
                    if app.assistant {
                        egui::Panel::right("test-assistant")
                            .exact_size(380.0)
                            .show(ui, |ui| app.assistant_panel(ui));
                    }
                    egui::CentralPanel::default().show(ui, |ui| app.timeline(ui));
                },
            );
            output.textures_delta.clear();
        };
        draw(&mut app, Vec::new());
        draw(&mut app, vec![egui::Event::Text("Normal editing".into())]);
        assert_eq!(app.question, "Normal editing");
        assert_eq!(app.active_request, Some(7));
        assert_eq!(app.source_status, "Synthetic update 199");
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        draw(&mut app, vec![key(Key::F, Modifiers::CTRL)]);
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("calendar-search"))
        );
        draw(&mut app, vec![egui::Event::Text("Synthetic".into())]);
        assert_eq!(app.query, "Synthetic");
        draw(&mut app, vec![key(Key::ArrowRight, Modifiers::ALT)]);
        assert_ne!(app.anchor, tomlook::calendar::fixture_anchor());
        draw(&mut app, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(!app.assistant);
        assert_eq!(app.active_request, Some(7));
        assert!(commands.try_recv().is_err());
        app.cancel_ai();
        assert!(matches!(commands.try_recv(), Ok(ai::Command::Cancel(7))));
        assert!(commands.try_recv().is_err());
        assert_eq!(app.calendar.events.len(), 5000);
        assert!(
            worker_done.try_recv().is_err(),
            "The UI waited for the slow progress lock instead of reading nonblockingly"
        );
        release.send(()).unwrap();
        assert!(
            worker_done
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
        );
        provider.join().unwrap();
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
            workiq_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
                result: Ok(fixture_answer("Old meeting")),
            })
            .unwrap();
        app.receive();
        assert!(app.answer.is_none());
        assert_eq!(app.active_request, Some(2));
        output
            .send(ai::Notice::Answer {
                id: 2,
                result: Ok(fixture_answer("Current request")),
            })
            .unwrap();
        app.receive();
        assert_eq!(
            app.answer.as_ref().map(|answer| answer.text.as_str()),
            Some("Current request")
        );
        assert!(app.active_request.is_none());
    }

    #[test]
    fn global_and_meeting_drafts_topics_and_last_answers_do_not_leak() {
        let (mut app, _, _input) = assistant_fixture();
        let meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
        app.question = "Global question".into();
        app.answer = Some(fixture_answer("Global answer"));
        assert!(app.switch_assistant_context(Some(meeting.clone())));
        assert!(app.question.is_empty());
        assert!(app.answer.is_none());
        app.question = "Meeting question".into();
        app.answer = Some(fixture_answer("Meeting answer"));
        assert!(app.switch_assistant_context(None));
        assert_eq!(app.question, "Global question");
        assert_eq!(app.answer.as_ref().unwrap().text.as_str(), "Global answer");
        assert!(app.switch_assistant_context(Some(meeting)));
        assert_eq!(app.question, "Meeting question");
        assert_eq!(app.answer.as_ref().unwrap().text.as_str(), "Meeting answer");
    }

    #[test]
    fn busy_context_is_frozen_and_its_answer_stays_in_its_original_scope() {
        let (mut app, output, mut input) = assistant_fixture();
        app.question = "Global question".into();
        app.ask_question();
        let Some(ai::Command::Chat(request)) = input.try_recv().ok() else {
            panic!("question was not admitted");
        };
        let question = request.question;
        let meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
        assert!(!app.switch_assistant_context(Some(meeting.clone())));
        assert!(input.try_recv().is_err());
        assert!(app.assistant_context.is_none());
        assert_eq!(app.active_request, Some(question.id));
        output
            .send(ai::Notice::Answer {
                id: question.id,
                result: Ok(fixture_answer("Late global answer")),
            })
            .unwrap();
        app.receive();
        assert_eq!(
            app.answer.as_ref().unwrap().text.as_str(),
            "Late global answer"
        );
        assert!(app.active_request.is_none());
        assert!(app.switch_assistant_context(Some(meeting)));
        assert!(app.answer.is_none());
        assert!(app.switch_assistant_context(None));
        assert!(app.question.is_empty());
        assert_eq!(app.chat.turns.back().unwrap().question, "Global question");
        assert_eq!(
            app.answer.as_ref().unwrap().text.as_str(),
            "Late global answer"
        );
    }

    #[test]
    fn ask_presentation_changes_keep_one_request_and_hidden_completion() {
        let (mut app, notices, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        app.assistant = true;
        app.question = "Summarize today".into();
        app.ask_question();
        let ai::Command::Chat(request) = commands.try_recv().unwrap() else {
            panic!("Question not admitted");
        };
        let question = request.question;
        assert!(!question.suggest_drafts);
        assert!(!question.question.starts_with('/'));
        for action in [ask::Action::Detach, ask::Action::Dock, ask::Action::Hide] {
            app.apply_ask_input(app.ask_view().input(action), &context);
            assert_eq!(app.active_request, Some(question.id));
            assert!(app.question.is_empty());
            assert_eq!(app.chat.turns.back().unwrap().question, "Summarize today");
            assert!(commands.try_recv().is_err());
        }
        notices
            .send(ai::Notice::Answer {
                id: question.id + 1,
                result: Ok(fixture_answer("Stale result")),
            })
            .unwrap();
        app.receive();
        assert!(!app.ask_unread);
        notices
            .send(ai::Notice::Answer {
                id: question.id,
                result: Ok(fixture_answer("Finished while hidden")),
            })
            .unwrap();
        app.receive();
        assert!(app.ask_unread);
        assert!(!app.messages_open);
        app.toggle_assistant(&context);
        assert!(!app.ask_unread);
        assert_eq!(
            app.answer.as_ref().unwrap().text.as_str(),
            "Finished while hidden"
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn detached_edits_survive_parent_updates_and_transfer_on_docking() {
        let (mut app, notices, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        app.assistant = true;
        app.assistant_detached = true;
        app.question = "Original question".into();
        app.sync_detached_ask(&context);
        app.detached_ask.lock().unwrap().view.question = "Edited in the native window".into();
        notices
            .send(ai::Notice::Sources("Unrelated background update".into()))
            .unwrap();
        app.receive();
        app.sync_detached_ask(&context);
        let input = app
            .detached_ask
            .lock()
            .unwrap()
            .view
            .input(ask::Action::Dock);
        assert_eq!(input.question, "Edited in the native window");
        app.ask_output.try_send(input).unwrap();
        app.receive_ask_inputs(&context);
        assert!(!app.assistant_detached);
        assert_eq!(app.question, "Edited in the native window");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn calendar_selection_does_not_retarget_an_active_contextual_question() {
        let (mut app, _, mut commands) = assistant_fixture();
        app.calendar = Arc::new(
            Calendar::build(tomlook::calendar::fixture(2), Default::default(), vec![]).unwrap(),
        );
        let original = app.calendar.events[0].clone();
        assert!(app.switch_assistant_context(Some(original.clone())));
        app.question = "/prepare".into();
        app.ask_question();
        let ai::Command::Chat(request) = commands.try_recv().unwrap() else {
            panic!("Question not admitted");
        };
        let question = request.question;
        app.select(1);
        assert_eq!(app.assistant_context.as_ref().unwrap().id, original.id);
        assert_eq!(question.meeting.as_ref().unwrap().id, original.id);
        assert_eq!(app.active_request, Some(question.id));
        assert!(commands.try_recv().is_err());
        app.apply_ask_input(
            app.ask_view().input(ask::Action::Stop),
            &egui::Context::default(),
        );
        assert!(matches!(commands.try_recv(), Ok(ai::Command::Cancel(id)) if id == question.id));
        assert!(app.active_request.is_none());
    }

    #[test]
    fn detached_submission_rejects_a_changed_meeting_snapshot() {
        let (mut app, _, mut commands) = assistant_fixture();
        let mut event = tomlook::calendar::fixture(1).remove(0);
        app.switch_assistant_context(Some(event.clone()));
        app.question = "Review this meeting".into();
        let stale = app.ask_view().input(ask::Action::Submit);
        event.title.push_str(" changed");
        app.switch_assistant_context(Some(event));
        app.apply_ask_input(stale, &egui::Context::default());
        assert!(commands.try_recv().is_err());
        assert!(app.ai_error.as_ref().unwrap().contains("context changed"));
    }

    #[test]
    fn detached_scope_changes_save_unsent_edits_and_do_not_leak_between_empty_contexts() {
        let (mut app, _, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        app.assistant_detached = true;
        app.sync_detached_ask(&context);
        app.detached_ask.lock().unwrap().view.question = "Unsent global question".into();
        let event = tomlook::calendar::fixture(1).remove(0);
        assert!(app.switch_assistant_context(Some(event)));
        app.sync_detached_ask(&context);
        assert!(app.detached_ask.lock().unwrap().view.question.is_empty());
        assert_eq!(
            app.assistant_states[&None].question,
            "Unsent global question"
        );
        assert!(app.switch_assistant_context(None));
        app.sync_detached_ask(&context);
        assert_eq!(
            app.detached_ask.lock().unwrap().view.question,
            "Unsent global question"
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn same_context_reopen_preserves_request_but_changed_snapshot_invalidates_answer() {
        let (mut app, _, mut input) = assistant_fixture();
        let mut meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
        app.switch_assistant_context(Some(meeting.clone()));
        app.question = "Question".into();
        app.ask_question();
        assert!(matches!(input.try_recv().unwrap(), ai::Command::Chat(_)));
        let active = app.active_request;
        assert!(app.switch_assistant_context(Some(meeting.clone())));
        assert_eq!(app.active_request, active);
        assert!(input.try_recv().is_err());
        app.cancel_ai();
        app.question = "New unsent draft".into();
        app.answer = Some(fixture_answer("Previous answer"));
        app.switch_assistant_context(None);
        meeting.title = "Updated meeting".into();
        app.switch_assistant_context(Some(meeting));
        assert!(app.answer.is_none());
        assert_eq!(app.question, "New unsent draft");
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
        assert!(matches!(input.try_recv().unwrap(), ai::Command::Chat(_)));
        assert!(input.try_recv().is_err());
        app.cancel_ai();
        assert!(matches!(input.try_recv().unwrap(), ai::Command::Cancel(_)));
        app.answer = Some(fixture_answer("Last completed answer"));
        drop(input);
        app.question = "Retry after worker failure".into();
        app.ask_question();
        assert!(app.active_request.is_none());
        assert_eq!(
            app.answer.as_ref().unwrap().text.as_str(),
            "Last completed answer"
        );
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
                result: Ok(fixture_answer("x".repeat(64_001))),
            })
            .unwrap();
        app.receive();
        assert!(app.answer.is_none());
        assert!(app.active_request.is_none());
        assert!(app.ai_error.as_ref().unwrap().contains("response limit"));
    }

    #[test]
    fn workiq_identity_changes_invalidate_answers_without_erasing_local_questions() {
        let (mut app, _, _input) = assistant_fixture();
        app.question = "Current local question".into();
        app.answer = Some(fixture_answer("Current old-account result"));
        app.assistant_states.insert(
            Some("fixture-other".into()),
            AssistantState {
                question: "Other local question".into(),
                answer: Some(fixture_answer("Other old-account result")),
                ..Default::default()
            },
        );
        app.invalidate_connection_answers();
        assert!(app.answer.is_none());
        let other = &app.assistant_states[&Some("fixture-other".into())];
        assert!(other.answer.is_none());
        assert_eq!(other.question, "Other local question");
        assert_eq!(app.question, "Current local question");
    }

    #[test]
    fn connection_dialog_does_not_apply_calendar_shortcuts_and_escape_closes_it() {
        let (mut app, _, _input) = assistant_fixture();
        let context = egui::Context::default();
        app.preferences.view = View::WorkWeek;
        app.connection_panel = Some(ConnectionPanel::Workiq);
        let key_input = |key| egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output = context.run_ui(key_input(Key::Num1), |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(app.preferences.view, View::WorkWeek);
        assert!(app.connection_panel.is_some());
        let mut output = context.run_ui(key_input(Key::Escape), |ui| {
            app.shortcuts(ui.ctx());
            app.header(ui);
        });
        output.textures_delta.clear();
        assert!(app.connection_panel.is_none());
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("calendar-search"))
        );
    }

    #[test]
    fn simple_question_panel_renders_markdown_without_concept_controls() {
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            let (mut app, _, mut commands) = assistant_fixture();
            app.preferences.language = language;
            app.assistant = true;
            app.focus_assistant = true;
            app.answer = Some(fixture_answer(
                "# Overview\n\n**Important** and *useful*.\n\n- First step\n- Second step\n\n[Source](https://example.com)\n\n```text\nsample code\n```",
            ));
            let context = egui::Context::default();
            let mut rendered = String::new();
            let mut heading_size = 0.0_f32;
            let mut body_size = 0.0_f32;
            for _ in 0..3 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(1366.0, 768.0),
                        )),
                        ..Default::default()
                    },
                    |ui| app.assistant_panel(ui),
                );
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut rendered);
                    if let egui::epaint::Shape::Text(text) = &shape.shape {
                        if text.galley.job.text == "Overview" {
                            heading_size = text.galley.job.sections[0].format.font_id.size;
                        }
                        if text.galley.job.text == "Important and useful." {
                            body_size = text.galley.job.sections[0].format.font_id.size;
                        }
                    }
                }
                output.textures_delta.clear();
            }
            assert!(
                rendered.contains(language.text("Ask AI").as_ref()),
                "{rendered}"
            );
            assert!(rendered.contains("Important and useful."), "{rendered}");
            assert!(rendered.contains("First step") && rendered.contains("sample code"));
            assert!(!rendered.contains("**Important**") && !rendered.contains("```"));
            assert!(
                !rendered.contains(language.text("Suggest drafts").as_ref()),
                "{rendered}"
            );
            assert!(
                !rendered.contains(language.text("New local draft").as_ref()),
                "{rendered}"
            );
            assert!(
                !rendered.contains(language.text("More options").as_ref()),
                "{rendered}"
            );
            assert!(heading_size > body_size && body_size > 0.0);
            assert!(
                commands.try_recv().is_err(),
                "Opening the panel must not ask AI"
            );
            assert_eq!(
                context.memory(|memory| memory.focused()),
                Some(Id::new("assistant-question"))
            );
        }
    }

    #[test]
    fn waiting_panel_does_not_expose_raw_partial_markdown_or_block_on_progress() {
        let (mut app, _, mut commands) = assistant_fixture();
        app.assistant = true;
        app.active_request = Some(7);
        let shared_progress = app.ai.as_ref().unwrap().progress.clone();
        let mut progress = shared_progress.lock().unwrap();
        progress.id = 7;
        progress.text = Arc::new("**Unfinished raw Markdown**".into());
        let context = egui::Context::default();
        let mut rendered = String::new();
        for _ in 0..3 {
            let mut output =
                context.run_ui(egui::RawInput::default(), |ui| app.assistant_panel(ui));
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut rendered);
            }
            output.textures_delta.clear();
        }
        assert!(rendered.contains(app.preferences.language.text("Thinking...").as_ref()));
        assert!(!rendered.contains("Unfinished raw Markdown"));
        assert!(commands.try_recv().is_err());
        drop(progress);
    }

    #[test]
    fn opening_assistant_focuses_question_and_escape_returns_focus_to_search() {
        let (mut app, _, _input) = assistant_fixture();
        let context = egui::Context::default();
        app.assistant = true;
        app.focus_assistant = true;
        app.question = "Saved ordinary question".into();
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
    fn enter_from_question_submits_once() {
        let (mut app, _, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        app.question = "Synthetic question".into();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.assistant_panel(ui);
        });
        output.textures_delta.clear();
        context.memory_mut(|memory| memory.request_focus(Id::new("assistant-question")));
        for _ in 0..2 {
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                app.assistant_panel(ui);
            });
            output.textures_delta.clear();
        }
        let ai::Command::Chat(request) = commands.try_recv().unwrap() else {
            panic!("expected one question");
        };
        let question = request.question;
        assert!(question.public_topic.is_none());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn shift_enter_adds_a_line_without_submitting() {
        let (mut app, _, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        app.question = "Fixture".into();
        context.memory_mut(|memory| memory.request_focus(Id::new("assistant-question")));
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::SHIFT,
                }],
                ..Default::default()
            },
            |ui| app.assistant_panel(ui),
        );
        output.textures_delta.clear();
        assert!(app.question.contains('\n'));
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn command_search_and_arrows_execute_the_filtered_command() {
        assert_eq!(command_matches("show month").len(), 1);
        let (mut app, _, _) = assistant_fixture();
        let context = egui::Context::default();
        app.preferences.view = View::Day;
        app.open_palette();
        app.palette_query = "show".into();
        for key in [Key::ArrowDown, Key::Enter] {
            let mut output = context.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| app.overlays(ui.ctx()),
            );
            output.textures_delta.clear();
        }
        assert!(!app.palette);
        assert_eq!(app.preferences.view, View::WorkWeek);
    }

    #[test]
    fn mini_calendar_pointer_selection_opens_day_or_exact_inclusive_columns() {
        for (start, end, count, focused) in [
            (6, 6, 1, true),
            (6, 8, 3, true),
            (8, 6, 3, true),
            (9, 12, 4, true),
            (6, 8, 3, false),
            (8, 6, 3, false),
        ] {
            let (mut app, _, mut commands) = assistant_fixture();
            app.anchor = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
            app.preferences.view = View::Month;
            let context = egui::Context::default();
            let draw = |app: &mut Tomlook, raw| {
                let mut output = context.run_ui(raw, |ui| {
                    ui.set_width(192.0);
                    app.sidebar(ui);
                });
                output.textures_delta.clear();
            };
            draw(&mut app, egui::RawInput::default());
            let date = |day| NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
            let position = |day| {
                context.data(|data| {
                    data.get_temp::<egui::Rect>(Id::new(("mini-date", date(day))))
                        .unwrap()
                        .center()
                })
            };
            let first = position(start);
            let last = position(end);
            draw(
                &mut app,
                egui::RawInput {
                    time: Some(0.1),
                    events: vec![
                        egui::Event::PointerMoved(first),
                        egui::Event::PointerButton {
                            pos: first,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
            );
            draw(
                &mut app,
                egui::RawInput {
                    time: Some(0.2),
                    events: vec![egui::Event::PointerMoved(last)],
                    ..Default::default()
                },
            );
            assert_eq!(app.visible_days().len(), count, "drag preview");
            app.refresh_workiq();
            assert!(commands.try_recv().is_err(), "drag preview must not fetch");
            draw(
                &mut app,
                egui::RawInput {
                    time: Some(0.3),
                    focused,
                    events: vec![egui::Event::PointerButton {
                        pos: last,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    }],
                    ..Default::default()
                },
            );
            assert!(app.date_drag.is_none());
            assert_eq!(app.preferences.view, View::Day);
            assert_eq!(app.visible_days().len(), count);
            let range = app.visible_workiq_range().unwrap();
            assert_eq!(range.first, date(start.min(end)));
            assert_eq!(range.last, date(start.max(end)));
            let mut output = context.run_ui(
                egui::RawInput {
                    time: Some(0.4),
                    ..Default::default()
                },
                |ui| app.timeline(ui),
            );
            let labels = output.shapes.iter().filter(|shape| matches!(&shape.shape,
                egui::epaint::Shape::Text(text) if ["Mon ", "Tue ", "Wed ", "Thu ", "Fri ", "Sat ", "Sun "].iter().any(|day| text.galley.job.text.starts_with(day))
            )).count();
            output.textures_delta.clear();
            assert_eq!(labels, count, "rendered calendar columns");
        }
    }

    #[test]
    fn preset_views_highlight_their_entire_period_after_a_day_click() {
        let (mut app, _, _) = assistant_fixture();
        let date = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        app.choose_dates(date, date);
        assert!(app.mini_selected(date));
        assert!(!app.mini_selected(date + chrono::Duration::days(1)));
        for view in [View::WorkWeek, View::Week, View::Month] {
            app.choose_view(view);
            let expected = view
                .days(date)
                .into_iter()
                .filter(|day| view != View::Month || day.month() == date.month())
                .collect::<Vec<_>>();
            for day in View::Month.days(date) {
                assert_eq!(
                    app.mini_selected(day),
                    expected.contains(&day),
                    "{view:?} {day}"
                );
            }
        }
    }

    #[test]
    fn mini_week_chevron_is_a_single_click_and_czech_commands_remain_searchable() {
        let (mut app, _, mut commands) = assistant_fixture();
        app.anchor = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        app.preferences.language = tomlook::i18n::Language::Cs;
        let monday = tomlook::calendar::monday(app.anchor);
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| app.sidebar(ui),
            );
            output.textures_delta.clear();
        };
        draw(&mut app, Vec::new());
        let rect = context.data(|data| {
            data.get_temp::<egui::Rect>(Id::new(("mini-week", monday)))
                .unwrap()
        });
        assert!(rect.width() >= 16.0 && rect.height() >= 25.0);
        let pos = rect.center();
        draw(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        draw(
            &mut app,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert_eq!(app.preferences.view, View::WorkWeek);
        assert_eq!(app.visible_days().len(), 5);
        assert!(app.date_selection.is_none());
        assert!(app.mini_selected(monday));
        assert!(
            commands.try_recv().is_err(),
            "Picking dates must not start AI"
        );
        let matches = command_matches_in("pracovní", tomlook::i18n::Language::Cs);
        assert_eq!(matches, vec![PaletteCommand::View(View::WorkWeek)]);
    }

    #[test]
    fn mini_calendar_dates_fit_beside_the_navigation_scrollbar() {
        for floating in [false, true] {
            let (mut app, _, _) = assistant_fixture();
            app.anchor = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
            app.preferences.language = tomlook::i18n::Language::Cs;
            let context = egui::Context::default();
            let mut clip = egui::Rect::NOTHING;
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(800.0, 420.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    ui.spacing_mut().scroll.floating = floating;
                    egui::Panel::left("navigation")
                        .exact_size(NAVIGATION_WIDTH)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                )
                                .show(ui, |ui| {
                                    clip = ui.clip_rect();
                                    app.sidebar(ui);
                                });
                        });
                },
            );
            output.textures_delta.clear();
            for date in View::Month.days(app.anchor) {
                let rect = context.data(|data| {
                    data.get_temp::<egui::Rect>(Id::new(("mini-date", date)))
                        .unwrap()
                });
                assert!(
                    clip.contains_rect(rect),
                    "Date {date} clips with floating scrollbar {floating}: {rect:?}, {clip:?}"
                );
            }
        }
    }

    #[test]
    fn ghost_filter_and_rendering_keep_titles_without_category_stripes() {
        let mut app = Tomlook::initial(None, None, None);
        let mut events = tomlook::calendar::fixture(2);
        events[0].status = "notResponded".into();
        events[0].category = "Work".into();
        events[0].title = "Today".into();
        events[1].status = "accepted".into();
        app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
        app.preferences.language = tomlook::i18n::Language::Cs;
        let query = tomlook::calendar::SearchQuery::new("");
        assert!(app.matches_event(0, &query));
        app.preferences.hide_unconfirmed = true;
        assert!(!app.matches_event(0, &query));
        assert!(app.matches_event(1, &query));
        let context = egui::Context::default();
        for height in [23.0, 38.0, 100.0] {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                app.event_card(
                    ui,
                    0,
                    egui::Rect::from_min_size(ui.cursor().min, Vec2::new(300.0, height)),
                );
            });
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
                assert!(
                    !matches!(&shape.shape, egui::epaint::Shape::LineSegment { stroke, .. } if stroke.width == 3.0),
                    "Ghost must have no category stripe"
                );
            }
            assert!(text.contains("Nepotvrzeno"));
            assert!(
                text.contains("Today"),
                "Meeting titles must not be translated"
            );
        }
    }

    #[test]
    fn czech_all_day_cards_use_a_localized_label_instead_of_a_midnight_time() {
        for status in ["accepted", "notResponded"] {
            let mut app = Tomlook::initial(None, None, None);
            let mut events = tomlook::calendar::fixture(1);
            events[0].status = status.into();
            events[0].is_all_day = true;
            events[0].title = "Today".into();
            app.calendar =
                Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
            app.preferences.language = tomlook::i18n::Language::Cs;
            let context = egui::Context::default();
            for height in [23.0, 38.0, 100.0] {
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    app.event_card(
                        ui,
                        0,
                        egui::Rect::from_min_size(ui.cursor().min, Vec2::new(320.0, height)),
                    );
                });
                output.textures_delta.clear();
                let mut text = String::new();
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                assert!(text.contains("Celý den"), "{status}, {height}: {text}");
                assert!(text.contains("Today"), "Original title must be preserved");
                assert!(!text.contains("All day"));
                if status == "notResponded" {
                    assert!(text.contains("Nepotvrzeno"));
                    assert!(!text.contains("00:00"));
                }
            }
        }
    }

    #[test]
    fn every_day_of_multiday_all_day_meetings_opens_the_same_event_details() {
        for view in [View::WorkWeek, View::Week, View::Month] {
            for status in ["accepted", "notResponded"] {
                let mut app = Tomlook::initial(None, None, None);
                let mut events = tomlook::calendar::fixture(2);
                for (index, event) in events.iter_mut().enumerate() {
                    event.start = chrono::DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z").unwrap()
                        + chrono::Duration::days(index as i64 * 2);
                    event.end = event.start + chrono::Duration::days(2);
                    event.is_all_day = true;
                    event.status = status.into();
                    event.title = format!("Span {index}");
                }
                app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
                app.anchor = "2026-11-02".parse().unwrap();
                app.preferences.view = view;
                app.preferences.hide_unconfirmed = false;
                let (commands, input) = mpsc::sync_channel(16);
                app.worker = Some(Worker {
                    commands,
                    notices: Arc::new(Default::default()),
                    activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                    stop: Arc::new(Default::default()),
                    done: None,
                });
                let context = egui::Context::default();
                let draw = |app: &mut Tomlook, events| {
                    let mut output = context.run_ui(egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1500.0, 1000.0))),
                        events,
                        ..Default::default()
                    }, |ui| {
                        if view == View::Month { app.month(ui); } else { app.timeline(ui); }
                    });
                    output.textures_delta.clear();
                    output
                };
                draw(&mut app, vec![]);
                let output = draw(&mut app, vec![]);
                for index in 0..2 {
                    let title = format!("Span {index}");
                    let positions = output.shapes.iter().filter_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text.ends_with(&title) =>
                            Some(text.pos + Vec2::new(2.0, 5.0)),
                        _ => None,
                    }).collect::<Vec<_>>();
                    assert_eq!(positions.len(), 2, "{view:?}, {status}: {title} must appear on both days");
                    for (day, position) in positions.into_iter().enumerate() {
                        app.selected = None;
                        for pressed in [true, false] {
                            draw(&mut app, vec![
                                egui::Event::PointerMoved(position),
                                egui::Event::PointerButton {
                                    pos: position, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                                },
                            ]);
                        }
                        assert_eq!(app.selected, Some(index), "{view:?}, {status}: {title}, day {day}");
                        assert!(matches!(input.try_recv().unwrap(), Command::Detail { id, .. } if id == app.calendar.events[index].id));
                        assert!(input.try_recv().is_err(), "One click must request details exactly once");
                        draw(&mut app, vec![egui::Event::PointerGone]);
                    }
                }
            }
        }
    }

    #[test]
    fn all_day_timeline_reserves_empty_columns_and_details_show_inclusive_range() {
        let mut app = Tomlook::initial(None, None, None);
        let mut events = tomlook::calendar::fixture(1);
        events[0].start = chrono::DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z").unwrap();
        events[0].end = chrono::DateTime::parse_from_rfc3339("2026-11-04T00:00:00Z").unwrap();
        events[0].is_all_day = true;
        events[0].title = "All-day fixture".into();
        app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
        app.anchor = "2026-11-02".parse().unwrap();
        app.preferences.language = tomlook::i18n::Language::Cs;
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.details(ui, app.calendar.events[0].clone());
        });
        output.textures_delta.clear();
        let mut text = String::new();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        assert!(text.contains("pondělí, 2. 11. 2026"));
        assert!(text.contains("úterý, 3. 11. 2026"));
        assert!(text.contains("Celý den"));
        assert!(!text.contains("01:00"));
        assert!(!text.contains("středa, 4."));
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1400.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| app.timeline(ui),
        );
        output.textures_delta.clear();
        let positions = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.text().contains("All-day fixture") =>
                {
                    Some(text.pos.x)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            positions.len(),
            2,
            "All-day cards must appear on both covered days: {positions:?}"
        );
        assert!(
            positions[0] < 300.0,
            "First card belongs to Monday: {positions:?}"
        );
        assert!(
            positions[1] > positions[0] + 200.0 && positions[1] < 600.0,
            "Second card belongs to Tuesday: {positions:?}"
        );
    }

    #[test]
    fn dismissed_activity_error_does_not_return_on_unrelated_activity_updates() {
        let activity = Arc::new(std::sync::Mutex::new(Arc::new(tomlook::worker::Activity {
            revision: 1,
            error: Some("First failure".into()),
            ..Default::default()
        })));
        let (commands, _input) = std::sync::mpsc::sync_channel(4);
        let worker = Worker {
            commands,
            notices: Arc::new(tomlook::worker::Notices::default()),
            activity: activity.clone(),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            done: None,
        };
        let mut app = Tomlook::initial(Some(worker), None, None);
        app.receive();
        assert_eq!(app.error.as_deref(), Some("First failure"));
        *activity.lock().unwrap() = Arc::new(tomlook::worker::Activity {
            revision: 2,
            error: Some("First failure".into()),
            ..Default::default()
        });
        app.receive();
        assert_eq!(app.error.as_deref(), Some("First failure"));
        app.error = None;
        *activity.lock().unwrap() = Arc::new(tomlook::worker::Activity {
            revision: 3,
            error: Some("First failure".into()),
            ..Default::default()
        });
        app.receive();
        assert!(app.error.is_none());
        assert_eq!(app.activity.error.as_deref(), Some("First failure"));
        *activity.lock().unwrap() = Arc::new(tomlook::worker::Activity {
            revision: 4,
            error: Some("New failure".into()),
            ..Default::default()
        });
        app.receive();
        assert_eq!(app.error.as_deref(), Some("New failure"));
    }

    #[test]
    fn czech_insights_use_saved_narrative_and_preserve_original_evidence() {
        let mut app = Tomlook::initial(None, None, None);
        app.preferences.language = tomlook::i18n::Language::Cs;
        app.briefing = Some(serde_json::json!({
            "summary": "English overview", "sources": [],
            "cs": { "summary": "Český přehled", "preparation": ["Demo: Otevřít připravenou ukázku"], "open_questions": ["Další krok: Potvrdit vlastníka"] },
            "retrieved_evidence": {"records": []}
        }));
        let context = egui::Context::default();
        for width in [320.0, 760.0] {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(width);
                app.render_insights(ui, true);
            });
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            assert!(text.contains("Český přehled"));
            assert!(!text.contains("English overview"));
            assert!(text.contains("Otevřít připravenou ukázku"));
            assert!(text.contains("Potvrdit vlastníka"));
        }
    }

    #[test]
    fn mini_calendar_focus_loss_cancels_unreleased_preview() {
        let (mut app, _, _) = assistant_fixture();
        let first = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        app.anchor = first;
        app.preferences.view = View::Day;
        app.date_selection = Some(DateSelection::new(first, first));
        app.date_drag = Some((first, first + chrono::Duration::days(2)));
        let context = egui::Context::default();
        let mut output = context.run_ui(
            egui::RawInput {
                focused: false,
                ..Default::default()
            },
            |ui| app.sidebar(ui),
        );
        output.textures_delta.clear();
        assert!(app.date_drag.is_none());
        assert_eq!(app.visible_days(), vec![first]);
    }

    #[test]
    fn mini_calendar_commits_drag_when_events_share_one_frame() {
        for (start, end) in [(6, 8), (8, 6)] {
            let (mut app, _, _) = assistant_fixture();
            app.anchor = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
            app.preferences.view = View::Month;
            let context = egui::Context::default();
            let draw = |app: &mut Tomlook, raw| {
                let mut output = context.run_ui(raw, |ui| {
                    ui.set_width(192.0);
                    app.sidebar(ui);
                });
                output.textures_delta.clear();
            };
            draw(&mut app, egui::RawInput::default());
            let position = |day| {
                context.data(|data| {
                    data.get_temp::<egui::Rect>(Id::new((
                        "mini-date",
                        NaiveDate::from_ymd_opt(2026, 10, day).unwrap(),
                    )))
                    .unwrap()
                    .center()
                })
            };
            let first = position(start);
            let last = position(end);
            draw(
                &mut app,
                egui::RawInput {
                    time: Some(0.1),
                    events: vec![
                        egui::Event::PointerMoved(first),
                        egui::Event::PointerButton {
                            pos: first,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        },
                        egui::Event::PointerMoved(last),
                        egui::Event::PointerButton {
                            pos: last,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
            );
            assert_eq!(position(start), first, "press target moved");
            assert!(app.date_drag.is_none());
            assert_eq!(app.preferences.view, View::Day);
            assert_eq!(
                app.visible_days(),
                DateSelection::new(
                    NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
                    NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
                )
                .days()
            );
        }
    }

    #[test]
    fn custom_period_navigation_preserves_length_and_presets_clear_it() {
        let (mut app, _, _) = assistant_fixture();
        let first = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        app.choose_dates(first, first + chrono::Duration::days(2));
        app.shift_period(1);
        assert_eq!(
            app.visible_days(),
            DateSelection::new(
                first + chrono::Duration::days(3),
                first + chrono::Duration::days(5)
            )
            .days()
        );
        app.shift_period(-1);
        assert_eq!(app.visible_days()[0], first);
        app.choose_view(View::Week);
        assert!(app.date_selection.is_none());
        assert_eq!(app.visible_days().len(), 7);
        app.choose_dates(first, first);
        assert_eq!(app.visible_days(), vec![first]);
        assert_eq!(app.preferences.view, View::Day);
        for (view, _) in View::ALL {
            app.choose_view(view);
            app.choose_dates(first, first);
            assert_eq!(app.preferences.view, View::Day);
            assert_eq!(app.visible_days(), vec![first]);
        }
    }

    #[test]
    fn person_addresses_and_photo_errors_are_bounded() {
        assert_eq!(
            person_email("Fixture Person <FIXTURE@example.com>"),
            Some("fixture@example.com".into())
        );
        for person in ["Fixture Person", "@example.com", "a@b@c", "person@"] {
            assert!(person_email(person).is_none());
        }
        let (mut app, notices, _) = assistant_fixture();
        app.workiq_ready = true;
        notices
            .send(ai::Notice::Photos(vec![(
                "fixture@example.com".into(),
                Err("Unavailable photo".into()),
            )]))
            .unwrap();
        app.receive();
        assert_eq!(
            app.photos["fixture@example.com"].error.as_deref(),
            Some("Unavailable photo")
        );
        notices
            .send(ai::Notice::HostedWorkiq {
                busy: false,
                ready: false,
                message: "Disconnected".into(),
            })
            .unwrap();
        app.receive();
        assert!(app.photos.is_empty());
    }
}
