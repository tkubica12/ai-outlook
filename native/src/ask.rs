use crate::{ai, calendar::Event, i18n::Language, recommendations::Review};
use chrono::{Datelike, Duration, Local};
use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Today,
    Week,
    Prepare,
    Conflicts,
    Find,
    Travel,
    Categories,
    Search,
    Insights,
}

impl Command {
    pub const ALL: [Self; 9] = [
        Self::Today,
        Self::Week,
        Self::Prepare,
        Self::Conflicts,
        Self::Find,
        Self::Travel,
        Self::Categories,
        Self::Search,
        Self::Insights,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Today => "/today",
            Self::Week => "/week",
            Self::Prepare => "/prepare",
            Self::Conflicts => "/conflicts",
            Self::Find => "/find",
            Self::Travel => "/travel",
            Self::Categories => "/categories",
            Self::Search => "/search",
            Self::Insights => "/insights",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Today => "Today's saved calendar / no AI",
            Self::Week => "This week's saved calendar / no AI",
            Self::Prepare => "Prepare for the contextual meeting",
            Self::Conflicts => "Explain calendar conflicts",
            Self::Find => "Find a meeting, message or material",
            Self::Travel => "Missing travel time: next 14 days, results in Messages",
            Self::Categories => "Review categories: next 14 days, results in Messages",
            Self::Search => "Fuzzy search saved meetings / no AI",
            Self::Insights => "Read saved meeting Insights / no AI",
        }
    }
}

pub fn matches(text: &str, language: Language) -> Vec<Command> {
    let query = text.trim().to_lowercase();
    if !query.starts_with('/')
        || query.len() > 64
        || text.contains('\n')
        || text.trim_start().starts_with('/') && text.trim_start().contains(char::is_whitespace)
    {
        return Vec::new();
    }
    Command::ALL
        .into_iter()
        .filter(|command| {
            command.name().contains(&query)
                || language
                    .text(command.description())
                    .to_lowercase()
                    .contains(query.trim_start_matches('/'))
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Submission {
    Question(String),
    Review(Review),
    Local(crate::chat_cache::LocalQuery),
}

pub fn expand(
    text: &str,
    meeting: Option<&Event>,
    language: Language,
) -> Result<Submission, String> {
    let text = text.trim();
    if !text.starts_with('/') {
        return Ok(Submission::Question(text.into()));
    }
    let (name, arguments) = text
        .split_once(char::is_whitespace)
        .map(|(name, arguments)| (name, arguments.trim()))
        .unwrap_or((text, ""));
    let command = Command::ALL
        .into_iter()
        .find(|command| command.name().eq_ignore_ascii_case(name))
        .ok_or("Unknown Ask command. Choose a command below or ask without a leading slash.")?;
    if matches!(
        command,
        Command::Today | Command::Week | Command::Search | Command::Insights
    ) {
        if command != Command::Search && !arguments.is_empty() {
            return Err(
                "This local command takes no arguments. Use /search followed by meeting words."
                    .into(),
            );
        }
        if command == Command::Search && (arguments.is_empty() || arguments.len() > 200) {
            return Err("After /search, enter up to 200 characters from the meeting title, person or location.".into());
        }
        if command == Command::Insights && meeting.is_none() {
            return Err("Choose a meeting context before reading saved Insights".into());
        }
        return Ok(Submission::Local(match command {
            Command::Today => crate::chat_cache::LocalQuery::Today,
            Command::Week => crate::chat_cache::LocalQuery::Week,
            Command::Search => crate::chat_cache::LocalQuery::Search(arguments.into()),
            Command::Insights => crate::chat_cache::LocalQuery::Insights,
            _ => unreachable!(),
        }));
    }
    if matches!(command, Command::Travel | Command::Categories) {
        if !arguments.is_empty() {
            return Err(
                "Use /travel or /categories without arguments; each checks the next 14 days."
                    .into(),
            );
        }
        return Ok(Submission::Review(if command == Command::Travel {
            Review::Travel
        } else {
            Review::Categories
        }));
    }
    let today = Local::now().date_naive();
    let prompt = match command {
        Command::Today | Command::Week | Command::Search | Command::Insights => {
            unreachable!("Local commands are dispatched separately")
        }
        Command::Prepare => {
            if meeting.is_none() {
                return Err("Choose a meeting context before using /prepare.".into());
            }
            language
                .text("Explain how to prepare for the contextual meeting using saved Insights and retrieved evidence. Distinguish verified needs, reusable material and missing information.")
                .into_owned()
        }
        Command::Conflicts => {
            let first = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
            let last = first + Duration::days(6);
            format!("{} {first} - {last}.", language.text("Find conflicts in my calendar for these local dates. Name both meetings and their full local time ranges, and explain practical resolution options without changing the calendar:"))
        }
        Command::Find => {
            if arguments.is_empty() {
                return Err("After /find, enter what you want to find.".into());
            }
            language
                .text("Find the requested meeting, message or preparation material in authorized workplace sources. Cite retrieved sources and distinguish no matches from unavailable access.")
                .into_owned()
        }
        Command::Travel | Command::Categories => unreachable!("Reviews are dispatched separately"),
    };
    let result = if arguments.is_empty() {
        prompt
    } else {
        format!("{prompt}\n\n{arguments}")
    };
    if result.len() > 8000 {
        return Err("Shorten the question to 8,000 bytes.".into());
    }
    Ok(Submission::Question(result))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Submit,
    Stop,
    Hide,
    Detach,
    Dock,
    UseSelected,
    ClearContext,
    Exit,
    ModeChanged,
    NewChat,
    StopRequest(u64),
    OpenMeeting(String),
    Resume(String),
    DiscardResearch(String),
    Retry(u64),
    ConfigureAuto,
}

#[derive(Clone)]
pub struct View {
    pub question: String,
    pub answer: Option<ai::Answer>,
    pub error: Option<String>,
    pub context: Option<Event>,
    pub revision: Option<String>,
    pub selected: bool,
    pub ready: bool,
    pub busy: bool,
    pub cursor: usize,
    pub menu_dismissed: bool,
    pub focus: bool,
    pub language: Language,
    pub accent: Color32,
    pub position: Option<egui::Pos2>,
    pub size: egui::Vec2,
    pub chat: Arc<crate::chat::Conversation>,
    pub recoveries: Arc<Vec<crate::chat::Checkpoint>>,
    pub mode: crate::chat::Mode,
    pub auto_status: crate::decision::Status,
}

pub struct Input {
    pub action: Action,
    pub question: String,
    pub cursor: usize,
    pub menu_dismissed: bool,
    pub context_id: Option<String>,
    pub revision: Option<String>,
    pub mode: crate::chat::Mode,
}

impl Default for View {
    fn default() -> Self {
        Self {
            question: String::new(),
            answer: None,
            error: None,
            context: None,
            revision: None,
            selected: false,
            ready: false,
            busy: false,
            cursor: 0,
            menu_dismissed: false,
            focus: false,
            language: Language::En,
            accent: Color32::LIGHT_BLUE,
            position: None,
            size: egui::vec2(560.0, 740.0),
            chat: Default::default(),
            recoveries: Default::default(),
            mode: Default::default(),
            auto_status: Default::default(),
        }
    }
}

impl View {
    pub fn input(&self, action: Action) -> Input {
        Input {
            action,
            question: self.question.clone(),
            cursor: self.cursor,
            menu_dismissed: self.menu_dismissed,
            context_id: self.context.as_ref().map(|event| event.id.clone()),
            revision: self.revision.clone(),
            mode: self.mode,
        }
    }
}

fn complete(context: &egui::Context, view: &mut View, command: Command) {
    view.question = format!("{} ", command.name());
    view.menu_dismissed = true;
    view.focus = true;
    let id = Id::new("assistant-question");
    if let Some(mut state) = egui::TextEdit::load_state(context, id) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(view.question.chars().count()),
            )));
        egui::TextEdit::store_state(context, id, state);
    }
}

#[derive(Clone, Copy, Debug, Hash)]
enum ToolbarIcon {
    NewChat,
    Detach,
    Dock,
    Hide,
}

fn toolbar_button(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    label: &str,
    enabled: bool,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let response = ui
        .push_id(("ask-toolbar", icon), |ui| {
            ui.add_enabled(
                enabled,
                egui::Button::new("")
                    .frame(false)
                    .min_size(egui::vec2(28.0, 28.0)),
            )
        })
        .inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if ui.is_rect_visible(response.rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        if response.hovered() || response.has_focus() || response.is_pointer_button_down_on() {
            painter.rect_filled(response.rect, 6, visuals.weak_bg_fill);
        }
        let origin = response.rect.center() - egui::vec2(10.0, 10.0);
        let point = |x, y| origin + egui::vec2(x, y);
        let stroke = egui::Stroke::new(1.5, visuals.fg_stroke.color);
        match icon {
            ToolbarIcon::NewChat => {
                painter.add(egui::Shape::closed_line(
                    vec![
                        point(2.0, 5.0),
                        point(2.0, 4.0),
                        point(3.0, 3.0),
                        point(17.0, 3.0),
                        point(18.0, 4.0),
                        point(18.0, 14.0),
                        point(17.0, 15.0),
                        point(9.0, 15.0),
                        point(5.0, 18.0),
                        point(5.0, 15.0),
                        point(3.0, 15.0),
                        point(2.0, 14.0),
                    ],
                    stroke,
                ));
                painter.line_segment([point(7.0, 9.0), point(13.0, 9.0)], stroke);
                painter.line_segment([point(10.0, 6.0), point(10.0, 12.0)], stroke);
            }
            ToolbarIcon::Detach => {
                painter.add(egui::Shape::line(
                    vec![
                        point(17.0, 11.0),
                        point(17.0, 17.0),
                        point(3.0, 17.0),
                        point(3.0, 3.0),
                        point(9.0, 3.0),
                    ],
                    stroke,
                ));
                painter.line_segment([point(9.0, 11.0), point(17.0, 3.0)], stroke);
                painter.add(egui::Shape::line(
                    vec![point(11.0, 3.0), point(17.0, 3.0), point(17.0, 9.0)],
                    stroke,
                ));
            }
            ToolbarIcon::Dock => {
                painter.rect_stroke(
                    egui::Rect::from_min_max(point(3.0, 3.0), point(17.0, 17.0)),
                    2,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                painter.line_segment([point(12.0, 3.0), point(12.0, 17.0)], stroke);
                painter.line_segment([point(5.0, 10.0), point(15.0, 10.0)], stroke);
                painter.add(egui::Shape::line(
                    vec![point(8.0, 7.0), point(5.0, 10.0), point(8.0, 13.0)],
                    stroke,
                ));
            }
            ToolbarIcon::Hide => {
                painter.line_segment([point(5.0, 5.0), point(15.0, 15.0)], stroke);
                painter.line_segment([point(15.0, 5.0), point(5.0, 15.0)], stroke);
            }
        }
        if response.has_focus() {
            painter.rect_stroke(
                response.rect,
                6,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
    response.on_hover_text(label).on_disabled_hover_text(label)
}

pub fn draw(ui: &mut egui::Ui, view: &mut View, detached: bool) -> Option<Action> {
    let language = view.language;
    let mut action = None;
    let focused = ui
        .ctx()
        .memory(|memory| memory.focused() == Some(Id::new("assistant-question")));
    let choices = if view.menu_dismissed {
        Vec::new()
    } else {
        matches(&view.question, language)
    };
    let context = ui.ctx().clone();
    let key = |key| {
        context.input_mut(|input| {
            let exact = input.events.iter().any(|event| {
                matches!(event,
                egui::Event::Key { key: pressed_key, pressed: true, modifiers, .. }
                    if *pressed_key == key && *modifiers == Modifiers::NONE)
            });
            exact && input.consume_key(Modifiers::NONE, key)
        })
    };
    if focused && !choices.is_empty() {
        if key(Key::ArrowDown) {
            view.cursor = (view.cursor + 1).min(choices.len() - 1);
            view.focus = true;
        }
        if key(Key::ArrowUp) {
            view.cursor = view.cursor.saturating_sub(1);
            view.focus = true;
        }
        if key(Key::Tab) {
            complete(ui.ctx(), view, choices[view.cursor.min(choices.len() - 1)]);
        }
    }
    if key(Key::Escape) {
        if !choices.is_empty() && !view.menu_dismissed {
            view.menu_dismissed = true;
            view.focus = true;
        } else {
            action = Some(Action::Hide);
        }
    }
    if detached {
        if ui
            .ctx()
            .input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Space))
        {
            action = Some(Action::Hide);
        }
        if ui
            .ctx()
            .input_mut(|input| input.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Q))
        {
            action = Some(Action::Exit);
        }
    }
    ui.horizontal(|ui| {
        ui.strong(language.text("Ask AI"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if toolbar_button(ui, ToolbarIcon::Hide, &language.text("Hide Ask"), true).clicked() {
                action = Some(Action::Hide);
            }
            if toolbar_button(
                ui,
                if detached {
                    ToolbarIcon::Dock
                } else {
                    ToolbarIcon::Detach
                },
                &language.text(if detached { "Dock Ask" } else { "Detach Ask" }),
                true,
            )
            .clicked()
            {
                action = Some(if detached {
                    Action::Dock
                } else {
                    Action::Detach
                });
            }
            if toolbar_button(
                ui,
                ToolbarIcon::NewChat,
                &language.text("New chat"),
                !view.chat.running() && !view.busy,
            )
            .clicked()
            {
                action = Some(Action::NewChat);
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        for mode in [
            crate::chat::Mode::Auto,
            crate::chat::Mode::Fast,
            crate::chat::Mode::Deep,
        ] {
            if ui
                .selectable_value(&mut view.mode, mode, language.text(mode.label()))
                .on_hover_text(language.text(match mode {
                    crate::chat::Mode::Auto => {
                        "Microsoft-Decision-1 selects Quick or Thorough; no local guessing"
                    }
                    crate::chat::Mode::Fast => "GPT-6 Luna / low / targeted reads",
                    crate::chat::Mode::Deep => "GPT-6.1 Sol / medium / resumable research",
                }))
                .changed()
            {
                action = Some(Action::ModeChanged);
            }
        }
    });
    if view.mode == crate::chat::Mode::Auto
        && view.auto_status.phase != crate::decision::Phase::SignedIn
    {
        ui.horizontal_wrapped(|ui| {
            ui.weak(language.text(view.auto_status.label()));
            if ui
                .small_button(language.text("Fix Foundry connection"))
                .clicked()
            {
                action = Some(Action::ConfigureAuto);
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        if let Some(event) = &view.context {
            ui.weak(language.text("Meeting context"));
            ui.label(&event.title);
            if ui
                .add_enabled(
                    !view.chat.running() && !view.busy,
                    egui::Button::new(language.text("Clear context")).small(),
                )
                .clicked()
            {
                action = Some(Action::ClearContext);
            }
        } else {
            ui.weak(language.text("General question"));
            if view.selected
                && ui
                    .add_enabled(
                        !view.chat.running() && !view.busy,
                        egui::Button::new(language.text("Use selected meeting")).small(),
                    )
                    .clicked()
            {
                action = Some(Action::UseSelected);
            }
        }
    });
    ui.separator();
    egui::Panel::bottom("ask-composer")
        .show_separator_line(false)
        .show(ui, |ui| {
            if let Some(error) = &view.error {
                ui.label(RichText::new(language.text(error)).strong());
            }
            let submit = focused && key(Key::Enter);
            let editor = ui.add(
                egui::TextEdit::multiline(&mut view.question)
                    .id(Id::new("assistant-question"))
                    .hint_text(language.text("Ask a question or type / for commands"))
                    .char_limit(8000)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            if editor.changed() {
                view.cursor = 0;
                view.menu_dismissed = false;
            }
            ui.horizontal(|ui| {
                let local = matches!(
                    expand(&view.question, view.context.as_ref(), language),
                    Ok(Submission::Local(_))
                );
                let lane_busy = if local {
                    view.chat
                        .turns
                        .iter()
                        .any(|turn| turn.running && turn.progress.stage == "Local calendar")
                } else if view.chat.turns.is_empty() {
                    view.busy
                } else {
                    view.chat.turns.iter().any(|turn| {
                        turn.running
                            && (turn.progress.mode == crate::chat::Mode::Deep)
                                == (view.mode == crate::chat::Mode::Deep)
                    })
                };
                let enabled =
                    (view.ready || local) && !lane_busy && !view.question.trim().is_empty();
                if (ui
                    .add_enabled(enabled, egui::Button::new(language.text("Send")))
                    .clicked()
                    || submit)
                    && enabled
                {
                    action = Some(Action::Submit);
                }
                if view.busy && view.chat.turns.is_empty() {
                    ui.label(RichText::new(language.text("Thinking...")).color(view.accent));
                    if ui.button(language.text("Stop")).clicked() {
                        action = Some(Action::Stop);
                    }
                }
            });
            let choices = if view.menu_dismissed {
                Vec::new()
            } else {
                matches(&view.question, language)
            };
            view.cursor = view.cursor.min(choices.len().saturating_sub(1));
            for (index, command) in choices.iter().enumerate() {
                let response = ui.add_sized(
                    [ui.available_width(), 28.0],
                    egui::Button::new(RichText::new(command.name()).monospace().color(view.accent))
                        .shortcut_text(language.text(command.description()))
                        .selected(view.cursor == index)
                        .wrap(),
                );
                if response.clicked() {
                    complete(ui.ctx(), view, *command);
                }
                if response.hovered() {
                    view.cursor = index;
                }
            }
        });
    egui::ScrollArea::vertical()
        .id_salt("assistant-answer")
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if view.chat.turns.is_empty() && view.answer.is_none() {
                ui.add_space(20.0);
                ui.heading(language.text("Your calendar, in conversation"));
                ui.add_space(8.0);
                ui.weak(language.text("Ask naturally, or type / for instant local commands. Thorough research leaves the quick-answer lane free."));
            }
            for (index,turn) in view.chat.turns.iter().enumerate() {
                ui.push_id(turn.id,|ui| {
                    if index+6<view.chat.turns.len() && !turn.running {
                        ui.collapsing(turn.question.chars().take(80).collect::<String>(),|ui| {
                            turn_ui(ui,turn,view, &mut action);
                        });
                    } else { turn_ui(ui,turn,view,&mut action); }
                });
            }
            if view.chat.turns.is_empty() && let Some(answer) = &view.answer {
                answer_ui(ui,answer,language);
            }
            for saved in view.recoveries.iter() {
                ui.push_id(&saved.key,|ui| {
                    ui.add_space(10.0);
                    egui::Frame::group(ui.style()).inner_margin(12.0).show(ui,|ui| {
                        ui.strong(language.text("Interrupted research"));
                        ui.label(saved.question.chars().take(240).collect::<String>());
                        ui.weak(language.text("Continue from saved findings; the original provider call is not resumed."));
                        if ui.add_enabled(view.ready && !view.chat.turns.iter().any(|turn| turn.running && turn.progress.mode==crate::chat::Mode::Deep),egui::Button::new(language.text("Continue research"))).clicked() {
                            action=Some(Action::Resume(saved.key.clone()));
                        }
                        ui.collapsing(language.text("Discard saved research"),|ui| {
                            ui.weak(language.text("This permanently removes only these saved findings, not meetings or source documents."));
                            if ui.small_button(language.text("Confirm discard")).clicked() {
                                action=Some(Action::DiscardResearch(saved.key.clone()));
                            }
                        });
                    });
                });
            }
        });
    if view.focus {
        ui.ctx().memory_mut(|memory| {
            memory.request_focus(Id::new("assistant-question"));
            memory.move_focus(egui::FocusDirection::None);
        });
        view.focus = false;
    }
    ui.ctx().memory_mut(|memory| {
        memory.set_focus_lock_filter(
            Id::new("assistant-question"),
            egui::EventFilter {
                tab: false,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        );
    });
    action
}

fn turn_ui(ui: &mut egui::Ui, turn: &crate::chat::Turn, view: &View, action: &mut Option<Action>) {
    let language = view.language;
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
        egui::Frame::new()
            .fill(view.accent.linear_multiply(0.14))
            .corner_radius(10.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(460.0) * 0.88);
                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.weak(language.text("You"));
                    ui.add(egui::Label::new(&turn.question).wrap());
                });
            });
    });
    ui.add_space(10.0);
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(10.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Tomlook");
                if turn.progress.stage == "Local calendar" {
                    ui.weak(language.text("Local calendar / no AI"));
                } else {
                    ui.weak(language.text(turn.progress.mode.label()))
                        .on_hover_text(format!(
                            "{} / {}",
                            turn.progress.mode.model(),
                            turn.progress.mode.effort()
                        ));
                }
            });
            if turn.running {
                ui.horizontal_wrapped(|ui| {
                    ui.spinner();
                    ui.label(language.text(&turn.progress.stage));
                    ui.weak(format!(
                        "{} / {} s",
                        language.text(turn.progress.mode.label()),
                        turn.started.elapsed().as_secs()
                    ));
                    if ui.small_button(language.text("Stop")).clicked() {
                        *action = Some(Action::StopRequest(turn.id));
                    }
                });
                if let Some(preview) = &turn.progress.preview {
                    preview.show(ui);
                }
                if ui.is_visible() {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(250));
                }
            }
            if !turn.progress.steps.is_empty() {
                ui.collapsing(language.text("Activity"), |ui| {
                    for step in &turn.progress.steps {
                        ui.weak(language.text(step));
                    }
                });
            }
            if let Some(answer) = &turn.answer {
                answer_ui(ui, answer, language);
            }
            if let Some(error) = &turn.error {
                ui.add_space(6.0);
                ui.strong(language.text("Could not complete"));
                ui.label(language.text(error));
                if crate::decision::routing_error(error).is_some()
                    && ui
                        .small_button(language.text("Fix Foundry connection"))
                        .clicked()
                {
                    *action = Some(Action::ConfigureAuto);
                }
                let local = matches!(
                    expand(&turn.question, view.context.as_ref(), language),
                    Ok(Submission::Local(_))
                );
                if ui
                    .add_enabled(
                        view.ready || local,
                        egui::Button::new(language.text("Retry question")).small(),
                    )
                    .clicked()
                {
                    *action = Some(Action::Retry(turn.id));
                }
            }
            if !turn.meetings.is_empty() {
                ui.collapsing(language.text("Open a meeting"), |ui| {
                    for (id, title) in &turn.meetings {
                        if ui
                            .add(
                                egui::Button::new(RichText::new(title).color(view.accent))
                                    .frame(false)
                                    .wrap_mode(egui::TextWrapMode::Wrap),
                            )
                            .clicked()
                        {
                            *action = Some(Action::OpenMeeting(id.clone()));
                        }
                    }
                });
            }
        });
}

fn answer_ui(ui: &mut egui::Ui, answer: &ai::Answer, language: Language) {
    answer.markdown.show(ui);
    ui.add_space(8.0);
    if ui.small_button(language.text("Copy answer")).clicked() {
        ui.ctx().copy_text(answer.text.as_ref().clone());
    }
    if !answer.evidence.records.is_empty() {
        ui.collapsing(
            language.text(&format!("Sources ({})", answer.evidence.records.len())),
            |ui| {
                for record in &answer.evidence.records {
                    let label =
                        crate::evidence::source_label(&record.tool, record.resource.as_deref());
                    ui.collapsing(
                        language.text(&format!("R{}: {label}", record.ordinal)),
                        |ui| {
                            ui.weak(language.text(&format!("Observed {}", record.observed_at)));
                            ui.label(language.text(if record.succeeded {
                                "Retrieved"
                            } else {
                                "Retrieval failed"
                            }));
                            if record.truncated {
                                ui.weak(
                                    language.text(
                                        "Excerpt truncated; this is not the complete response.",
                                    ),
                                );
                            }
                            for link in record
                                .links
                                .iter()
                                .filter(|link| crate::evidence::safe_source_url(link))
                            {
                                ui.hyperlink_to(link, link);
                            }
                            ui.label(&record.excerpt);
                        },
                    );
                }
            },
        );
    }
    if answer.evidence.omitted > 0 {
        ui.weak(language.text(&format!(
            "{} additional tool results not retained.",
            answer.evidence.omitted
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(foreground: Color32, background: Color32) -> f32 {
        let luminance = |color: Color32| {
            let [red, green, blue, _] = color.to_array();
            let linear = |channel: u8| {
                let value = f32::from(channel) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
        };
        let foreground = luminance(foreground);
        let background = luminance(background);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    }

    #[test]
    fn toolbar_icons_are_vectors_with_consistent_targets_and_light_dark_contrast() {
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let context = egui::Context::default();
            let background = visuals.panel_fill;
            context.set_visuals(visuals);
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                ui.horizontal(|ui| {
                    for icon in [
                        ToolbarIcon::NewChat,
                        ToolbarIcon::Detach,
                        ToolbarIcon::Dock,
                        ToolbarIcon::Hide,
                    ] {
                        let response = toolbar_button(ui, icon, "Accessible action", true);
                        assert_eq!(response.rect.size(), egui::vec2(28.0, 28.0));
                    }
                });
            });
            let mut strokes = 0;
            for shape in &output.shapes {
                assert!(
                    !matches!(&shape.shape, egui::Shape::Text(text) if !text.galley.job.text.is_empty()),
                    "Icons must not depend on font glyphs"
                );
                if let egui::Shape::LineSegment { stroke, .. } = &shape.shape {
                    assert!(contrast(stroke.color, background) >= 3.0);
                    strokes += 1;
                }
            }
            assert!(strokes >= 7);
            output.textures_delta.clear();
        }
    }

    #[test]
    fn toolbar_icons_accept_keyboard_activation_and_respect_disabled_state() {
        for icon in [
            ToolbarIcon::NewChat,
            ToolbarIcon::Detach,
            ToolbarIcon::Dock,
            ToolbarIcon::Hide,
        ] {
            for enabled in [false, true] {
                let context = egui::Context::default();
                let mut identity = None;
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    let response = toolbar_button(ui, icon, "Accessible action", enabled);
                    identity = Some(response.id);
                    assert_eq!(response.enabled(), enabled);
                });
                output.textures_delta.clear();
                context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
                let mut clicked = false;
                let mut output = context.run_ui(
                    egui::RawInput {
                        events: vec![key(Key::Enter)],
                        ..Default::default()
                    },
                    |ui| {
                        clicked = toolbar_button(ui, icon, "Accessible action", enabled).clicked();
                    },
                );
                output.textures_delta.clear();
                assert_eq!(clicked, enabled);
            }
        }
    }

    #[test]
    fn commands_filter_locally_and_completed_commands_leave_room_for_arguments() {
        assert!(matches("", Language::En).is_empty());
        assert_eq!(matches("/", Language::En), Command::ALL);
        assert_eq!(matches("/PRE", Language::En), [Command::Prepare]);
        assert!(matches("Important meetings", Language::En).is_empty());
        assert!(matches("/find agenda", Language::En).is_empty());
        assert!(matches("/find ", Language::En).is_empty());
        assert!(matches("An ordinary unrelated question", Language::En).is_empty());
    }

    #[test]
    fn command_expansion_is_bounded_read_only_and_requires_explicit_preparation_context() {
        assert!(expand("/prepare", None, Language::En).is_err());
        assert!(expand("/find", None, Language::En).is_err());
        assert!(expand("/unknown", None, Language::En).is_err());
        assert!(expand(&format!("/find {}", "x".repeat(8000)), None, Language::En).is_err());
        for language in [Language::En, Language::Cs] {
            let event = crate::calendar::fixture(1).remove(0);
            for command in Command::ALL {
                if matches!(
                    command,
                    Command::Travel
                        | Command::Categories
                        | Command::Today
                        | Command::Week
                        | Command::Search
                        | Command::Insights
                ) {
                    continue;
                }
                let text = format!("{} agenda", command.name());
                let Submission::Question(prompt) = expand(&text, Some(&event), language).unwrap()
                else {
                    panic!("Expected a read-only question");
                };
                assert!(!prompt.starts_with('/'));
                assert!(prompt.ends_with("agenda"));
                assert!(prompt.len() <= 8000);
            }
        }
        assert_eq!(
            expand("Normal question", None, Language::En).unwrap(),
            Submission::Question("Normal question".into())
        );
    }

    #[test]
    fn recommendation_commands_are_local_actions_not_model_prompts() {
        for language in [Language::En, Language::Cs] {
            for (name, review) in [
                ("/TRAVEL ", Review::Travel),
                ("/categories", Review::Categories),
            ] {
                assert_eq!(
                    expand(name, None, language).unwrap(),
                    Submission::Review(review)
                );
                assert!(expand(&format!("{name} 30 days"), None, language).is_err());
            }
        }
        assert_eq!(matches("/trav", Language::Cs), [Command::Travel]);
        assert_eq!(matches("/kategori", Language::Cs), [Command::Categories]);
    }

    fn frame(
        context: &egui::Context,
        view: &mut View,
        events: Vec<egui::Event>,
    ) -> (Option<Action>, egui::FullOutput) {
        let mut action = None;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(520.0, 740.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                action = draw(ui, view, false);
            },
        );
        output.textures_delta.clear();
        (action, output)
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn auto_routing_errors_have_a_direct_connection_action_without_submission() {
        let context = egui::Context::default();
        let mut view = View {
            ready: true,
            mode: crate::chat::Mode::Auto,
            auto_status: crate::decision::Status::new(
                crate::decision::Phase::SignInRequired,
                "Azure sign-in required",
            ),
            ..Default::default()
        };
        let error = "Microsoft-Decision-1 returned HTTP 401; choose a mode explicitly";
        Arc::make_mut(&mut view.chat)
            .admit(1, "Synthetic Auto question".into(), crate::chat::Mode::Auto)
            .unwrap();
        Arc::make_mut(&mut view.chat).finish(1, Err(error.into()));
        assert!(frame(&context, &mut view, vec![]).0.is_none());
        let (_, output) = frame(&context, &mut view, vec![]);
        let positions = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text == "Fix Foundry connection" =>
                {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            positions.len(),
            2,
            "Both Auto mode and the failed turn must expose a repair action"
        );
        let position = positions[1];
        let (action, _) = frame(
            &context,
            &mut view,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert_eq!(action, Some(Action::ConfigureAuto));
        assert_eq!(view.chat.turns[0].error.as_deref(), Some(error));
    }

    #[test]
    fn command_keyboard_completion_moves_the_caret_and_never_submits() {
        let context = egui::Context::default();
        let mut view = View {
            question: "/".into(),
            focus: true,
            ready: true,
            ..Default::default()
        };
        frame(&context, &mut view, vec![]);
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("assistant-question"))
        );
        assert!(
            frame(&context, &mut view, vec![key(Key::ArrowDown)])
                .0
                .is_none()
        );
        assert_eq!(view.cursor, 1);
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("assistant-question"))
        );
        assert!(frame(&context, &mut view, vec![key(Key::Tab)]).0.is_none());
        assert_eq!(view.question, "/week ");
        assert!(
            frame(
                &context,
                &mut view,
                vec![egui::Event::Text("agenda".into())]
            )
            .0
            .is_none()
        );
        assert_eq!(view.question, "/week agenda");
        assert_eq!(
            frame(&context, &mut view, vec![key(Key::Enter)]).0,
            Some(Action::Submit)
        );
    }

    #[test]
    fn escape_dismisses_suggestions_before_hiding() {
        let context = egui::Context::default();
        let mut view = View {
            question: "/".into(),
            focus: true,
            ..Default::default()
        };
        frame(&context, &mut view, vec![]);
        assert!(
            frame(&context, &mut view, vec![key(Key::Escape)])
                .0
                .is_none()
        );
        assert!(view.menu_dismissed);
        assert_eq!(
            frame(&context, &mut view, vec![key(Key::Escape)]).0,
            Some(Action::Hide)
        );
    }

    #[test]
    fn clicking_command_description_completes_the_whole_row_without_submission() {
        let context = egui::Context::default();
        let mut view = View {
            ready: true,
            question: "/".into(),
            ..Default::default()
        };
        frame(&context, &mut view, vec![]);
        let (_, output) = frame(&context, &mut view, vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text == Command::Find.description() =>
                {
                    Some(text.pos + egui::vec2(text.galley.size().x - 16.0, 6.0))
                }
                _ => None,
            })
            .expect("Find command is visible");
        for pressed in [true, false] {
            assert!(
                frame(
                    &context,
                    &mut view,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Modifiers::NONE
                        },
                    ]
                )
                .0
                .is_none()
            );
        }
        assert_eq!(view.question, "/find ");
    }

    #[test]
    fn shift_tab_leaves_the_composer_without_completing_a_command() {
        let context = egui::Context::default();
        let mut view = View {
            question: "/".into(),
            focus: true,
            ..Default::default()
        };
        frame(&context, &mut view, vec![]);
        frame(&context, &mut view, vec![]);
        let mut event = key(Key::Tab);
        if let egui::Event::Key { modifiers, .. } = &mut event {
            *modifiers = Modifiers::SHIFT;
        }
        assert!(frame(&context, &mut view, vec![event]).0.is_none());
        frame(&context, &mut view, vec![]);
        assert_ne!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("assistant-question"))
        );
        assert_eq!(view.question, "/");
    }
}
