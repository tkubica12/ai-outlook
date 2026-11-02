use super::*;

pub(super) const ASSISTANT_CONTEXT_LIMIT: usize = 32;

fn ask_viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("tomlook-ask")
}

impl Tomlook {
    pub(super) fn ai_command(&mut self, command: ai::Command) -> bool {
        match self
            .ai
            .as_ref()
            .map(|engine| engine.commands.try_send(command))
        {
            Some(Ok(())) => return true,
            Some(Err(tokio::sync::mpsc::error::TrySendError::Full(_))) => {
                self.ai_error =
                    Some("AI command queue is full; nothing was submitted. Try again.".into())
            }
            _ => self.ai_error = Some("The AI worker is unavailable. Restart Tomlook.".into()),
        }
        false
    }

    pub(super) fn cancel_ai(&mut self) {
        let mut ids = self
            .chat
            .turns
            .iter()
            .filter(|turn| turn.running)
            .map(|turn| turn.id)
            .collect::<std::collections::BTreeSet<_>>();
        ids.extend(self.active_request);
        for id in ids {
            self.cancel_chat_request(id);
        }
    }

    fn cancel_chat_request(&mut self, id: u64) {
        let local = self
            .chat
            .turns
            .iter()
            .any(|turn| turn.id == id && turn.progress.stage == "Local calendar");
        if self.active_request == Some(id) {
            self.active_request = None;
        }
        self.ask_unread = !self.assistant;
        let message = "Cancellation requested; partial output is not a completed answer. Provider outcome may be unknown.";
        self.ai_error = Some(message.into());
        Arc::make_mut(&mut self.chat).finish(id, Err(message.into()));
        if !local && !self.ai_command(ai::Command::Cancel(id)) {
            self.error=Some("AI cancellation could not be submitted. Its result will be discarded, but the provider request may continue until timeout or application exit.".into());
        }
    }

    pub(super) fn invalidate_connection_answers(&mut self) {
        self.ai_error = None;
        self.cancel_ai();
        self.answer = None;
        self.chat = Default::default();
        for state in self.assistant_states.values_mut() {
            state.answer = None;
            state.error = None;
            state.chat = Default::default();
        }
    }

    pub(super) fn switch_assistant_context(&mut self, next: Option<Event>) -> bool {
        if self.assistant_detached {
            let shared = match self.detached_ask.try_lock() {
                Ok(shared) => shared,
                Err(std::sync::TryLockError::WouldBlock) => {
                    self.context_error =
                        Some("Ask is updating. Try changing context again.".into());
                    return false;
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    self.context_error =
                        Some("The Ask window state is unavailable. Restart Tomlook.".into());
                    return false;
                }
            };
            if shared.view.revision == self.assistant_revision
                && shared.view.context.as_ref().map(|event| event.id.as_str())
                    == self
                        .assistant_context
                        .as_ref()
                        .map(|event| event.id.as_str())
            {
                self.question = shared.view.question.clone();
            }
        }
        let current_id = self
            .assistant_context
            .as_ref()
            .map(|event| event.id.clone());
        let next_id = next.as_ref().map(|event| event.id.clone());
        let fingerprint = next.as_ref().map(Event::fingerprint);
        if current_id == next_id {
            self.context_error = None;
            if self.assistant_context.as_ref().map(Event::fingerprint) != fingerprint {
                self.cancel_ai();
                self.answer = None;
                self.chat = Default::default();
                self.ai_error = Some("Meeting context changed. The previous answer was invalidated; review your draft before asking again.".into());
            }
            self.assistant_context = next;
            self.assistant_revision = fingerprint;
            return true;
        }
        if self.active_request.is_some() || self.chat.running() {
            self.ai_error =
                Some("Stop the current question before changing its meeting context.".into());
            return false;
        }
        if !self.assistant_states.contains_key(&next_id)
            && self.assistant_states.len() + 1 >= ASSISTANT_CONTEXT_LIMIT
        {
            self.context_error = Some("Assistant memory holds 32 contexts. Return to an existing context or restart Tomlook to clear memory. No new question can be submitted for the requested context.".into());
            return false;
        }
        let mut restored = self.assistant_states.remove(&next_id).unwrap_or_default();
        self.assistant_states.insert(
            current_id,
            AssistantState {
                question: std::mem::take(&mut self.question),
                answer: self.answer.take(),
                error: self.ai_error.take(),
                fingerprint: self.assistant_context.as_ref().map(Event::fingerprint),
                chat: std::mem::take(&mut self.chat),
            },
        );
        if restored.fingerprint != fingerprint
            && (restored.answer.is_some() || !restored.chat.turns.is_empty())
        {
            restored.answer = None;
            restored.chat = Default::default();
            restored.error = Some("Meeting context changed. The previous answer was invalidated; review your draft before asking again.".into());
        }
        self.question = restored.question;
        self.answer = restored.answer;
        self.ai_error = restored.error;
        self.chat = restored.chat;
        self.assistant_context = next;
        self.assistant_revision = fingerprint;
        self.context_error = None;
        self.focus_assistant = true;
        self.ask_cursor = 0;
        self.ask_menu_dismissed = false;
        true
    }

    pub(super) fn run_meeting_ask_command(&mut self, event: Event, command: ask::Command) {
        self.assistant = true;
        self.focus_assistant = true;
        self.meeting_menu = None;
        if !self.switch_assistant_context(Some(event)) {
            return;
        }
        let draft = std::mem::replace(&mut self.question, command.name().into());
        self.ask_question();
        if !draft.is_empty() {
            self.question = draft;
        }
    }

    pub(super) fn ask_question(&mut self) {
        if self.context_error.is_some() {
            self.ai_error = Some("Connect AI before asking.".into());
            return;
        }
        if self.chat.turns.len() >= tomlook::chat::TURN_LIMIT {
            self.ai_error =
                Some("This chat holds 32 questions; start a new chat to continue".into());
            return;
        }
        if self.question.trim().is_empty() || self.question.len() > 8000 {
            self.ai_error = Some("Use a nonempty question of at most 8,000 bytes.".into());
            return;
        }
        let submission = match ask::expand(
            &self.question,
            self.assistant_context.as_ref(),
            self.preferences.language,
        ) {
            Ok(submission) => submission,
            Err(error) => {
                self.ai_error = Some(error);
                return;
            }
        };
        if ask::requires_thorough(&self.question) {
            self.ask_mode = tomlook::chat::Mode::Deep;
        }
        if matches!(submission, ask::Submission::Question(_))
            && self.chat.turns.iter().any(|turn| {
                turn.running
                    && turn.progress.stage != "Local calendar"
                    && (turn.progress.mode == tomlook::chat::Mode::Deep)
                        == (self.ask_mode == tomlook::chat::Mode::Deep)
            })
        {
            self.ai_error = Some(
                "This answer lane is busy; choose the other mode, wait or stop its question".into(),
            );
            return;
        }
        if matches!(submission, ask::Submission::Question(_))
            && self.active_request.is_some()
            && self.chat.turns.is_empty()
            && self.ask_mode != tomlook::chat::Mode::Deep
        {
            return;
        }
        if matches!(submission, ask::Submission::Local(_))
            && self
                .chat
                .turns
                .iter()
                .any(|turn| turn.running && turn.progress.stage == "Local calendar")
        {
            self.ai_error = Some("A local command is already running; wait for its result".into());
            return;
        }
        let Some(id) = self.request_id.checked_add(1).filter(|id| *id < 1 << 63) else {
            self.ai_error = Some("Assistant request identity limit reached. No question was submitted; restart Tomlook.".into());
            return;
        };
        self.request_id = id;
        self.ai_error = None;
        let raw = self.question.clone();
        let question = match submission {
            ask::Submission::Question(question) => question,
            ask::Submission::Review(kind) => {
                if !self.ai_ready {
                    self.ai_error = Some("Connect AI before asking.".into());
                    return;
                }
                if self.request_recommendation_review(kind) {
                    if let Err(error) =
                        Arc::make_mut(&mut self.chat).admit(id, raw, tomlook::chat::Mode::Fast)
                    {
                        self.ai_error = Some(error);
                        return;
                    }
                    if let Some(answer) = self.answer.clone() {
                        Arc::make_mut(&mut self.chat).finish(id, Ok(answer));
                    }
                    self.question.clear();
                    self.focus_assistant = true;
                }
                return;
            }
            ask::Submission::Local(query) => {
                let command = Command::AskLocal {
                    id,
                    query,
                    account: self.calendar_owner.clone(),
                    meeting: self.assistant_context.clone(),
                    language: self.preferences.language,
                };
                let sent = self.worker.as_ref().map(|worker| {
                    worker
                        .commands
                        .try_send(command)
                        .map_err(|error| matches!(error, mpsc::TrySendError::Full(_)))
                });
                match sent {
                    Some(Ok(())) => {
                        if let Err(error) =
                            Arc::make_mut(&mut self.chat).admit(id, raw, tomlook::chat::Mode::Fast)
                        {
                            self.ai_error = Some(error);
                            return;
                        }
                        if let Some(turn) = Arc::make_mut(&mut self.chat).turns.back_mut() {
                            turn.progress.stage = "Local calendar".into();
                        }
                        self.question.clear();
                        self.focus_assistant = true;
                        self.ask_unread = false;
                    }
                    Some(Err(true)) => {
                        self.ai_error =
                            Some("Local queue is full. Nothing was submitted; try again.".into())
                    }
                    _ => {
                        self.ai_error =
                            Some("The storage worker is unavailable. Restart Tomlook.".into())
                    }
                }
                return;
            }
        };
        if !self.ai_ready {
            self.ai_error = Some("Connect AI before asking.".into());
            return;
        }
        let request = tomlook::chat::Request {
            question: Question {
                id,
                question,
                meeting: self.assistant_context.clone(),
                account: self.calendar_owner.clone(),
                public_topic: None,
                suggest_drafts: false,
                language: self.preferences.language,
            },
            mode: self.ask_mode,
            history: self.chat.history(),
            resume: None,
        };
        if self.ai_command(ai::Command::Chat(Box::new(request))) {
            if let Err(error) = Arc::make_mut(&mut self.chat).admit(id, raw, self.ask_mode) {
                self.ai_error = Some(error);
                return;
            }
            if self.ask_mode != tomlook::chat::Mode::Deep {
                self.active_request = Some(id);
            }
            self.answer = None;
            self.ask_unread = false;
            self.question.clear();
            self.focus_assistant = true;
        }
    }

    fn request_recommendation_review(&mut self, kind: tomlook::recommendations::Review) -> bool {
        let Some(account) = self
            .calendar_owner
            .clone()
            .filter(|account| !account.is_empty())
        else {
            self.ai_error = Some("Connect AI and WorkIQ before starting a calendar review.".into());
            return false;
        };
        let command = Command::ReviewRecommendations { kind, account };
        let Some(worker) = &self.worker else {
            self.ai_error = Some("The storage worker is unavailable. Restart Tomlook.".into());
            return false;
        };
        match worker.commands.try_send(command) {
            Ok(()) => {
                let message = match kind {
                    tomlook::recommendations::Review::Travel => {
                        "Travel review queued for the next 14 days. Suggestions will appear in Messages; calendar changes still need approval."
                    }
                    tomlook::recommendations::Review::Categories => {
                        "Category review queued for the next 14 days. Suggestions will appear in Messages; calendar changes still need approval."
                    }
                };
                self.answer = Some(ai::Answer::new(
                    self.preferences.language.text(message).into_owned(),
                    Arc::new(Default::default()),
                    Arc::new(Vec::new()),
                ));
                self.ask_unread = false;
                true
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.ai_error =
                    Some("Local queue is full. Nothing was submitted; try again.".into());
                false
            }
            _ => {
                self.ai_error = Some("The storage worker is unavailable. Restart Tomlook.".into());
                false
            }
        }
    }

    fn resume_research(&mut self, key: String) {
        let Some(saved) = self
            .research_recoveries
            .iter()
            .find(|saved| saved.key == key)
            .cloned()
        else {
            self.ai_error = Some("This saved research task is unavailable".into());
            return;
        };
        let meeting = if let Some(event) = &saved.meeting {
            match self
                .calendar
                .events
                .iter()
                .find(|candidate| {
                    candidate.id == event.id && candidate.fingerprint() == event.fingerprint
                })
                .cloned()
            {
                Some(current) => Some(current),
                None => {
                    self.ai_error =
                        Some("The saved research meeting changed; start a new question".into());
                    return;
                }
            }
        } else {
            None
        };
        if !self.ai_ready || !self.switch_assistant_context(meeting.clone()) {
            return;
        }
        if self.chat.turns.len() >= tomlook::chat::TURN_LIMIT {
            self.ai_error =
                Some("This chat holds 32 questions; start a new chat to continue".into());
            return;
        }
        let Some(id) = self.request_id.checked_add(1).filter(|id| *id < 1 << 63) else {
            self.ai_error=Some("Assistant request identity limit reached. No question was submitted; restart Tomlook.".into());
            return;
        };
        let request = tomlook::chat::Request {
            question: Question {
                id,
                question: saved.question.clone(),
                meeting,
                account: self.calendar_owner.clone(),
                public_topic: None,
                suggest_drafts: false,
                language: saved.language,
            },
            mode: tomlook::chat::Mode::Deep,
            history: saved.history,
            resume: Some(key.clone()),
        };
        if self.ai_command(ai::Command::Chat(Box::new(request))) {
            self.request_id = id;
            if let Err(error) =
                Arc::make_mut(&mut self.chat).admit(id, saved.question, tomlook::chat::Mode::Deep)
            {
                self.ai_error = Some(error);
                return;
            }
            self.research_recoveries = Arc::new(
                self.research_recoveries
                    .iter()
                    .filter(|saved| saved.key != key)
                    .cloned()
                    .collect(),
            );
            self.ai_error = None;
            self.ask_unread = false;
        }
    }

    fn retry_chat_question(&mut self, id: u64) {
        let Some(turn) = self
            .chat
            .turns
            .iter()
            .find(|turn| turn.id == id && !turn.running && turn.error.is_some())
            .cloned()
        else {
            return;
        };
        let draft = std::mem::replace(&mut self.question, turn.question);
        let mode = std::mem::replace(&mut self.ask_mode, turn.progress.mode);
        self.ask_question();
        if !draft.is_empty() {
            self.question = draft;
        }
        self.ask_mode = mode;
    }

    pub(super) fn prepare_follow_up(&mut self, recommendation: &tomlook::recommendations::Recommendation) {
        let now = chrono::Local::now().fixed_offset();
        if recommendation.suggestion.kind != tomlook::recommendations::Kind::FollowUp
            || !recommendation.visible(&self.calendar, self.calendar_owner.as_deref(), now)
        {
            self.error = Some("Recommendation changed or is no longer available; refresh Insights before preparing a draft.".into());
            return;
        }
        let Some(event) = recommendation
            .current(&self.calendar, self.calendar_owner.as_deref(), now)
            .cloned()
        else {
            self.error = Some("Recommendation changed or is no longer available; refresh Insights before preparing a draft.".into());
            return;
        };
        self.assistant = true;
        if !self.switch_assistant_context(Some(event)) {
            return;
        }
        if self.question.trim().is_empty() {
            self.question = self.preferences.language.text("Prepare a local follow-up draft for this meeting using saved Insights and retrieved evidence. Separate verified outcomes from pre-meeting plans; use placeholders for unknown decisions, owners and dates. Do not send any message, invite attendees or modify the calendar.").into_owned();
        } else {
            self.ai_error = Some("An unsent question is already saved for this meeting; it was preserved. Review it before asking for a follow-up draft.".into());
        }
        self.focus_assistant = true;
        self.messages_open = false;
    }

    pub(super) fn ask_view(&self) -> ask::View {
        ask::View {
            question: self.question.clone(),
            answer: self.answer.clone(),
            error: self.context_error.clone().or_else(|| {
                self.ai_error
                    .as_ref()
                    .filter(|error| {
                        !self
                            .chat
                            .turns
                            .iter()
                            .any(|turn| turn.error.as_ref() == Some(*error))
                    })
                    .cloned()
            }),
            context: self.assistant_context.clone(),
            revision: self.assistant_revision.clone(),
            selected: self.selected.is_some(),
            ready: self.ai_ready && self.context_error.is_none(),
            busy: self.active_request.is_some(),
            cursor: self.ask_cursor,
            menu_dismissed: self.ask_menu_dismissed,
            focus: self.focus_assistant,
            language: self.preferences.language,
            accent: self.accent(),
            chat: self.chat.clone(),
            recoveries: self.research_recoveries.clone(),
            mode: self.ask_mode,
            auto_status: self.auto_status.clone(),
            added_tasks: self.tasks.added_drafts.clone(),
            ..Default::default()
        }
    }

    pub(super) fn toggle_assistant(&mut self, context: &egui::Context) {
        if self.assistant {
            self.apply_ask_input(self.ask_view().input(ask::Action::Hide), context);
        } else {
            self.assistant = true;
            self.ask_unread = false;
            self.focus_assistant = true;
            if self.assistant_detached {
                context.send_viewport_cmd_to(ask_viewport(), egui::ViewportCommand::Visible(true));
                context.send_viewport_cmd_to(ask_viewport(), egui::ViewportCommand::Focus);
            }
        }
    }

    pub(super) fn apply_ask_input(&mut self, input: ask::Input, context: &egui::Context) {
        let same_context = input.context_id.as_deref()
            == self
                .assistant_context
                .as_ref()
                .map(|event| event.id.as_str())
            && input.revision == self.assistant_revision;
        if same_context {
            self.question = input.question;
            self.ask_cursor = input.cursor;
            self.ask_menu_dismissed = input.menu_dismissed;
            self.ask_mode = input.mode;
        }
        match input.action {
            ask::Action::Submit if !same_context => {
                self.ai_error = Some(
                    "Meeting context changed. Review the question before submitting it.".into(),
                );
            }
            ask::Action::Submit => self.ask_question(),
            ask::Action::Stop => self.cancel_ai(),
            ask::Action::StopRequest(id) => self.cancel_chat_request(id),
            ask::Action::ModeChanged => {}
            ask::Action::NewChat => {
                if !self.chat.running() && self.active_request.is_none() {
                    self.chat = Default::default();
                    self.answer = None;
                    self.ai_error = None;
                    self.focus_assistant = true;
                }
            }
            ask::Action::OpenMeeting(id) => {
                if let Some(index) = self.calendar.events.iter().position(|event| event.id == id) {
                    let event = self.calendar.events[index].clone();
                    self.open_recommended_meeting(&event);
                } else {
                    self.ai_error = Some(
                        "Meeting is no longer in the local calendar; refresh and search again"
                            .into(),
                    );
                }
            }
            ask::Action::Resume(key) => self.resume_research(key),
            ask::Action::DiscardResearch(key) => {
                if let Some(account) = self.calendar_owner.clone()
                    && self
                        .research_recoveries
                        .iter()
                        .any(|saved| saved.key == key && saved.account == account)
                {
                    self.ai_command(ai::Command::DiscardChat { account, key });
                }
            }
            ask::Action::Retry(id) => self.retry_chat_question(id),
            ask::Action::AddTask(proposal) => self.add_draft_task(&proposal),
            ask::Action::OpenEmail(proposal) => self.open_email_draft(&proposal, context),
            ask::Action::ConfigureAuto => {
                self.open_ai_connection();
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
            ask::Action::ReviewChanges(ids) => {
                let now = chrono::Local::now().fixed_offset();
                self.recommendation_selected = ids.iter().find(|id|
                    self.activity.recommendations.get(*id).is_some_and(|record|
                        record.visible(&self.calendar, self.calendar_owner.as_deref(), now))
                    || self.activity.event_creations.get(*id).is_some_and(|record|
                        self.calendar_owner.as_deref() == Some(record.account.as_str())
                        && record.state == tomlook::event_creation::State::Pending))
                    .cloned().or_else(|| ids.first().cloned());
                self.messages_open = true;
                let creation = self.recommendation_selected.as_ref()
                    .is_some_and(|id| self.activity.event_creations.contains_key(id));
                self.recommendations_tab = !creation;
                let local = self.recommendation_selected.as_ref().and_then(|id| self.activity.recommendations.get(id))
                    .is_some_and(|record| matches!(record.manual_change, Some(tomlook::recommendations::Change::Document(_))));
                if !local && self.ai.is_some() { self.ai_command(ai::Command::RefreshCalendarAvailability); }
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
            ask::Action::Hide => {
                self.assistant = false;
                self.focus_assistant = false;
                self.focus_search = true;
                context.memory_mut(|memory| memory.surrender_focus(Id::new("assistant-question")));
                context.send_viewport_cmd_to(ask_viewport(), egui::ViewportCommand::Visible(false));
            }
            ask::Action::Detach => {
                self.assistant_detached = true;
                self.focus_assistant = true;
                context.memory_mut(|memory| memory.surrender_focus(Id::new("assistant-question")));
            }
            ask::Action::Dock => {
                self.assistant_detached = false;
                self.focus_assistant = true;
                context.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Visible(true),
                );
                context.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Minimized(false),
                );
                context.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                context.send_viewport_cmd_to(ask_viewport(), egui::ViewportCommand::Visible(false));
            }
            ask::Action::UseSelected => {
                if let Some(event) = self
                    .selected
                    .map(|index| self.calendar.events[index].clone())
                {
                    self.switch_assistant_context(Some(event));
                } else {
                    self.ai_error = Some("Select a meeting first.".into());
                }
            }
            ask::Action::ClearContext => {
                self.switch_assistant_context(None);
            }
            ask::Action::Exit => self.exit(context),
        }
        context.request_repaint();
    }

    pub(super) fn receive_ask_inputs(&mut self, context: &egui::Context) {
        for _ in 0..16 {
            match self.ask_input.try_recv() {
                Ok(input) => self.apply_ask_input(input, context),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.ai_error = Some("The Ask window is unavailable. Restart Tomlook.".into());
                    break;
                }
            }
        }
    }

    pub(super) fn sync_detached_ask(&mut self, context: &egui::Context) {
        if !self.assistant_detached {
            return;
        }
        let desired = self.ask_view();
        let mut shared = match self.detached_ask.try_lock() {
            Ok(shared) => shared,
            Err(std::sync::TryLockError::WouldBlock) => {
                context.request_repaint_after(std::time::Duration::from_millis(16));
                return;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                self.ai_error =
                    Some("The Ask window state is unavailable. Restart Tomlook.".into());
                return;
            }
        };
        if let Some(error) = shared.failure.take() {
            self.ai_error = Some(error.clone());
            shared.view.error = Some(error);
            context.request_repaint();
            context.request_repaint_of(ask_viewport());
            return;
        }
        let answer_changed = match (&shared.view.answer, &desired.answer) {
            (Some(old), Some(new)) => !Arc::ptr_eq(&old.text, &new.text),
            (None, None) => false,
            _ => true,
        };
        let context_changed = shared.view.revision != desired.revision
            || shared.view.context.as_ref().map(|event| event.id.as_str())
                != desired.context.as_ref().map(|event| event.id.as_str());
        let changed = shared.synced_question != desired.question
            || !Arc::ptr_eq(&shared.view.chat, &desired.chat)
            || !Arc::ptr_eq(&shared.view.recoveries, &desired.recoveries)
            || shared.synced_mode != desired.mode
            || answer_changed
            || shared.view.error != desired.error
            || context_changed
            || shared.view.selected != desired.selected
            || shared.view.ready != desired.ready
            || shared.view.busy != desired.busy
            || shared.view.language != desired.language
            || shared.view.accent != desired.accent
            || !Arc::ptr_eq(&shared.view.added_tasks, &desired.added_tasks)
            || desired.focus;
        if shared.synced_question != desired.question || context_changed {
            shared.view.question = desired.question.clone();
            shared.synced_question = desired.question;
            shared.view.cursor = desired.cursor;
            shared.view.menu_dismissed = desired.menu_dismissed;
        }
        shared.view.answer = desired.answer;
        shared.view.chat = desired.chat;
        shared.view.recoveries = desired.recoveries;
        if shared.synced_mode != desired.mode {
            shared.synced_mode = desired.mode;
            shared.view.mode = desired.mode;
        }
        shared.view.error = desired.error;
        shared.view.context = desired.context;
        shared.view.revision = desired.revision;
        shared.view.selected = desired.selected;
        shared.view.ready = desired.ready;
        shared.view.busy = desired.busy;
        shared.view.language = desired.language;
        shared.view.accent = desired.accent;
        shared.view.added_tasks = desired.added_tasks;
        shared.view.focus |= desired.focus;
        self.focus_assistant = false;
        if changed {
            context.request_repaint_of(ask_viewport());
        }
    }

    pub(super) fn assistant_panel(&mut self, ui: &mut egui::Ui) {
        let mut view = self.ask_view();
        let action = ask::draw(ui, &mut view, false);
        self.question = view.question.clone();
        self.ask_cursor = view.cursor;
        self.ask_menu_dismissed = view.menu_dismissed;
        self.ask_mode = view.mode;
        self.focus_assistant = view.focus;
        self.ask_unread = false;
        if let Some(action) = action {
            self.apply_ask_input(view.input(action), ui.ctx());
        }
    }

    pub(super) fn assistant_dialog(&mut self, context: &egui::Context) {
        if !self.assistant || !self.assistant_detached {
            return;
        }
        self.sync_detached_ask(context);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!(
                "Tomlook — {}",
                self.preferences.language.text("Ask AI")
            ))
            .with_taskbar(true)
            .with_inner_size([560.0, 740.0])
            .with_min_inner_size([340.0, 420.0]);
        if let Ok(shared) = self.detached_ask.try_lock() {
            builder = builder.with_inner_size(shared.view.size);
            if let Some(position) = shared.view.position {
                builder = builder.with_position(position);
            }
        }
        let shared = self.detached_ask.clone();
        let output = self.ask_output.clone();
        let language = self.preferences.language;
        context.show_viewport_deferred(ask_viewport(), builder, move |ui, class| {
            let mut shared = match shared.try_lock() {
                Ok(shared) => shared,
                Err(std::sync::TryLockError::WouldBlock) => {
                    ui.label(language.text("Ask is updating..."));
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(16));
                    return;
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    ui.label(
                        language.text("The Ask window state is unavailable. Restart Tomlook."),
                    );
                    return;
                }
            };
            let action = if class == egui::ViewportClass::EmbeddedWindow {
                shared.failure = Some(
                    "Separate Ask windows are not supported by this renderer. Ask remains docked."
                        .into(),
                );
                Some(ask::Action::Dock)
            } else {
                let viewport = ui.ctx().input(|input| input.viewport().clone());
                if let Some(rect) = viewport.outer_rect {
                    shared.view.position = Some(rect.min);
                }
                if let Some(rect) = viewport.inner_rect {
                    shared.view.size = rect.size();
                }
                if viewport.close_requested() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    Some(ask::Action::Hide)
                } else {
                    egui::CentralPanel::default()
                        .show(ui, |ui| ask::draw(ui, &mut shared.view, true))
                        .inner
                }
            };
            if let Some(action) = action {
                match output.try_send(shared.view.input(action)) {
                    Ok(()) => {
                        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    }
                    Err(error) => {
                        let message = match error {
                            mpsc::TrySendError::Full(_) => {
                                "The Ask action queue is full. Nothing was submitted; try again."
                            }
                            mpsc::TrySendError::Disconnected(_) => {
                                "The Ask window is unavailable. Restart Tomlook."
                            }
                        };
                        shared.failure = Some(message.into());
                        shared.view.error = Some(message.into());
                        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    }
                }
            }
        });
    }
}
