#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use eframe::egui;
use std::path::PathBuf;
use tomlook::worker::Options;

fn options() -> Result<Options, String> {
    let state = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is not defined; Windows application state is required")?
        .join("Tomlook");
    let mut result = Options {
        state,
        legacy_root: std::env::current_dir().map_err(|e| e.to_string())?,
        demo: None,
    };
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
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
    Ok(result)
}

fn main() -> eframe::Result {
    let options = match options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("Tomlook: {error}");
            std::process::exit(1);
        }
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1366.0, 860.0])
            .with_min_inner_size([960.0, 640.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let shutdown = std::sync::Arc::new(std::sync::Mutex::new(None));
    let on_shutdown = shutdown.clone();
    let result = eframe::run_native(
        "Tomlook",
        native,
        Box::new(move |creation| Ok(Box::new(app::Tomlook::new(creation, options, on_shutdown)))),
    );
    // The UI is already closed; shutdown waiting never blocks a UI callback.
    if let Ok(mut coordinator) = shutdown.lock()
        && let Some(handle) = coordinator.take()
    {
        if handle
            .commands
            .blocking_send(tomlook::ai::Command::Stop)
            .is_err()
        {
            eprintln!("Tomlook AI worker was already stopped");
        }
        if let Err(error) = handle.done.recv_timeout(std::time::Duration::from_secs(75)) {
            eprintln!("Tomlook AI shutdown could not be verified: {error}");
        }
    }
    result
}
