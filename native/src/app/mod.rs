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

mod ask_panel;
mod calendar;
mod connections;
mod event_editor;
mod feedback;
mod meeting;
mod palette;
mod recommendations;
mod settings;

use connections::status_text;
use event_editor::EventDraft;
use feedback::FeedbackUi;
use meeting::MeetingActionDraft;
use recommendations::inbox_button;
use settings::PreferencesSection;

const ACCENTS: [(&str, [u8; 3], [u8; 3]); 4] = [
    ("Blue", [0, 105, 161], [50, 170, 240]),
    ("Red", [177, 54, 41], [246, 115, 99]),
    ("Green", [43, 112, 54], [112, 195, 124]),
    ("Yellow", [131, 96, 0], [242, 197, 68]),
];

const NAVIGATION_WIDTH: f32 = 208.0;

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
    feedback: FeedbackUi,
    preview: bool,
    demo_screenshot: Option<std::path::PathBuf>,
    demo_scene: tomlook::showcase::Scene,
    demo_frames: u32,
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
    updater: Option<tomlook::updater::Updater>,
    update_status: tomlook::updater::Status,
    update_configured: bool,
    reminders: Option<tomlook::reminders::Reminders>,
    reminder_status: tomlook::reminders::Status,
    reminder_open: Option<mpsc::Receiver<Option<String>>>,
}

impl Tomlook {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        options: Options,
        shutdown: Arc<std::sync::Mutex<crate::Lifecycle>>,
        startup: Option<crate::StartupTrace>,
        preload: Option<tomlook::worker::Preload>,
        interaction_trace: Option<crate::interaction_trace::Trace>,
        demo_screenshot: Option<(std::path::PathBuf, tomlook::showcase::Scene)>,
    ) -> Self {
        let preview = options.demo.is_some();
        let measurement = startup.is_some()
            || interaction_trace.is_some()
            || env!("CARGO_BIN_NAME") != "tomlook";
        let reminder_state = options.state.clone();
        let update_disabled = if preview {
            Some("Updates are disabled in the offline demo.")
        } else if measurement {
            Some("Updates are disabled in measurement and diagnostic modes.")
        } else {
            None
        };
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
        if let Some((path, scene)) = demo_screenshot {
            app.demo_screenshot = Some(path);
            app.demo_scene = scene;
        }
        app.tray = tray;
        if measurement {
            app.reminder_status.disabled =
                Some("Notifications are disabled in measurement and diagnostic modes.".into());
        } else {
            let mode = if preview {
                tomlook::reminders::Mode::Preview
            } else {
                tomlook::reminders::Mode::Live
            };
            let (opened, reminder_open) = mpsc::sync_channel(8);
            let repaint = creation.egui_ctx.clone();
            let restore = creation.egui_ctx.clone();
            match tomlook::reminders::Reminders::start(
                reminder_state,
                mode,
                move || repaint.request_repaint(),
                move |meeting| {
                    // Runs on a Windows notification thread; the UI applies it in `logic`.
                    let _ = opened.try_send(meeting);
                    crate::tray::restore(&restore);
                },
            ) {
                Ok(mut reminders) => {
                    if let Some(handle) = reminders.shutdown()
                        && let Ok(mut lifecycle) = shutdown.lock()
                    {
                        lifecycle.reminders = Some(handle);
                    }
                    app.reminder_status = Default::default();
                    app.reminders = Some(reminders);
                    app.reminder_open = Some(reminder_open);
                }
                Err(problem) => app.reminder_status.error = Some(problem),
            }
        }
        app.update_status.disabled = update_disabled.map(Into::into);
        if update_disabled.is_none() {
            let context = creation.egui_ctx.clone();
            match std::env::current_exe()
                .map_err(|error| format!("Locate Tomlook.exe for updates: {error}"))
                .and_then(|executable| {
                    tomlook::updater::Updater::start(executable, move || context.request_repaint())
                }) {
                Ok(updater) => {
                    match shutdown.lock() {
                        Ok(mut handle) => handle.update = Some(updater.exit_handle()),
                        Err(_) => {
                            app.update_status.error =
                                Some("Updates cannot be installed on exit in this session.".into())
                        }
                    }
                    app.updater = Some(updater);
                }
                Err(problem) => app.update_status.disabled = Some(problem),
            }
        }
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
            feedback: FeedbackUi::default(),
            preview: false,
            demo_screenshot: None,
            demo_scene: Default::default(),
            demo_frames: 0,
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
            updater: None,
            update_status: tomlook::updater::Status {
                disabled: Some("Updates are not available in this session.".into()),
                ..Default::default()
            },
            update_configured: false,
            reminders: None,
            reminder_status: tomlook::reminders::Status {
                disabled: Some("Notifications are not available in this session.".into()),
                ..Default::default()
            },
            reminder_open: None,
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
                    self.sync_reminders();
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
                Notice::Feedback { document, entries } => self.receive_feedback(document, entries),
                Notice::FeedbackSummary { account, result } => {
                    self.receive_feedback_summary(account, result)
                }
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
                ai::Notice::PromptSuggestion { id, result } => {
                    self.receive_prompt_suggestion(id, result)
                }
                ai::Notice::Stopped => {
                    self.stop_prompt_suggestion();
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

    fn receive_update(&mut self) {
        let Some(updater) = &self.updater else {
            return;
        };
        if !self.update_configured && !self.loading {
            self.update_configured = true;
            if let Err(error) = updater.configure(
                self.preferences.automatic_update_check,
                self.preferences.update_checked_at,
            ) {
                self.update_status.error = Some(error);
            }
        }
        let Some(status) = updater.status(self.update_status.revision) else {
            return;
        };
        let checked = status.checked_at.is_some()
            && (status.checked_at != self.preferences.update_checked_at
                || status.latest.as_deref().unwrap_or_default()
                    != self.preferences.update_latest_version);
        self.update_status = status;
        if checked && !self.loading {
            self.preferences.update_checked_at = self.update_status.checked_at;
            self.preferences.update_latest_version =
                self.update_status.latest.clone().unwrap_or_default();
            self.save_preferences();
        }
    }

    fn check_for_updates(&mut self) {
        if let Some(updater) = &self.updater {
            match updater.check_now() {
                Ok(()) => self.update_status.error = None,
                Err(error) => self.update_status.error = Some(error),
            }
        }
    }

    fn set_automatic_updates(&mut self, enabled: bool) {
        self.preferences.automatic_update_check = enabled;
        self.save_preferences();
        if let Some(updater) = &self.updater
            && let Err(error) = updater.configure(enabled, self.preferences.update_checked_at)
        {
            self.update_status.error = Some(error);
        }
    }

    fn restart_for_update(&mut self, context: &egui::Context) {
        if let Some(updater) = &self.updater
            && self.update_status.staged.is_some()
        {
            updater.request_restart();
            self.exit(context);
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

    fn save_preferences(&mut self) {
        self.command(Command::Preferences(self.preferences.clone()));
        self.sync_reminders();
    }

    /// Hands the cached calendar and reminder preferences to the reminder thread.
    fn sync_reminders(&self) {
        if let Some(reminders) = &self.reminders
            && !self.loading
        {
            reminders.update(
                &self.calendar,
                tomlook::reminders::Settings::from_preferences(&self.preferences),
            );
        }
    }

    fn receive_reminders(&mut self) {
        if let Some(status) = self
            .reminders
            .as_ref()
            .and_then(|reminders| reminders.status(self.reminder_status.revision))
        {
            self.reminder_status = status;
        }
        let opened: Vec<Option<String>> = self
            .reminder_open
            .as_ref()
            .map(|opened| opened.try_iter().take(8).collect())
            .unwrap_or_default();
        for meeting in opened.into_iter().flatten() {
            self.open_reminder_meeting(&meeting);
        }
    }

    /// Opens a meeting's details after its reminder was clicked; the window is already restored.
    fn open_reminder_meeting(&mut self, id: &str) {
        let Some(index) = self.calendar.event_index(id) else {
            self.error = Some("Meeting is no longer in the current calendar.".into());
            return;
        };
        let event = self.calendar.events[index].clone();
        self.open_recommended_meeting(&event);
        self.insights_expanded = false;
        if !self.assistant_detached {
            self.assistant = false;
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
        self.receive_update();
        self.receive_reminders();
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
                    if self.demo_scene_ready() {
                        context.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                            Default::default(),
                        ));
                        self.screenshot_requested = true;
                    } else {
                        context.request_repaint_after(std::time::Duration::from_millis(50));
                    }
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
                if let Some(staged) = &self.update_status.staged
                    && status_text(ui, "status-update", language.text(&format!("Update {} ready", staged.version)))
                        .on_hover_text(language.text("A verified update is ready. Restart from Settings, or it is installed when you exit Tomlook."))
                        .clicked()
                {
                    self.preferences_open = true;
                    self.preferences_section = PreferencesSection::Updates;
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
mod tests;
