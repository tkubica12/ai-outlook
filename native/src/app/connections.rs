use super::*;

impl Tomlook {
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let compact = ui.available_width() < 1120.0;
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
        ui.spacing_mut().button_padding = Vec2::new(2.0, 2.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                version_label(ui, language);
                if !self.preview {
                    let completed = self.activity.index.len(ActivityFilter::Ready);
                    let problems = self.activity.index.len(ActivityFilter::Problems);
                    let working = self.activity.queue.flights.len();
                    let waiting = self.activity.queued + self.activity.deferred;
                    let has_messages = self.error.is_some() || self.activity.error.is_some() || self.activity.sync.error.is_some() || problems > 0
                        || self.activity.event_creations.values().any(|record| self.calendar_owner.as_deref() == Some(record.account.as_str()))
                        || self.activity.proposals.values().any(|proposal| proposal.in_flight() || proposal.needs_review())
                        || self.activity.recommendations.values().any(|recommendation|
                            recommendation.state == tomlook::recommendations::State::Review
                                && self.calendar_owner.as_deref() == Some(recommendation.account.as_str()))
                        || self.external_status.values().any(|status| !status.ready && !status.busy && !status.message.is_empty() && status.message != "Not connected");
                    let now = Local::now().fixed_offset();
                    let recommendations = self.activity.recommendation_view.iter().filter_map(|entry| self.activity.recommendations.get(&entry.id)).filter(|recommendation|
                        recommendation.visible(&self.calendar, self.calendar_owner.as_deref(), now)).count()
                        + self.activity.event_creations.values().filter(|record|
                            record.state == tomlook::event_creation::State::Pending
                                && self.calendar_owner.as_deref() == Some(record.account.as_str())).count();
                    if (has_messages || recommendations > 0) && inbox_button(ui, language, has_messages, recommendations).clicked() {
                        self.messages_open = true;
                        if self.ai.is_some() { self.ai_command(ai::Command::RefreshCalendarAvailability); }
                    }
                    let summary = language.text(&format!("{completed} ready | {working} researching | {waiting} waiting | {problems} to review")).into_owned();
                    for (id, icon, count, label, state) in [
                        ("status-activity-problems", StatusIcon::Alert, problems, "Problems", ConnectionState::Offline),
                        ("status-activity-waiting", StatusIcon::Clock, waiting, "Waiting", ConnectionState::Unknown),
                        ("status-activity-working", StatusIcon::Search, working, "Researching", ConnectionState::Busy),
                        ("status-activity", StatusIcon::Check, completed, "Ready", ConnectionState::Ready),
                    ] {
                        if count == 0 && label != "Ready" { continue; }
                        let hint = format!("{}: {count}\n{summary}\n{}", language.text(label),
                            language.text("Show meetings, queue and local context"));
                        if status_item(ui, id, icon, &short_count(count), state, &hint).clicked() { self.settings = true; }
                    }
                }
                if self.activity.sync.total > 0 {
                    let sync = &self.activity.sync;
                    let label = if !self.workiq_ready {
                        format!("Sync {}/{} · offline", sync.fresh, sync.total)
                    } else if sync.error.is_some() {
                        format!("Sync {}/{} · retrying", sync.fresh, sync.total)
                    } else { format!("Sync {}/{}", sync.fresh, sync.total) };
                    let hint = format!("{}\n{}", language.text(&label), language.text(&format!(
                        "One month back, six months ahead through {}.\n{}{}",
                        sync.through.map(|day| day.to_string()).unwrap_or_default(),
                        sync.updated.map(|time| format!("Last saved refresh: {}.", time.with_timezone(&Local).format("%H:%M"))).unwrap_or_else(|| "No synchronized days yet.".into()),
                        sync.error.as_ref().map(|error| format!("\n{error}")).unwrap_or_default()
                    )));
                    let state = if sync.error.is_some() { ConnectionState::Offline }
                        else if !self.workiq_ready { ConnectionState::Unknown }
                        else if sync.fresh < sync.total { ConnectionState::Busy } else { ConnectionState::Ready };
                    if status_item(ui, "status-sync", StatusIcon::Sync, &format!("{}/{}", sync.fresh, sync.total), state, &hint).clicked() {
                        self.connection_panel = Some(ConnectionPanel::Workiq);
                    }
                }
                let cache = language.text(&format!("{} cached meetings", self.calendar.events.len())).into_owned();
                let owner = self.calendar_owner.as_deref().map(|owner| format!("Locally saved calendar owned by {owner}; retained when disconnected."))
                    .unwrap_or_else(|| "Local calendar cache; no connected account assigned.".into());
                if status_item(ui, "status-cache", StatusIcon::Calendar, &short_count(self.calendar.events.len()),
                    ConnectionState::Unknown, &format!("{cache}\n{}", language.text(&owner))).clicked() {
                    self.connection_panel = Some(ConnectionPanel::Workiq);
                }
                if self.loading { ui.add(egui::Spinner::new().size(14.0)); }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if self.preview {
                        ui.label(RichText::new(language.text("Offline demo - fictional calendar")).small().weak());
                        return;
                    }
                    let state = ConnectionState::of(self.ai_ready, self.ai_login_busy);
                    let label = if self.ai_login_busy { "GitHub Copilot: signing in..." }
                        else if self.ai_ready { "GitHub Copilot: connected" } else { "GitHub Copilot: disconnected" };
                    if connection_status(ui, "status-ai", if compact { "AI" } else { "Copilot" }, state,
                        &format!("{}\n{}", language.text(label), language.text(&self.connection))).clicked() {
                        self.open_ai_connection();
                    }
                    use tomlook::decision::Phase;
                    let state = match self.auto_status.phase {
                        Phase::SignedIn => ConnectionState::Ready,
                        Phase::Checking | Phase::SigningIn => ConnectionState::Busy,
                        Phase::Unknown | Phase::Unconfigured => ConnectionState::Unknown,
                        Phase::SignInRequired | Phase::Failed => ConnectionState::Offline,
                    };
                    if connection_status(ui, "status-auto", if compact { "Fdy" } else { "Foundry" }, state,
                        &format!("{}\n{}", language.text(self.auto_status.label()), language.text(&self.auto_status.message))).clicked() {
                        self.open_ai_connection();
                    }
                    let label = if self.hosted_workiq_busy { "WorkIQ: working..." }
                        else if self.workiq_ready { "WorkIQ: connected" } else { "WorkIQ: disconnected" };
                    if connection_status(ui, "status-workiq", if compact { "WIQ" } else { "WorkIQ" },
                        ConnectionState::of(self.workiq_ready, self.hosted_workiq_busy),
                        &format!("{}\n{}", language.text(label), language.text(&self.hosted_workiq_status))).clicked() {
                        self.connection_panel = Some(ConnectionPanel::Workiq);
                    }
                    if self.preferences.msx_enabled
                        && connection_status(ui, "status-msx", "MSX",
                            ConnectionState::of(self.msx_status.phase == tomlook::msx::Phase::Connected, self.msx_status.busy()),
                            &format!("{}\n{}", language.text(self.msx_status.label()), language.text(&self.msx_status.message))).clicked() {
                        self.connection_panel = Some(ConnectionPanel::Msx);
                    }
                    for kind in [tomlook::connectors::Kind::Webiq, tomlook::connectors::Kind::Github] {
                        let status = self.external_status.entry(kind).or_default();
                        let label = if status.forgetting { "disconnecting..." } else if status.busy { "connecting..." }
                            else if status.ready { "connected" } else { "disconnected" };
                        let name = match (kind, compact) {
                            (tomlook::connectors::Kind::Webiq, true) => "Web",
                            (tomlook::connectors::Kind::Github, true) => "GH",
                            _ => kind.label(),
                        };
                        if connection_status(ui, kind.server(), name,
                            ConnectionState::of(status.ready, status.busy || status.forgetting),
                            &format!("{}\n{}", language.text(&format!("{}: {label}", kind.label())), language.text(&status.message))).clicked() {
                            self.connection_panel = Some(ConnectionPanel::External(kind));
                        }
                    }
                    if let Some(staged) = &self.update_status.staged
                        && status_item(ui, "status-update", StatusIcon::Update, if compact { "" } else { &staged.version },
                            ConnectionState::Ready, &format!("{}\n{}", language.text(&format!("Update {} ready", staged.version)),
                            language.text("A verified update is ready. Restart from Settings, or it is installed when you exit Tomlook."))).clicked() {
                        self.preferences_open = true;
                        self.preferences_section = PreferencesSection::Updates;
                    }
                    self.setup_indicator(ui);
                });
            });
        });
    }

    pub(super) fn visible_workiq_range(&self) -> Result<tomlook::workiq::connection::Range, String> {
        let days = self.visible_days();
        tomlook::workiq::connection::Range::new(
            *days.first().ok_or("Calendar view has no first day")?,
            *days.last().ok_or("Calendar view has no last day")?,
        )
    }

    pub(super) fn refresh_workiq(&mut self) {
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

    pub(super) fn open_ai_connection(&mut self) {
        self.connection_panel = Some(ConnectionPanel::Ai);
        if !self.auto_status.busy() && self.ai_command(ai::Command::CheckAuto) {
            self.auto_status = tomlook::decision::Status::new(tomlook::decision::Phase::Checking, "Checking Foundry Azure sign-in...");
        }
    }

    /// The AI dialog's isolated browser sign-in; the setup checklist calls the same flow.
    pub(super) fn sign_in_ai(&mut self) {
        if !self.ai_login_busy && !self.ai_ready && self.ai_command(ai::Command::SignInAi) {
            self.ai_login_busy = true;
            self.ai_login_status = None;
            self.connection = "AI browser sign-in queued...".into();
        }
    }

    fn msx_pending(&mut self, phase: tomlook::msx::Phase, message: &str) {
        self.msx_status.phase = phase;
        self.msx_status.message = message.into();
        self.msx_status.device_code = None;
    }

    /// MSX (Dataverse) and Azure consumption (Power BI). Every sign-in is an explicit click.
    fn msx_connection(&mut self, ui: &mut egui::Ui) {
        use tomlook::msx::Phase;
        let language = self.preferences.language;
        let status = self.msx_status.clone();
        ui.strong(language.text("MSX · Dataverse and Power BI"));
        ui.label(language.text(if status.message.is_empty() {
            status.label()
        } else {
            &status.message
        }));
        egui::Grid::new("msx-connection")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.weak(language.text("Microsoft tenant"));
                ui.monospace(&self.preferences.msx_tenant);
                ui.end_row();
                ui.weak(language.text("Signed-in Azure account"));
                match &status.account {
                    Some(account) => ui.label(account),
                    None => ui.label(language.text("Not known yet")),
                };
                ui.end_row();
                for (label, service) in [
                    ("Dataverse (MSX)", &status.dataverse),
                    ("Power BI (Azure consumption)", &status.powerbi),
                ] {
                    ui.weak(language.text(label));
                    let state = ui.label(language.text(service.phase.label()));
                    if !service.message.is_empty() {
                        state.on_hover_text(language.text(&service.message));
                    }
                    ui.end_row();
                }
            });
        if let Some(code) = &status.device_code {
            ui.horizontal(|ui| {
                ui.monospace(code);
                if ui.button(language.text("Copy code")).clicked() {
                    ui.ctx().copy_text(code.clone());
                }
            });
            ui.hyperlink_to(
                language.text("Open Microsoft device sign-in"),
                tomlook::decision::DEVICE_LOGIN_URL,
            );
        }
        if status.busy() {
            ui.horizontal(|ui| {
                if status.device_code.is_none() {
                    ui.spinner();
                }
                if ui.button(language.text("Cancel")).clicked() {
                    self.ai_command(ai::Command::CancelMsx);
                }
            });
        } else if status.phase != Phase::Disabled {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(language.text("Sign in to MSX"))
                    .on_hover_text(language.text(
                        "Opens Microsoft sign-in through Azure CLI (browser or Windows prompt).",
                    ))
                    .clicked()
                    && self.ai_command(ai::Command::SignInMsx { device_code: false })
                {
                    self.msx_pending(Phase::SigningIn, tomlook::msx::BROWSER_PROMPT);
                }
                if ui
                    .button(language.text("Use device code"))
                    .on_hover_text(language.text(
                        "Alternative sign-in with a code on another device; your tenant may block it.",
                    ))
                    .clicked()
                    && self.ai_command(ai::Command::SignInMsx { device_code: true })
                {
                    self.msx_pending(Phase::SigningIn, tomlook::msx::DEVICE_REQUEST);
                }
                if ui.button(language.text("Check connection")).clicked()
                    && self.ai_command(ai::Command::CheckMsx)
                {
                    self.msx_pending(Phase::Checking, tomlook::msx::CHECKING);
                }
                if ui
                    .button(language.text("Disconnect"))
                    .on_hover_text(language.text(
                        "Forgets Tomlook's in-memory MSX tokens; your Azure CLI sign-in stays.",
                    ))
                    .clicked()
                    && self.ai_command(ai::Command::DisconnectMsx)
                {
                    self.msx_pending(Phase::NotConnected, tomlook::msx::DISCONNECTED);
                }
            });
        }
        ui.add_space(6.0);
        ui.weak(language.text(
            "Uses your Azure CLI sign-in; may change Azure CLI's default account.",
        ));
        ui.weak(language.text(
            "Read-only: Tomlook never creates or changes MSX records. Tokens stay in memory.",
        ));
    }

    pub(super) fn connection_dialog(&mut self, context: &egui::Context) {
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
            ConnectionPanel::Msx => "MSX connection",
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
                } else if !self.ai_ready && ui.button(language.text("Sign in")).clicked() {
                    self.sign_in_ai();
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
            } else if panel == ConnectionPanel::Msx {
                self.msx_connection(ui);
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
}

pub(super) fn version_label(ui: &mut egui::Ui, language: tomlook::i18n::Language) {
    ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).size(11.0).weak())
        .on_hover_text(language.text("Current version"));
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectionState { Ready, Busy, Offline, Unknown }

impl ConnectionState {
    fn of(ready: bool, busy: bool) -> Self {
        if busy { Self::Busy } else if ready { Self::Ready } else { Self::Offline }
    }

    fn color(self, ui: &egui::Ui) -> Color32 {
        match self {
            Self::Offline => if ui.visuals().dark_mode {
                Color32::from_rgb(194, 137, 132)
            } else {
                Color32::from_rgb(153, 78, 73)
            },
            Self::Ready | Self::Busy | Self::Unknown => ui.visuals().weak_text_color(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum StatusIcon { Check, Cross, Busy, Unknown, Calendar, Sync, Search, Clock, Alert, Update, Setup }

fn short_count(count: usize) -> String {
    if count >= 1_000_000 { format!("{:.1}m", count as f64 / 1_000_000.0) }
    else if count >= 10_000 { format!("{}k", count / 1_000) }
    else { count.to_string() }
}

fn connection_status(ui: &mut egui::Ui, id: &str, name: &str, state: ConnectionState, hint: &str) -> egui::Response {
    let icon = match state {
        ConnectionState::Ready => StatusIcon::Check,
        ConnectionState::Busy => StatusIcon::Busy,
        ConnectionState::Offline => StatusIcon::Cross,
        ConnectionState::Unknown => StatusIcon::Unknown,
    };
    status_item(ui, id, icon, name, state, hint)
}

pub(super) fn status_item(ui: &mut egui::Ui, id: &str, icon: StatusIcon, value: &str,
    state: ConnectionState, hint: &str,
) -> egui::Response {
    const ICON_SIZE: f32 = 10.0;
    const ICON_GAP: f32 = 6.0;
    const PADDING: f32 = 2.0;
    let color = state.color(ui);
    let galley = ui.painter().layout_no_wrap(value.to_owned(), egui::FontId::proportional(11.0), color);
    let width = PADDING * 2.0 + ICON_SIZE + if value.is_empty() { 0.0 } else { ICON_GAP + galley.size().x };
    let response = status_button(ui, id, egui::Button::new("").min_size(Vec2::new(width.max(18.0), 24.0)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), hint));
    let rect = egui::Rect::from_center_size(
        egui::pos2(response.rect.left() + PADDING + ICON_SIZE / 2.0, response.rect.center().y),
        Vec2::splat(ICON_SIZE),
    );
    let painter = ui.painter();
    if !value.is_empty() {
        painter.galley(
            egui::pos2(rect.right() + ICON_GAP, response.rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    let point = |x, y| rect.min + Vec2::new(x * rect.width(), y * rect.height());
    let stroke = Stroke::new(1.3, color);
    let line = |a, b| { painter.line_segment([a, b], stroke); };
    match icon {
        StatusIcon::Check => { line(point(0.1, 0.5), point(0.4, 0.8)); line(point(0.4, 0.8), point(0.9, 0.2)); }
        StatusIcon::Cross => { line(point(0.2, 0.2), point(0.8, 0.8)); line(point(0.2, 0.8), point(0.8, 0.2)); }
        StatusIcon::Unknown => { painter.circle_stroke(rect.center(), 3.5, stroke); }
        StatusIcon::Busy | StatusIcon::Sync => {
            let points = (0..=12).map(|step| {
                let angle = (step as f32 / 12.0 * 1.5 + 0.1) * std::f32::consts::PI;
                rect.center() + Vec2::new(angle.cos(), angle.sin()) * 4.0
            }).collect();
            painter.add(egui::Shape::line(points, stroke));
            line(point(0.55, 0.0), point(0.8, 0.1));
            line(point(0.8, 0.1), point(0.7, 0.4));
        }
        StatusIcon::Calendar | StatusIcon::Setup => {
            painter.rect_stroke(rect.shrink(0.5), 1, stroke, egui::StrokeKind::Inside);
            line(point(0.2, 0.3), point(0.8, 0.3));
            line(point(0.2, 0.55), point(0.8, 0.55));
            if matches!(icon, StatusIcon::Setup) { line(point(0.2, 0.8), point(0.6, 0.8)); }
        }
        StatusIcon::Search => { painter.circle_stroke(point(0.4, 0.4), 3.0, stroke); line(point(0.65, 0.65), point(1.0, 1.0)); }
        StatusIcon::Clock => {
            painter.circle_stroke(rect.center(), 4.5, stroke);
            line(point(0.5, 0.2), point(0.5, 0.5)); line(point(0.5, 0.5), point(0.75, 0.65));
        }
        StatusIcon::Alert => {
            painter.add(egui::Shape::closed_line(vec![point(0.5, 0.0), point(1.0, 0.95), point(0.0, 0.95)], stroke));
            line(point(0.5, 0.3), point(0.5, 0.6)); painter.circle_filled(point(0.5, 0.8), 0.7, color);
        }
        StatusIcon::Update => {
            line(point(0.5, 0.1), point(0.5, 0.75));
            line(point(0.2, 0.4), point(0.5, 0.1)); line(point(0.5, 0.1), point(0.8, 0.4));
            line(point(0.1, 0.9), point(0.9, 0.9));
        }
    }
    response.on_hover_text(hint)
}

#[cfg(test)]
pub(super) fn status_text(
    ui: &mut egui::Ui,
    identity: &str,
    label: impl Into<egui::WidgetText>,
) -> egui::Response {
    status_button(ui, identity, egui::Button::new(label).min_size(Vec2::new(18.0, 24.0)))
}

fn status_button(ui: &mut egui::Ui, identity: &str, button: egui::Button<'_>) -> egui::Response {
    let response = ui
        .push_id(identity, |ui| {
            ui.add(button.frame(false))
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
