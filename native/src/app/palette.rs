use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaletteCommand {
    View(View),
    Today,
    MyDay,
    Search,
    Ask,
    Tasks,
    AddTask,
    Prepare,
    Settings,
    Theme,
    Refresh,
    Exit,
}

impl PaletteCommand {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::View(View::Day) => "Show day",
            Self::View(View::WorkWeek) => "Show work week",
            Self::View(View::Week) => "Show week",
            Self::View(View::Month) => "Show month",
            Self::Today => "Go to today",
            Self::MyDay => "My day",
            Self::Search => "Search meetings",
            Self::Ask => "Ask a question",
            Self::Tasks => "Open tasks",
            Self::AddTask => "Add task",
            Self::Prepare => "Research selected meeting now",
            Self::Settings => "Insights activity",
            Self::Theme => "Toggle dark / light",
            Self::Refresh => "Refresh calendar",
            Self::Exit => "Exit Tomlook",
        }
    }
}

#[cfg(test)]
pub(super) fn command_matches(query: &str) -> Vec<PaletteCommand> {
    command_matches_in(query, tomlook::i18n::Language::En)
}

pub(super) fn command_matches_in(query: &str, language: tomlook::i18n::Language) -> Vec<PaletteCommand> {
    let query = query.trim().to_lowercase();
    let mut matches: Vec<_> = [
        PaletteCommand::View(View::Day),
        PaletteCommand::View(View::WorkWeek),
        PaletteCommand::View(View::Week),
        PaletteCommand::View(View::Month),
        PaletteCommand::Today,
        PaletteCommand::MyDay,
        PaletteCommand::Search,
        PaletteCommand::Ask,
        PaletteCommand::Tasks,
        PaletteCommand::AddTask,
        PaletteCommand::Prepare,
        PaletteCommand::Settings,
        PaletteCommand::Theme,
        PaletteCommand::Refresh,
        PaletteCommand::Exit,
    ]
    .into_iter()
    .filter(|command| {
        query.split_whitespace().all(|word| {
            language.text(command.label()).to_lowercase().contains(word)
                || command.label().to_lowercase().contains(word)
        })
    })
    .collect();
    matches.sort_by_key(|command| language.text(command.label()).to_lowercase() != query);
    matches
}

impl Tomlook {
    pub(super) fn open_palette(&mut self) {
        self.palette = true;
        self.palette_query.clear();
        self.palette_cursor = 0;
        self.palette_focus = true;
    }

    pub(super) fn close_palette(&mut self, context: &egui::Context) {
        self.palette = false;
        context.memory_mut(|memory| memory.surrender_focus(Id::new("command-search")));
    }

    pub(super) fn run_palette_command(&mut self, command: PaletteCommand, context: &egui::Context) {
        self.close_palette(context);
        match command {
            PaletteCommand::View(view) => {
                self.choose_view(view);
            }
            PaletteCommand::Today => self.go_today(),
            PaletteCommand::MyDay => self.open_myday(),
            PaletteCommand::Search => {
                self.focus_search = true;
                context.request_repaint();
            }
            PaletteCommand::Ask => {
                self.assistant = true;
                self.focus_assistant = true;
            }
            PaletteCommand::Tasks => self.open_tasks(false),
            PaletteCommand::AddTask => self.open_tasks(true),
            PaletteCommand::Prepare => {
                if let Some(index) = self.selected {
                    self.request_insights(self.calendar.events[index].id.clone());
                } else {
                    self.error = Some("Select a meeting first.".into());
                }
            }
            PaletteCommand::Settings => self.settings = true,
            PaletteCommand::Theme => {
                self.preferences.dark = !self.preferences.dark;
                self.save_preferences();
            }
            PaletteCommand::Refresh => self.refresh_workiq(),
            PaletteCommand::Exit => self.exit(context),
        }
        context.request_repaint();
    }
}
