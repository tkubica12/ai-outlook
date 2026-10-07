use std::{fs::File, path::Path};

#[cfg(windows)]
pub fn acquire(state: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::create_dir_all(state).map_err(|error| format!("Create Tomlook state: {error}"))?;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(state.join("instance.lock"))
        .map_err(|error| {
            if matches!(error.raw_os_error(), Some(32 | 33)) {
                "This Tomlook profile is already open. Restore its window from the Tomlook tray icon; do not start a second preparation worker.".into()
            } else {
                format!("Acquire Tomlook profile ownership: {error}")
            }
        })
}

#[cfg(not(windows))]
pub fn acquire(_: &Path) -> Result<File, String> {
    Err("Native profile ownership currently requires Windows".into())
}
