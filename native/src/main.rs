#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod tray;

#[derive(Default)]
struct Lifecycle {
    ai: Option<tomlook::ai::Shutdown>,
    storage: Option<tomlook::worker::Shutdown>,
    startup: Option<serde_json::Value>,
}

/// Opt-in measurement mode: records the first presented calendar frame, then exits normally.
pub struct StartupTrace {
    pub path: PathBuf,
    pub launched: std::time::Instant,
    pub marks: Vec<(&'static str, f64)>,
}

impl StartupTrace {
    pub fn mark(&mut self, name: &'static str) {
        self.marks
            .push((name, self.launched.elapsed().as_secs_f64() * 1000.0));
    }
}

use eframe::egui;
use std::path::PathBuf;
use tomlook::worker::Options;

fn options() -> Result<(Options, Option<PathBuf>), String> {
    let state = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is not defined; Windows application state is required")?
        .join("Tomlook");
    let mut result = Options {
        state,
        legacy_root: std::env::current_dir().map_err(|e| e.to_string())?,
        demo: None,
    };
    let mut trace = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--startup-trace") => {
                trace = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or("--startup-trace needs a file path")?,
                )
            }
            Some("--state") => {
                result.state = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--state needs a directory")?
            }
            Some("--legacy-root") => {
                result.legacy_root = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--legacy-root needs a directory")?
            }
            Some("--demo") => result.demo = Some(200),
            Some("--stress") => result.demo = Some(5000),
            _ => return Err(format!("Unknown argument: {}", arg.to_string_lossy())),
        }
    }
    result.state = std::path::absolute(result.state)
        .map_err(|e| format!("Resolve Tomlook state directory: {e}"))?;
    result.legacy_root = std::path::absolute(result.legacy_root)
        .map_err(|e| format!("Resolve legacy directory: {e}"))?;
    let trace = trace
        .map(std::path::absolute)
        .transpose()
        .map_err(|e| format!("Resolve startup trace path: {e}"))?;
    Ok((result, trace))
}

fn main() -> eframe::Result {
    let launched = std::time::Instant::now();
    let (options, trace) = match options() {
        Ok(options) => options,
        Err(error) => {
            show_error(error);
            std::process::exit(1);
        }
    };
    let _ownership = match tomlook::instance::acquire(&options.state) {
        Ok(ownership) => ownership,
        Err(error) => {
            show_error(error);
            std::process::exit(1);
        }
    };
    let preload = tomlook::worker::Preload::start(&options).ok();
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1366.0, 860.0])
            .with_min_inner_size([960.0, 640.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let shutdown = std::sync::Arc::new(std::sync::Mutex::new(Lifecycle::default()));
    let on_shutdown = shutdown.clone();
    let trace_path = trace.clone();
    let startup = trace.map(|path| {
        let mut trace = StartupTrace {
            path,
            launched,
            marks: Vec::new(),
        };
        trace.mark("before_native_window");
        trace
    });
    let result = eframe::run_native(
        "Tomlook",
        native,
        Box::new(move |creation| {
            Ok(Box::new(app::Tomlook::new(
                creation,
                options,
                on_shutdown,
                startup,
                preload,
            )))
        }),
    );
    // The UI is already closed; shutdown waiting never blocks a UI callback.
    let mut errors = Vec::new();
    let startup_record = {
        let mut coordinator = match shutdown.lock() {
            Ok(coordinator) => coordinator,
            Err(poisoned) => {
                errors.push(
                    "Shutdown coordination was poisoned; attempting owned-worker cleanup".into(),
                );
                poisoned.into_inner()
            }
        };
        if let Some(handle) = coordinator.ai.take() {
            if handle.stop.send(true).is_err() {
                eprintln!("Tomlook AI worker was already stopped");
            }
            if let Err(error) = handle.done.recv_timeout(std::time::Duration::from_secs(75)) {
                errors.push(format!("AI shutdown could not be verified: {error}"));
            }
        }
        if let Some(handle) = coordinator.storage.take() {
            handle
                .stop
                .store(true, std::sync::atomic::Ordering::Release);
            // A full inbox already wakes the worker; the stop flag forbids new dispatch.
            let _ = handle.commands.try_send(tomlook::worker::Command::Wake);
            match handle.done.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("Storage shutdown failed: {error}")),
                Err(error) => {
                    errors.push(format!("Storage shutdown could not be verified: {error}"))
                }
            }
        }
        coordinator.startup.take()
    };
    if let Some(path) = trace_path {
        let mut record = startup_record.unwrap_or_else(|| {
            serde_json::json!({ "error": "Window closed before a calendar frame was presented" })
        });
        record["shutdown_complete_ms"] = (launched.elapsed().as_secs_f64() * 1000.0).into();
        record["shutdown_errors"] = errors.clone().into();
        if let Err(error) = std::fs::write(&path, record.to_string()) {
            errors.push(format!("Write startup trace: {error}"));
        }
    }
    if !errors.is_empty() {
        show_error(errors.join("\n"));
        std::process::exit(1);
    }
    if let Err(error) = &result {
        show_error(format!("Native window failed: {error}"));
    }
    result
}

fn show_error(message: String) {
    eprintln!("Tomlook: {message}");
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBoxW(
                owner: *mut std::ffi::c_void,
                text: *const u16,
                title: *const u16,
                flags: u32,
            ) -> i32;
        }
        let mut text: String = message.chars().take(4000).collect();
        if text.len() < message.len() {
            text.push_str("\n[Error display truncated]");
        }
        let text: Vec<u16> = text
            .replace('\0', "\\0")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let title: Vec<u16> = "Tomlook - application error"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // Both terminated UTF-16 buffers remain live until the synchronous dialog closes.
        if unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10) } == 0 {
            eprintln!(
                "Tomlook could not display its error: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}
