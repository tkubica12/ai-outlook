use crate::{
    calendar::Event,
    copilot::{Config, Harness},
};
use eframe::egui;
use github_copilot_sdk::MessageOptions;
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
}

pub enum Command {
    Connect,
    Ask(Box<Question>),
    Cancel(u64),
    Stop,
}

pub enum Notice {
    Connection {
        ready: bool,
        message: String,
    },
    Answer {
        id: u64,
        result: Result<Arc<String>, String>,
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
    pub notices: mpsc::Receiver<Notice>,
    pub progress: Arc<Mutex<Progress>>,
    pub done: Option<mpsc::Receiver<()>>,
}

pub struct Shutdown {
    pub commands: async_mpsc::Sender<Command>,
    pub done: mpsc::Receiver<()>,
}

enum Completion {
    Connection(Result<Arc<Harness>, String>),
    Answer {
        id: u64,
        result: Result<Arc<String>, String>,
    },
}

impl Engine {
    pub fn start(root: PathBuf, demo: bool, context: egui::Context) -> Result<Self, String> {
        let (commands, mut input) = async_mpsc::channel(64);
        let (output, notices) = mpsc::sync_channel(64);
        let (finished, done) = mpsc::sync_channel(1);
        let progress = Arc::new(Mutex::new(Progress::default()));
        let streaming = progress.clone();
        thread::Builder::new().name("tomlook-sdk".into()).spawn(move || {
            let notify = |notice| {
                let delivered = output.send(notice).is_ok();
                context.request_repaint();
                delivered
            };
            let runtime = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    notify(Notice::Connection { ready: false, message: format!("Start AI executor: {error}") });
                    let _ = finished.send(());
                    return;
                }
            };
            runtime.block_on(async {
                let mut harness: Option<Arc<Harness>> = None;
                let mut starting = false;
                let mut tasks = tokio::task::JoinSet::new();
                let mut cancellations = BTreeMap::<u64, watch::Sender<bool>>::new();
                let mut stopping = false;
                loop {
                    tokio::select! {
                        command = input.recv(), if !stopping => {
                            match command {
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
                                                    Err("Isolated Copilot identity is missing. Sign in using the Tomlook-only COPILOT_HOME; personal tokens are never imported.".into())
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
                                        cancellations.insert(question.id, cancel);
                                        let progress = streaming.clone();
                                        let context = context.clone();
                                        tasks.spawn(async move {
                                            let id = question.id;
                                            let result = ask(harness, *question, cancelled, progress, context).await.map(Arc::new);
                                            Completion::Answer { id, result }
                                        });
                                    } else {
                                        notify(Notice::Answer { id: question.id, result: Err("Connect the isolated SDK first. No AI request was made.".into()) });
                                    }
                                }
                                Some(Command::Cancel(id)) => {
                                    if let Some(cancel) = cancellations.get(&id) { let _ = cancel.send(true); }
                                }
                                Some(Command::Stop) | None => {
                                    stopping = true;
                                    for cancel in cancellations.values() { let _ = cancel.send(true); }
                                }
                            }
                        }
                        completion = tasks.join_next(), if !tasks.is_empty() => {
                            match completion {
                                Some(Ok(Completion::Connection(result))) => {
                                    starting = false;
                                    match result {
                                        Ok(connected) => {
                                            harness = Some(connected);
                                            notify(Notice::Connection { ready: true, message: "Connected: isolated SDK, registered read-only tools".into() });
                                        }
                                        Err(error) => { notify(Notice::Connection { ready: false, message: error }); }
                                    }
                                }
                                Some(Ok(Completion::Answer { id, result })) => {
                                    cancellations.remove(&id);
                                    notify(Notice::Answer { id, result });
                                }
                                Some(Err(error)) => {
                                    notify(Notice::Connection { ready: false, message: format!("AI worker failed: {error}; reconnect after restart") });
                                    stopping = true;
                                    for cancel in cancellations.values() { let _ = cancel.send(true); }
                                }
                                None => {}
                            }
                        }
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
            let _ = finished.send(());
        }).map_err(|e| format!("Create AI worker: {e}"))?;
        Ok(Self {
            commands,
            notices,
            progress,
            done: Some(done),
        })
    }
}

async fn ask(
    harness: Arc<Harness>,
    question: Question,
    mut cancel: watch::Receiver<bool>,
    progress: Arc<Mutex<Progress>>,
    context: egui::Context,
) -> Result<String, String> {
    if question.question.trim().is_empty() || question.question.len() > 8000 {
        return Err("Question must contain 1-8000 bytes; nothing was submitted".into());
    }
    let session = tokio::time::timeout(
        Duration::from_secs(30),
        harness.session(question.public_topic),
    )
    .await
    .map_err(|_| "Creating the SDK session timed out".to_string())??;
    let mut subscription = session.subscribe();
    let meeting = question
        .meeting
        .map(|event| serde_json::to_string(&event))
        .transpose()
        .map_err(|e| e.to_string())?;
    let prompt = format!(
        "Answer the question concisely (normally under 180 words). Cite original sources as markdown links when supplied by tools; never fabricate URLs. If relevant data is inaccessible, state that explicitly. No briefing is required to answer. No writes.\nMEETING_DATA: {}\nUSER_QUESTION: {}",
        meeting
            .as_deref()
            .unwrap_or("No meeting selected - global question"),
        question.question,
    );
    let result = async {
        if *cancel.borrow() { return Err("Cancelled before sending".into()); }
        tokio::time::timeout(Duration::from_secs(30), session.send(MessageOptions::new(prompt))).await
            .map_err(|_| "Sending the SDK request timed out".to_string())?
            .map_err(|e| format!("SDK request failed: {e}"))?;
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
                                if last_update.elapsed() >= Duration::from_millis(100) {
                                    let mut state = progress.lock().map_err(|_| "AI progress state is unavailable")?;
                                    *state = Progress { id: question.id, text: Arc::new(accumulated.clone()) };
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
        (Ok(answer), Ok(()), Ok(())) => Ok(answer),
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
            Err(errors.join("; "))
        }
    }
}
