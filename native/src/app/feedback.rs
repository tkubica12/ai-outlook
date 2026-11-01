use super::*;
use tomlook::feedback::{self as model, DOCUMENT, Output, Rating, Reason, Target};
use tomlook::prompts::Kind;

/// Where a "not helpful" reason panel opened; the same rating can be shown in several places.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Panel {
    Details,
    Expanded,
}

pub(super) struct Draft {
    pub(super) target: Target,
    pub(super) panel: Panel,
    pub(super) reasons: BTreeSet<Reason>,
    pub(super) comment: String,
}

#[derive(Default)]
pub(super) enum Suggestion {
    #[default]
    Idle,
    Running {
        id: u64,
        kind: Kind,
    },
    Ready(Box<model::Suggestion>),
    Failed {
        kind: Kind,
        message: String,
    },
}

/// Feedback state of the open document and the Settings summary; disk work runs on the worker.
#[derive(Default)]
pub(super) struct FeedbackUi {
    pub(super) document: Option<model::DocumentId>,
    pub(super) loaded: bool,
    pub(super) entries: Vec<model::Entry>,
    pub(super) draft: Option<Draft>,
    pub(super) summary: Option<Arc<model::Summary>>,
    pub(super) summary_error: Option<String>,
    pub(super) summary_requested: Option<String>,
    pub(super) summary_stale: bool,
    pub(super) suggestion: Suggestion,
    pub(super) request_id: u64,
    pub(super) demo_seeded: bool,
}

fn paint_thumb(painter: &egui::Painter, rect: egui::Rect, up: bool, filled: bool, color: Color32) {
    let scale = rect.width().min(rect.height()) / 16.0;
    let origin = rect.center() - Vec2::splat(8.0 * scale);
    let point = |x: f32, y: f32| origin + Vec2::new(x, if up { y } else { 16.0 - y }) * scale;
    let stroke = Stroke::new(1.3, color);
    let cuff = egui::Rect::from_two_pos(point(1.5, 7.5), point(4.5, 14.5));
    if filled {
        painter.rect_filled(cuff, 1.0, color);
    } else {
        painter.rect_stroke(cuff, 1.0, stroke, egui::StrokeKind::Middle);
    }
    painter.add(egui::Shape::closed_line(
        vec![
            point(6.0, 7.5),
            point(8.7, 2.0),
            point(9.8, 2.2),
            point(10.4, 3.4),
            point(9.7, 6.4),
            point(13.4, 6.4),
            point(14.5, 7.6),
            point(13.5, 13.6),
            point(12.5, 14.5),
            point(6.0, 14.5),
        ],
        stroke,
    ));
}

/// Small toggle with a vector thumb; muted until hovered, accent when selected.
fn thumb(
    ui: &mut egui::Ui,
    up: bool,
    selected: bool,
    accent: Color32,
    language: tomlook::i18n::Language,
) -> egui::Response {
    let label = if up {
        language.text("Helpful")
    } else {
        language.text("Not helpful")
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label.to_string())
    });
    if ui.is_rect_visible(rect) {
        let active = response.hovered() || response.has_focus();
        let color = if selected {
            accent
        } else if active {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color().gamma_multiply(0.85)
        };
        if selected {
            ui.painter().rect_filled(rect, 5.0, accent.gamma_multiply(0.18));
        } else if active {
            ui.painter().rect_filled(rect, 5.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(rect, 5.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
        }
        paint_thumb(ui.painter(), rect.shrink(3.0), up, selected, color);
    }
    let hint = if selected {
        format!("{label}\n{}", language.text("Click again to clear the rating."))
    } else {
        label.into_owned()
    };
    response.on_hover_text(hint)
}

fn chip(ui: &mut egui::Ui, text: String) {
    egui::Frame::new()
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small());
        });
}

impl Tomlook {
    /// Feedback is bound to the calendar owner; the isolated demo uses its own fixed account.
    pub(super) fn feedback_account(&self) -> Option<String> {
        if self.preview {
            Some(model::PREVIEW_ACCOUNT.into())
        } else {
            self.calendar_owner.clone()
        }
    }

    /// Identity of the selected meeting's displayed document; loads its saved ratings once.
    pub(super) fn feedback_output(&mut self, payload: &serde_json::Value) -> Option<Output> {
        let account = self.feedback_account()?;
        let event = self.selected.and_then(|index| self.calendar.events.get(index))?;
        let output = Output::new(
            &account,
            event,
            payload,
            self.preferences.language,
            &self.preferences.prompts,
        )?;
        if self.feedback.document.as_ref() != Some(&output.id) {
            self.feedback.document = Some(output.id.clone());
            self.feedback.entries.clear();
            self.feedback.loaded = false;
            self.feedback.draft = None;
            if self.worker.is_some() {
                self.command(Command::LoadFeedback(Box::new(output.id.clone())));
            }
        }
        Some(output)
    }

    pub(super) fn document_feedback(&mut self) -> Option<(Output, Target)> {
        let payload = self.briefing.take()?;
        let result = self.feedback_output(&payload).and_then(|output| {
            output
                .target(&payload, DOCUMENT)
                .map(|target| (output, target))
        });
        self.briefing = Some(payload);
        result
    }

    pub(super) fn feedback_thumbs(
        &mut self,
        ui: &mut egui::Ui,
        output: &Output,
        target: &Target,
        panel: Panel,
    ) {
        let language = self.preferences.language;
        let accent = self.accent();
        let rating = model::current(&self.feedback.entries, &target.key).map(|entry| entry.rating);
        let drafting = self
            .feedback
            .draft
            .as_ref()
            .is_some_and(|draft| draft.target.key == target.key);
        let (mut up, mut down) = (false, false);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let down_selected = rating == Some(Rating::Down) || drafting;
            // Right-to-left rows place the first widget rightmost; keep the visual order up, down.
            if ui.layout().prefer_right_to_left() {
                down = thumb(ui, false, down_selected, accent, language).clicked();
                up = thumb(ui, true, rating == Some(Rating::Up), accent, language).clicked();
            } else {
                up = thumb(ui, true, rating == Some(Rating::Up), accent, language).clicked();
                down = thumb(ui, false, down_selected, accent, language).clicked();
            }
        });
        if up {
            self.feedback.draft = None;
            if rating == Some(Rating::Up) {
                self.clear_feedback(target);
            } else {
                self.save_feedback(output, target, Rating::Up, BTreeSet::new(), "");
            }
        }
        if down {
            if rating == Some(Rating::Down) {
                self.feedback.draft = None;
                self.clear_feedback(target);
            } else if drafting {
                self.feedback.draft = None;
            } else {
                self.feedback.draft = Some(Draft {
                    target: target.clone(),
                    panel,
                    reasons: BTreeSet::new(),
                    comment: String::new(),
                });
            }
        }
    }

    /// Inline reason chips and an optional comment for a "not helpful" rating.
    pub(super) fn feedback_reasons(
        &mut self,
        ui: &mut egui::Ui,
        output: &Output,
        target: &Target,
        panel: Panel,
    ) {
        let language = self.preferences.language;
        let Some(draft) = self
            .feedback
            .draft
            .as_mut()
            .filter(|draft| draft.target.key == target.key && draft.panel == panel)
        else {
            return;
        };
        let (mut save, mut cancel) = (false, false);
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(6)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.label(RichText::new(language.text("What was not helpful?")).strong());
                ui.horizontal_wrapped(|ui| {
                    for reason in Reason::ALL {
                        let selected = draft.reasons.contains(&reason);
                        if ui
                            .add(egui::Button::new(RichText::new(language.text(reason.label())).small())
                                .selected(selected))
                            .clicked()
                        {
                            if selected {
                                draft.reasons.remove(&reason);
                            } else {
                                draft.reasons.insert(reason);
                            }
                        }
                    }
                });
                let label = ui.weak(language.text("Comment (optional, up to 500 characters)"));
                ui.add(
                    egui::TextEdit::multiline(&mut draft.comment)
                        .id(Id::new(("feedback-comment", panel, &target.key.section)))
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .char_limit(model::COMMENT_LIMIT),
                )
                .labelled_by(label.id);
                ui.horizontal(|ui| {
                    save = ui.button(language.text("Save")).clicked();
                    cancel = ui.button(language.text("Cancel")).clicked();
                    ui.weak(format!(
                        "{}/{}",
                        draft.comment.trim().chars().count(),
                        model::COMMENT_LIMIT
                    ));
                });
                ui.weak(language.text("Feedback stays on this computer and is never sent automatically."));
            });
        ui.add_space(4.0);
        if save {
            if let Some(draft) = self.feedback.draft.take() {
                self.save_feedback(output, target, Rating::Down, draft.reasons, &draft.comment);
            }
        } else if cancel {
            self.feedback.draft = None;
        }
    }

    pub(super) fn save_feedback(
        &mut self,
        output: &Output,
        target: &Target,
        rating: Rating,
        reasons: BTreeSet<Reason>,
        comment: &str,
    ) {
        match output.entry(target, rating, reasons, comment, chrono::Utc::now()) {
            Ok(entry) => {
                if let Some(previous) = model::current(&self.feedback.entries, &entry.key) {
                    let created = previous.created_at;
                    self.feedback.entries.retain(|saved| saved.key != entry.key);
                    let mut entry = entry.clone();
                    entry.created_at = created.min(entry.updated_at);
                    self.feedback.entries.insert(0, entry);
                } else {
                    self.feedback.entries.insert(0, entry.clone());
                }
                self.feedback.summary_stale = true;
                self.command(Command::SaveFeedback(Box::new(entry)));
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn clear_feedback(&mut self, target: &Target) {
        self.feedback.entries.retain(|entry| entry.key != target.key);
        self.feedback.summary_stale = true;
        self.command(Command::ClearFeedback(Box::new(target.key.clone())));
    }

    pub(super) fn receive_feedback(
        &mut self,
        document: model::DocumentId,
        entries: Result<Vec<model::Entry>, String>,
    ) {
        if self.feedback.document.as_ref() != Some(&document) {
            return;
        }
        self.feedback.loaded = true;
        match entries {
            Ok(entries) => self.feedback.entries = entries,
            Err(error) => {
                self.feedback.entries.clear();
                self.error = Some(error);
            }
        }
    }

    pub(super) fn receive_feedback_summary(
        &mut self,
        account: String,
        result: Result<Arc<model::Summary>, String>,
    ) {
        if self.feedback.summary_requested.as_deref() == Some(account.as_str()) {
            self.feedback.summary_requested = None;
        }
        match result {
            Ok(summary) => {
                self.feedback.summary = Some(summary);
                self.feedback.summary_error = None;
            }
            Err(error) => self.feedback.summary_error = Some(error),
        }
    }

    pub(super) fn receive_prompt_suggestion(
        &mut self,
        id: u64,
        result: Result<model::Suggestion, String>,
    ) {
        if let Suggestion::Running { id: running, kind } = self.feedback.suggestion
            && running == id
        {
            self.feedback.suggestion = match result {
                Ok(suggestion) if suggestion.kind == kind => Suggestion::Ready(Box::new(suggestion)),
                Ok(_) => Suggestion::Failed {
                    kind,
                    message: "AI returned an invalid prompt suggestion; nothing was changed.".into(),
                },
                Err(message) => Suggestion::Failed { kind, message },
            };
        }
    }

    pub(super) fn stop_prompt_suggestion(&mut self) {
        if let Suggestion::Running { kind, .. } = self.feedback.suggestion {
            self.feedback.suggestion = Suggestion::Failed {
                kind,
                message: "AI stopped without a completed answer; provider outcome may be unknown".into(),
            };
        }
    }

    fn request_prompt_suggestion(&mut self, kind: Kind, stats: &model::KindSummary) {
        self.feedback.request_id += 1;
        let id = self.feedback.request_id;
        let current = self.prompt_draft.text(kind).to_owned();
        self.feedback.suggestion =
            match model::SuggestionRequest::new(id, kind, self.preferences.language, &current, stats) {
                Ok(request) => {
                    if self.ai_command(ai::Command::SuggestPrompt(Box::new(request))) {
                        Suggestion::Running { id, kind }
                    } else {
                        Suggestion::Failed {
                            kind,
                            message: self
                                .ai_error
                                .take()
                                .unwrap_or_else(|| "The AI worker is unavailable. Restart Tomlook.".into()),
                        }
                    }
                }
                Err(message) => Suggestion::Failed { kind, message },
            };
    }

    /// Settings > AI prompts: 90-day feedback summary and the reviewable suggestion flow.
    pub(super) fn prompt_feedback(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let kind = self.prompt_kind;
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        let heading = ui.strong(language.text("Feedback on AI output"));
        if self.preview
            && self.demo_screenshot.is_some()
            && self.demo_scene == tomlook::showcase::Scene::Prompts
        {
            heading.scroll_to_me(Some(egui::Align::Min));
        }
        if !matches!(kind, Kind::Preparation | Kind::Recap) {
            ui.weak(language.text("Feedback is collected for the Before the meeting and After the meeting prompts."));
            return;
        }
        ui.weak(language.text("Ratings from the last 90 days. They stay on this computer and are never sent automatically."));
        let Some(account) = self.feedback_account() else {
            ui.weak(language.text("Feedback is available once the calendar account is known."));
            return;
        };
        if self.worker.is_some()
            && self.feedback.summary_requested.is_none()
            && (self.feedback.summary_stale
                || self
                    .feedback
                    .summary
                    .as_ref()
                    .is_none_or(|summary| summary.account != account))
        {
            self.feedback.summary_stale = false;
            self.feedback.summary_requested = Some(account.clone());
            self.command(Command::FeedbackSummary(account.clone()));
        }
        if let Some(error) = &self.feedback.summary_error {
            ui.colored_label(ui.visuals().error_fg_color, language.text(error));
        }
        let Some(summary) = self
            .feedback
            .summary
            .clone()
            .filter(|summary| summary.account == account)
        else {
            ui.weak(language.text("Loading feedback..."));
            return;
        };
        let stats = summary.kind(kind).cloned().unwrap_or_default();
        let document = if kind == Kind::Recap {
            tomlook::changes::Document::Notes
        } else {
            tomlook::changes::Document::Insights
        };
        let accent = self.accent();
        ui.add_space(6.0);
        if stats.up + stats.down == 0 {
            ui.weak(language.text("No feedback in the last 90 days. Rate Insights or meeting notes with the thumb icons."));
        } else {
            ui.horizontal(|ui| {
                for (up, text) in [
                    (true, format!("{}: {}", language.text("Helpful"), stats.up)),
                    (false, format!("{}: {}", language.text("Not helpful"), stats.down)),
                ] {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                    paint_thumb(ui.painter(), rect.shrink(1.0), up, false, if up { accent } else { ui.visuals().text_color() });
                    ui.label(text);
                    ui.add_space(10.0);
                }
            });
            if !stats.reasons.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(language.text("Top reasons:"));
                    for (reason, count) in stats.reasons.iter().take(4) {
                        chip(ui, format!("{} · {count}", language.text(reason.label())));
                    }
                });
            }
            if !stats.hot_spots.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(language.text("Most criticized sections:"));
                    for (section, count) in stats.hot_spots.iter().take(4) {
                        chip(ui, format!("{} · {count}", language.text(model::section_label(document, section))));
                    }
                });
            }
            if !stats.comments.is_empty() {
                ui.add_space(6.0);
                ui.weak(language.text("Recent comments"));
                for comment in &stats.comments {
                    let date = comment
                        .meeting_start
                        .map(|start| start.with_timezone(&Local).format("%d.%m.%Y").to_string())
                        .unwrap_or_default();
                    ui.horizontal_top(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                        paint_thumb(ui.painter(), rect.shrink(1.0), comment.rating == Rating::Up, false,
                            ui.visuals().weak_text_color());
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(format!(
                                "{} · {date} · {}",
                                comment.meeting_title,
                                language.text(model::section_label(document, &comment.section))
                            )).small().weak()).truncate());
                            let text = if comment.text.chars().count() > 160 {
                                format!("{}…", comment.text.chars().take(159).collect::<String>())
                            } else {
                                comment.text.clone()
                            };
                            ui.label(text).on_hover_text(&comment.text);
                        });
                    });
                }
            }
        }
        self.prompt_suggestion(ui, kind, &stats);
    }

    fn prompt_suggestion(&mut self, ui: &mut egui::Ui, kind: Kind, stats: &model::KindSummary) {
        let language = self.preferences.language;
        let running = matches!(self.feedback.suggestion, Suggestion::Running { .. });
        let blocker = if running {
            None
        } else if stats.down == 0 {
            Some("A suggestion needs at least one negative rating.")
        } else if self.preview {
            Some("Offline demo never sends AI requests.")
        } else if !self.ai_ready || self.ai.is_none() {
            Some("Connect GitHub Copilot to suggest prompt improvements.")
        } else {
            None
        };
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let response = ui
                .add_enabled(
                    !running && blocker.is_none(),
                    egui::Button::new(language.text("Suggest prompt improvement")),
                )
                .on_hover_text(language.text("Sends the editor's prompt and summarized feedback to GitHub Copilot once (Thorough profile, no tools). Counts toward the daily AI limit."));
            if response.clicked() {
                self.request_prompt_suggestion(kind, stats);
            }
            if running {
                ui.spinner();
                ui.weak(language.text("Preparing a prompt suggestion..."));
            }
        });
        if let Some(blocker) = blocker {
            ui.weak(language.text(blocker));
        }
        let mut use_draft = false;
        let mut discard = false;
        match &self.feedback.suggestion {
            Suggestion::Failed { kind: failed, message } if *failed == kind => {
                ui.colored_label(ui.visuals().error_fg_color, language.text(message));
            }
            Suggestion::Ready(suggestion) if suggestion.kind == kind => {
                let accent = self.accent();
                let edited = suggestion.base != self.prompt_draft.text(kind);
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(8)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.strong(language.text("Suggested prompt (draft)"));
                        ui.weak(language.text("Use draft copies it into the editor above; nothing is saved until you click Save prompts."));
                        ui.add_space(6.0);
                        ui.label(RichText::new(language.text("Why these changes")).strong());
                        for item in &suggestion.rationale {
                            ui.horizontal_top(|ui| {
                                ui.label("•");
                                ui.label(item);
                            });
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new(language.text("Changes")).strong());
                        if edited {
                            ui.weak(language.text("The editor changed after this suggestion was requested; Use draft replaces the editor text."));
                        }
                        for (change, text) in model::diff(&suggestion.base, &suggestion.prompt) {
                            match change {
                                model::Change::Same => ui.label(RichText::new(text).weak()),
                                model::Change::Removed => ui.label(
                                    RichText::new(format!("− {text}"))
                                        .strikethrough()
                                        .color(ui.visuals().error_fg_color),
                                ),
                                model::Change::Added => {
                                    ui.label(RichText::new(format!("+ {text}")).color(accent))
                                }
                            };
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new(language.text("Suggested prompt")).strong());
                        egui::Frame::new()
                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                            .corner_radius(6)
                            .inner_margin(8)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(&suggestion.prompt);
                            });
                        ui.horizontal(|ui| {
                            use_draft = ui.button(language.text("Use draft")).clicked();
                            discard = ui.button(language.text("Discard")).clicked();
                        });
                    });
            }
            _ => {}
        }
        if use_draft {
            if let Suggestion::Ready(suggestion) = std::mem::take(&mut self.feedback.suggestion) {
                *self.prompt_draft.edit(kind) = suggestion.prompt;
            }
        } else if discard {
            self.feedback.suggestion = Suggestion::Idle;
        }
    }

    /// Advances the documented `--demo-scene` and reports when the frame can be captured.
    pub(super) fn demo_scene_ready(&mut self) -> bool {
        use tomlook::showcase::Scene;
        self.demo_frames = self.demo_frames.saturating_add(1);
        match self.demo_scene {
            Scene::Details => true,
            Scene::Feedback => {
                self.insights_expanded = true;
                let Some(payload) = self.briefing.take() else {
                    return false;
                };
                let output = self.feedback_output(&payload);
                let targets = output.as_ref().and_then(|output| {
                    Some((
                        output.target(&payload, DOCUMENT)?,
                        output.target(&payload, "talking_points")?,
                    ))
                });
                self.briefing = Some(payload);
                let (Some(output), Some((document, section))) = (output, targets) else {
                    return false;
                };
                if !self.feedback.loaded {
                    return false;
                }
                if !self.feedback.demo_seeded {
                    self.feedback.demo_seeded = true;
                    self.save_feedback(&output, &document, Rating::Up, BTreeSet::new(), "");
                }
                if self.feedback.draft.is_none() {
                    self.feedback.draft = Some(Draft {
                        target: section,
                        panel: Panel::Expanded,
                        reasons: [Reason::TooVague].into(),
                        comment: "Name the concrete trade-offs to compare.".into(),
                    });
                    self.demo_frames = 0;
                }
                self.demo_frames > 10
            }
            Scene::Prompts => {
                if !self.feedback.demo_seeded {
                    self.feedback.demo_seeded = true;
                    match tomlook::showcase::sample_feedback(
                        &self.calendar.events,
                        &self.preferences.prompts,
                        chrono::Utc::now(),
                    ) {
                        Ok(entries) => {
                            for entry in entries {
                                self.command(Command::SaveFeedback(Box::new(entry)));
                            }
                        }
                        Err(error) => self.error = Some(error),
                    }
                    self.feedback.summary_stale = true;
                    self.preferences_open = true;
                    self.preferences_section = PreferencesSection::Prompts;
                    self.prompt_kind = Kind::Preparation;
                    self.demo_frames = 0;
                }
                let ready = self.feedback.summary.as_ref().is_some_and(|summary| {
                    summary.account == model::PREVIEW_ACCOUNT && summary.preparation.down > 0
                });
                if !ready {
                    self.demo_frames = 0;
                }
                ready && self.demo_frames > 5
            }
            Scene::Tasks => {
                self.tasks_open = true;
                let ready = self.open_task_count() > 0;
                if !ready {
                    self.demo_frames = 0;
                }
                ready && self.demo_frames > 5
            }
            Scene::FollowUp => {
                let ready = self.briefing.as_ref().is_some_and(|briefing| briefing["tomlook_recap"] == true)
                    && self.followup.editing.is_some();
                if !ready {
                    self.followup.focus = true;
                    self.demo_frames = 0;
                } else if self.demo_frames < 3 {
                    self.followup.focus = true;
                }
                ready && self.demo_frames > 8
            }
            Scene::MyDay => {
                if !self.myday.open {
                    self.open_myday();
                }
                let ready = self.open_task_count() > 0 && self.myday_overview().is_some();
                if !ready {
                    self.demo_frames = 0;
                }
                ready && self.demo_frames > 5
            }
            Scene::Notifications => {
                self.preferences_open = true;
                self.preferences_section = PreferencesSection::Notifications;
                self.demo_frames > 8
            }
            Scene::Weekly => {
                if !self.weekly.open {
                    self.open_weekly(None);
                }
                let ready = self
                    .weekly_review()
                    .is_some_and(|(review, _)| review.headline.meetings > 0);
                if !ready {
                    self.demo_frames = 0;
                }
                ready && self.demo_frames > 5
            }
            Scene::Invite => {
                let ready = self.demo_invitation_ready();
                if !ready {
                    self.demo_frames = 0;
                }
                ready && self.demo_frames > 8
            }
            Scene::FindTime | Scene::Teams => {
                let ready = self.demo_find_time_ready(self.demo_scene == Scene::Teams);
                if !ready {
                    self.demo_frames = 0;
                }
                ready && self.demo_frames > 8
            }
        }
    }
}
