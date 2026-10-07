use crate::{
    calendar::{self, Calendar},
    storage::{Preferences, Store},
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};

#[derive(Clone)]
pub struct Options {
    pub state: PathBuf,
    pub legacy_root: PathBuf,
    pub demo: Option<usize>,
}

pub enum Command {
    Preferences(Preferences),
    Detail { revision: u64, id: String },
    Reload,
    Exit,
}

pub enum Notice {
    Loaded {
        calendar: Arc<Calendar>,
        preferences: Preferences,
    },
    Detail {
        revision: u64,
        payload: Option<String>,
    },
    Error(String),
}

pub struct Worker {
    pub commands: SyncSender<Command>,
    pub notices: Receiver<Notice>,
}

impl Worker {
    pub fn start(options: Options, context: egui::Context) -> Result<Self, String> {
        let (commands, input) = mpsc::sync_channel(64);
        let (output, notices) = mpsc::sync_channel(64);
        thread::Builder::new()
            .name("tomlook-storage".into())
            .spawn(move || {
                let notify = |notice| {
                    if output.send(notice).is_ok() {
                        context.request_repaint();
                        true
                    } else {
                        false
                    }
                };
                let mut store = match Store::open(&options.state) {
                    Ok(store) => store,
                    Err(error) => {
                        notify(Notice::Error(error));
                        return;
                    }
                };
                let load = |store: &mut Store| -> Result<Notice, String> {
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
                    Ok(Notice::Loaded {
                        calendar: Arc::new(calendar),
                        preferences,
                    })
                };
                match load(&mut store) {
                    Ok(notice) => {
                        notify(notice);
                    }
                    Err(error) => {
                        notify(Notice::Error(error));
                    }
                }
                while let Ok(command) = input.recv() {
                    let result = match command {
                        Command::Preferences(preferences) => store.save_preferences(&preferences),
                        Command::Detail { revision, id } => match store.briefing(&id) {
                            Ok(payload) => {
                                notify(Notice::Detail { revision, payload });
                                Ok(())
                            }
                            Err(error) => Err(error),
                        },
                        Command::Reload => match load(&mut store) {
                            Ok(notice) => {
                                notify(notice);
                                Ok(())
                            }
                            Err(error) => Err(error),
                        },
                        Command::Exit => break,
                    };
                    if let Err(error) = result {
                        notify(Notice::Error(error));
                    }
                }
            })
            .map_err(|e| format!("Start storage worker: {e}"))?;
        Ok(Self { commands, notices })
    }
}
