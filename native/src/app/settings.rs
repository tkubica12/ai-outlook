use super::*;
use super::feedback::Panel;
use super::palette::command_matches_in;

pub(super) fn insight_state(state: tomlook::scheduler::State) -> &'static str {
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

pub(super) const ACTIVITY_ROW_HEIGHT: f32 = 34.0;

pub(super) fn activity_row(
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum PreferencesSection {
    Display,
    Notifications,
    Prompts,
    Updates,
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

impl Tomlook {
    pub(super) fn preferences_dialog(&mut self, context: &egui::Context) {
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
                            (PreferencesSection::Notifications, "Notifications"),
                            (PreferencesSection::Prompts, "AI prompts"),
                            (PreferencesSection::Updates, "Updates"),
                        ] {
                            if ui.add_sized([ui.available_width(), 36.0],
                                egui::Button::new(language.text(label))
                                    .selected(self.preferences_section == section)).clicked() {
                                self.preferences_section = section;
                                if section == PreferencesSection::Notifications
                                    && let Some(reminders) = &self.reminders {
                                    reminders.refresh();
                                }
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
                                self.prompt_feedback(ui);
                            }
                            PreferencesSection::Notifications => self.notification_preferences(ui),
                            PreferencesSection::Updates => self.update_preferences(ui),
                        }
                    });
                });
            });
        self.preferences_open = open;
    }

    fn notification_preferences(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let status = self.reminder_status.clone();
        let time = |at: chrono::DateTime<chrono::Utc>| {
            at.with_timezone(&Local).format("%d.%m.%Y %H:%M").to_string()
        };
        ui.heading(language.text("Notifications"));
        ui.add_space(16.0);
        ui.strong(language.text("Meeting reminders"));
        let mut changed = ui
            .checkbox(
                &mut self.preferences.meeting_reminders,
                language.text("Remind me before meetings"),
            )
            .changed();
        ui.add_enabled_ui(self.preferences.meeting_reminders, |ui| {
            ui.add_space(8.0);
            ui.label(language.text("Minutes before the start"));
            ui.horizontal(|ui| {
                for minutes in tomlook::reminders::MINUTES {
                    changed |= ui
                        .selectable_value(
                            &mut self.preferences.reminder_minutes,
                            minutes,
                            language.text(&format!("{minutes} min")),
                        )
                        .changed();
                }
            });
            ui.add_space(4.0);
            changed |= ui
                .checkbox(
                    &mut self.preferences.remind_tentative,
                    language.text("Also remind me of tentatively accepted meetings"),
                )
                .changed();
        });
        if changed {
            self.save_preferences();
        }
        ui.add_space(8.0);
        ui.weak(language.text("Reminders cover timed meetings you organize or have accepted. Unanswered, declined, cancelled, free and followed meetings are skipped."));
        ui.weak(language.text("A reminder shows up to three points from saved Insights and never starts AI research."));
        ui.add_space(16.0);
        if let Some(reason) = &status.disabled {
            ui.weak(language.text(reason));
        } else if !self.preferences.meeting_reminders {
            ui.weak(language.text("Meeting reminders are turned off."));
        } else if let Some(next) = status.next {
            ui.label(language.text(&format!("Next reminder: {}", time(next))));
        } else {
            ui.weak(language.text("No upcoming reminders."));
        }
        match status.windows.map(tomlook::reminders::WindowsSetting::message) {
            Some(Some(message)) => {
                ui.colored_label(ui.visuals().warn_fg_color, language.text(message));
            }
            Some(None) => {
                ui.weak(language.text("Windows notifications are on for Tomlook."));
            }
            None => {}
        }
        if let Some(at) = status.last_reminder {
            ui.weak(language.text(&format!("Last reminder shown at {}.", time(at))));
        }
        ui.add_space(12.0);
        if ui
            .add_enabled(
                self.reminders.is_some(),
                egui::Button::new(language.text("Send test notification")),
            )
            .clicked()
            && let Some(reminders) = &self.reminders
        {
            reminders.test(language);
        }
        ui.weak(language.text("Shows a sample reminder with fictional content."));
        if let Some(at) = status.last_test {
            ui.weak(language.text(&format!("Test notification sent at {}.", time(at))));
        }
        if let Some(error) = &status.error {
            ui.colored_label(ui.visuals().error_fg_color, language.text(error));
        }
        self.morning_preferences(ui, &status);
        self.weekly_preferences(ui, &status);
        ui.add_space(20.0);
        ui.weak(language.text("Tomlook is listed in Windows Settings > System > Notifications, where you can turn its notifications off or change how they appear."));
    }

    pub(super) fn morning_preferences(&mut self, ui: &mut egui::Ui, status: &tomlook::reminders::Status) {
        let language = self.preferences.language;
        ui.add_space(20.0);
        ui.separator();
        ui.add_space(12.0);
        ui.strong(language.text("Morning overview"));
        let mut changed = ui
            .checkbox(
                &mut self.preferences.morning_overview,
                language.text("Show my day in the morning"),
            )
            .changed();
        ui.add_enabled_ui(self.preferences.morning_overview, |ui| {
            ui.add_space(8.0);
            ui.label(language.text("Time"));
            ui.horizontal_wrapped(|ui| {
                for minutes in tomlook::myday::TIMES {
                    changed |= ui
                        .selectable_value(
                            &mut self.preferences.morning_overview_time,
                            minutes,
                            format!("{:02}:{:02}", minutes / 60, minutes % 60),
                        )
                        .changed();
                }
            });
            ui.add_space(4.0);
            changed |= ui
                .checkbox(
                    &mut self.preferences.morning_overview_weekends,
                    language.text("Also on weekends"),
                )
                .changed();
        });
        if changed {
            self.save_preferences();
        }
        ui.add_space(8.0);
        ui.weak(language.text("Once a day (on workdays unless weekends are included) with today's meetings, conflicts, meetings without Insights and tasks. If Tomlook is not running at that time, it appears at the next opportunity before 12:00. It stays quiet when today's calendar is not synced or there is nothing to act on."));
        if self.preview {
            ui.weak(language.text("The morning overview does not run in the offline demo; the test uses the demo's fictional calendar."));
        } else if !self.preferences.morning_overview {
            ui.weak(language.text("Morning overview is turned off."));
        }
        if let Some(at) = status.last_morning {
            ui.weak(language.text(&format!(
                "Last morning overview shown: {}",
                at.with_timezone(&Local).format("%d.%m.%Y %H:%M")
            )));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(language.text("Show now")).clicked() {
                self.open_myday();
            }
            if ui
                .add_enabled(
                    self.reminders.is_some() && !self.loading,
                    egui::Button::new(language.text("Send test")),
                )
                .clicked()
            {
                self.send_morning_test();
            }
        });
        ui.weak(language.text("Shows the My day notification with the current counts."));
        if self.preview
            && self.demo_screenshot.is_some()
            && self.demo_scene == tomlook::showcase::Scene::Notifications
        {
            ui.scroll_to_cursor(Some(egui::Align::Max));
        }
    }

    pub(super) fn weekly_preferences(&mut self, ui: &mut egui::Ui, status: &tomlook::reminders::Status) {
        let language = self.preferences.language;
        ui.add_space(20.0);
        ui.separator();
        ui.add_space(12.0);
        ui.strong(language.text("Weekly review"));
        let mut changed = ui
            .checkbox(
                &mut self.preferences.weekly_review,
                language.text("Show a weekly review notification"),
            )
            .changed();
        ui.add_enabled_ui(self.preferences.weekly_review, |ui| {
            ui.add_space(8.0);
            ui.label(language.text("Day"));
            ui.horizontal_wrapped(|ui| {
                for (day, label) in [(5, "Friday (this week)"), (1, "Monday (previous week)")] {
                    changed |= ui
                        .selectable_value(
                            &mut self.preferences.weekly_review_day,
                            day,
                            language.text(label),
                        )
                        .changed();
                }
            });
            ui.add_space(4.0);
            ui.label(language.text("Time"));
            ui.horizontal_wrapped(|ui| {
                for minutes in tomlook::weekly::TIMES {
                    changed |= ui
                        .selectable_value(
                            &mut self.preferences.weekly_review_time,
                            minutes,
                            format!("{:02}:{:02}", minutes / 60, minutes % 60),
                        )
                        .changed();
                }
            });
        });
        if changed {
            self.save_preferences();
        }
        ui.add_space(8.0);
        ui.weak(language.text("Once a week with the number of meetings, meeting time, conflicts and meetings without an agenda. If Tomlook is not running at that time, it appears later the same day. It stays quiet unless at least 4 of the week's 5 workdays are synced."));
        if self.preview {
            ui.weak(language.text("The weekly review notification does not run in the offline demo; the test uses the demo's fictional calendar."));
        } else if !self.preferences.weekly_review {
            ui.weak(language.text("Weekly review is turned off."));
        }
        if let Some(at) = status.last_weekly {
            ui.weak(language.text(&format!(
                "Last weekly review shown: {}",
                at.with_timezone(&Local).format("%d.%m.%Y %H:%M")
            )));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(language.text("Show now")).clicked() {
                let schedule = tomlook::weekly::Schedule::from_preferences(&self.preferences);
                let week = tomlook::weekly::reviewed_week(&schedule, self.weekly_now().date_naive());
                self.open_weekly(Some(week));
            }
            if ui
                .add_enabled(
                    self.reminders.is_some() && !self.loading,
                    egui::Button::new(language.text("Send test")),
                )
                .clicked()
            {
                self.send_weekly_test();
            }
        });
        ui.weak(language.text("Shows the weekly review notification with the current counts."));
    }

    fn update_preferences(&mut self, ui: &mut egui::Ui) {
        use tomlook::updater::Activity;
        let language = self.preferences.language;
        let status = self.update_status.clone();
        ui.heading(language.text("Updates"));
        ui.add_space(16.0);
        ui.strong(language.text("Current version"));
        ui.label(env!("CARGO_PKG_VERSION"));
        ui.add_space(16.0);
        if let Some(reason) = &status.disabled {
            ui.weak(language.text(reason));
            return;
        }
        let mut automatic = self.preferences.automatic_update_check;
        if ui
            .checkbox(
                &mut automatic,
                language.text("Check for updates automatically once a day"),
            )
            .changed()
        {
            self.set_automatic_updates(automatic);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    status.activity.is_none(),
                    egui::Button::new(language.text("Check now")),
                )
                .clicked()
            {
                self.check_for_updates();
            }
            if status.activity.is_some() {
                ui.spinner();
            }
        });
        match status.checked_at.or(self.preferences.update_checked_at) {
            Some(checked) => ui.weak(language.text(&format!(
                "Last checked: {}",
                checked.with_timezone(&Local).format("%d.%m.%Y %H:%M")
            ))),
            None => ui.weak(language.text("Not checked yet.")),
        };
        ui.add_space(12.0);
        match &status.activity {
            Some(Activity::Checking) => {
                ui.label(language.text("Checking GitHub Releases..."));
            }
            Some(Activity::Downloading {
                version,
                received,
                total,
            }) => {
                ui.label(language.text(&format!(
                    "Downloading {version}: {} of {} MB",
                    megabytes(*received),
                    megabytes(*total)
                )));
                ui.add(
                    egui::ProgressBar::new(*received as f32 / (*total).max(1) as f32)
                        .desired_width(360.0),
                );
            }
            Some(Activity::Verifying { version }) => {
                ui.label(language.text(&format!("Verifying {version}...")));
            }
            None => {}
        }
        let latest = status
            .latest
            .clone()
            .or_else(|| Some(self.preferences.update_latest_version.clone()))
            .filter(|latest| !latest.is_empty());
        if let Some(staged) = &status.staged {
            ui.strong(language.text(&format!(
                "Tomlook {} is downloaded and verified.",
                staged.version
            )));
            ui.horizontal(|ui| {
                if ui.button(language.text("Restart now")).clicked() {
                    self.restart_for_update(ui.ctx());
                }
                if ui.button(language.text("Release notes")).clicked()
                    && let Err(error) = tomlook::browser::open(&staged.notes_url)
                {
                    self.update_status.error = Some(error);
                }
            });
            ui.weak(language.text("Otherwise the update is installed when you exit Tomlook."));
        } else if let Some(offer) = &status.offer {
            ui.strong(language.text(&format!("Tomlook {} is available.", offer.version)));
            ui.weak(language.text(&offer.reason));
            if ui.button(language.text("Open release page")).clicked()
                && let Err(error) = tomlook::browser::open(&offer.notes_url)
            {
                self.update_status.error = Some(error);
            }
        } else if status.activity.is_none()
            && status.error.is_none()
            && latest.is_some_and(|latest| {
                !tomlook::updater::is_newer(&latest, env!("CARGO_PKG_VERSION")).unwrap_or(true)
            })
        {
            ui.label(language.text("Tomlook is up to date."));
        }
        if let Some(error) = &status.error {
            ui.colored_label(ui.visuals().error_fg_color, language.text(error));
        }
        ui.add_space(20.0);
        ui.weak(language.text("Tomlook makes at most one unauthenticated request per day to the GitHub Releases API. No account, calendar or usage data is sent."));
        ui.weak(language.text("An update is installed only with a valid Tomlook release signature and matching SHA-256 hashes."));
    }

    pub(super) fn overlays(&mut self, context: &egui::Context) {
        let language = self.preferences.language;
        self.preferences_dialog(context);
        self.meeting_action_dialog(context);
        self.event_creation_dialog(context);
        self.find_time_dialog(context);
        self.blocker_release_dialog(context);
        self.assistant_dialog(context);
        self.tasks_window(context);
        self.myday_window(context);
        self.weekly_window(context);
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
                                        match &record.hold {
                                            Some(hold) => for slot in &hold.slots {
                                                ui.weak(slot.label(language));
                                            },
                                            None => { ui.weak(format!("{} - {}", record.draft.start.with_timezone(&Local).format("%d.%m. %H:%M"),
                                                record.draft.end.with_timezone(&Local).format("%d.%m. %H:%M"))); }
                                        }
                                        if !record.draft.attendees.is_empty() {
                                            ui.weak(format!("{}: {}", language.text("Invited"), record.draft.attendees.iter()
                                                .map(|attendee| attendee.label()).collect::<Vec<_>>().join(", ")));
                                        }
                                    }
                                });
                            });
                    }
                    self.proposal_messages(ui);
                    let inbox = self.activity.recommendations.clone();
                    let mut applied = inbox.values().filter(|record|
                        record.account == self.calendar_owner.as_deref().unwrap_or("")
                        && record.suggestion.kind == tomlook::recommendations::Kind::Manual
                        && record.state == tomlook::recommendations::State::Applied).collect::<Vec<_>>();
                    applied.sort_by_key(|record| std::cmp::Reverse(record.created_at));
                    if !applied.is_empty() {
                        egui::CollapsingHeader::new(language.text("Applied changes")).show(ui, |ui| {
                            egui::ScrollArea::vertical().id_salt("applied-assistant-changes").max_height(140.0).show(ui, |ui| {
                                for record in applied.iter().take(10) {
                                    ui.strong(&record.event_title);
                                    ui.weak(language.text(&record.message));
                                }
                            });
                        });
                    }
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
                        let document_feedback = self.document_feedback();
                        ui.horizontal(|ui| {
                            ui.heading(language.text(&self.calendar.events[index].title));
                            if let Some((output, target)) = &document_feedback {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    self.feedback_thumbs(ui, output, target, Panel::Expanded);
                                });
                            }
                        });
                        if let Some((output, target)) = &document_feedback {
                            self.feedback_reasons(ui, output, target, Panel::Expanded);
                        }
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
}
