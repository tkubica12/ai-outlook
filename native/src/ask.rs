use crate::{ai, calendar::Event, i18n::Language, recommendations::Review};
use chrono::{Datelike, Duration, Local};
use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Today,
    Week,
    Prepare,
    Conflicts,
    Find,
    Travel,
    Categories,
}

impl Command {
    pub const ALL: [Self; 7] = [
        Self::Today,
        Self::Week,
        Self::Prepare,
        Self::Conflicts,
        Self::Find,
        Self::Travel,
        Self::Categories,
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
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Today => "What is on my calendar today",
            Self::Week => "Important meetings this week",
            Self::Prepare => "Prepare for the contextual meeting",
            Self::Conflicts => "Explain calendar conflicts",
            Self::Find => "Find a meeting, message or material",
            Self::Travel => "Missing travel time: next 14 days, results in Messages",
            Self::Categories => "Review categories: next 14 days, results in Messages",
        }
    }
}

pub fn matches(text: &str, language: Language) -> Vec<Command> {
    let query = text.trim().to_lowercase();
    if query.len() > 64
        || text.contains('\n')
        || text.trim_start().starts_with('/') && text.trim_start().contains(char::is_whitespace)
    {
        return Vec::new();
    }
    Command::ALL
        .into_iter()
        .filter(|command| {
            query.is_empty()
                || command.name().contains(&query)
                || !query.starts_with('/')
                    && language
                        .text(command.description())
                        .to_lowercase()
                        .contains(&query)
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Submission {
    Question(String),
    Review(Review),
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
        Command::Today => format!(
            "{} {today}.",
            language.text("Summarize my calendar for this local date, with important meetings and preparation needs:")
        ),
        Command::Week => {
            let first = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
            let last = first + Duration::days(6);
            format!(
                "{} {first} - {last}.",
                language.text("Summarize important meetings and preparation needs for this local calendar week:")
            )
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Submit,
    Stop,
    Hide,
    Detach,
    Dock,
    UseSelected,
    ClearContext,
    Exit,
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
}

pub struct Input {
    pub action: Action,
    pub question: String,
    pub cursor: usize,
    pub menu_dismissed: bool,
    pub context_id: Option<String>,
    pub revision: Option<String>,
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

pub fn draw(ui: &mut egui::Ui, view: &mut View, detached: bool) -> Option<Action> {
    let language = view.language;
    let mut action = None;
    let focused = ui
        .ctx()
        .memory(|memory| memory.focused() == Some(Id::new("assistant-question")));
    let choices = if view.menu_dismissed || view.busy {
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
    if focused && !view.busy && !choices.is_empty() {
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
            if ui.small_button(language.text("Hide Ask")).clicked() {
                action = Some(Action::Hide);
            }
            if ui
                .small_button(language.text(if detached { "Dock Ask" } else { "Detach Ask" }))
                .clicked()
            {
                action = Some(if detached {
                    Action::Dock
                } else {
                    Action::Detach
                });
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if let Some(event) = &view.context {
            ui.weak(language.text("Meeting context"));
            ui.label(&event.title);
            if ui
                .add_enabled(
                    !view.busy,
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
                        !view.busy,
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
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if editor.changed() {
                view.cursor = 0;
                view.menu_dismissed = false;
            }
            ui.horizontal(|ui| {
                let enabled = view.ready && !view.busy && !view.question.trim().is_empty();
                if (ui
                    .add_enabled(enabled, egui::Button::new(language.text("Ask")))
                    .clicked()
                    || submit)
                    && enabled
                {
                    action = Some(Action::Submit);
                }
                if view.busy {
                    ui.label(RichText::new(language.text("Thinking...")).color(view.accent));
                    if ui.button(language.text("Stop")).clicked() {
                        action = Some(Action::Stop);
                    }
                }
            });
            let choices = if view.menu_dismissed || view.busy {
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
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(answer) = &view.answer {
                answer.markdown.show(ui);
                ui.add_space(8.0);
                if ui.small_button(language.text("Copy answer")).clicked() {
                    ui.ctx().copy_text(answer.text.as_ref().clone());
                }
                if !answer.evidence.records.is_empty() {
                    ui.collapsing(language.text(&(format!("Sources ({})", answer.evidence.records.len()))), |ui| {
                        for record in &answer.evidence.records {
                            let label = crate::evidence::source_label(&record.tool, record.resource.as_deref());
                            ui.collapsing(language.text(&(format!("R{}: {label}", record.ordinal))), |ui| {
                                ui.weak(language.text(&(format!("Observed {}", record.observed_at))));
                                ui.label(language.text(if record.succeeded { "Retrieved" } else { "Retrieval failed" }));
                                if record.truncated {
                                    ui.weak(language.text("Excerpt truncated; this is not the complete response."));
                                }
                                for link in record.links.iter().filter(|link| crate::evidence::safe_source_url(link)) {
                                    ui.hyperlink_to(link, link);
                                }
                                ui.label(&record.excerpt);
                            });
                        }
                    });
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_filter_locally_and_completed_commands_leave_room_for_arguments() {
        assert_eq!(matches("", Language::En), Command::ALL);
        assert_eq!(matches("/PRE", Language::En), [Command::Prepare]);
        assert_eq!(matches("Important meetings", Language::En), [Command::Week]);
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
                if matches!(command, Command::Travel | Command::Categories) {
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
        assert_eq!(matches("kategori", Language::Cs), [Command::Categories]);
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
