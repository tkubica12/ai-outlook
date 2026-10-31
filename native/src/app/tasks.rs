use super::*;
use tomlook::tasks::{self as model, Group, Status, Task};
use tomlook::worker::TaskCommand;

/// Tasks window and task-row state; disk work runs on the storage worker.
#[derive(Default)]
pub(super) struct TasksUi {
    pub(super) quick: String,
    pub(super) quick_due: Option<NaiveDate>,
    pub(super) focus_quick: bool,
    pub(super) editing: Option<Edit>,
    pub(super) picker_month: Option<NaiveDate>,
    /// The last deleted task, kept in memory so the deletion can be undone.
    pub(super) deleted: Option<Task>,
    /// Ask task drafts already added in this session.
    pub(super) added_drafts: Arc<BTreeSet<String>>,
}

pub(super) struct Edit {
    pub(super) id: i64,
    pub(super) title: String,
    pub(super) note: String,
    pub(super) due: Option<NaiveDate>,
    focus: bool,
}

/// Short due date: relative words near today, otherwise day and month (and year when needed).
pub(super) fn due_label(due: NaiveDate, today: NaiveDate, language: tomlook::i18n::Language) -> String {
    match (due - today).num_days() {
        0 => return language.text("Today").into_owned(),
        1 => return language.text("Tomorrow").into_owned(),
        -1 => return language.text("Yesterday").into_owned(),
        _ => {}
    }
    let same_year = due.year() == today.year();
    match language {
        tomlook::i18n::Language::En if same_year => due.format("%a %-d %b").to_string(),
        tomlook::i18n::Language::En => due.format("%-d %b %Y").to_string(),
        tomlook::i18n::Language::Cs => {
            const DAYS: [&str; 7] = ["po", "út", "st", "čt", "pá", "so", "ne"];
            if same_year {
                format!(
                    "{} {}. {}.",
                    DAYS[due.weekday().num_days_from_monday() as usize],
                    due.day(),
                    due.month()
                )
            } else {
                format!("{}. {}. {}", due.day(), due.month(), due.year())
            }
        }
    }
}

impl Tomlook {
    pub(super) fn open_task_count(&self) -> usize {
        self.feedback_account()
            .map_or(0, |account| model::open_count(&self.activity.tasks, &account))
    }

    pub(super) fn task_command(&mut self, command: TaskCommand) {
        self.command(Command::Task(command));
    }

    pub(super) fn open_tasks(&mut self, add: bool) {
        self.tasks_open = true;
        self.tasks.focus_quick = add;
    }

    fn add_quick_task(&mut self, account: &str) {
        match model::New::manual(&self.tasks.quick, self.tasks.quick_due, self.preferences.language) {
            Ok(task) => {
                self.task_command(TaskCommand::Add {
                    account: account.into(),
                    task: Box::new(task),
                });
                self.tasks.quick.clear();
                self.tasks.quick_due = None;
            }
            Err(error) => self.error = Some(error),
        }
        self.tasks.focus_quick = true;
    }

    /// Adds an Ask task draft as a local task, linked to the Ask meeting context if any.
    pub(super) fn add_draft_task(&mut self, proposal: &tomlook::proposals::Proposal) {
        let Some(account) = self.feedback_account() else {
            self.ai_error = Some("Tasks are available once the calendar account is known.".into());
            return;
        };
        let meeting = self
            .assistant_context
            .as_ref()
            .map(|event| model::Meeting::new(&account, event));
        match model::New::from_draft(proposal, meeting, self.preferences.language) {
            Ok(task) => {
                self.task_command(TaskCommand::Add {
                    account,
                    task: Box::new(task),
                });
                Arc::make_mut(&mut self.tasks.added_drafts).insert(model::draft_key(proposal));
            }
            Err(error) => self.ai_error = Some(error),
        }
    }

    fn open_task_meeting(&mut self, meeting: &model::Meeting) {
        let Some(index) = self.calendar.event_index(&meeting.event_id) else {
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

    /// Due date chooser; returns the chosen date, or `Some(None)` when the date was cleared.
    fn due_picker(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        due: Option<NaiveDate>,
    ) -> Option<Option<NaiveDate>> {
        let language = self.preferences.language;
        let today = Local::now().date_naive();
        let label = due.map_or_else(
            || language.text("Set due date").into_owned(),
            |due| format!("{} {}", language.text("Due"), due_label(due, today, language)),
        );
        let mut chosen = None;
        ui.push_id(salt, |ui| {
            ui.menu_button(RichText::new(label).small(), |ui| {
                ui.horizontal(|ui| {
                    let next_week = tomlook::calendar::monday(today) + chrono::Duration::days(7);
                    for (label, date) in [
                        ("Today", today),
                        ("Tomorrow", today + chrono::Duration::days(1)),
                        ("Next week", next_week),
                    ] {
                        if ui.button(language.text(label)).clicked() {
                            chosen = Some(Some(date));
                            ui.close();
                        }
                    }
                    if due.is_some() && ui.button(language.text("No due date")).clicked() {
                        chosen = Some(None);
                        ui.close();
                    }
                });
                let mut month = self.tasks.picker_month.unwrap_or(due.unwrap_or(today));
                ui.horizontal(|ui| {
                    if ui
                        .small_button("<")
                        .on_hover_text(language.text("Previous month"))
                        .clicked()
                    {
                        month = View::Month.shift(month, -1);
                    }
                    ui.strong(language.month(month));
                    if ui
                        .small_button(">")
                        .on_hover_text(language.text("Next month"))
                        .clicked()
                    {
                        month = View::Month.shift(month, 1);
                    }
                });
                self.tasks.picker_month = Some(month);
                egui::Grid::new("task-due-calendar")
                    .spacing([2.0, 2.0])
                    .show(ui, |ui| {
                        for day in if language == tomlook::i18n::Language::Cs {
                            ["Po", "Út", "St", "Čt", "Pá", "So", "Ne"]
                        } else {
                            ["M", "T", "W", "T", "F", "S", "S"]
                        } {
                            ui.weak(day);
                        }
                        ui.end_row();
                        for (index, date) in View::Month.days(month).into_iter().enumerate() {
                            let color = if date == today {
                                self.accent()
                            } else if date.month() == month.month() {
                                ui.visuals().text_color()
                            } else {
                                ui.visuals().weak_text_color()
                            };
                            if ui
                                .add_sized(
                                    [26.0, 22.0],
                                    egui::Button::new(
                                        RichText::new(date.day().to_string()).color(color),
                                    )
                                    .selected(due == Some(date)),
                                )
                                .on_hover_text(language.date(date, true))
                                .clicked()
                            {
                                chosen = Some(Some(date));
                                ui.close();
                            }
                            if index % 7 == 6 {
                                ui.end_row();
                            }
                        }
                    });
            })
            .response
            .on_hover_text(language.text("Due date"));
        });
        if chosen.is_some() {
            self.tasks.picker_month = None;
        }
        chosen
    }

    fn task_row(&mut self, ui: &mut egui::Ui, task: &Task, account: &str, today: NaiveDate) {
        let language = self.preferences.language;
        ui.push_id(("task", task.id), |ui| {
            if self.tasks.editing.as_ref().is_some_and(|edit| edit.id == task.id) {
                self.task_editor(ui, task, account);
                return;
            }
            let mut done = task.status == Status::Done;
            let title = if done {
                RichText::new(&task.title).strikethrough().weak()
            } else {
                RichText::new(&task.title)
            };
            if ui
                .checkbox(&mut done, title)
                .on_hover_text(language.text(if done { "Mark as not done" } else { "Mark as done" }))
                .changed()
            {
                self.task_command(TaskCommand::SetDone {
                    account: account.into(),
                    id: task.id,
                    done,
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.add_space(24.0);
                if task.status == Status::Open {
                    if let Some(due) = self.due_picker(ui, "due", task.due) {
                        self.task_command(TaskCommand::Edit {
                            account: account.into(),
                            id: task.id,
                            title: task.title.clone(),
                            note: task.note.clone(),
                            due,
                        });
                    }
                } else if let Some(done_at) = task.done_at {
                    ui.weak(format!(
                        "{} {}",
                        language.text("Done"),
                        due_label(done_at.with_timezone(&Local).date_naive(), today, language)
                    ));
                }
                if let Some(meeting) = &task.meeting {
                    let label = format!(
                        "{} · {}",
                        tomlook::tasks::clip(&meeting.title, 48),
                        due_label(meeting.start.with_timezone(&Local).date_naive(), today, language)
                    );
                    if ui
                        .add(egui::Button::new(RichText::new(label).small().color(self.accent())))
                        .on_hover_text(format!(
                            "{}\n{}",
                            language.text(task.source.label()),
                            language.text("Open meeting details")
                        ))
                        .clicked()
                    {
                        self.open_task_meeting(meeting);
                    }
                } else {
                    ui.weak(RichText::new(language.text(task.source.label())).small());
                }
                if ui.small_button(language.text("Edit")).clicked() {
                    self.tasks.editing = Some(Edit {
                        id: task.id,
                        title: task.title.clone(),
                        note: task.note.clone(),
                        due: task.due,
                        focus: true,
                    });
                }
                if ui
                    .small_button(language.text("Delete"))
                    .on_hover_text(language.text("Delete the task; you can undo it"))
                    .clicked()
                {
                    self.task_command(TaskCommand::Delete {
                        account: account.into(),
                        id: task.id,
                    });
                    self.tasks.deleted = Some(task.clone());
                }
            });
            if !task.note.is_empty() {
                let preview = task.note.lines().take(2).collect::<Vec<_>>().join("\n");
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.add(egui::Label::new(RichText::new(tomlook::tasks::clip(&preview, 220)).small().weak()).wrap())
                        .on_hover_text(&task.note);
                });
            }
            ui.add_space(6.0);
        });
    }

    fn task_editor(&mut self, ui: &mut egui::Ui, task: &Task, account: &str) {
        let language = self.preferences.language;
        let Some(mut edit) = self.tasks.editing.take() else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        egui::Frame::new()
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                let label = ui.label(language.text("Task"));
                let title = ui
                    .add(
                        egui::TextEdit::singleline(&mut edit.title)
                            .id(Id::new(("task-title", task.id)))
                            .desired_width(f32::INFINITY)
                            .char_limit(model::TITLE_LIMIT),
                    )
                    .labelled_by(label.id);
                if edit.focus {
                    title.request_focus();
                    edit.focus = false;
                }
                let enter = title.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                let label = ui.label(language.text("Note (optional)"));
                ui.add(
                    egui::TextEdit::multiline(&mut edit.note)
                        .id(Id::new(("task-note", task.id)))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .char_limit(model::NOTE_LIMIT),
                )
                .labelled_by(label.id);
                if let Some(due) = self.due_picker(ui, "edit-due", edit.due) {
                    edit.due = due;
                }
                let valid = model::validate_title(&edit.title).and(model::validate_note(&edit.note));
                if let Err(error) = &valid {
                    ui.colored_label(ui.visuals().error_fg_color, language.text(error));
                }
                ui.horizontal(|ui| {
                    save = ui
                        .add_enabled(valid.is_ok(), egui::Button::new(language.text("Save")))
                        .clicked()
                        || (enter && valid.is_ok());
                    cancel = ui.button(language.text("Cancel")).clicked();
                });
            });
        if save {
            self.task_command(TaskCommand::Edit {
                account: account.into(),
                id: task.id,
                title: edit.title.trim().into(),
                note: edit.note.clone(),
                due: edit.due,
            });
        } else if !cancel {
            self.tasks.editing = Some(edit);
        }
    }

    pub(super) fn tasks_window(&mut self, context: &egui::Context) {
        if !self.tasks_open {
            return;
        }
        let language = self.preferences.language;
        let mut open = true;
        egui::Window::new(language.text("Tasks"))
            .id(Id::new("tasks-window"))
            .open(&mut open)
            .default_pos(egui::pos2(NAVIGATION_WIDTH + 24.0, 96.0))
            .default_width(520.0)
            .default_height(700.0)
            .min_width(380.0)
            .resizable(true)
            .show(context, |ui| self.tasks_panel(ui));
        if !open {
            self.tasks_open = false;
            self.tasks.editing = None;
        }
    }

    pub(super) fn tasks_panel(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let Some(account) = self.feedback_account() else {
            ui.weak(language.text("Tasks are available once the calendar account is known."));
            return;
        };
        ui.weak(language.text(
            "A local list on this computer. Nothing is sent to To Do, Planner or Dataverse.",
        ));
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.tasks.quick)
                    .id(Id::new("task-quick-add"))
                    .hint_text(language.text("Add a task and press Enter"))
                    .desired_width((ui.available_width() - 190.0).max(160.0))
                    .char_limit(model::TITLE_LIMIT),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, language.text("Add task"))
            });
            if self.tasks.focus_quick {
                response.request_focus();
                self.tasks.focus_quick = false;
            }
            let enter = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            if let Some(due) = self.due_picker(ui, "quick-due", self.tasks.quick_due) {
                self.tasks.quick_due = due;
            }
            let filled = !self.tasks.quick.trim().is_empty();
            if (ui
                .add_enabled(filled, egui::Button::new(language.text("Add")))
                .clicked()
                || enter)
                && filled
            {
                self.add_quick_task(&account);
            }
        });
        if let Some(deleted) = self.tasks.deleted.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{}: {}",
                    language.text("Task deleted"),
                    tomlook::tasks::clip(&deleted.title, 60)
                ));
                if ui.button(language.text("Undo")).clicked() {
                    self.task_command(TaskCommand::Restore {
                        account: account.clone(),
                        task: Box::new(deleted),
                    });
                    self.tasks.deleted = None;
                }
                if ui.small_button(language.text("Dismiss")).clicked() {
                    self.tasks.deleted = None;
                }
            });
        }
        ui.separator();
        let tasks = self.activity.tasks.clone();
        let today = Local::now().date_naive();
        let rows = model::arranged(&tasks, today, chrono::Utc::now())
            .into_iter()
            .filter(|(_, task)| task.account == account)
            .collect::<Vec<_>>();
        egui::ScrollArea::vertical()
            .id_salt("task-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !rows.iter().any(|(group, _)| *group != Group::Done) {
                    ui.add_space(8.0);
                    ui.weak(language.text("No open tasks. Tasks from meeting notes appear here automatically."));
                }
                let mut heading = None;
                for (group, task) in rows.iter().filter(|(group, _)| *group != Group::Done) {
                    if heading != Some(*group) {
                        heading = Some(*group);
                        ui.add_space(6.0);
                        let text = RichText::new(language.text(group.label())).strong();
                        ui.label(if *group == Group::Overdue {
                            text.color(ui.visuals().error_fg_color)
                        } else {
                            text
                        });
                    }
                    self.task_row(ui, task, &account, today);
                }
                let done = rows
                    .iter()
                    .filter(|(group, _)| *group == Group::Done)
                    .collect::<Vec<_>>();
                if !done.is_empty() {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new(format!(
                        "{} ({})",
                        language.text("Done in the last 30 days"),
                        done.len()
                    ))
                    .id_salt("task-done")
                    .default_open(false)
                    .show(ui, |ui| {
                        for (_, task) in done {
                            self.task_row(ui, task, &account, today);
                        }
                    });
                }
            });
    }

    /// Tasks of the selected meeting, below the notes' personal tasks.
    pub(super) fn meeting_tasks(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let Some(event) = self
            .selected
            .and_then(|index| self.calendar.events.get(index))
            .cloned()
        else {
            return;
        };
        let Some(account) = self.feedback_account() else {
            return;
        };
        let tasks = self.activity.tasks.clone();
        let linked = model::for_meeting(&tasks, &account, &event);
        if linked.is_empty() {
            return;
        }
        let today = Local::now().date_naive();
        let frame = egui::Frame::new()
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(8)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.strong(language.text("In my task list"));
                    ui.weak(linked.len().to_string());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(language.text("Open tasks")).clicked() {
                            self.open_tasks(false);
                        }
                    });
                });
                for task in linked {
                    ui.push_id(("meeting-task", task.id), |ui| {
                        let mut done = task.status == Status::Done;
                        let title = if done {
                            RichText::new(&task.title).strikethrough().weak()
                        } else {
                            RichText::new(&task.title)
                        };
                        if ui.checkbox(&mut done, title).changed() {
                            self.task_command(TaskCommand::SetDone {
                                account: account.clone(),
                                id: task.id,
                                done,
                            });
                        }
                        if let Some(due) = task.due.filter(|_| task.status == Status::Open) {
                            ui.horizontal(|ui| {
                                ui.add_space(24.0);
                                let text = RichText::new(format!(
                                    "{} {}",
                                    language.text("Due"),
                                    due_label(due, today, language)
                                ))
                                .small();
                                ui.label(if due < today {
                                    text.color(ui.visuals().error_fg_color)
                                } else {
                                    text.weak()
                                });
                            });
                        }
                    });
                }
            });
        if self.preview
            && self.demo_screenshot.is_some()
            && self.demo_scene == tomlook::showcase::Scene::Tasks
        {
            frame.response.scroll_to_me(Some(egui::Align::Center));
        }
        ui.add_space(8.0);
    }
}
