use crate::{
    ai,
    calendar::{self, Calendar},
    scheduler::Queue,
    storage::{Preferences, Store},
};
use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

#[derive(Clone)]
pub struct Options {
    pub state: PathBuf,
    pub legacy_root: PathBuf,
    pub demo: Option<usize>,
}

pub enum Command {
    Preferences(Preferences),
    Detail {
        revision: u64,
        id: String,
    },
    Reload,
    Promote(String),
    Retry(String),
    TogglePause,
    EnablePreparation(bool),
    PausePreparation(bool),
    ResumeQueued,
    Wake,
    AttachAi {
        commands: tokio::sync::mpsc::Sender<ai::Command>,
        ready: Arc<AtomicBool>,
        occupied: Arc<AtomicUsize>,
        terminated: Arc<AtomicBool>,
    },
    Exit,
}

pub enum Notice {
    Loaded {
        calendar: Arc<Calendar>,
        preferences: Preferences,
    },
    Detail {
        revision: u64,
        payload: Option<serde_json::Value>,
    },
    Error(String),
}

pub struct Worker {
    pub commands: SyncSender<Command>,
    pub notices: Arc<Notices>,
    pub activity: Arc<Mutex<Arc<Activity>>>,
    pub stop: Arc<AtomicBool>,
    pub done: Option<Receiver<Result<(), String>>>,
}

#[derive(Default)]
pub struct Notices {
    pending: Mutex<[Option<Notice>; 3]>,
    poisoned: AtomicBool,
}

impl Notices {
    fn send(&self, notice: Notice) -> Result<(), String> {
        let index = match &notice {
            Notice::Loaded { .. } => 0,
            Notice::Detail { .. } => 1,
            Notice::Error(_) => 2,
        };
        self.pending
            .lock()
            .map_err(|_| "Storage notifications are unavailable")?[index] = Some(notice);
        Ok(())
    }

    pub fn try_recv(&self) -> Result<Notice, mpsc::TryRecvError> {
        match self.pending.try_lock() {
            Ok(mut pending) => pending
                .iter_mut()
                .find_map(Option::take)
                .ok_or(mpsc::TryRecvError::Empty),
            Err(std::sync::TryLockError::WouldBlock) => Err(mpsc::TryRecvError::Empty),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                if !self.poisoned.swap(true, Ordering::AcqRel) {
                    Ok(Notice::Error(
                        "Storage notifications are unavailable; restart Tomlook".into(),
                    ))
                } else {
                    Err(mpsc::TryRecvError::Disconnected)
                }
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Activity {
    pub revision: u64,
    pub queue: Queue,
    pub enabled: bool,
    pub paused: bool,
    pub error: Option<String>,
    pub queued: usize,
    pub deferred: usize,
}

pub struct Shutdown {
    pub commands: SyncSender<Command>,
    pub stop: Arc<AtomicBool>,
    pub done: Receiver<Result<(), String>>,
}

struct Sdk {
    commands: tokio::sync::mpsc::Sender<ai::Command>,
    ready: Arc<AtomicBool>,
    occupied: Arc<AtomicUsize>,
    terminated: Arc<AtomicBool>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Briefing {
    summary: String,
    sources: Vec<Source>,
    gaps: Vec<String>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    title: String,
    url: String,
}

pub fn validate_prepared(raw: &str, key: &str) -> Result<serde_json::Value, String> {
    if raw.len() > 64_000 {
        return Err("Briefing exceeds 64 KiB".into());
    }
    let briefing: Briefing =
        serde_json::from_str(raw).map_err(|e| format!("Invalid briefing JSON: {e}"))?;
    if briefing.summary.trim().is_empty()
        || briefing.summary.len() > 16_000
        || briefing.sources.len() > 20
        || briefing.gaps.len() > 20
    {
        return Err("Briefing fields exceed their bounds or summary is empty".into());
    }
    if briefing.sources.iter().any(|source| {
        source.title.trim().is_empty()
            || source.title.len() > 1000
            || source.url.len() > 4000
            || !calendar::safe_url(&source.url)
    }) || briefing.gaps.iter().any(|gap| gap.len() > 2000)
    {
        return Err("Briefing contains an unsafe source or oversized evidence gap".into());
    }
    let mut value: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    value["tomlook_fingerprint"] = key.into();
    value["evidence_status"] = "Model-supplied citations; not independently verified".into();
    Ok(value)
}

impl Worker {
    pub fn start(options: Options, context: egui::Context) -> Result<Self, String> {
        let (commands, input) = mpsc::sync_channel(64);
        let notices = Arc::new(Notices::default());
        let output = notices.clone();
        let (finished, done) = mpsc::sync_channel(1);
        let activity = Arc::new(Mutex::new(Arc::new(Activity::default())));
        let state = activity.clone();
        let wake = commands.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        thread::Builder::new()
            .name("tomlook-storage".into())
            .spawn(move || {
                let notify = |notice| {
                    let result = output.send(notice);
                    context.request_repaint();
                    if let Err(error) = &result { eprintln!("Tomlook: {error}"); }
                    result.is_ok()
                };
                let run = || -> Result<(), String> {
                let mut store = match Store::open(&options.state) {
                    Ok(store) => store,
                    Err(error) => return Err(error),
                };
                let load = |store: &mut Store| -> Result<(Arc<Calendar>, Preferences), String> {
                    let preferences = store.preferences()?;
                    let calendar = if let Some(count) = options.demo {
                        let now = chrono::Local::now();
                        let coverage = (0..42)
                            .map(|i| calendar::monday(now.date_naive()) + chrono::Duration::days(i))
                            .collect();
                        Calendar::build(
                            calendar::demo(now, count),
                            coverage,
                            vec!["Synthetic preview - no live AI or work data".into()],
                        )?
                    } else {
                        let saved = store.calendar()?;
                        if saved.events.is_empty() {
                            store.import_legacy(&options.legacy_root)?.unwrap_or(saved)
                        } else {
                            saved
                        }
                    };
                    Ok((Arc::new(calendar), preferences))
                };
                let (mut calendar, mut preferences) = match load(&mut store) {
                    Ok((calendar, preferences)) => { notify(Notice::Loaded { calendar: calendar.clone(), preferences: preferences.clone() }); (calendar, preferences) }
                    Err(error) => return Err(error),
                };
                let mut queue = store.queue()?;
                let (result_sender, mut results) = tokio::sync::mpsc::channel::<ai::Prepared>(2);
                let mut sdk: Option<Sdk> = None;
                let mut last_saved = String::new();
                let mut revision = 0;
                let mut failure = queue.failure.clone();
                let mut exiting = false;
                let mut dirty = true;
                let mut deadline = std::time::Instant::now();
                loop {
                    exiting |= stopping.load(Ordering::Acquire);
                    let now = chrono::Local::now().fixed_offset();
                    let dispatch_ready = !exiting && preferences.prepare_enabled && !preferences.paused && options.demo.is_none() && failure.is_none()
                        && queue.can_dispatch() && sdk.as_ref().is_some_and(|sdk| sdk.ready.load(Ordering::Acquire) && !sdk.terminated.load(Ordering::Acquire) && sdk.occupied.load(Ordering::Acquire) < 2);
                    let cancelled = if dirty || std::time::Instant::now() >= deadline || !results.is_empty() || exiting || dispatch_ready {
                        dirty = false;
                        deadline = std::time::Instant::now() + Duration::from_secs(60);
                        queue.reconcile(&calendar, now)
                    } else { Vec::new() };
                    if !exiting && (preferences.paused || !preferences.prepare_enabled)
                        && let Some(Sdk { commands, .. }) = &sdk {
                            for id in queue.flights.keys() {
                                if commands.try_send(ai::Command::Cancel(*id | (1 << 63))).is_err() {
                                    failure = Some("Preparation cancellation could not be admitted; no new jobs will start".into());
                                }
                            }
                        }
                    if !exiting && let Some(Sdk { commands, .. }) = &sdk {
                        for id in cancelled {
                            if commands.try_send(ai::Command::Cancel(id | (1 << 63))).is_err() { failure = Some("Changed-meeting cancellation could not be admitted".into()); }
                        }
                    }
                    while let Ok(prepared) = results.try_recv() {
                        match prepared.result {
                            ai::PreparationResult::NotAdmitted(message) => queue.defer(prepared.id, message),
                            ai::PreparationResult::Finished(result) => {
                                if let Some(flight) = queue.current(prepared.id).cloned() {
                                    match result.and_then(|raw| validate_prepared(&raw, &flight.fingerprint)) {
                                        Ok(payload) => {
                                            let mut completed = queue.clone();
                                            completed.finish(prepared.id, Ok(()));
                                            match store.save_prepared(&completed, &flight.event_id, &payload) {
                                                Ok(()) => queue = completed,
                                                Err(error) => { queue.finish(prepared.id, Err(error.clone())); failure = Some(error); }
                                            }
                                        }
                                        Err(error) => {
                                            queue.finish(prepared.id, Err(error.clone()));
                                            failure = Some(format!("Preparation stopped after a failed operation: {error}. Inspect saved evidence and explicitly retry."));
                                        }
                                    }
                                } else { queue.finish(prepared.id, Err("Obsolete result was not published".into())); }
                            }
                        }
                    }
                    if sdk.as_ref().is_some_and(|sdk| sdk.terminated.load(Ordering::Acquire)) && !queue.flights.is_empty() {
                        queue.interrupt("AI executor stopped before an outcome was recorded; inspect saved evidence before explicit retry");
                        failure = queue.failure.clone();
                    }
                    if !exiting && preferences.prepare_enabled && !preferences.paused && options.demo.is_none() && failure.is_none()
                        && let Some(Sdk { commands, ready, occupied, terminated }) = &sdk
                        && ready.load(Ordering::Acquire) && !terminated.load(Ordering::Acquire) && occupied.load(Ordering::Acquire) < 2
                            && let Some((id, flight)) = queue.dispatch() {
                                match store.save_queue(&queue) {
                                    Ok(()) => {
                                        if let Some(event) = calendar.events.iter().find(|event| event.id == flight.event_id) {
                                            if commands.try_send(ai::Command::Prepare(Box::new(ai::Preparation {
                                                id, event: event.clone(), foreground: flight.foreground, reply: result_sender.clone(), wake: wake.clone(),
                                            }))).is_err() { queue.defer(id, "SDK admission channel is unavailable; request was not sent".into()); failure = Some("SDK admission channel is unavailable".into()); }
                                        } else { queue.finish(id, Err("Meeting disappeared before dispatch".into())); }
                                    }
                                    Err(error) => { queue.defer(id, error.clone()); failure = Some(error); }
                                }
                            }
                    if failure.is_none() { failure = queue.failure.clone(); }
                    queue.failure = failure.clone();
                    let serialized = serde_json::to_string(&queue).map_err(|error| format!("Serialize preparation state: {error}"))?;
                    if serialized != last_saved {
                        match store.save_queue(&queue) { Ok(()) => last_saved = serialized, Err(error) => failure = Some(error) }
                    }
                    let next = Activity { revision, queue: queue.clone(), enabled: preferences.prepare_enabled, paused: preferences.paused, error: failure.clone(),
                        queued: queue.jobs.values().filter(|job| job.state == crate::scheduler::State::Queued).count(),
                        deferred: queue.jobs.values().filter(|job| job.state == crate::scheduler::State::Deferred).count() };
                    match state.lock() {
                        Ok(mut previous) => {
                            if previous.queue != next.queue || previous.enabled != next.enabled || previous.paused != next.paused || previous.error != next.error {
                                revision += 1;
                                *previous = Arc::new(Activity { revision, ..next });
                                context.request_repaint();
                            }
                        }
                        Err(_) => return Err("Preparation state is unavailable".into()),
                    }
                    let command = if exiting {
                        match input.try_recv() {
                            Ok(command) => command,
                            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
                        }
                    } else { match input.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
                        Ok(command) => command,
                        Err(mpsc::RecvTimeoutError::Timeout) => Command::Wake,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    } };
                    let result = match command {
                        Command::Preferences(mut next) => {
                            next.prepare_enabled = preferences.prepare_enabled;
                            next.paused = preferences.paused;
                            preferences = next; store.save_preferences(&preferences)
                        },
                        Command::EnablePreparation(enabled) => { preferences.prepare_enabled = enabled; store.save_preferences(&preferences) },
                        Command::PausePreparation(paused) => { preferences.paused = paused; store.save_preferences(&preferences) },
                        Command::ResumeQueued if queue.flights.is_empty() => {
                            failure = None; queue.failure = None; preferences.paused = false;
                            store.save_preferences(&preferences)
                        },
                        Command::ResumeQueued => Err("Wait for running operations to stop before resuming queued work".into()),
                        Command::Detail { revision, id } => match store.briefing(&id) {
                            Ok(payload) => match payload
                                .map(|json| serde_json::from_str(&json))
                                .transpose()
                            {
                                Ok(payload) => {
                                    notify(Notice::Detail { revision, payload });
                                    Ok(())
                                }
                                Err(error) => Err(format!("Saved briefing is invalid: {error}")),
                            },
                            Err(error) => Err(error),
                        },
                        Command::Reload => match load(&mut store) {
                            Ok((next, next_preferences)) => {
                                calendar = next;
                                dirty = true;
                                preferences = next_preferences;
                                notify(Notice::Loaded { calendar: calendar.clone(), preferences: preferences.clone() });
                                Ok(())
                            }
                            Err(error) => Err(error),
                        },
                        Command::Promote(id) => { queue.promote(&id); if failure.is_none() { failure = queue.failure.clone(); } dirty = true; Ok(()) }
                        Command::Retry(id) => { if queue.retry(&id) { failure = None; } Ok(()) }
                        Command::TogglePause => { preferences.paused = !preferences.paused; store.save_preferences(&preferences) }
                        Command::Wake => Ok(()),
                        Command::AttachAi { commands, ready, occupied, terminated } => {
                            let result = commands.try_send(ai::Command::AttachWake(wake.clone())).map_err(|e| format!("Attach preparation wake channel: {e}"));
                            sdk = Some(Sdk { commands, ready, occupied, terminated });
                            result
                        }
                        Command::Exit => { exiting = true; Ok(()) },
                    };
                    if let Err(error) = result {
                        failure = Some(format!("Preparation stopped after a local operation failed: {error}"));
                        notify(Notice::Error(error));
                    }
                }
                queue.failure = failure;
                store.save_queue(&queue)
                };
                let result = run();
                if let Err(error) = &result { notify(Notice::Error(error.clone())); }
                let _ = finished.send(result);
            })
            .map_err(|e| format!("Start storage worker: {e}"))?;
        Ok(Self {
            commands,
            notices,
            activity,
            stop,
            done: Some(done),
        })
    }
}
