use super::*;
use tomlook::setup::{Action, Connection, FirstRun, Item, State, Step};

/// First-run welcome, the background probe of runtime prerequisites and the status-bar hint.
#[derive(Default)]
pub(super) struct SetupUi {
    /// A normal launch of a real profile. Demo, stress, measurement and acceptance runs never
    /// show the welcome or the hint and do not probe at startup.
    pub eligible: bool,
    pub root: Option<std::path::PathBuf>,
    pub probe: Option<Arc<tomlook::setup::Probe>>,
    pub probing: Option<mpsc::Receiver<tomlook::setup::Probe>>,
    pub welcome: bool,
    pub decided: bool,
}

const WELCOME: &str = "Tomlook is a fast Windows calendar for your Microsoft 365 meetings. With GitHub Copilot it prepares Insights before meetings and notes after them. Nothing connects until you sign in.";

/// A probe that could not run: treated as an existing profile so no welcome is forced on it.
fn unavailable_probe(error: String) -> tomlook::setup::Probe {
    tomlook::setup::Probe {
        azure_cli: None,
        copilot: Err(error),
        saved_connections: true,
    }
}

pub(super) fn status_icon(
    ui: &mut egui::Ui,
    state: State,
    language: tomlook::i18n::Language,
) -> egui::Response {
    let size = Vec2::splat(18.0);
    let response = if state == State::Checking {
        ui.add_sized(size, egui::Spinner::new().size(14.0))
    } else {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        let dark = ui.visuals().dark_mode;
        let color = |light: [u8; 3], night: [u8; 3]| {
            let [r, g, b] = if dark { night } else { light };
            Color32::from_rgb(r, g, b)
        };
        let weak = ui.visuals().weak_text_color();
        let painter = ui.painter();
        let center = rect.center();
        let mark = Stroke::new(2.0, Color32::WHITE);
        match state {
            State::Done => {
                painter.circle_filled(center, 8.0, color([26, 127, 55], [46, 160, 67]));
                painter.line_segment(
                    [center + Vec2::new(-4.0, 0.0), center + Vec2::new(-1.0, 3.0)],
                    mark,
                );
                painter.line_segment(
                    [center + Vec2::new(-1.0, 3.0), center + Vec2::new(4.0, -3.0)],
                    mark,
                );
            }
            State::Action => {
                painter.circle_filled(center, 8.0, color([154, 103, 0], [191, 135, 0]));
                painter.line_segment(
                    [center + Vec2::new(0.0, -4.5), center + Vec2::new(0.0, 1.0)],
                    mark,
                );
                painter.circle_filled(center + Vec2::new(0.0, 4.0), 1.2, Color32::WHITE);
            }
            State::Error => {
                painter.circle_filled(center, 8.0, color([207, 34, 46], [218, 54, 51]));
                painter.line_segment(
                    [center + Vec2::splat(-3.5), center + Vec2::splat(3.5)],
                    mark,
                );
                painter.line_segment(
                    [center + Vec2::new(-3.5, 3.5), center + Vec2::new(3.5, -3.5)],
                    mark,
                );
            }
            State::NotSet | State::Checking => {
                painter.circle_stroke(center, 7.0, Stroke::new(1.5, weak));
            }
        }
        response
    };
    let label = language.text(state.label()).into_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
    response.on_hover_text(label)
}

impl Tomlook {
    /// Looks for Azure CLI, the configured Copilot runtime and saved sign-ins off the UI thread.
    pub(super) fn start_setup_probe(&mut self, context: &egui::Context) {
        if self.preview || self.setup.probing.is_some() {
            return;
        }
        let Some(root) = self.setup.root.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        match std::thread::Builder::new()
            .name("tomlook-setup-probe".into())
            .spawn(move || {
                let _ = sender.send(tomlook::setup::probe(&root));
                repaint.request_repaint();
            }) {
            Ok(_) => self.setup.probing = Some(receiver),
            Err(error) => {
                self.setup.probe = Some(Arc::new(unavailable_probe(format!(
                    "Prerequisite check could not start: {error}"
                ))))
            }
        }
    }

    /// Applies a finished probe and decides the first-run welcome once.
    pub(super) fn sync_setup(&mut self) {
        if let Some(receiver) = &self.setup.probing {
            match receiver.try_recv() {
                Ok(probe) => {
                    self.setup.probe = Some(Arc::new(probe));
                    self.setup.probing = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.setup.probing = None;
                    self.setup.probe = Some(Arc::new(unavailable_probe(
                        "Prerequisite check stopped unexpectedly".into(),
                    )));
                }
            }
        }
        if self.setup.decided {
            return;
        }
        match tomlook::setup::first_run(
            &self.preferences,
            self.setup.eligible,
            !self.loading,
            self.calendar_owner.as_deref(),
            self.setup.probe.as_deref(),
        ) {
            FirstRun::Wait => {}
            FirstRun::Nothing => self.setup.decided = true,
            decision @ (FirstRun::Show | FirstRun::MarkShown) => {
                self.setup.decided = true;
                self.setup.welcome = decision == FirstRun::Show;
                self.preferences.welcome_shown = true;
                self.save_preferences();
            }
        }
    }

    /// The checklist of the current connection, notification and update states.
    pub(super) fn setup_items(&self) -> Vec<Item> {
        let external = |kind: tomlook::connectors::Kind| {
            self.external_status
                .get(&kind)
                .map(|status: &ExternalStatus| Connection {
                    busy: status.busy,
                    ready: status.ready,
                    message: &status.message,
                })
                .unwrap_or_default()
        };
        let typed = Some(self.hosted_workiq_account.trim()).filter(|account| !account.is_empty());
        let workiq_account = if self.workiq_ready {
            self.activity
                .identity
                .as_deref()
                .map(|identity| identity.account.as_str())
                .or(typed)
        } else {
            typed
        };
        tomlook::setup::checklist(&tomlook::setup::Inputs {
            demo: self.preview,
            today: Local::now().date_naive(),
            workiq: Connection {
                busy: self.hosted_workiq_busy,
                ready: self.workiq_ready,
                message: &self.hosted_workiq_status,
            },
            workiq_account,
            calendar_owner: self.calendar_owner.as_deref(),
            coverage: &self.calendar.coverage,
            sync: &self.activity.sync,
            copilot: Connection {
                // Connection progress messages end with an ellipsis, such as "Connecting isolated SDK...".
                busy: self.ai_login_busy || (!self.ai_ready && self.connection.ends_with("...")),
                ready: self.ai_ready,
                message: &self.connection,
            },
            prepare_enabled: self.preferences.prepare_enabled,
            paused: self.preferences.paused,
            reminders_running: self.reminders.is_some(),
            reminders: &self.reminder_status,
            updates: &self.update_status,
            automatic_updates: self.preferences.automatic_update_check,
            update_checked_at: self.preferences.update_checked_at,
            latest_version: &self.preferences.update_latest_version,
            webiq: external(tomlook::connectors::Kind::Webiq),
            github: external(tomlook::connectors::Kind::Github),
            foundry: &self.auto_status,
            msx_enabled: self.preferences.msx_enabled,
            msx: &self.msx_status,
            probe: self.setup.probe.as_deref(),
        })
    }

    pub(super) fn open_setup_checklist(&mut self) {
        self.preferences_open = true;
        self.preferences_section = PreferencesSection::Setup;
    }

    /// Opens or calls the same flow as the status bar and its dialogs; never in the demo.
    pub(super) fn run_setup_action(&mut self, action: Action, context: &egui::Context) {
        use tomlook::connectors::Kind;
        if self.preview {
            return;
        }
        match action {
            Action::SignInWorkiq | Action::OpenWorkiq => {
                self.connection_panel = Some(ConnectionPanel::Workiq)
            }
            Action::RetrySync => self.refresh_workiq(),
            Action::SignInCopilot => {
                self.connection_panel = Some(ConnectionPanel::Ai);
                self.sign_in_ai();
            }
            Action::OpenAi | Action::OpenFoundry => self.open_ai_connection(),
            Action::InstallCopilot => context.open_url(egui::OpenUrl::new_tab(
                tomlook::setup::COPILOT_CLI_INSTALL_URL,
            )),
            Action::OpenInsights => self.settings = true,
            Action::TestNotification => {
                if let Some(reminders) = &self.reminders {
                    reminders.test(self.preferences.language);
                }
            }
            Action::EnableUpdates => self.set_automatic_updates(true),
            Action::CheckUpdates => self.check_for_updates(),
            Action::EnterWebiqKey => {
                self.connection_panel = Some(ConnectionPanel::External(Kind::Webiq))
            }
            Action::ConnectGithub => {
                self.connection_panel = Some(ConnectionPanel::External(Kind::Github))
            }
            Action::OpenMsx => self.connection_panel = Some(ConnectionPanel::Msx),
            Action::OpenMsxSettings => {
                self.preferences_open = true;
                self.preferences_section = PreferencesSection::Msx;
            }
            Action::CheckAgain => {
                self.start_setup_probe(context);
                if let Some(reminders) = &self.reminders {
                    reminders.refresh();
                }
            }
        }
        context.request_repaint();
    }

    fn setup_item(&self, ui: &mut egui::Ui, item: &Item) -> Option<Action> {
        let language = self.preferences.language;
        let mut chosen = None;
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    status_icon(ui, item.state, language);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(language.text(item.step.title()));
                            if let Some(action) = item.action.filter(|_| !self.preview) {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button(language.text(action.label())).clicked() {
                                            chosen = Some(action);
                                        }
                                    },
                                );
                            }
                        });
                        ui.weak(language.text(item.step.purpose()));
                        ui.label(language.text(&item.summary));
                        if item.step == Step::Copilot && item.state != State::Done && !self.preview
                        {
                            ui.hyperlink_to(
                                language.text("Copilot CLI installation guide"),
                                tomlook::setup::COPILOT_CLI_INSTALL_URL,
                            );
                        }
                        if !item.details.is_empty() {
                            egui::CollapsingHeader::new(language.text("Details"))
                                .id_salt(("setup-details", item.step))
                                .show(ui, |ui| {
                                    for detail in &item.details {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.weak(format!("{}:", language.text(detail.label)));
                                            ui.label(language.text(&detail.value));
                                        });
                                    }
                                });
                        }
                    });
                });
            });
        chosen
    }

    /// Settings > Setup.
    pub(super) fn setup_preferences(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        if self.setup.probe.is_none() {
            self.start_setup_probe(ui.ctx());
        }
        let items = self.setup_items();
        let summary = tomlook::setup::summarize(&items);
        let mut chosen = None;
        ui.heading(language.text("Setup checklist"));
        ui.weak(language.text("What is connected, what is missing and what to do next. Nothing signs in until you click."));
        ui.add_space(12.0);
        if self.preview {
            ui.strong(language.text("Offline demo: Tomlook never connects to Microsoft 365, GitHub Copilot or other services."));
        } else {
            ui.strong(language.text(&summary.text()));
            ui.horizontal_wrapped(|ui| {
                if let Some((step, action)) = tomlook::setup::next_step(&items)
                    .and_then(|item| Some((item.step, item.action?)))
                {
                    if ui
                        .button(language.text(&format!("Next step: {}", action.label())))
                        .on_hover_text(language.text(step.title()))
                        .clicked()
                    {
                        chosen = Some(action);
                    }
                } else if summary.complete() {
                    ui.label(language.text("All required steps are done."));
                }
                if ui
                    .add_enabled(self.setup.probing.is_none(), egui::Button::new(language.text("Check again")))
                    .on_hover_text(language.text("Looks for Azure CLI and the Copilot runtime again and rereads the Windows notification setting. No sign-in or network request."))
                    .clicked()
                {
                    chosen = Some(Action::CheckAgain);
                }
            });
            let mut visible = !self.preferences.setup_hint_hidden;
            if ui
                .checkbox(
                    &mut visible,
                    language.text("Show setup progress in the status bar"),
                )
                .changed()
            {
                self.preferences.setup_hint_hidden = !visible;
                self.save_preferences();
            }
        }
        for (heading, group) in [
            ("Required steps", Step::required as fn(Step) -> bool),
            ("Optional connections", Step::optional),
            ("Prerequisites", |step| step == Step::Runtime),
        ] {
            ui.add_space(16.0);
            ui.strong(language.text(heading));
            for item in items.iter().filter(|item| group(item.step)) {
                ui.add_space(6.0);
                if let Some(action) = self.setup_item(ui, item) {
                    chosen = Some(action);
                }
            }
        }
        if let Some(action) = chosen {
            self.run_setup_action(action, ui.ctx());
        }
    }

    /// The compact first-run window: what Tomlook is, language and theme, and the required steps.
    pub(super) fn welcome_window(&mut self, context: &egui::Context) {
        if !self.setup.welcome {
            return;
        }
        let language = self.preferences.language;
        let items = self.setup_items();
        let mut open = true;
        let mut chosen = None;
        let (mut checklist, mut later) = (false, false);
        egui::Window::new(language.text("Welcome"))
            .id(Id::new("welcome"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(500.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(context, |ui| {
                ui.heading(language.text("Welcome to Tomlook"));
                ui.label(language.text(WELCOME));
                ui.add_space(8.0);
                egui::Grid::new("welcome-appearance")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        ui.strong(language.text("Language"));
                        self.language_choice(ui);
                        ui.end_row();
                        ui.strong(language.text("Theme"));
                        self.theme_choice(ui);
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.separator();
                ui.strong(language.text("Required steps"));
                for item in items.iter().filter(|item| item.step.required()) {
                    ui.horizontal(|ui| {
                        status_icon(ui, item.state, language);
                        ui.label(language.text(item.step.title()))
                            .on_hover_text(language.text(&item.summary));
                        if let Some(action) = item
                            .action
                            .filter(|_| !self.preview && item.state != State::Done)
                        {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button(language.text(action.label())).clicked() {
                                        chosen = Some(action);
                                    }
                                },
                            );
                        }
                    });
                }
                if self.preview {
                    ui.weak(language.text(tomlook::setup::DEMO));
                }
                ui.add_space(4.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(language.text("Open setup checklist")).clicked() {
                        checklist = true;
                    }
                    if ui.button(language.text("Later")).clicked() {
                        later = true;
                    }
                });
                ui.weak(language.text(
                    "The checklist stays in Settings > Setup and in Commands (Ctrl+Shift+P).",
                ));
            });
        if let Some(action) = chosen {
            self.run_setup_action(action, context);
        }
        if checklist {
            self.open_setup_checklist();
        }
        if checklist || later || chosen.is_some() || !open {
            self.setup.welcome = false;
        }
    }

    /// Compact setup progress while a required step is known to be incomplete.
    pub(super) fn setup_indicator(&mut self, ui: &mut egui::Ui) {
        if !self.setup.eligible
            || self.preview
            || self.loading
            || self.preferences.setup_hint_hidden
        {
            return;
        }
        let language = self.preferences.language;
        let summary = tomlook::setup::summarize(&self.setup_items());
        if !summary.show_hint(self.preferences.setup_hint_hidden) {
            return;
        }
        let hint = format!("{}\n{}", language.text(&summary.badge()),
            language.text("Open the setup checklist. Right-click to hide this indicator."));
        let response = connections::status_item(ui, "status-setup", connections::StatusIcon::Setup,
            &format!("{}/{}", summary.required_done, summary.required),
            connections::ConnectionState::Unknown, &hint);
        let mut hide = false;
        response.context_menu(|ui| {
            if ui.button(language.text("Don't show")).clicked() {
                hide = true;
                ui.close();
            }
        });
        if response.clicked() {
            self.open_setup_checklist();
        }
        if hide {
            self.preferences.setup_hint_hidden = true;
            self.save_preferences();
        }
    }
}
