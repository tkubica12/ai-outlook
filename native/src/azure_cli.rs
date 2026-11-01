//! Shell-free Azure CLI process construction shared by Foundry routing and MSX sign-in.
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Builds a hidden, non-interactive Azure CLI process with piped output and no stdin.
/// An MSI `az.cmd` runs through its bundled Python directly, without an intermediate shell.
pub(crate) fn command(cli: &Path, args: &[&str]) -> Result<tokio::process::Command, ()> {
    #[cfg(windows)]
    let mut command = if cli
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"))
    {
        // Match the installed MSI az.cmd bootstrap, without an intermediate shell.
        let python = cli
            .parent()
            .and_then(Path::parent)
            .ok_or(())?
            .join("python.exe");
        if !python.is_file() {
            return Err(());
        }
        let mut command = tokio::process::Command::new(python);
        command
            .args(["-u", "-IBm", "azure.cli"])
            .args(args)
            .env("AZ_INSTALLER", "MSI");
        command
    } else {
        let mut command = tokio::process::Command::new(cli);
        command.args(args);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = tokio::process::Command::new(cli);
        command.args(args);
        command
    };
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("AZURE_EXTENSION_USE_DYNAMIC_INSTALL", "no")
        .env("AZURE_CORE_COLLECT_TELEMETRY", "0")
        .env("AZURE_CORE_NO_COLOR", "true");
    Ok(command)
}

/// The machine-wide MSI installation; PATH is not searched, so a planted `az` is never run.
pub fn installed() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join(r"Microsoft SDKs\Azure\CLI2\wbin\az.cmd"))
        .find(|path| path.is_absolute() && path.is_file())
}

pub(crate) fn device_code(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    if !text.split_whitespace().any(|word| {
        matches!(
            word,
            "https://microsoft.com/devicelogin" | "https://aka.ms/devicelogin"
        )
    }) {
        return None;
    }
    let code = text
        .split_once("enter the code ")?
        .1
        .split_once(char::is_whitespace)?
        .0;
    (code.len() >= 6
        && code.len() <= 16
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-'))
    .then(|| code.to_owned())
}

/// Ends a timed-out CLI and any helper it started; only this process tree is targeted.
pub(crate) async fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let taskkill = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join(r"System32\taskkill.exe");
        let mut command = tokio::process::Command::new(taskkill);
        command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .creation_flags(CREATE_NO_WINDOW);
        if let Ok(mut killer) = command.spawn() {
            let _ = tokio::time::timeout(Duration::from_secs(5), killer.wait()).await;
        }
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
}
