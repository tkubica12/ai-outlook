use super::myday::{card, time_column};
use super::*;
use tomlook::weekly::{self as model, Direction, Review, Trend, Unit};

/// Inputs the cached sources were built from; held so their addresses cannot be reused.
struct Inputs {
    calendar: Arc<Calendar>,
    activity: Arc<tomlook::worker::Activity>,
    owner: Option<String>,
    work_email: String,
    preview: bool,
    first: NaiveDate,
    last: NaiveDate,
}

struct Cached {
    sources: Arc<model::Sources>,
    week: NaiveDate,
    minute: i64,
    review: Arc<Review>,
    trend: Option<Arc<Trend>>,
}

pub(super) struct Running {
    pub(super) id: u64,
    pub(super) request: Box<model::Request>,
}

/// Weekly review panel state and its cached review; nothing is recomputed per frame.
#[derive(Default)]
pub(super) struct WeeklyUi {
    pub(super) open: bool,
    raise: bool,
    /// Monday of the reviewed week; `None` follows the current week.
    pub(super) week: Option<NaiveDate>,
    inputs: Option<Inputs>,
    sources: Option<Arc<model::Sources>>,
    cached: Option<Cached>,
    request_id: u64,
    pub(super) running: Option<Running>,
    /// Week and message of the last AI summary failure.
    pub(super) error: Option<(NaiveDate, String)>,
    /// Summaries received in this session until the storage worker publishes them.
    received: BTreeMap<NaiveDate, model::Saved>,
    /// Week whose summary was just copied.
    copied: Option<NaiveDate>,
    /// Fixed clock for tests.
    #[cfg(test)]
    pub(super) now: Option<chrono::DateTime<Local>>,
}

/// Painted trend arrow (no glyph or emoji font) followed by the change.
fn trend_mark(ui: &mut egui::Ui, delta: &model::Delta, hover: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let color = ui.visuals().weak_text_color();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(9.0, 12.0), egui::Sense::hover());
        let painter = ui.painter();
        let center = rect.center();
        match delta.direction {
            Direction::Up | Direction::Down => {
                let sign = if delta.direction == Direction::Up { -1.0 } else { 1.0 };
                let tip = center + Vec2::new(0.0, 5.0 * sign);
                let tail = center - Vec2::new(0.0, 5.0 * sign);
                let stroke = Stroke::new(1.5, color);
                painter.line_segment([tail, tip], stroke);
                painter.line_segment([tip, tip + Vec2::new(-3.5, -3.5 * sign)], stroke);
                painter.line_segment([tip, tip + Vec2::new(3.5, -3.5 * sign)], stroke);
            }
            Direction::Same => {
                painter.line_segment(
                    [center - Vec2::new(4.0, 0.0), center + Vec2::new(4.0, 0.0)],
                    Stroke::new(1.5, color),
                );
            }
        }
        response.on_hover_text(hover);
        ui.label(RichText::new(&delta.text).small().color(color))
            .on_hover_text(hover);
    });
}

/// One headline number with its label, detail and change against earlier weeks.
fn tile(
    ui: &mut egui::Ui,
    value: &str,
    label: &str,
    detail: Option<String>,
    change: Option<(model::Delta, String)>,
) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(138.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(value).strong().size(20.0));
                ui.label(RichText::new(label).small());
                if let Some(detail) = detail {
                    ui.weak(RichText::new(detail).small());
                }
                if let Some((delta, hover)) = change {
                    trend_mark(ui, &delta, &hover);
                }
            });
        });
}

/// A horizontal bar filled to `fraction` of its width.
fn bar(ui: &mut egui::Ui, width: f32, fraction: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 10.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3, ui.visuals().extreme_bg_color);
    let filled = egui::Rect::from_min_size(
        rect.min,
        Vec2::new((rect.width() * fraction.clamp(0.0, 1.0)).max(2.0), rect.height()),
    );
    painter.rect_filled(filled, 3, color);
}

fn day_time(time: chrono::DateTime<chrono::FixedOffset>, language: tomlook::i18n::Language) -> String {
    format!(
        "{} {}",
        language.date(time.with_timezone(&Local).date_naive(), false).trim(),
        tomlook::myday::clock(time)
    )
}

impl Tomlook {
    pub(super) fn weekly_now(&self) -> chrono::DateTime<Local> {
        #[cfg(test)]
        if let Some(now) = self.weekly.now {
            return now;
        }
        Local::now()
    }

    pub(super) fn current_week(&self) -> NaiveDate {
        model::week_of(self.weekly_now().date_naive())
    }

    /// The reviewed week: the chosen past week, otherwise the current one.
    pub(super) fn weekly_week(&self) -> NaiveDate {
        let current = self.current_week();
        self.weekly.week.filter(|week| *week < current).unwrap_or(current)
    }

    /// Opens the weekly review of the week containing `week`, or of the current week. Future
    /// weeks are not reviewed.
    pub(super) fn open_weekly(&mut self, week: Option<NaiveDate>) {
        let current = self.current_week();
        self.weekly.week = week.map(model::week_of).filter(|week| *week < current);
        self.weekly.open = true;
        self.weekly.raise = true;
    }

    fn show_weekly_week(&mut self, week: NaiveDate) {
        let current = self.current_week();
        self.weekly.week = (week < current).then_some(week);
        self.weekly.copied = None;
    }

    /// Rebuilds the weekly review inputs when the calendar, activity, account, window or day
    /// changed and hands them to the weekly notification; otherwise only compares pointers.
    pub(super) fn sync_weekly(&mut self) {
        let today = self.weekly_now().date_naive();
        let current = model::week_of(today);
        let week = self.weekly_week();
        // The reviewed week, last week (Monday notifications) and four earlier weeks for trends.
        let first = week.min(current - chrono::Duration::days(7))
            - chrono::Duration::days(7 * model::TREND_WEEKS);
        let last = current + chrono::Duration::days(6);
        if self.weekly.inputs.as_ref().is_some_and(|inputs| {
            Arc::ptr_eq(&inputs.calendar, &self.calendar)
                && Arc::ptr_eq(&inputs.activity, &self.activity)
                && inputs.owner == self.calendar_owner
                && inputs.work_email == self.preferences.work_email
                && inputs.preview == self.preview
                && inputs.first == first
                && inputs.last == last
        }) {
            return;
        }
        self.weekly.sources = Some(Arc::new(self.weekly_sources(first, last)));
        self.weekly.inputs = Some(Inputs {
            calendar: self.calendar.clone(),
            activity: self.activity.clone(),
            owner: self.calendar_owner.clone(),
            work_email: self.preferences.work_email.clone(),
            preview: self.preview,
            first,
            last,
        });
        self.sync_weekly_notification();
    }

    /// Hands the weekly review inputs and preferences to the notification thread.
    pub(super) fn sync_weekly_notification(&self) {
        if let (Some(reminders), Some(sources)) = (&self.reminders, &self.weekly.sources)
            && !self.loading
        {
            reminders.update_weekly(
                sources,
                model::Schedule::from_preferences(&self.preferences),
                self.preferences.language,
            );
        }
    }

    /// Snapshot of the cached data the review needs: Insights readiness and saved notes only for
    /// meetings from `first` through `last`.
    fn weekly_sources(&self, first: NaiveDate, last: NaiveDate) -> model::Sources {
        let calendar: &Calendar = &self.calendar;
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
        let mut notes = BTreeSet::new();
        if self.preview {
            let now = self.weekly_now().fixed_offset();
            notes.extend(
                window()
                    .filter(|event| {
                        event.title == tomlook::showcase::ENDED_MEETING
                            && event.end <= now
                            && tomlook::showcase::notes(event).is_ok()
                    })
                    .map(|event| event.id.clone()),
            );
        } else if let Some(owner) = self.calendar_owner.as_deref() {
            notes.extend(
                self.activity
                    .recaps
                    .records
                    .values()
                    .filter(|record| record.account == owner && record.payload.is_some())
                    .filter(|record| {
                        calendar.event_index(&record.event_id).is_some_and(|index| {
                            let (start, end) = calendar.events[index].calendar_dates();
                            end >= first && start <= last
                        })
                    })
                    .map(|record| record.event_id.clone()),
            );
        }
        let (own, mut own_domains) = model::Sources::own_addresses(&self.own_emails());
        if self.preview {
            // The demo's fictional colleagues share one reserved domain.
            own_domains.insert("example.invalid".into());
        }
        model::Sources {
            calendar: self.calendar.clone(),
            own,
            own_domains,
            task_account: self.feedback_account(),
            tasks: self.activity.tasks.clone(),
            ready: Arc::new(ready),
            notes: Arc::new(notes),
        }
    }

    /// The reviewed week and its trend, recomputed at most once a minute or when the inputs or
    /// the week change.
    pub(super) fn weekly_review(&mut self) -> Option<(Arc<Review>, Option<Arc<Trend>>)> {
        self.sync_weekly();
        let sources = self.weekly.sources.clone()?;
        let week = self.weekly_week();
        let now = self.weekly_now();
        let minute = now.timestamp().div_euclid(60);
        if let Some(cached) = &self.weekly.cached
            && Arc::ptr_eq(&cached.sources, &sources)
            && cached.week == week
            && cached.minute == minute
        {
            return Some((cached.review.clone(), cached.trend.clone()));
        }
        let review = Arc::new(model::review(&sources, week, now));
        let trend = model::trend(&sources, week).map(Arc::new);
        self.weekly.cached = Some(Cached {
            sources,
            week,
            minute,
            review: review.clone(),
            trend: trend.clone(),
        });
        Some((review, trend))
    }

    /// Settings → Notifications → Send test: the weekly notification with the current counts of
    /// the week it would review today.
    pub(super) fn send_weekly_test(&mut self) {
        self.sync_weekly();
        let Some(sources) = self.weekly.sources.clone() else {
            return;
        };
        let now = self.weekly_now();
        let schedule = model::Schedule::from_preferences(&self.preferences);
        let week = model::reviewed_week(&schedule, now.date_naive());
        let review = model::review(&sources, week, now);
        let language = self.preferences.language;
        let digest = model::digest(&review, week < model::week_of(now.date_naive()), language);
        if let Some(reminders) = &self.reminders {
            reminders.test_weekly(digest, week, language);
        }
    }

    /// The latest saved AI summary of `week` for the calendar account.
    fn weekly_summary(&self, week: NaiveDate) -> Option<model::Saved> {
        let account = self.feedback_account()?;
        [self.activity.weekly_summaries.get(&week), self.weekly.received.get(&week)]
            .into_iter()
            .flatten()
            .filter(|saved| saved.account == account)
            .max_by_key(|saved| saved.created_at)
            .cloned()
    }

    /// Why the AI summary cannot be requested right now.
    fn weekly_ai_unavailable(&self) -> Option<&'static str> {
        if self.preview {
            Some("Offline demo never sends AI requests.")
        } else if self.ai.is_none() || !self.ai_ready {
            Some("Connect GitHub Copilot first. No AI request was made.")
        } else if self.feedback_account().is_none() {
            Some("Connect WorkIQ to your calendar account first.")
        } else {
            None
        }
    }

    fn request_weekly_summary(&mut self, review: &Review, trend: Option<&Trend>) {
        let Some(account) = self.feedback_account() else {
            return;
        };
        let language = self.preferences.language;
        let calendar = self.calendar.clone();
        let notes = self
            .activity
            .recaps
            .records
            .values()
            .filter(|record| record.account == account)
            .filter_map(|record| {
                let payload = record.payload.as_ref()?;
                let event = &calendar.events[calendar.event_index(&record.event_id)?];
                (event.start < review.end && event.end > review.start)
                    .then(|| model::MeetingNotes::new(event, payload, language))
                    .flatten()
            })
            .collect();
        self.weekly.request_id += 1;
        let id = self.weekly.request_id;
        self.weekly.copied = None;
        match model::Request::new(id, &account, review, trend, notes, language) {
            Ok(request) => {
                if self.ai_command(ai::Command::WeeklySummary(Box::new(request.clone()))) {
                    self.weekly.running = Some(Running {
                        id,
                        request: Box::new(request),
                    });
                    self.weekly.error = None;
                } else {
                    let error = self
                        .ai_error
                        .take()
                        .unwrap_or_else(|| "The AI worker is unavailable. Restart Tomlook.".into());
                    self.weekly.error = Some((review.week, error));
                }
            }
            Err(error) => self.weekly.error = Some((review.week, error)),
        }
    }

    pub(super) fn receive_weekly_summary(
        &mut self,
        id: u64,
        result: Result<model::AiSummary, String>,
    ) {
        let Some(running) = self.weekly.running.take_if(|running| running.id == id) else {
            return;
        };
        let week = running.request.week;
        let saved = result.and_then(|summary| {
            let saved = model::Saved::new(&running.request, summary, chrono::Utc::now());
            saved.validate().map(|()| saved)
        });
        match saved {
            Ok(saved) => {
                self.command(Command::SaveWeeklySummary(Box::new(saved.clone())));
                self.weekly.received.insert(week, saved);
                self.weekly.error = None;
            }
            Err(error) => self.weekly.error = Some((week, error)),
        }
    }

    pub(super) fn stop_weekly_summary(&mut self) {
        if let Some(running) = self.weekly.running.take() {
            self.weekly.error = Some((
                running.request.week,
                "AI stopped without a completed answer; provider outcome may be unknown".into(),
            ));
        }
    }

    fn cancel_weekly_summary(&mut self) {
        if let Some(running) = self.weekly.running.take() {
            self.ai_command(ai::Command::Cancel(running.id | ai::WEEKLY_BIT));
            self.weekly.error = Some((
                running.request.week,
                "Weekly summary cancelled; nothing was saved.".into(),
            ));
        }
    }

    fn open_weekly_meeting(&mut self, id: &str) {
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

    /// A clickable meeting title that opens its details.
    fn weekly_meeting_link(&mut self, ui: &mut egui::Ui, id: &str, title: &str, color: Option<Color32>) {
        let language = self.preferences.language;
        let text = match color {
            Some(color) => RichText::new(title).color(color),
            None => RichText::new(title),
        };
        if ui
            .add(egui::Button::new(text).frame(false).wrap_mode(egui::TextWrapMode::Wrap))
            .on_hover_text(language.text("Open meeting details"))
            .clicked()
        {
            self.open_weekly_meeting(id);
        }
    }

    pub(super) fn weekly_window(&mut self, context: &egui::Context) {
        if !self.weekly.open {
            return;
        }
        let language = self.preferences.language;
        let mut open = true;
        let shown = egui::Window::new(language.text("Weekly review"))
            .id(Id::new("weekly-window"))
            .open(&mut open)
            .default_pos(egui::pos2(NAVIGATION_WIDTH + 40.0, 80.0))
            .default_size(Vec2::new(980.0, 760.0))
            .min_width(460.0)
            .resizable(true)
            .show(context, |ui| self.weekly_panel(ui));
        if std::mem::take(&mut self.weekly.raise)
            && let Some(shown) = shown
        {
            context.move_to_top(shown.response.layer_id);
        }
        if !open {
            self.weekly.open = false;
        }
    }
}

impl Tomlook {
    pub(super) fn weekly_panel(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let Some((review, trend)) = self.weekly_review() else {
            return;
        };
        let current = self.current_week();
        let summary = self.weekly_summary(review.week);
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("<")
                .on_hover_text(language.text("Previous week"))
                .clicked()
            {
                self.show_weekly_week(review.week - chrono::Duration::days(7));
            }
            ui.label(
                RichText::new(language.range(review.week, review.last_day()))
                    .strong()
                    .size(17.0),
            );
            if ui
                .add_enabled(review.week < current, egui::Button::new(">"))
                .on_hover_text(language.text("Next week"))
                .on_disabled_hover_text(language.text("Future weeks cannot be reviewed yet."))
                .clicked()
            {
                self.show_weekly_week(review.week + chrono::Duration::days(7));
            }
            if review.week < current && ui.button(language.text("This week")).clicked() {
                self.show_weekly_week(current);
            }
            if self.preview {
                ui.weak(language.text("Offline demo - fictional calendar"));
            }
            if ui
                .button(language.text("Copy summary"))
                .on_hover_text(language.text("Copies a short Markdown summary of this week"))
                .clicked()
            {
                ui.ctx().copy_text(model::summary_text(
                    &review,
                    trend.as_deref(),
                    summary.as_ref().map(|saved| &saved.summary),
                    language,
                ));
                self.weekly.copied = Some(review.week);
            }
            if self.weekly.copied == Some(review.week) {
                ui.weak(language.text("Summary copied."));
            }
        });
        if review.current {
            ui.weak(language.text("This week so far, including meetings still scheduled."));
        }
        if review.coverage.partial() {
            let days = review
                .coverage
                .missing
                .iter()
                .map(|day| language.date(*day, false).trim().to_owned())
                .collect::<Vec<_>>()
                .join(", ");
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("{}: {days}", language.text("Not synced")),
            );
            ui.weak(language.text("The numbers may be incomplete. Days that are not synced are never counted as free."));
        }
        if !review.coverage.sufficient() {
            ui.weak(language.text("Fewer than 4 of 5 workdays are synced, so this week is not used for comparisons or the weekly notification."));
        }
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("weekly-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.weekly_headline(ui, &review, trend.as_deref());
                if ui.available_width() >= 760.0 {
                    ui.columns(2, |columns| {
                        let (left, right) = columns.split_at_mut(1);
                        self.weekly_time(&mut left[0], &review);
                        self.weekly_follow_through(&mut right[0], &review, trend.as_deref(), summary.as_ref());
                    });
                } else {
                    self.weekly_time(ui, &review);
                    self.weekly_follow_through(ui, &review, trend.as_deref(), summary.as_ref());
                }
            });
    }

    fn weekly_headline(&mut self, ui: &mut egui::Ui, review: &Review, trend: Option<&Trend>) {
        let language = self.preferences.language;
        let headline = &review.headline;
        let compared = trend.map(|trend| {
            language.text(&format!("Compared with the average of earlier weeks: {}", trend.weeks.len())).into_owned()
        });
        let change = |current: f64, average: Option<f64>, unit: Unit| {
            average
                .zip(compared.clone())
                .map(|(average, hover)| (model::delta(current, average, unit, language), hover))
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            tile(
                ui,
                &headline.meetings.to_string(),
                &language.text("Meetings attended"),
                (headline.tentative > 0).then(|| {
                    tomlook::myday::count(headline.tentative, tomlook::myday::Noun::Tentative, language)
                }),
                change(headline.meetings as f64, trend.map(|trend| trend.meetings), Unit::Count),
            );
            tile(
                ui,
                &model::hours(headline.meeting_minutes, language),
                &language.text("Meeting time"),
                Some(language.text("Overlaps count once").into_owned()),
                change(headline.meeting_minutes as f64, trend.map(|trend| trend.meeting_minutes), Unit::Minutes),
            );
            tile(
                ui,
                &model::hours(headline.average_minutes(), language),
                &language.text("Per workday"),
                Some(language.text("Monday to Friday").into_owned()),
                change(headline.average_minutes() as f64, trend.map(|trend| trend.average_minutes), Unit::Minutes),
            );
            tile(
                ui,
                &model::percent(headline.share(), language),
                &language.text("Of working time"),
                Some(language.text("Mon–Fri 08:00–18:00").into_owned()),
                change(headline.share(), trend.map(|trend| trend.share), Unit::Percent),
            );
            let (focus, focus_detail) = match headline.focus_minutes {
                Some(minutes) => (
                    model::hours(minutes, language),
                    Some(language.text(&format!("Synced workdays: {} of 5", headline.focus_days)).into_owned()),
                ),
                None => ("–".to_owned(), Some(language.text("Not synced").into_owned())),
            };
            tile(
                ui,
                &focus,
                &language.text("Focus time"),
                focus_detail,
                headline.focus_minutes.and_then(|minutes| {
                    change(minutes as f64, trend.and_then(|trend| trend.focus_minutes), Unit::Minutes)
                }),
            );
            tile(
                ui,
                &headline
                    .longest_free_minutes()
                    .map_or_else(|| "–".to_owned(), |minutes| model::hours(minutes, language)),
                &language.text("Longest free block"),
                headline
                    .longest_free
                    .map(|(start, end)| format!("{}–{}", day_time(start, language), tomlook::myday::clock(end))),
                headline.longest_free_minutes().and_then(|minutes| {
                    change(minutes as f64, trend.and_then(|trend| trend.longest_free_minutes), Unit::Minutes)
                }),
            );
        });
        if trend.is_none() {
            ui.weak(language.text("No earlier weeks with enough synced days to compare with."));
        }
        ui.add_space(10.0);
    }
    /// Time by category, conflicts and meetings without an agenda.
    fn weekly_time(&mut self, ui: &mut egui::Ui, review: &Review) {
        let language = self.preferences.language;
        let accent = self.accent();
        let dark = self.preferences.dark;
        let total = review.category_minutes();
        card(ui, accent, &language.text("Time by category"), None, |ui| {
            if review.categories.is_empty() {
                ui.weak(language.text("No meetings this week."));
                return;
            }
            let longest = review.categories.first().map_or(1, |category| category.minutes.max(1));
            egui::Grid::new("weekly-categories")
                .num_columns(3)
                .spacing(Vec2::new(10.0, 6.0))
                .show(ui, |ui| {
                    for category in &review.categories {
                        let color = if category.key.is_empty() {
                            accent
                        } else {
                            category_color(&category.label, dark)
                        };
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, color);
                            ui.label(model::category_label(category, language));
                        });
                        bar(ui, 180.0, category.minutes as f32 / longest as f32, color);
                        ui.label(format!(
                            "{} · {}",
                            model::hours(category.minutes, language),
                            model::percent(model::share(category.minutes, total), language)
                        ))
                        .on_hover_text(tomlook::myday::count(category.meetings, tomlook::myday::Noun::Meeting, language));
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            ui.weak(language.text("Each meeting counts toward its first category in full, so overlapping meetings can add up to more than the meeting time."));
        });
        card(ui, accent, &language.text("Conflicts"), Some(review.conflicts.len()), |ui| {
            if review.conflicts.is_empty() {
                ui.weak(language.text("No conflicts this week."));
                return;
            }
            ui.label(language.text(&format!(
                "Overlapping time: {}",
                model::hours(review.overlap_minutes, language)
            )));
            for (index, conflict) in review.conflicts.iter().enumerate() {
                ui.push_id(("weekly-conflict", index), |ui| {
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{}–{}",
                            day_time(conflict.start, language),
                            tomlook::myday::clock(conflict.end)
                        ))
                        .small()
                        .color(ui.visuals().warn_fg_color),
                    );
                    for (id, title) in [
                        (&conflict.first, &conflict.first_title),
                        (&conflict.second, &conflict.second_title),
                    ] {
                        self.weekly_meeting_link(ui, id, title, Some(accent));
                    }
                });
            }
        });
        let agenda = &review.agenda;
        card(ui, accent, &language.text("Meetings without an agenda"), Some(agenda.missing.len()), |ui| {
            if agenda.eligible == 0 {
                ui.weak(language.text("No meetings with other participants this week."));
                return;
            }
            if agenda.checked == 0 {
                ui.weak(language.text("Meeting descriptions arrive with the next calendar refresh; agendas are checked after that."));
                return;
            }
            if agenda.missing.is_empty() {
                ui.weak(language.text("Every checked meeting has an agenda."));
            }
            for (organized, heading, hint) in [
                (true, "You organize", "Add an agenda so participants can prepare."),
                (false, "Others organize", "Consider asking the organizer for an agenda."),
            ] {
                let items = agenda
                    .missing
                    .iter()
                    .filter(|item| item.organized == organized)
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    continue;
                }
                ui.add_space(4.0);
                ui.strong(format!("{} ({})", language.text(heading), items.len()));
                ui.weak(language.text(hint));
                for item in items {
                    ui.push_id(("weekly-agenda", &item.event_id), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            time_column(ui, day_time(item.start, language));
                            self.weekly_meeting_link(ui, &item.event_id, &item.title, None);
                        });
                    });
                }
            }
            if agenda.unknown > 0 {
                ui.add_space(4.0);
                ui.weak(language.text(&format!("Not checked (description arrives with the next calendar refresh): {}", agenda.unknown)));
            }
            ui.weak(language.text("An agenda is any description beyond the Teams join block, links and boilerplate (at least 20 letters or digits)."));
        });
    }

    /// Meeting load, people and customers, preparation and the AI summary.
    fn weekly_follow_through(
        &mut self,
        ui: &mut egui::Ui,
        review: &Review,
        trend: Option<&Trend>,
        summary: Option<&model::Saved>,
    ) {
        let language = self.preferences.language;
        let accent = self.accent();
        let load = &review.load;
        card(ui, accent, &language.text("Meeting load"), None, |ui| {
            egui::Grid::new("weekly-load")
                .num_columns(2)
                .spacing(Vec2::new(12.0, 4.0))
                .show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, label: &str, value: String| {
                        ui.label(language.text(label));
                        ui.strong(value);
                        ui.end_row();
                    };
                    row(ui, "Back-to-back chains (3+ meetings, under 10 min apart)", load.chains.len().to_string());
                    if let Some(longest) = load.longest() {
                        row(ui, "Longest chain", format!(
                            "{} · {} · {}–{}",
                            tomlook::myday::count(longest.meetings, tomlook::myday::Noun::Meeting, language),
                            model::hours((longest.end - longest.start).num_minutes(), language),
                            day_time(longest.start, language),
                            tomlook::myday::clock(longest.end)
                        ));
                    }
                    row(ui, "Days with more than 6 h of meetings", if load.heavy_days.is_empty() {
                        "0".into()
                    } else {
                        format!(
                            "{} ({})",
                            load.heavy_days.len(),
                            load.heavy_days
                                .iter()
                                .map(|(day, minutes)| format!("{} {}", language.date(*day, false).trim(), model::hours(*minutes, language)))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    });
                    row(ui, "Short meetings (30 min or less)", load.short.to_string());
                    row(ui, "Long meetings (90 min or more)", load.long.to_string());
                });
        });
        card(ui, accent, &language.text("People and customers"), None, |ui| {
            if review.people.is_empty() {
                ui.weak(language.text("No shared meetings with other people this week."));
                return;
            }
            ui.strong(language.text("Most time together"));
            egui::Grid::new("weekly-people")
                .num_columns(2)
                .spacing(Vec2::new(12.0, 4.0))
                .show(ui, |ui| {
                    for person in &review.people {
                        ui.label(&person.name)
                            .on_hover_text(person.email.as_deref().unwrap_or(&person.name));
                        ui.weak(model::hours(person.minutes, language));
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.strong(language.text("External companies"));
            if review.companies.is_empty() {
                ui.weak(language.text("No meetings with external companies this week."));
            }
            egui::Grid::new("weekly-companies")
                .num_columns(2)
                .spacing(Vec2::new(12.0, 4.0))
                .show(ui, |ui| {
                    for company in &review.companies {
                        ui.horizontal(|ui| {
                            if let Some(image) = self.companies.get(&company.domain).and_then(|icon| icon.image.as_ref()) {
                                ui.add(egui::Image::new((image.id(), Vec2::splat(16.0))));
                            }
                            ui.label(&company.domain).on_hover_text(language.text(&format!(
                                "People from this company: {}",
                                company.people
                            )));
                        });
                        ui.weak(model::hours(company.minutes, language));
                        ui.end_row();
                    }
                });
            ui.weak(language.text("Shared meeting time; you, your other mailboxes and rooms are excluded."));
        });
        let preparation = &review.preparation;
        card(ui, accent, &language.text("Preparation and follow-through"), None, |ui| {
            egui::Grid::new("weekly-preparation")
                .num_columns(2)
                .spacing(Vec2::new(12.0, 4.0))
                .show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, label: &str, value: String| {
                        ui.label(language.text(label));
                        ui.strong(value);
                        ui.end_row();
                    };
                    row(ui, "Insights ready", model::ratio(preparation.ready, preparation.eligible, language));
                    row(ui, "Notes for ended Teams meetings", model::ratio(preparation.notes, preparation.teams_ended, language));
                    row(ui, "Tasks created this week", review.tasks.created.to_string());
                    row(ui, "Tasks completed this week", review.tasks.completed.to_string());
                });
        });
        self.weekly_ai(ui, review, trend, summary);
    }

    fn weekly_ai(
        &mut self,
        ui: &mut egui::Ui,
        review: &Review,
        trend: Option<&Trend>,
        summary: Option<&model::Saved>,
    ) {
        let language = self.preferences.language;
        let accent = self.accent();
        card(ui, accent, &language.text("AI summary"), None, |ui| {
            let running = self
                .weekly
                .running
                .as_ref()
                .is_some_and(|running| running.request.week == review.week);
            if let Some(saved) = summary {
                for (label, items) in [
                    ("Highlights", &saved.summary.highlights),
                    ("Decisions", &saved.summary.decisions),
                    ("Open loops", &saved.summary.open_loops),
                    ("Suggestions", &saved.summary.suggestions),
                ] {
                    if items.is_empty() {
                        continue;
                    }
                    ui.strong(language.text(label));
                    for item in items {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("•");
                            ui.add(egui::Label::new(item.as_str()).wrap());
                        });
                    }
                    ui.add_space(4.0);
                }
                ui.weak(language.text(&format!(
                    "Generated by AI: {}",
                    saved.created_at.with_timezone(&Local).format("%d.%m.%Y %H:%M")
                )));
                if saved.omitted > 0 {
                    ui.weak(language.text(&format!(
                        "Notes of the oldest meetings left out for size: {}",
                        saved.omitted
                    )));
                }
            }
            if running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(language.text("Summarizing the week..."));
                    if ui.small_button(language.text("Stop")).clicked() {
                        self.cancel_weekly_summary();
                    }
                });
            }
            if let Some((week, message)) = &self.weekly.error
                && *week == review.week
            {
                ui.colored_label(ui.visuals().error_fg_color, language.text(message));
            }
            let unavailable = self.weekly_ai_unavailable();
            let label = if summary.is_some() { "Regenerate" } else { "Summarize the week with AI" };
            if !running {
                let button = ui
                    .add_enabled(
                        unavailable.is_none() && self.weekly.running.is_none(),
                        egui::Button::new(language.text(label)),
                    )
                    .on_hover_text(language.text(
                        "One Quick AI request with this week's numbers and saved meeting notes; it counts toward the daily AI limit.",
                    ));
                let clicked = button.clicked();
                if let Some(reason) = unavailable {
                    button.on_disabled_hover_text(language.text(reason));
                    ui.weak(language.text(reason));
                } else if clicked {
                    self.request_weekly_summary(review, trend);
                }
            }
            ui.weak(language.text("Never automatic. Only the computed numbers and this week's saved meeting notes are sent; the summary is kept on this computer."));
        });
    }
}