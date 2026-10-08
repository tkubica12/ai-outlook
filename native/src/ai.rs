use crate::{
    calendar::Event,
    copilot::{Config, Harness, PREPARATION_PROMPT},
};
use eframe::egui;
use github_copilot_sdk::MessageOptions;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc as async_mpsc, watch};

#[derive(Clone)]
pub struct Question {
    pub id: u64,
    pub question: String,
    pub meeting: Option<Event>,
    pub public_topic: Option<String>,
    pub suggest_drafts: bool,
}

#[derive(Clone)]
pub struct Answer {
    pub text: Arc<String>,
    pub evidence: Arc<crate::evidence::Snapshot>,
    pub proposals: Arc<Vec<crate::proposals::Proposal>>,
}

pub enum Command {
    Connect,
    Ask(Box<Question>),
    Cancel(u64),
    Prepare(Box<Preparation>),
    AttachWake(std::sync::mpsc::SyncSender<crate::worker::Command>),
    Stop,
}

pub struct Preparation {
    pub id: u64,
    pub event: Event,
    pub foreground: bool,
    pub reply: async_mpsc::Sender<Prepared>,
    pub wake: std::sync::mpsc::SyncSender<crate::worker::Command>,
}

pub enum PreparationResult {
    NotAdmitted(String),
    Finished(Result<Arc<String>, String>),
}

pub struct Prepared {
    pub id: u64,
    pub result: PreparationResult,
}

pub enum Notice {
    Connection {
        ready: bool,
        message: String,
    },
    Answer {
        id: u64,
        result: Result<Answer, String>,
    },
    Stopped,
}

#[derive(Default)]
pub struct Progress {
    pub id: u64,
    pub text: Arc<String>,
}

pub struct Engine {
    pub commands: async_mpsc::Sender<Command>,
    pub notices: Arc<Notices>,
    pub progress: Arc<Mutex<Progress>>,
    pub done: Option<mpsc::Receiver<()>>,
    pub ready: Arc<AtomicBool>,
    pub occupied: Arc<AtomicUsize>,
    pub terminated: Arc<AtomicBool>,
    pub stop: watch::Sender<bool>,
    pub analysis_revision: Arc<std::sync::OnceLock<String>>,
}

pub struct Shutdown {
    pub stop: watch::Sender<bool>,
    pub done: mpsc::Receiver<()>,
}

#[derive(Default)]
struct Pending {
    connection: Option<Notice>,
    answers: BTreeMap<u64, Notice>,
    stopped: bool,
}

impl Pending {
    fn take(&mut self) -> Option<Notice> {
        self.connection
            .take()
            .or_else(|| self.answers.pop_last().map(|(_, notice)| notice))
            .or_else(|| std::mem::take(&mut self.stopped).then_some(Notice::Stopped))
    }
}

#[derive(Default)]
pub struct Notices {
    pending: Mutex<Pending>,
    changed: std::sync::Condvar,
    poisoned: AtomicBool,
}

impl Notices {
    pub fn send(&self, notice: Notice) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "AI notifications are unavailable")?;
        match notice {
            Notice::Connection { .. } => pending.connection = Some(notice),
            Notice::Answer { id, .. } => {
                pending.answers.insert(id, notice);
                if pending.answers.len() > 2 {
                    pending.answers.pop_first();
                }
            }
            Notice::Stopped => pending.stopped = true,
        }
        drop(pending);
        self.changed.notify_all();
        Ok(())
    }

    fn unavailable(&self) -> Result<Notice, mpsc::TryRecvError> {
        if !self.poisoned.swap(true, Ordering::AcqRel) {
            Ok(Notice::Connection {
                ready: false,
                message: "AI notifications are unavailable; restart Tomlook".into(),
            })
        } else {
            Err(mpsc::TryRecvError::Disconnected)
        }
    }

    pub fn try_recv(&self) -> Result<Notice, mpsc::TryRecvError> {
        match self.pending.try_lock() {
            Ok(mut pending) => pending.take().ok_or(mpsc::TryRecvError::Empty),
            Err(std::sync::TryLockError::WouldBlock) => Err(mpsc::TryRecvError::Empty),
            Err(std::sync::TryLockError::Poisoned(_)) => self.unavailable(),
        }
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Notice, mpsc::RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        let mut pending = match self.pending.lock() {
            Ok(pending) => pending,
            Err(_) => {
                return self
                    .unavailable()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected);
            }
        };
        loop {
            if let Some(notice) = pending.take() {
                return Ok(notice);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(mpsc::RecvTimeoutError::Timeout);
            }
            pending = match self.changed.wait_timeout(pending, remaining) {
                Ok((pending, _)) => pending,
                Err(_) => {
                    return self
                        .unavailable()
                        .map_err(|_| mpsc::RecvTimeoutError::Disconnected);
                }
            };
        }
    }
}

enum Completion {
    Connection(Result<Arc<Harness>, String>),
    Answer {
        id: u64,
        result: Result<Answer, String>,
    },
    Prepared {
        request: Box<Preparation>,
        result: Result<Arc<String>, String>,
    },
}

impl Engine {
    pub fn start(root: PathBuf, demo: bool, context: egui::Context) -> Result<Self, String> {
        let (commands, mut input) = async_mpsc::channel(64);
        let notices = Arc::new(Notices::default());
        let output = notices.clone();
        let (stop, mut stopped) = watch::channel(false);
        let (finished, done) = mpsc::sync_channel(1);
        let progress = Arc::new(Mutex::new(Progress::default()));
        let streaming = progress.clone();
        let ready = Arc::new(AtomicBool::new(false));
        let connected = ready.clone();
        let occupied = Arc::new(AtomicUsize::new(0));
        let active = occupied.clone();
        let terminated = Arc::new(AtomicBool::new(false));
        let ended = terminated.clone();
        let analysis_revision = Arc::new(std::sync::OnceLock::new());
        let profile = analysis_revision.clone();
        thread::Builder::new().name("tomlook-sdk".into()).spawn(move || {
            let notify = |notice| {
                let result = output.send(notice);
                context.request_repaint();
                if let Err(error) = &result { eprintln!("Tomlook: {error}"); }
                result.is_ok()
            };
            let runtime = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    notify(Notice::Connection { ready: false, message: format!("Start AI executor: {error}") });
                    ended.store(true, Ordering::Release);
                    let _ = finished.send(());
                    return;
                }
            };
            let mut wake: Option<std::sync::mpsc::SyncSender<crate::worker::Command>> = None;
            runtime.block_on(async {
                let mut harness: Option<Arc<Harness>> = None;
                let mut starting = false;
                let mut tasks = tokio::task::JoinSet::new();
                let mut cancellations = BTreeMap::<u64, (watch::Sender<bool>, bool)>::new();
                let mut stopping = false;
                loop {
                    if *stopped.borrow() { stopping = true; }
                    if stopping {
                        connected.store(false, Ordering::Release);
                        for (cancel, _) in cancellations.values() { let _ = cancel.send(true); }
                    }
                    if stopping && tasks.is_empty() {
                        if let Some(harness) = harness.take()
                            && let Err(error) = harness.stop().await {
                                notify(Notice::Connection { ready: false, message: error });
                            }
                        notify(Notice::Stopped);
                        break;
                    }
                    tokio::select! {
                        biased;
                        _ = stopped.changed(), if !stopping => { stopping = true; }
                        command = input.recv(), if !stopping => {
                            match command {
                                Some(Command::AttachWake(sender)) => { wake = Some(sender); }
                                Some(Command::Connect) if demo => {
                                    notify(Notice::Connection { ready: false, message: "Synthetic preview never starts Copilot or live connectors.".into() });
                                }
                                Some(Command::Connect) if harness.is_some() || starting => {
                                    notify(Notice::Connection { ready: harness.is_some(), message: if starting { "Connecting isolated SDK..." } else { "Isolated SDK is already connected" }.into() });
                                }
                                Some(Command::Connect) => {
                                    starting = true;
                                    notify(Notice::Connection { ready: false, message: "Connecting isolated SDK on its own worker...".into() });
                                    let root = root.clone();
                                    tasks.spawn(async move {
                                        let result = async {
                                            let config = Config::load(&root)?;
                                            let harness = Harness::start(&root, config).await?;
                                            let health = harness.health().await;
                                            match health {
                                                Ok(health) if health.authenticated => Ok(Arc::new(harness)),
                                                Ok(_) => {
                                                    harness.stop().await?;
                                                    Err("Isolated Copilot identity is missing. Use the Tomlook-only profile or configure an explicit TOMLOOK_* Copilot credential variable; personal tokens and gh login are never imported.".into())
                                                }
                                                Err(error) => {
                                                    let stopped = harness.stop().await;
                                                    Err(match stopped { Ok(()) => error, Err(stop) => format!("{error}; {stop}") })
                                                }
                                            }
                                        }.await;
                                        Completion::Connection(result)
                                    });
                                }
                                Some(Command::Ask(question)) => {
                                    if cancellations.contains_key(&question.id) {
                                        notify(Notice::Answer { id: question.id, result: Err("Request identity is already active".into()) });
                                    } else if cancellations.len() >= 2 {
                                        notify(Notice::Answer { id: question.id, result: Err("Both AI slots are busy; nothing was submitted. Cancel or wait, then retry.".into()) });
                                    } else if let Some(harness) = harness.clone() {
                                        let (cancel, cancelled) = watch::channel(false);
                                        cancellations.insert(question.id, (cancel, false));
                                        let progress = streaming.clone();
                                        let context = context.clone();
                                        tasks.spawn(async move {
                                            let id = question.id;
                                            let result = ask(harness, *question, cancelled, progress, context, true).await;
                                            Completion::Answer { id, result }
                                        });
                                    } else {
                                        notify(Notice::Answer { id: question.id, result: Err("Connect the isolated SDK first. No AI request was made.".into()) });
                                    }
                                }
                                Some(Command::Cancel(id)) => {
                                    if let Some((cancel, _)) = cancellations.get(&id) { let _ = cancel.send(true); }
                                }
                                Some(Command::Prepare(request)) => {
                                    let id = request.id | (1 << 63);
                                    let denied = if demo || harness.is_none() { Some("Isolated AI is not available") }
                                        else if cancellations.len() >= 2 || (!request.foreground && cancellations.values().any(|(_, background)| *background)) { Some("Execution capacity is reserved or busy") }
                                        else if cancellations.contains_key(&id) { Some("Preparation identity is already active") } else { None };
                                    if let Some(reason) = denied {
                                        if request.reply.try_send(Prepared { id: request.id, result: PreparationResult::NotAdmitted(reason.into()) }).is_err() {
                                            notify(Notice::Connection { ready: false, message: "Preparation admission could not reach storage; restart to inspect the interrupted ledger.".into() });
                                            stopping = true;
                                        }
                                        let _ = request.wake.try_send(crate::worker::Command::Wake);
                                    } else if let Some(harness) = harness.clone() {
                                        let (cancel, cancelled) = watch::channel(false);
                                        cancellations.insert(id, (cancel, !request.foreground));
                                        let progress = streaming.clone();
                                        let context = context.clone();
                                        tasks.spawn(async move {
                                            let question = Question {
                                                id, question: PREPARATION_PROMPT.into(),
                                                meeting: Some(request.event.clone()), public_topic: None, suggest_drafts: false,
                                            };
                                            let result = ask(harness, question, cancelled, progress, context, false).await.map(|answer| answer.text);
                                            Completion::Prepared { request, result }
                                        });
                                    }
                                }
                                Some(Command::Stop) | None => {
                                    stopping = true;
                                    connected.store(false, Ordering::Release);
                                    for (cancel, _) in cancellations.values() { let _ = cancel.send(true); }
                                }
                            }
                        }
                        completion = tasks.join_next(), if !tasks.is_empty() => {
                            match completion {
                                Some(Ok(Completion::Connection(result))) => {
                                    starting = false;
                                    match result {
                                        Ok(connected_harness) => {
                                            if profile.set(connected_harness.analysis_revision.clone()).is_err() {
                                                notify(Notice::Connection { ready: false, message: "Active AI configuration identity cannot be replaced; restart Tomlook".into() });
                                                stopping = true;
                                            }
                                            harness = Some(connected_harness);
                                            if !stopping { connected.store(true, Ordering::Release); }
                                            notify(Notice::Connection { ready: !stopping, message: if stopping { "Stopping the isolated SDK; no new work is admitted" } else { "Connected: isolated SDK, registered read-only tools" }.into() });
                                        }
                                        Err(error) => { notify(Notice::Connection { ready: false, message: error }); }
                                    }
                                    if let Some(wake) = &wake { let _ = wake.try_send(crate::worker::Command::Wake); }
                                }
                                Some(Ok(Completion::Answer { id, result })) => {
                                    cancellations.remove(&id);
                                    active.store(cancellations.len(), Ordering::Release);
                                    if let Some(wake) = &wake { let _ = wake.try_send(crate::worker::Command::Wake); }
                                    notify(Notice::Answer { id, result });
                                }
                                Some(Ok(Completion::Prepared { request, result })) => {
                                    cancellations.remove(&(request.id | (1 << 63)));
                                    active.store(cancellations.len(), Ordering::Release);
                                    if request.reply.try_send(Prepared { id: request.id, result: PreparationResult::Finished(result) }).is_err() {
                                        notify(Notice::Connection { ready: false, message: "Preparation outcome could not reach storage; restart and inspect the interrupted ledger. It was not replayed.".into() });
                                        stopping = true;
                                    }
                                    let _ = request.wake.try_send(crate::worker::Command::Wake);
                                }
                                Some(Err(error)) => {
                                    notify(Notice::Connection { ready: false, message: format!("AI worker failed: {error}; reconnect after restart") });
                                    stopping = true;
                                    for (cancel, _) in cancellations.values() { let _ = cancel.send(true); }
                                }
                                None => {}
                            }
                        }
                    }
                    active.store(cancellations.len(), Ordering::Release);
                    if stopping {
                        connected.store(false, Ordering::Release);
                        for (cancel, _) in cancellations.values() { let _ = cancel.send(true); }
                    }
                    if stopping && tasks.is_empty() {
                        if let Some(harness) = harness
                            && let Err(error) = harness.stop().await {
                                notify(Notice::Connection { ready: false, message: error });
                            }
                        notify(Notice::Stopped);
                        break;
                    }
                }
            });
            drop(runtime);
            connected.store(false, Ordering::Release);
            active.store(0, Ordering::Release);
            ended.store(true, Ordering::Release);
            if let Some(wake) = wake { let _ = wake.try_send(crate::worker::Command::Wake); }
            let _ = finished.send(());
        }).map_err(|e| format!("Create AI worker: {e}"))?;
        Ok(Self {
            commands,
            notices,
            progress,
            done: Some(done),
            ready,
            occupied,
            terminated,
            stop,
            analysis_revision,
        })
    }
}

fn question_prompt(
    question: &Question,
    redactor: &crate::evidence::Redactor,
) -> Result<String, String> {
    if question.question.trim().is_empty()
        || question.question.len() > 8000
        || question
            .public_topic
            .as_ref()
            .is_some_and(|topic| topic.len() > 2000)
    {
        return Err(
            "Question/public topic exceeds its bounds or is empty; nothing was submitted".into(),
        );
    }
    let meeting = question
        .meeting
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| format!("Encode meeting context: {e}"))?;
    if meeting.as_ref().is_some_and(|data| data.len() > 32_000) {
        return Err("Meeting context exceeds the 32 KiB limit; no AI request was made".into());
    }
    if redactor.contains(&question.question)
        || question
            .public_topic
            .as_ref()
            .is_some_and(|topic| redactor.contains(topic))
        || meeting.as_ref().is_some_and(|data| redactor.contains(data))
    {
        return Err("Question, meeting or public topic contains an app credential; no SDK session or model request was created".into());
    }
    let mut prompt = format!(
        "Answer the question concisely (normally under 180 words). Cite original sources as markdown links when supplied by tools; never fabricate URLs. If relevant data is inaccessible, state that explicitly. No briefing is required to answer. No writes.\nMEETING_DATA: {}\nUSER_QUESTION: {}",
        meeting
            .as_deref()
            .unwrap_or("No meeting selected - global question"),
        question.question,
    );
    if question.suggest_drafts {
        prompt.push_str(crate::proposals::RESPONSE_INSTRUCTIONS);
    }
    Ok(prompt)
}

async fn ask(
    harness: Arc<Harness>,
    question: Question,
    mut cancel: watch::Receiver<bool>,
    progress: Arc<Mutex<Progress>>,
    context: egui::Context,
    stream: bool,
) -> Result<Answer, String> {
    let prompt = question_prompt(&question, &harness.redactor)?;
    if *cancel.borrow() {
        return Err("Cancelled before SDK session creation; no question was sent".into());
    }
    let (session, recorder) = tokio::time::timeout(
        Duration::from_secs(30),
        harness.observed_session(question.public_topic),
    )
    .await
    .map_err(|_| "Creating the SDK session timed out".to_string())??;
    let mut subscription = session.subscribe();
    let result = async {
        if *cancel.borrow() { return Err("Cancelled before sending".into()); }
        tokio::select! {
            changed = cancel.changed() => {
                let _ = changed;
                return Err("Cancelled while submitting the request; provider outcome may be unknown".into());
            }
            sent = tokio::time::timeout(Duration::from_secs(30), session.send(MessageOptions::new(prompt))) => {
                sent.map_err(|_| "Sending the SDK request timed out".to_string())?
                    .map_err(|e| format!("SDK request failed: {e}"))?;
            }
        }
        let timeout = tokio::time::sleep(Duration::from_secs(120));
        tokio::pin!(timeout);
        let mut accumulated = String::new();
        let mut final_answer = None;
        let mut last_update = Instant::now() - Duration::from_secs(1);
        loop {
            tokio::select! {
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() { return Err("Cancelled. Any partial answer is not a completed result.".into()); }
                }
                _ = &mut timeout => return Err("AI answer exceeded the two-minute limit; retry explicitly".into()),
                event = subscription.recv() => {
                    let event = event.map_err(|e| format!("SDK event stream failed: {e}"))?;
                    match event.event_type.as_str() {
                        "assistant.message_delta" => {
                            if let Some(delta) = event.data.get("deltaContent").and_then(|v| v.as_str()) {
                                if accumulated.len() + delta.len() > 64_000 { return Err("AI answer exceeded the 64 KiB response limit".into()); }
                                accumulated.push_str(delta);
                                if stream && !question.suggest_drafts && last_update.elapsed() >= Duration::from_millis(100) {
                                    let mut state = progress.lock().map_err(|_| "AI progress state is unavailable")?;
                                    let text = harness.redactor.partial(&accumulated);
                                    if text.len() > 64_000 { return Err("Redacted AI progress exceeded the 64 KiB limit".into()); }
                                    *state = Progress { id: question.id, text: Arc::new(text) };
                                    drop(state);
                                    context.request_repaint();
                                    last_update = Instant::now();
                                }
                            }
                        }
                        "assistant.message" => {
                            if let Some(text) = event.data.get("content").and_then(|v| v.as_str()) {
                                if text.len() > 64_000 { return Err("AI answer exceeded the 64 KiB response limit".into()); }
                                final_answer = Some(text.to_owned());
                            }
                        }
                        "session.error" => return Err("Copilot reported a session error. The partial answer is not success; check the isolated connection and retry.".into()),
                        "session.idle" => return final_answer.filter(|text| !text.trim().is_empty()).ok_or_else(|| "Copilot completed without an answer".into()),
                        _ => {}
                    }
                }
            }
        }
    }.await;
    let aborted = if result.is_err() {
        tokio::time::timeout(Duration::from_secs(5), session.abort())
            .await
            .map_err(|_| "Cancel SDK session timed out".to_string())
            .and_then(|r| r.map_err(|e| format!("Cancel SDK session: {e}")))
    } else {
        Ok(())
    };
    let disconnected = tokio::time::timeout(Duration::from_secs(5), session.disconnect())
        .await
        .map_err(|_| "Detach SDK session timed out".to_string())
        .and_then(|r| r.map_err(|e| format!("Detach SDK session: {e}")));
    match (result, aborted, disconnected) {
        (Ok(answer), Ok(()), Ok(())) => {
            let (text, proposals) = if question.suggest_drafts {
                crate::proposals::parse(&answer, &harness.redactor)?
            } else {
                (harness.redactor.text(&answer), Vec::new())
            };
            if text.len() > 64_000 {
                return Err("Redacted AI answer exceeded the 64 KiB limit".into());
            }
            Ok(Answer {
                text: Arc::new(text),
                evidence: recorder.snapshot()?,
                proposals: Arc::new(proposals),
            })
        }
        (result, aborted, disconnected) => {
            let mut errors = Vec::new();
            if let Err(error) = result {
                errors.push(error);
            }
            if let Err(error) = aborted {
                errors.push(error);
            }
            if let Err(error) = disconnected {
                errors.push(error);
            }
            Err(harness.redactor.text(&errors.join("; ")))
        }
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn known_app_credentials_cannot_enter_question_meeting_or_public_topic_prompts() {
        let redactor = crate::evidence::Redactor::new(["fixture-app-credential".into()]);
        let mut question = Question {
            id: 1,
            question: "Safe fixture question".into(),
            meeting: None,
            public_topic: None,
            suggest_drafts: false,
        };
        assert!(
            question_prompt(&question, &redactor)
                .unwrap()
                .contains("Safe fixture question")
        );
        question.question = "Question containing fixture-app-credential".into();
        assert!(
            question_prompt(&question, &redactor)
                .unwrap_err()
                .contains("no SDK session")
        );
        question.question = "Safe fixture question".into();
        question.public_topic = Some("fixture-app-credential".into());
        assert!(question_prompt(&question, &redactor).is_err());
        question.public_topic = None;
        let mut meeting = crate::calendar::demo(chrono::Local::now(), 1).remove(0);
        meeting.title = "fixture-app-credential".into();
        question.meeting = Some(meeting);
        assert!(question_prompt(&question, &redactor).is_err());
    }

    #[test]
    fn prompt_payload_limits_apply_before_sdk_session_creation() {
        let redactor = crate::evidence::Redactor::default();
        let mut question = Question {
            id: 1,
            question: "\u{010d}".repeat(4001),
            meeting: None,
            public_topic: None,
            suggest_drafts: false,
        };
        assert!(question_prompt(&question, &redactor).is_err());
        question.question = "Safe fixture".into();
        question.public_topic = Some("x".repeat(2001));
        assert!(question_prompt(&question, &redactor).is_err());
        question.public_topic = None;
        let mut meeting = crate::calendar::demo(chrono::Local::now(), 1).remove(0);
        meeting.title = "x".repeat(32_001);
        question.meeting = Some(meeting);
        assert!(question_prompt(&question, &redactor).is_err());
    }

    #[test]
    fn local_proposal_mode_is_explicit_and_does_not_change_preparation_or_plain_answers() {
        let redactor = crate::evidence::Redactor::default();
        let mut question = Question {
            id: 1,
            question: "Fixture question".into(),
            meeting: None,
            public_topic: None,
            suggest_drafts: false,
        };
        assert!(
            !question_prompt(&question, &redactor)
                .unwrap()
                .contains("LOCAL_DRAFTS:")
        );
        question.suggest_drafts = true;
        let prompt = question_prompt(&question, &redactor).unwrap();
        assert!(prompt.contains("LOCAL_DRAFTS:"));
        assert!(prompt.contains("never actions or approvals"));
    }
}
