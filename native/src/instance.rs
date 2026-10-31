use std::{fs::File, path::Path};

#[cfg(windows)]
fn claim(state: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::create_dir_all(state)?;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(state.join("instance.lock"))
}

#[cfg(windows)]
pub fn acquire(state: &Path) -> Result<File, String> {
    claim(state).map_err(|error| {
            if matches!(error.raw_os_error(), Some(32 | 33)) {
                "This Tomlook profile is already open. Restore its window from the Tomlook tray icon; do not start a second preparation worker.".into()
            } else {
                format!("Acquire Tomlook profile ownership: {error}")
            }
    })
}

#[cfg(windows)]
pub fn acquire_or_restore(state: &Path) -> Result<Option<File>, String> {
    match claim(state) {
        Ok(owner) => Ok(Some(owner)),
        Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
            windows::request_restore(state)?;
            Ok(None)
        }
        Err(error) => Err(format!("Acquire Tomlook profile ownership: {error}")),
    }
}

#[cfg(not(windows))]
pub fn acquire(_: &Path) -> Result<File, String> {
    Err("Native profile ownership currently requires Windows".into())
}

#[cfg(not(windows))]
pub fn acquire_or_restore(state: &Path) -> Result<Option<File>, String> {
    acquire(state).map(Some)
}

/// Used by the relaunch after an update: waits for the previous process to exit,
/// then takes the profile without ever signalling the closing instance.
pub fn acquire_after_update(
    state: &Path,
    previous: u32,
    timeout: std::time::Duration,
) -> Result<File, String> {
    let deadline = std::time::Instant::now() + timeout;
    #[cfg(windows)]
    if previous != std::process::id() {
        windows::wait_for_exit(previous, timeout);
    }
    #[cfg(not(windows))]
    let _ = previous;
    loop {
        match acquire(state) {
            Ok(owner) => return Ok(owner),
            Err(error)
                if error.contains("already open") && std::time::Instant::now() < deadline =>
            {
                // Pre-window handover only; the UI has not started yet.
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(error) if error.contains("already open") => {
                return Err("The previous Tomlook is still closing after an update. Open Tomlook again in a moment.".into());
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(windows)]
pub use windows::{Activation, Listener};

#[cfg(windows)]
mod windows {
    use std::{
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        path::Path,
        sync::Arc,
        thread::JoinHandle,
        time::{Duration, Instant},
    };

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateEventW(
            attributes: *const std::ffi::c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> *mut std::ffi::c_void;
        fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
        fn SetEvent(event: *mut std::ffi::c_void) -> i32;
        fn WaitForMultipleObjects(
            count: u32,
            handles: *const *mut std::ffi::c_void,
            wait_all: i32,
            timeout: u32,
        ) -> u32;
        fn GetLastError() -> u32;
        fn OpenProcess(access: u32, inherit: i32, process: u32) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, timeout: u32) -> u32;
    }

    /// Waits up to `timeout` for a process to exit; an unknown process counts as exited.
    pub(super) fn wait_for_exit(process: u32, timeout: Duration) {
        // SYNCHRONIZE only; the handle cannot read or control the previous process.
        let raw = unsafe { OpenProcess(0x0010_0000, 0, process) };
        if raw.is_null() {
            return;
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        unsafe { WaitForSingleObject(handle.as_raw_handle(), milliseconds) };
    }

    fn event_name(state: &Path) -> Result<Vec<u16>, String> {
        use sha2::{Digest, Sha256};
        let path = std::fs::canonicalize(state)
            .map_err(|error| format!("Resolve Tomlook window profile: {error}"))?;
        let path = path
            .to_str()
            .ok_or("Tomlook window profile is not valid Unicode")?;
        let identity = Sha256::digest(path.to_lowercase().as_bytes());
        Ok(format!("Local\\Tomlook-window-v1-{identity:x}")
            .encode_utf16()
            .chain(Some(0))
            .collect())
    }

    fn create(name: Option<&[u16]>, manual: bool) -> Result<OwnedHandle, String> {
        // Default creator-token ACL, non-inheritable handles and live UTF-16 name.
        let raw = unsafe {
            CreateEventW(
                std::ptr::null(),
                i32::from(manual),
                0,
                name.map_or(std::ptr::null(), |name| name.as_ptr()),
            )
        };
        let error = unsafe { GetLastError() };
        if raw.is_null() {
            return Err(format!(
                "Create Tomlook window signal: {}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
        // CreateEvent returns a new owned handle even for an existing named object.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        if name.is_some() && error == 183 {
            return Err(
                "Tomlook window signal already exists; no second listener was started".into(),
            );
        }
        Ok(handle)
    }

    fn signal(event: &OwnedHandle) -> Result<(), String> {
        // The shared owned handle cannot be closed during this call.
        if unsafe { SetEvent(event.as_raw_handle()) } == 0 {
            return Err(format!(
                "Signal Tomlook window: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub(super) fn request_restore(state: &Path) -> Result<(), String> {
        let name = event_name(state)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // Secondary launches can only signal; they cannot read state or change the ACL.
            let raw = unsafe { OpenEventW(0x0002, 0, name.as_ptr()) };
            if !raw.is_null() {
                let event = unsafe { OwnedHandle::from_raw_handle(raw) };
                return signal(&event);
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(2) || Instant::now() >= deadline {
                return Err(format!(
                    "This Tomlook profile is already open, but its window could not be requested: {error}. Open it from the Tomlook tray icon."
                ));
            }
            // Bound only the secondary launcher/primary-startup race, never a UI callback.
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub struct Activation {
        restore: Arc<OwnedHandle>,
        stop: Arc<OwnedHandle>,
    }

    impl Activation {
        pub fn new(state: &Path) -> Result<Self, String> {
            Ok(Self {
                restore: Arc::new(create(Some(&event_name(state)?), false)?),
                stop: Arc::new(create(None, true)?),
            })
        }

        pub fn listen(
            self,
            notify: impl Fn(Result<(), String>) + Send + 'static,
        ) -> Result<Listener, String> {
            let stop = self.stop.clone();
            let thread = std::thread::Builder::new()
                .name("tomlook-window-restore".into())
                .spawn(move || {
                    let handles = [self.stop.as_raw_handle(), self.restore.as_raw_handle()];
                    loop {
                        // Both owned events outlive the blocking wait; stop has priority.
                        match unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, u32::MAX) } {
                            0 => return,
                            1 => notify(Ok(())),
                            _ => {
                                notify(Err(format!(
                                    "Tomlook window listener stopped: {}",
                                    std::io::Error::last_os_error()
                                )));
                                return;
                            }
                        }
                    }
                })
                .map_err(|error| format!("Start Tomlook window listener: {error}"))?;
            Ok(Listener {
                stop,
                thread: Some(thread),
            })
        }
    }

    pub struct Listener {
        stop: Arc<OwnedHandle>,
        thread: Option<JoinHandle<()>>,
    }

    impl Listener {
        /// Called after the native event loop has closed, never during painting.
        pub fn shutdown(&mut self) -> Result<(), String> {
            if self.thread.is_some() {
                signal(&self.stop)?;
                self.thread
                    .take()
                    .expect("listener exists")
                    .join()
                    .map_err(|_| "Tomlook window listener panicked".to_string())?;
            }
            Ok(())
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            if let Err(error) = self.shutdown() {
                eprintln!("Tomlook: {error}");
            }
        }
    }
}
