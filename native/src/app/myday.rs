use super::*;
use super::tasks::due_label;
use tomlook::myday::{self as model, Overview, Reason};
use tomlook::worker::TaskCommand;

/// Inputs the cached sources were built from; held so their addresses cannot be reused.
struct Inputs {
    calendar: Arc<Calendar>,
    activity: Arc<tomlook::worker::Activity>,
    owner: Option<String>,
    preview: bool,
    today: NaiveDate,
}

struct Cached {
    sources: Arc<model::Sources>,
    language: tomlook::i18n::Language,
    minute: i64,
    overview: Arc<Overview>,
}

/// My day panel state and its cached overview; nothing is recomputed per frame.
#[derive(Default)]
pub(super) struct MyDayUi {
    pub(super) open: bool,
    raise: bool,
    inputs: Option<Inputs>,
    sources: Option<Arc<model::Sources>>,
    cached: Option<Cached>,
    /// Tasks checked off here while the storage worker saves them.
    completed: BTreeSet<i64>,
    /// Fixed clock for tests.
    #[cfg(test)]
    pub(super) now: Option<chrono::DateTime<Local>>,
}

fn span(start: chrono::DateTime<chrono::FixedOffset>, end: chrono::DateTime<chrono::FixedOffset>) -> String {
    format!("{}–{}", model::clock(start), model::clock(end))
}

pub(super) fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.7)))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(color)));
}

/// A section card in the style of the Insights section cards.
pub(super) fn card<R>(
    ui: &mut egui::Ui,
    accent: Color32,
    title: &str,
    count: Option<usize>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let inner = egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (marker, _) =
                    ui.allocate_exact_size(Vec2::new(3.0, 18.0), egui::Sense::hover());
                ui.painter().rect_filled(marker, 1, accent);
                ui.label(RichText::new(title).strong().size(15.0));
                if let Some(count) = count.filter(|count| *count > 0) {
                    ui.weak(count.to_string());
                }
            });
            ui.add_space(4.0);
            add(ui)
        })
        .inner;
    ui.add_space(10.0);
    inner
}

/// Fixed-width, left-aligned time column of a row.
pub(super) fn time_column(ui: &mut egui::Ui, text: String) {
    ui.allocate_ui_with_layout(
        Vec2::new(84.0, 18.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(84.0);
            ui.weak(text);
        },
    );
}

impl Tomlook {
    pub(super) fn myday_now(&self) -> chrono::DateTime<Local> {
        #[cfg(test)]
        if let Some(now) = self.myday.now {
            return now;
        }
        Local::now()
    }

    pub(super) fn open_myday(&mut self) {
        self.myday.open = true;
        self.myday.raise = true;
    }

    pub(super) fn toggle_myday(&mut self) {
        if self.myday.open {
            self.myday.open = false;
        } else {
            self.open_myday();
        }
    }

    /// Rebuilds the My day inputs when the calendar, activity, account or day changed and hands
    /// them to the morning notification; otherwise only compares pointers.
    pub(super) fn sync_myday(&mut self) {
        let today = self.myday_now().date_naive();
        if self.myday.inputs.as_ref().is_some_and(|inputs| {
            Arc::ptr_eq(&inputs.calendar, &self.calendar)
                && Arc::ptr_eq(&inputs.activity, &self.activity)
                && inputs.owner == self.calendar_owner
                && inputs.preview == self.preview
                && inputs.today == today
        }) {
            return;
        }
        let sources = self.myday_sources(today);
        self.myday.completed.retain(|id| {
            sources
                .tasks
                .iter()
                .any(|task| task.id == *id && task.status == tomlook::tasks::Status::Open)
        });
        self.myday.sources = Some(Arc::new(sources));
        self.myday.inputs = Some(Inputs {
            calendar: self.calendar.clone(),
            activity: self.activity.clone(),
            owner: self.calendar_owner.clone(),
            preview: self.preview,
            today,
        });
        self.sync_morning();
    }

    /// Hands the My day inputs and the morning preferences to the notification thread.
    pub(super) fn sync_morning(&self) {
        if let (Some(reminders), Some(sources)) = (&self.reminders, &self.myday.sources)
            && !self.loading
        {
            reminders.update_morning(
                sources,
                model::Schedule::from_preferences(&self.preferences),
                self.preferences.language,
            );
        }
    }

    /// Snapshot of the cached data My day needs: Insights readiness and saved notes are taken
    /// only for meetings from five days back to the day after tomorrow.
    fn myday_sources(&self, today: NaiveDate) -> model::Sources {
        let calendar: &Calendar = &self.calendar;
        let first = today - chrono::Duration::days(model::NOTES_DAYS);
        let last = today + chrono::Duration::days(2);
        let window = move || {
            calendar.days.range(first..=last).flat_map(move |(_, day)| {
                day.events.iter().map(move |index| &calendar.events[*index])
            })
        };
        let ready = window()
            .filter(|event| {
                if self.preview {
                    event.extra.get("synthetic").and_then(serde_json::Value::as_bool) == Some(true)
                        && event.briefing_status == "ready"
                } else {
                    self.activity.queue.jobs.get(&event.id).is_some_and(|job| {
                        job.state == tomlook::scheduler::State::Completed
                    })
                }
            })
            .map(|event| event.id.clone())
            .collect::<BTreeSet<_>>();
        let mut notes = BTreeMap::new();
        if self.preview {
            let now = self.myday_now().fixed_offset();
            for event in window()
                .filter(|event| event.title == tomlook::showcase::ENDED_MEETING && event.end <= now)
            {
                if let Some(saved) = tomlook::showcase::notes(event)
                    .ok()
                    .as_ref()
                    .and_then(model::Notes::from_payload)
                {
                    notes.insert(event.id.clone(), saved);
                }
            }
        } else if let Some(owner) = self.calendar_owner.as_deref() {
            for record in self
                .activity
                .recaps
                .records
                .values()
                .filter(|record| record.account == owner)
            {
                let (Some(payload), Some(index)) =
                    (&record.payload, calendar.event_index(&record.event_id))
                else {
                    continue;
                };
                let (start, end) = calendar.events[index].calendar_dates();
                if end >= first
                    && start <= last
                    && let Some(saved) = model::Notes::from_payload(payload)
                {
                    notes.insert(record.event_id.clone(), saved);
                }
            }
        }
        model::Sources {
            calendar: self.calendar.clone(),
            owner: self.calendar_owner.clone(),
            task_account: self.feedback_account(),
            tasks: self.activity.tasks.clone(),
            recommendations: self.activity.recommendations.clone(),
            recommendation_view: self.activity.recommendation_view.clone(),
            ready: Arc::new(ready),
            notes: Arc::new(notes),
        }
    }

    /// The overview of today, recomputed at most once a minute or when its inputs change.
    pub(super) fn myday_overview(&mut self) -> Option<Arc<Overview>> {
        self.sync_myday();
        let sources = self.myday.sources.clone()?;
        let now = self.myday_now();
        let minute = now.timestamp().div_euclid(60);
        let language = self.preferences.language;
        if let Some(cached) = &self.myday.cached
            && Arc::ptr_eq(&cached.sources, &sources)
            && cached.language == language
            && cached.minute == minute
        {
            return Some(cached.overview.clone());
        }
        let overview = Arc::new(model::overview(&sources, now, language));
        self.myday.cached = Some(Cached {
            sources,
            language,
            minute,
            overview: overview.clone(),
        });
        Some(overview)
    }

    /// Settings → Notifications → Send test: the morning notification with the current counts.
    pub(super) fn send_morning_test(&mut self) {
        let Some(overview) = self.myday_overview() else {
            return;
        };
        let language = self.preferences.language;
        let digest = model::digest(&overview, self.myday_now(), language);
        if let Some(reminders) = &self.reminders {
            reminders.test_morning(digest, language);
        }
    }

    fn open_myday_meeting(&mut self, id: &str) {
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

    fn open_myday_messages(&mut self, recommendation: Option<String>) {
        self.myday.open = false;
        self.messages_open = true;
        self.recommendations_tab = true;
        if recommendation.is_some() {
            self.recommendation_selected = recommendation;
        }
        if self.ai.is_some() {
            self.ai_command(ai::Command::RefreshCalendarAvailability);
        }
    }

    /// Why a meeting cannot be researched again right now.
    fn research_state(&self, id: &str) -> Option<&'static str> {
        if self.activity.queue.flights.values().any(|flight| flight.event_id == id) {
            Some("Researching...")
        } else if self.activity.queue.jobs.get(id).is_some_and(|job| {
            job.manual && matches!(job.state, tomlook::scheduler::State::Queued | tomlook::scheduler::State::Running)
        }) {
            Some("Waiting")
        } else {
            None
        }
    }

    pub(super) fn myday_window(&mut self, context: &egui::Context) {
        if !self.myday.open {
            return;
        }
        let language = self.preferences.language;
        let mut open = true;
        let shown = egui::Window::new(language.text("My day"))
            .id(Id::new("myday-window"))
            .open(&mut open)
            .default_pos(egui::pos2(NAVIGATION_WIDTH + 24.0, 96.0))
            .default_size(Vec2::new(900.0, 700.0))
            .min_width(420.0)
            .resizable(true)
            .show(context, |ui| self.myday_panel(ui));
        if std::mem::take(&mut self.myday.raise)
            && let Some(shown) = shown
        {
            context.move_to_top(shown.response.layer_id);
        }
        if !open {
            self.myday.open = false;
        }
        // Keeps the countdown and the next meeting current while the panel is open.
        context.request_repaint_after(std::time::Duration::from_secs(30));
    }

    pub(super) fn myday_panel(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let Some(overview) = self.myday_overview() else {
            return;
        };
        let now = self.myday_now().fixed_offset();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(language.date(overview.date, true)).strong().size(17.0));
            if self.preview {
                ui.weak(language.text("Offline demo - fictional calendar"));
            }
        });
        ui.label(model::summary(&overview, language));
        if !overview.verified {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                language.text("Today's calendar is not synced yet; the overview may be incomplete."),
            );
        }
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("myday-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if ui.available_width() >= 760.0 {
                    ui.columns(2, |columns| {
                        let (left, right) = columns.split_at_mut(1);
                        self.myday_meetings(&mut left[0], &overview, now);
                        self.myday_follow_up(&mut right[0], &overview, now);
                    });
                } else {
                    self.myday_meetings(ui, &overview, now);
                    self.myday_follow_up(ui, &overview, now);
                }
                if ui
                    .link(language.text("Weekly review"))
                    .on_hover_text(language.text("Open the weekly review of this week"))
                    .clicked()
                {
                    self.myday.open = false;
                    self.open_weekly(None);
                }
            });
    }

    fn myday_badges(&self, ui: &mut egui::Ui, item: &model::Item) {
        let language = self.preferences.language;
        let accent = self.accent();
        let weak = ui.visuals().weak_text_color();
        let warn = ui.visuals().warn_fg_color;
        if item.notes {
            chip(ui, &language.text("Notes ready"), accent);
        } else if item.ready {
            chip(ui, &language.text("Insights ready"), accent);
        } else if item.unprepared() {
            chip(ui, &language.text("Not prepared"), warn);
        }
        if item.tentative {
            chip(ui, &language.text("Tentative"), weak);
        }
        if item.join_url.is_some() {
            chip(ui, "Teams", weak);
        }
    }

    fn myday_meetings(
        &mut self,
        ui: &mut egui::Ui,
        overview: &Overview,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) {
        let language = self.preferences.language;
        let accent = self.accent();
        let current = overview.current(now).cloned();
        let heading = if current.as_ref().is_some_and(|item| item.start <= now) {
            "Current meeting"
        } else {
            "Next meeting"
        };
        card(ui, accent, &language.text(heading), None, |ui| {
            let Some(item) = current else {
                ui.weak(language.text("No more meetings today."));
                return;
            };
            ui.label(RichText::new(&item.title).strong().size(16.0));
            ui.horizontal_wrapped(|ui| {
                ui.label(span(item.start, item.end));
                ui.label(
                    RichText::new(model::countdown(item.start, item.end, now, language))
                        .strong()
                        .color(accent),
                );
                self.myday_badges(ui, &item);
            });
            ui.horizontal(|ui| {
                if let Some(url) = item.join_url.as_deref().filter(|url| safe_url(url))
                    && ui.button(language.text("Join")).clicked()
                    && let Err(error) = tomlook::browser::open(url)
                {
                    self.error = Some(error);
                }
                if ui
                    .button(language.text("Open"))
                    .on_hover_text(language.text("Open meeting details"))
                    .clicked()
                {
                    self.open_myday_meeting(&item.event_id);
                }
            });
        });
        card(ui, accent, &language.text("Today's agenda"), Some(overview.agenda.len()), |ui| {
            if overview.agenda.is_empty() {
                ui.weak(language.text("No meetings today."));
            }
            for item in &overview.agenda {
                ui.push_id(("myday-agenda", &item.event_id), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        time_column(ui, if item.all_day {
                            language.text("All day").into_owned()
                        } else {
                            span(item.start, item.end)
                        });
                        let title = RichText::new(&item.title);
                        let title = if !item.all_day && item.end <= now { title.weak() } else { title };
                        if ui
                            .add(egui::Button::new(title).frame(false).wrap_mode(egui::TextWrapMode::Wrap))
                            .on_hover_text(language.text("Open meeting details"))
                            .clicked()
                        {
                            self.open_myday_meeting(&item.event_id);
                        }
                        self.myday_badges(ui, item);
                    });
                });
            }
        });
        card(ui, accent, &language.text("Conflicts"), Some(overview.conflicts.len()), |ui| {
            if overview.conflicts.is_empty() {
                ui.weak(language.text("No conflicts today."));
            }
            for (index, conflict) in overview.conflicts.iter().enumerate() {
                ui.push_id(("myday-conflict", index), |ui| {
                    if index > 0 {
                        ui.separator();
                    }
                    ui.label(
                        RichText::new(format!(
                            "{}: {} – {}",
                            language.text("Overlap"),
                            model::clock(conflict.start),
                            model::clock(conflict.end)
                        ))
                        .small()
                        .color(ui.visuals().warn_fg_color),
                    );
                    for (id, title) in [
                        (&conflict.first, &conflict.first_title),
                        (&conflict.second, &conflict.second_title),
                    ] {
                        if ui
                            .add(
                                egui::Button::new(RichText::new(title).color(accent))
                                    .frame(false)
                                    .wrap_mode(egui::TextWrapMode::Wrap),
                            )
                            .on_hover_text(language.text("Open meeting details"))
                            .clicked()
                        {
                            self.open_myday_meeting(id);
                        }
                    }
                });
            }
        });
        let unprepared = overview
            .unprepared
            .iter()
            .filter_map(|id| overview.item(id))
            .collect::<Vec<_>>();
        card(ui, accent, &language.text("Not prepared"), Some(unprepared.len()), |ui| {
            if unprepared.is_empty() {
                ui.weak(language.text("Every meeting that needs preparation today has Insights."));
                return;
            }
            let states = unprepared
                .iter()
                .map(|item| (*item, self.research_state(&item.event_id)))
                .collect::<Vec<_>>();
            let pending = states
                .iter()
                .filter(|(_, state)| state.is_none())
                .map(|(item, _)| item.event_id.clone())
                .collect::<Vec<_>>();
            let reason = if self.preview {
                Some("Research is not available in the offline demo.")
            } else if self.ai.is_none() || !self.ai_ready || !self.workiq_ready {
                Some("Connect AI and WorkIQ to research these meetings.")
            } else if !self.activity.enabled {
                Some("Automatic Insights are disabled.")
            } else if self.activity.paused {
                Some("Automatic Insights are paused.")
            } else if pending.is_empty() {
                Some("All of them are already queued.")
            } else {
                None
            };
            ui.horizontal_wrapped(|ui| {
                let button = ui.add_enabled(
                    reason.is_none(),
                    egui::Button::new(language.text("Research all")),
                );
                let clicked = button.clicked();
                if let Some(reason) = reason {
                    button.on_disabled_hover_text(language.text(reason));
                    ui.weak(language.text(reason));
                } else if clicked {
                    // The latest request is dispatched first, so the earliest meeting goes last.
                    for id in pending.iter().rev() {
                        self.request_insights(id.clone());
                    }
                }
            });
            for (item, state) in &states {
                ui.push_id(("myday-unprepared", &item.event_id), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        time_column(ui, if item.all_day {
                            language.text("All day").into_owned()
                        } else {
                            span(item.start, item.end)
                        });
                        if ui
                            .add(egui::Button::new(item.title.as_str()).frame(false).wrap_mode(egui::TextWrapMode::Wrap))
                            .on_hover_text(language.text("Open meeting details"))
                            .clicked()
                        {
                            self.open_myday_meeting(&item.event_id);
                        }
                        match state {
                            Some(state) => {
                                ui.weak(language.text(state));
                            }
                            None => {
                                let research = ui.add_enabled(
                                    !self.preview,
                                    egui::Button::new(language.text("Research now")).small(),
                                );
                                if self.preview {
                                    research.on_disabled_hover_text(language.text(
                                        "Research is not available in the offline demo.",
                                    ));
                                } else if research.clicked() {
                                    self.request_insights(item.event_id.clone());
                                }
                            }
                        }
                    });
                });
            }
        });
        card(ui, accent, &language.text("Missing materials"), Some(overview.materials.len()), |ui| {
            if overview.materials.is_empty() {
                ui.weak(language.text("No missing materials for today's meetings."));
            }
            let warn = ui.visuals().warn_fg_color;
            for material in &overview.materials {
                ui.push_id(("myday-material", &material.recommendation), |ui| {
                    let label = match material.kind {
                        tomlook::recommendations::Kind::MissingMaterials => "Missing materials",
                        tomlook::recommendations::Kind::PreparationAdvance => "Prepare in advance",
                        _ => "Make time to prepare",
                    };
                    ui.horizontal_wrapped(|ui| {
                        chip(ui, &language.text(label), warn);
                        ui.weak(model::clock(material.start));
                        ui.label(&material.title);
                    });
                    if ui.small_button(language.text("Open in Messages")).clicked() {
                        self.open_myday_messages(Some(material.recommendation.clone()));
                    }
                });
            }
        });
    }

    fn myday_follow_up(
        &mut self,
        ui: &mut egui::Ui,
        overview: &Overview,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) {
        let language = self.preferences.language;
        let accent = self.accent();
        let account = self.feedback_account();
        card(ui, accent, &language.text("Tasks"), Some(overview.tasks.len()), |ui| {
            if overview.tasks.is_empty() {
                ui.weak(language.text("No overdue tasks, tasks due today or tasks from recent notes."));
            }
            for task in overview.tasks.iter().take(10) {
                ui.push_id(("myday-task", task.id), |ui| {
                    let mut done = self.myday.completed.contains(&task.id);
                    let title = if done {
                        RichText::new(&task.title).strikethrough().weak()
                    } else {
                        RichText::new(&task.title)
                    };
                    if ui
                        .checkbox(&mut done, title)
                        .on_hover_text(language.text(if done { "Mark as not done" } else { "Mark as done" }))
                        .changed()
                        && let Some(account) = &account
                    {
                        if done {
                            self.myday.completed.insert(task.id);
                        } else {
                            self.myday.completed.remove(&task.id);
                        }
                        self.task_command(TaskCommand::SetDone {
                            account: account.clone(),
                            id: task.id,
                            done,
                        });
                    }
                    let (text, color) = match task.reason {
                        Reason::Overdue => (
                            format!(
                                "{} · {}",
                                language.text("Overdue"),
                                task.due.map(|due| due_label(due, overview.date, language)).unwrap_or_default()
                            ),
                            ui.visuals().error_fg_color,
                        ),
                        Reason::Today => (language.text("Due today").into_owned(), accent),
                        Reason::Notes => (
                            format!(
                                "{} · {}",
                                language.text("From meeting notes"),
                                task.meeting.as_deref().unwrap_or_default()
                            ),
                            ui.visuals().weak_text_color(),
                        ),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(24.0);
                        ui.label(RichText::new(text).small().color(color));
                    });
                });
            }
            if overview.tasks.len() > 10 {
                ui.weak(language.text(&format!("+{} more", overview.tasks.len() - 10)));
            }
            if ui.button(language.text("Open tasks")).clicked() {
                self.myday.open = false;
                self.open_tasks(false);
            }
        });
        card(ui, accent, &language.text("Mentions of me"), Some(overview.mentions.len()), |ui| {
            let (first, last) = overview.notes_days;
            let days = if first == last {
                due_label(first, overview.date, language)
            } else {
                language.range(first, last)
            };
            ui.weak(format!("{} · {}", language.text("Meeting notes"), days));
            if overview.mentions.is_empty() {
                ui.weak(language.text("No mentions of you in recent meeting notes."));
            }
            for (index, mention) in overview.mentions.iter().enumerate() {
                ui.push_id(("myday-mention", index), |ui| {
                    ui.add(egui::Label::new(mention.text.as_str()).wrap());
                    if ui
                        .add(egui::Button::new(RichText::new(&mention.title).small().color(accent)).frame(false))
                        .on_hover_text(language.text("Open meeting details"))
                        .clicked()
                    {
                        self.open_myday_meeting(&mention.event_id);
                    }
                });
            }
        });
        card(ui, accent, &language.text("Focus time"), None, |ui| {
            if overview.focus.is_none() {
                ui.weak(language.text("Today's calendar is not synced yet, so free time is not shown."));
                return;
            }
            let windows = overview.focus_remaining(now);
            if windows.is_empty() {
                ui.weak(language.text("No free block of 30 minutes or more is left between 08:00 and 18:00."));
            }
            for (start, end) in windows {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(span(start, end)).strong());
                    ui.weak(model::duration((end - start).num_minutes()));
                });
            }
            ui.weak(language.text("Free in the confirmed calendar between 08:00 and 18:00."));
        });
        card(ui, accent, &language.text("Waiting for approval"), Some(overview.approvals), |ui| {
            if overview.approvals == 0 {
                ui.weak(language.text("Nothing is waiting for approval."));
                return;
            }
            ui.label(format!("{}: {}", language.text("Recommendations"), overview.approvals));
            if ui.button(language.text("Open Messages")).clicked() {
                self.open_myday_messages(None);
            }
        });
        card(ui, accent, &language.text("Tomorrow"), None, |ui| {
            ui.label(model::tomorrow_line(overview, language));
            ui.weak(language.date(overview.tomorrow.date, true));
            if !overview.tomorrow.verified {
                ui.weak(language.text("Tomorrow's calendar is not synced yet."));
            }
        });
    }
}
