use super::*;

impl Tomlook {
    pub(super) fn open_recommended_meeting(&mut self, event: &Event) {
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

    pub(super) fn recommendation_meeting(&mut self, ui: &mut egui::Ui, event: &Event, actions: bool) {
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

    pub(super) fn recommendation_card(
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
            RecommendationKind::Manual => match &recommendation.manual_change {
                Some(Change::Document(edit)) => edit.document.label(),
                Some(Change::Text(_)) => "Edit meeting text",
                Some(Change::Buffer { .. }) => "Add travel buffer",
                _ => recommendation.suggestion.kind.label(),
            },
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
                        Change::Document(edit) => {
                            ui.weak(language.text("Updates both languages; untouched sections and original evidence stay unchanged."));
                            for (before, after) in edit.before.iter().zip(&edit.after) {
                                let label = match after.name.as_str() {
                                    "summary" => "Overview", "preparation" => "Preparation",
                                    "talking_points" => "Talking points", "open_questions" => "Open questions",
                                    "risks" => "Risks", "gaps" => "Gaps", _ => &after.name,
                                };
                                egui::CollapsingHeader::new(language.text(label))
                                    .id_salt((&recommendation.id, &after.name)).default_open(true).show(ui, |ui| {
                                        ui.weak(language.text("Before"));
                                        ui.label(before.text(language));
                                        ui.add_space(6.0);
                                        ui.strong(language.text("After"));
                                        ui.label(after.text(language));
                                    });
                            }
                            ui.weak(language.text("Approved sections are preserved during automatic updates."));
                        }
                        Change::Text(edit) => {
                            if let Some(title) = &edit.title {
                                ui.weak(language.text("Before"));
                                if let Some(event) = event { ui.label(&event.title); }
                                ui.strong(language.text("After"));
                                ui.label(title);
                            }
                            if let Some(body) = &edit.body {
                                if let Some(text) = &body.append {
                                    ui.strong(language.text("Append to description"));
                                    ui.label(text);
                                    ui.collapsing(language.text("Current description"), |ui| { ui.label(body.before_text()); });
                                } else if let Some(text) = &body.replacement {
                                    ui.weak(language.text("Before"));
                                    ui.label(body.before_text());
                                    ui.strong(language.text("After"));
                                    ui.label(text);
                                }
                            }
                            ui.weak(language.text("This occurrence only"));
                            if event.is_some_and(|event| event.attendee_count() > 0) {
                                ui.label(RichText::new(language.text("Changing meeting text can notify attendees.")).strong());
                            }
                        }
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
                            if recommendation.state == State::Pending { "Approve change" } else { "Retry calendar change" }
                        } else if recommendation.suggestion.kind == RecommendationKind::Category {
                            "Use category"
                        } else { "Add to calendar" };
                        let local = matches!(change, Change::Document(_));
                        if ui.add_enabled(local || self.ai.is_some(),
                            egui::Button::new(language.text(label))).clicked() {
                            if !local { self.ai_command(ai::Command::RefreshCalendarAvailability); }
                            self.command(Command::ConfirmRecommendation { id: recommendation.id.clone(), change: change.clone() });
                            if local && let Some(event) = event { self.open_recommended_meeting(event); }
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
}

#[cfg(test)]
pub(super) fn messages_button(ui: &mut egui::Ui, language: tomlook::i18n::Language) -> egui::Response {
    inbox_button(ui, language, true, 0)
}

pub(super) fn inbox_button(
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
