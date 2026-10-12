#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod interaction_trace;
mod tray;

#[derive(Default)]
struct Lifecycle {
    #[cfg(windows)]
    activation: Option<tomlook::instance::Listener>,
    ai: Option<tomlook::ai::Shutdown>,
    storage: Option<tomlook::worker::Shutdown>,
    startup: Option<serde_json::Value>,
}

/// Opt-in measurement mode: records the first presented calendar frame, then exits normally.
pub struct StartupTrace {
    pub path: PathBuf,
    pub launched: std::time::Instant,
    pub marks: Vec<(&'static str, f64)>,
    pub process_created_unix_ms: f64,
    pub entry_after_creation_ms: f64,
    pub idle: bool,
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

struct Launch {
    worker: Options,
    startup: Option<PathBuf>,
    interactions: Option<PathBuf>,
    fixture: Option<tomlook::fixture_cache::Manifest>,
    screenshot: Option<PathBuf>,
    idle: bool,
}

fn options() -> Result<Launch, String> {
    let state = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is not defined; Windows application state is required")?
        .join("Tomlook");
    parse_options(
        std::env::args_os().skip(1),
        state,
        std::env::current_dir().map_err(|e| e.to_string())?,
    )
}

fn parse_options(
    mut args: impl Iterator<Item = std::ffi::OsString>,
    state: PathBuf,
    legacy_root: PathBuf,
) -> Result<Launch, String> {
    let mut result = Options {
        state: state.clone(),
        legacy_root,
        demo: None,
    };
    let mut trace = None;
    let mut interactions = None;
    let mut explicit_state = false;
    let mut explicit_legacy = false;
    let mut fixture_cache = false;
    let mut screenshot = None;
    let mut idle = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--demo-screenshot") => {
                screenshot = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or("--demo-screenshot needs a new PNG file path")?,
                );
            }
            Some("--fixture-cache") => fixture_cache = true,
            Some("--interaction-trace") => {
                interactions = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or("--interaction-trace needs a file path")?,
                );
            }
            Some("--startup-trace" | "--idle-trace") => {
                if trace.is_some() {
                    return Err("Choose one startup or idle measurement".into());
                }
                idle = arg.to_str() == Some("--idle-trace");
                trace = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or("Startup or idle trace needs a file path")?,
                )
            }
            Some("--state") => {
                explicit_state = true;
                result.state = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--state needs a directory")?
            }
            Some("--legacy-root") => {
                explicit_legacy = true;
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
    let interactions = interactions
        .map(std::path::absolute)
        .transpose()
        .map_err(|e| format!("Resolve interaction trace path: {e}"))?;
    let screenshot = screenshot
        .map(std::path::absolute)
        .transpose()
        .map_err(|e| format!("Resolve demo screenshot path: {e}"))?;
    if let Some(path) = &screenshot
        && (result.demo.is_none()
            || trace.is_some()
            || interactions.is_some()
            || !path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
            || path.exists())
    {
        return Err("Demo screenshots require --demo or --stress, a new PNG output, and no measurement mode".into());
    }
    if result.demo.is_some() {
        if !explicit_state
            || result.state == std::path::absolute(&state).map_err(|e| e.to_string())?
            || std::fs::canonicalize(&result.state)
                .ok()
                .zip(std::fs::canonicalize(&state).ok())
                .is_some_and(|(candidate, personal)| candidate == personal)
        {
            return Err("Demo previews require --state pointing to a new, empty directory; the personal profile is never used".into());
        }
        match std::fs::read_dir(&result.state) {
            Ok(mut entries) => {
                if entries
                    .next()
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .is_some()
                {
                    return Err("Demo previews require a new, empty state directory; existing files are never loaded".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Inspect demo profile: {error}")),
        }
    }
    if env!("CARGO_BIN_NAME") == "tomlook-acceptance" && interactions.is_none() && trace.is_none() {
        return Err("The acceptance executable requires a measurement mode".into());
    }
    if idle && (!fixture_cache || env!("CARGO_BIN_NAME") != "tomlook-acceptance") {
        return Err(
            "Idle measurements require the acceptance executable and frozen fixture cache".into(),
        );
    }
    if let Some(path) = &interactions {
        if trace.is_some()
            || !fixture_cache
            || result.demo.is_some()
            || !explicit_state
            || !explicit_legacy
        {
            return Err(
                "Interaction traces require --fixture-cache, an explicitly seeded --state and empty --legacy-root; startup tracing and dynamic previews must be separate".into(),
            );
        }
        if result.state == state
            || std::fs::canonicalize(&result.state)
                .ok()
                .zip(std::fs::canonicalize(&state).ok())
                .is_some_and(|(candidate, personal)| candidate == personal)
        {
            return Err("Interaction tracing cannot use the normal Tomlook profile".into());
        }
        if path.exists() {
            return Err(
                "Interaction trace output already exists; choose a new evidence file".into(),
            );
        }
    }
    let fixture = if fixture_cache {
        if env!("CARGO_BIN_NAME") != "tomlook-acceptance"
            || !explicit_state
            || !explicit_legacy
            || result.demo.is_some()
            || (trace.is_none() && interactions.is_none())
        {
            return Err("Frozen fixture caches are restricted to the acceptance executable with explicit measurement/profile arguments".into());
        }
        match std::fs::read_dir(&result.legacy_root) {
            Ok(mut entries) => {
                if entries
                    .next()
                    .transpose()
                    .map_err(|error| format!("Inspect measurement directory: {error}"))?
                    .is_some()
                {
                    return Err("Fixture measurements require an empty legacy directory".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Inspect measurement directory: {error}")),
        }
        if trace.as_ref().is_some_and(|path| path.exists()) {
            return Err("Startup trace output already exists; choose a new evidence file".into());
        }
        Some(tomlook::fixture_cache::read(&result.state)?)
    } else {
        None
    };
    if env!("CARGO_BIN_NAME") == "tomlook-acceptance" && fixture.is_none() {
        return Err("Acceptance measurements require a seeded frozen fixture cache".into());
    }
    Ok(Launch {
        worker: result,
        startup: trace,
        interactions,
        fixture,
        screenshot,
        idle,
    })
}

fn main() -> eframe::Result {
    let launched = std::time::Instant::now();
    let entry_clock = std::time::SystemTime::now();
    let Launch {
        worker: options,
        startup: trace,
        interactions,
        fixture,
        screenshot,
        idle,
    } = match options() {
        Ok(options) => options,
        Err(error) => {
            show_error(error);
            std::process::exit(1);
        }
    };
    let normal_launch = trace.is_none() && interactions.is_none() && options.demo.is_none();
    let _ownership = match if normal_launch {
        tomlook::instance::acquire_or_restore(&options.state)
    } else {
        tomlook::instance::acquire(&options.state).map(Some)
    } {
        Ok(Some(ownership)) => ownership,
        Ok(None) => return Ok(()),
        Err(error) => {
            show_error(error);
            std::process::exit(1);
        }
    };
    #[cfg(windows)]
    let activation = if normal_launch {
        Some(
            tomlook::instance::Activation::new(&options.state).map_err(|error| {
                show_error(error);
                eframe::Error::AppCreation(Box::new(std::io::Error::other(
                    "Window restoration could not start",
                )))
            })?,
        )
    } else {
        None
    };
    let preload = match fixture.clone() {
        Some(manifest) => tomlook::worker::Preload::start_fixture(&options, manifest),
        None => tomlook::worker::Preload::start(&options),
    }
    .map_err(|error| {
        show_error(error);
        eframe::Error::AppCreation(Box::new(std::io::Error::other(
            "Calendar preload could not start",
        )))
    })?;
    let mut native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(egui::IconData {
                rgba: tomlook::brand::rgba(64),
                width: 64,
                height: 64,
            })
            .with_inner_size(if screenshot.is_some() {
                [1600.0, 900.0]
            } else {
                [1366.0, 860.0]
            })
            .with_min_inner_size([960.0, 640.0])
            .with_active(interactions.is_none() && trace.is_none()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    native.glow_options.vsync = false;
    let shutdown = std::sync::Arc::new(std::sync::Mutex::new(Lifecycle::default()));
    let on_shutdown = shutdown.clone();
    let trace_path = interactions.clone().or_else(|| trace.clone());
    let interaction_trace = interactions
        .as_ref()
        .and(fixture.as_ref().map(|manifest| manifest.events))
        .map(interaction_trace::Trace::new);
    let startup = trace
        .map(|path| -> Result<_, String> {
            let created = process_created()?;
            let entry_after_creation_ms = entry_clock
                .duration_since(created)
                .map_err(|error| format!("Align startup process clock: {error}"))?
                .as_secs_f64()
                * 1000.0;
            let mut trace = StartupTrace {
                path,
                launched,
                marks: Vec::new(),
                process_created_unix_ms: created
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| format!("Read process creation clock: {error}"))?
                    .as_secs_f64()
                    * 1000.0,
                entry_after_creation_ms,
                idle,
            };
            trace.mark("before_native_window");
            Ok(trace)
        })
        .transpose()
        .map_err(|error| {
            show_error(error);
            eframe::Error::AppCreation(Box::new(std::io::Error::other("Startup clock unavailable")))
        })?;
    let result = eframe::run_native(
        if options.demo.is_some() {
            "Tomlook - Demo"
        } else {
            "Tomlook"
        },
        native,
        Box::new(move |creation| {
            #[cfg(windows)]
            if let Some(activation) = activation {
                let context = creation.egui_ctx.clone();
                let listener = activation
                    .listen(move |result| match result {
                        Ok(()) => tray::restore(&context),
                        Err(error) => {
                            eprintln!("Tomlook: {error}");
                            context.data_mut(|data| {
                                data.insert_temp(egui::Id::new("tray-error"), error)
                            });
                            context.request_repaint();
                        }
                    })
                    .map_err(std::io::Error::other)?;
                on_shutdown
                    .lock()
                    .map_err(|_| {
                        std::io::Error::other("Window restoration coordinator is unavailable")
                    })?
                    .activation = Some(listener);
            }
            Ok(Box::new(app::Tomlook::new(
                creation,
                options,
                on_shutdown,
                startup,
                Some(preload),
                interaction_trace,
                screenshot,
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
        #[cfg(windows)]
        if let Some(mut listener) = coordinator.activation.take()
            && let Err(error) = listener.shutdown()
        {
            errors.push(error);
        }
        if let Some(handle) = coordinator.ai.take() {
            if handle.stop.send(true).is_err() {
                eprintln!("Tomlook AI worker was already stopped");
            }
            if let Err(error) = handle
                .done
                .recv_timeout(std::time::Duration::from_secs(240))
            {
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
            serde_json::json!({ "error": "Window closed before the measurement finished", "threshold_pass": false })
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

#[cfg(windows)]
fn process_created() -> Result<std::time::SystemTime, String> {
    #[derive(Default)]
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessTimes(
            process: *mut std::ffi::c_void,
            created: *mut FileTime,
            exited: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let mut created = FileTime::default();
    let mut exited = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    // Current-process pseudo handle and writable FILETIME buffers stay valid for the call.
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(format!(
            "Read process creation time: {}",
            std::io::Error::last_os_error()
        ));
    }
    let ticks = ((u64::from(created.high) << 32) | u64::from(created.low))
        .checked_sub(116_444_736_000_000_000)
        .ok_or("Process creation predates the Unix epoch")?;
    std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::new(
            ticks / 10_000_000,
            u32::try_from((ticks % 10_000_000) * 100).map_err(|error| error.to_string())?,
        ))
        .ok_or_else(|| "Process creation time is unsupported".into())
}

#[cfg(not(windows))]
fn process_created() -> Result<std::time::SystemTime, String> {
    Err("Process-creation startup measurements require Windows".into())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshots_are_demo_only_and_cannot_replace_existing_outputs() {
        let root = tempfile::tempdir().unwrap();
        let png = root.path().join("demo.png");
        let parse = |args: Vec<std::ffi::OsString>| {
            parse_options(
                args.into_iter(),
                root.path().join("personal"),
                root.path().into(),
            )
        };
        assert!(
            parse(vec![
                "--demo-screenshot".into(),
                png.clone().into_os_string()
            ])
            .is_err()
        );
        let arguments = || {
            vec![
                "--demo".into(),
                "--state".into(),
                root.path().join("fresh").into_os_string(),
                "--demo-screenshot".into(),
                png.clone().into_os_string(),
            ]
        };
        assert_eq!(parse(arguments()).unwrap().screenshot, Some(png.clone()));
        std::fs::write(&png, "existing").unwrap();
        assert!(parse(arguments()).is_err());
        assert_eq!(std::fs::read_to_string(png).unwrap(), "existing");
    }

    #[test]
    fn demo_launch_never_reads_default_or_existing_profiles() {
        let root = tempfile::tempdir().unwrap();
        let personal = root.path().join("personal");
        let owned = root.path().join("existing");
        std::fs::create_dir(&owned).unwrap();
        std::fs::write(owned.join("private.txt"), "must not read").unwrap();
        for args in [
            vec!["--demo".into()],
            vec![
                "--demo".into(),
                "--state".into(),
                personal.clone().into_os_string(),
            ],
            vec![
                "--stress".into(),
                "--state".into(),
                owned.clone().into_os_string(),
            ],
        ] {
            assert!(parse_options(args.into_iter(), personal.clone(), root.path().into()).is_err());
        }
        let fresh = root.path().join("fresh");
        let launch = parse_options(
            ["--demo".into(), "--state".into(), fresh.into_os_string()].into_iter(),
            personal,
            root.path().into(),
        )
        .unwrap();
        assert_eq!(launch.worker.demo, Some(200));
        assert_eq!(
            std::fs::read_to_string(owned.join("private.txt")).unwrap(),
            "must not read"
        );
    }

    #[test]
    fn idle_measurements_cannot_share_startup_reports_or_run_in_production() {
        let root = tempfile::tempdir().unwrap();
        let parse = |args: Vec<&str>| {
            parse_options(
                args.into_iter().map(Into::into),
                root.path().join("personal"),
                root.path().into(),
            )
        };
        assert!(
            matches!(parse(vec!["--idle-trace", "idle.json"]), Err(error) if error.contains("acceptance executable"))
        );
        assert!(
            matches!(parse(vec!["--startup-trace", "startup.json", "--idle-trace", "idle.json"]), Err(error) if error.contains("Choose one"))
        );
    }

    #[test]
    #[cfg(windows)]
    fn native_process_creation_clock_is_available_and_precedes_observation() {
        let created = process_created().unwrap();
        assert!(created > std::time::UNIX_EPOCH);
        assert!(created <= std::time::SystemTime::now());
    }

    #[test]
    fn interaction_measurement_rejects_existing_state() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing");
        std::fs::create_dir(&existing).unwrap();
        std::fs::write(existing.join("owned-data"), "must not change").unwrap();
        let args = [
            "--demo".into(),
            "--interaction-trace".into(),
            root.path().join("trace.json").into_os_string(),
            "--state".into(),
            existing.into_os_string(),
            "--legacy-root".into(),
            root.path().join("empty").into_os_string(),
        ];
        assert!(
            parse_options(
                args.into_iter(),
                root.path().join("personal"),
                root.path().into()
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("existing").join("owned-data")).unwrap(),
            "must not change"
        );
    }

    #[test]
    fn interaction_measurement_requires_synthetic_data_and_both_owned_directories() {
        let root = tempfile::tempdir().unwrap();
        for args in [
            vec!["--interaction-trace", "trace.json"],
            vec!["--demo", "--interaction-trace", "trace.json"],
            vec![
                "--demo",
                "--state",
                "isolated",
                "--interaction-trace",
                "trace.json",
            ],
        ] {
            assert!(
                parse_options(
                    args.into_iter().map(Into::into),
                    root.path().join("personal"),
                    root.path().into()
                )
                .is_err()
            );
        }
    }
}
