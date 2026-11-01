use super::*;

impl Tomlook {
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

pub(super) fn status_text(
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
