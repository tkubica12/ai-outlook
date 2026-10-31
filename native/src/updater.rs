//! Signed automatic updates from the public GitHub Releases of Tomlook.
//!
//! The executable is unsigned, so trust comes from an Ed25519-signed release
//! manifest that carries the SHA-256 of the archive and the executable.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// Base64 Ed25519 public keys accepted for release manifests; append a new key before rotating.
pub const TRUSTED_KEYS: &[&str] = &["3blSGgAUgH/fLUKOV+iCHnQLcm6gDXNJgVYMO5rVAUs="];

pub const REPOSITORY: &str = "tkubica12/ai-outlook";
pub const PRODUCT: &str = "Tomlook";
pub const MANIFEST_NAME: &str = "tomlook-update.json";
pub const SIGNATURE_NAME: &str = "tomlook-update.json.sig";
pub const EXECUTABLE: &str = "Tomlook.exe";
pub const NOTICES: [&str; 2] = ["LICENSE", "THIRD_PARTY_NOTICES.txt"];
/// Staging directory created next to the running executable.
pub const STAGING: &str = ".tomlook-update";
pub const MANIFEST_LIMIT: usize = 64 * 1024;
pub const ARCHIVE_LIMIT: u64 = 150 * 1024 * 1024;
pub const AFTER_UPDATE: &str = "--after-update";

const LATEST: &str = "https://api.github.com/repos/tkubica12/ai-outlook/releases/latest";
const RELEASE_LIMIT: usize = 1024 * 1024;
const NOTICE_LIMIT: u64 = 16 * 1024 * 1024;
const API_TIMEOUT: Duration = Duration::from_secs(30);
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY: Duration = Duration::from_secs(60 * 60);
const STARTUP_DELAY: Duration = Duration::from_secs(60);

pub const INVALID_SIGNATURE: &str = "The update signature is invalid; the update was rejected.";
pub const ARCHIVE_MISMATCH: &str =
    "The downloaded archive does not match its signed SHA-256; it was discarded.";
pub const EXECUTABLE_MISMATCH: &str =
    "The extracted Tomlook.exe does not match its signed SHA-256; it was discarded.";
pub const UNSAFE_PATH: &str = "The update archive contains an unsafe path; it was rejected.";
pub const UNEXPECTED_ENTRY: &str =
    "The update archive contains an unexpected entry; it was rejected.";
const NETWORK: &str = "GitHub Releases could not be reached; check the network and try again.";
const RATE_LIMITED: &str = "GitHub API rate limit reached; Tomlook will try again later.";
const NO_RELEASE: &str = "No published Tomlook release was found.";
const NO_MANIFEST: &str = "The latest release has no signed update information.";
const OVERSIZED: &str = "A GitHub response exceeded its size limit; the update was rejected.";
const MISMATCHED_RELEASE: &str =
    "Signed update information does not match the GitHub release; the update was rejected.";
const UNREADABLE_RELEASE: &str = "GitHub returned an unreadable release description.";
const UNREADABLE_MANIFEST: &str = "Signed update information is not readable.";
const INVALID_MANIFEST: &str = "Signed update information is invalid; the update was rejected.";
const UNREADABLE_ARCHIVE: &str = "The update archive could not be read; it was discarded.";
const MISSING_EXECUTABLE: &str = "The update archive does not contain Tomlook.exe.";
const STAGING_FAILED: &str = "The update could not be saved next to Tomlook.exe.";
const STOPPED: &str = "The update download stopped because Tomlook is closing.";
const RENAMED: &str =
    "Tomlook can replace itself only as Tomlook.exe; download the update from the release page.";
const NOT_WRITABLE: &str =
    "Tomlook cannot write to its own folder; download the update from the release page.";
const CHANGED: &str = "The staged update changed after verification and was not installed.";
const IN_USE: &str =
    "A previous Tomlook update is still in use; exit all Tomlook windows and try again.";
pub const DEVELOPMENT: &str =
    "Development builds are not updated automatically; rebuild them instead.";
const UNAVAILABLE: &str = "The update checker could not start.";

/// Static user-facing messages; each one has a Czech catalog entry.
pub const MESSAGES: &[&str] = &[
    INVALID_SIGNATURE,
    ARCHIVE_MISMATCH,
    EXECUTABLE_MISMATCH,
    UNSAFE_PATH,
    UNEXPECTED_ENTRY,
    NETWORK,
    RATE_LIMITED,
    NO_RELEASE,
    NO_MANIFEST,
    OVERSIZED,
    MISMATCHED_RELEASE,
    UNREADABLE_RELEASE,
    UNREADABLE_MANIFEST,
    INVALID_MANIFEST,
    UNREADABLE_ARCHIVE,
    MISSING_EXECUTABLE,
    STAGING_FAILED,
    STOPPED,
    RENAMED,
    NOT_WRITABLE,
    CHANGED,
    IN_USE,
    DEVELOPMENT,
    UNAVAILABLE,
];

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub product: String,
    pub version: String,
    pub tag: String,
    pub published: String,
    pub notes_url: String,
    pub archive: Archive,
    pub executable: Executable,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Archive {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Executable {
    pub name: String,
    pub sha256: String,
}

fn sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn archive_name(tag: &str) -> String {
    format!("{PRODUCT}-{tag}-win-x64.zip")
}

pub fn release_page(tag: &str) -> String {
    format!("https://github.com/{REPOSITORY}/releases/tag/{tag}")
}

impl Manifest {
    pub fn validate(&self) -> Result<semver::Version, String> {
        let version = semver::Version::parse(&self.version).map_err(|_| INVALID_MANIFEST)?;
        if self.schema != 1
            || self.product != PRODUCT
            || !version.pre.is_empty()
            || !version.build.is_empty()
            || self.tag != format!("v{version}")
            || DateTime::parse_from_rfc3339(&self.published).is_err()
            || self.notes_url != release_page(&self.tag)
            || self.archive.name != archive_name(&self.tag)
            || !(1..=ARCHIVE_LIMIT).contains(&self.archive.size)
            || !sha256_hex(&self.archive.sha256)
            || self.executable.name != EXECUTABLE
            || !sha256_hex(&self.executable.sha256)
        {
            return Err(INVALID_MANIFEST.into());
        }
        Ok(version)
    }
}

/// Verifies a base64 Ed25519 signature over the exact manifest bytes.
pub fn verify_signature(message: &[u8], signature: &[u8], keys: &[&str]) -> Result<(), String> {
    let signature = std::str::from_utf8(signature)
        .ok()
        .and_then(|text| STANDARD.decode(text.trim()).ok())
        .filter(|signature| signature.len() == 64)
        .ok_or(INVALID_SIGNATURE)?;
    for key in keys {
        let Ok(key) = STANDARD.decode(key) else {
            continue;
        };
        if key.len() == 32
            && ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
                .verify(message, &signature)
                .is_ok()
        {
            return Ok(());
        }
    }
    Err(INVALID_SIGNATURE.into())
}

/// Checks the signature before the manifest is parsed, then validates its contents.
pub fn verify_manifest(
    manifest: &[u8],
    signature: &[u8],
    keys: &[&str],
) -> Result<Manifest, String> {
    if manifest.len() > MANIFEST_LIMIT || signature.len() > MANIFEST_LIMIT {
        return Err(OVERSIZED.into());
    }
    verify_signature(manifest, signature, keys)?;
    let manifest: Manifest = serde_json::from_slice(manifest).map_err(|_| UNREADABLE_MANIFEST)?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn is_newer(candidate: &str, current: &str) -> Result<bool, String> {
    let candidate = semver::Version::parse(candidate).map_err(|_| INVALID_MANIFEST)?;
    let current = semver::Version::parse(current).map_err(|_| INVALID_MANIFEST)?;
    Ok(candidate > current)
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct ReleaseBody {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

/// A stable GitHub release; the tag is unauthenticated until a signed manifest confirms it.
pub struct Release {
    pub tag: String,
    pub version: semver::Version,
    assets: Vec<ReleaseAsset>,
}

impl Release {
    /// Returns the exact download URL and listed size of a release asset.
    pub fn asset(&self, name: &str) -> Result<(String, u64), String> {
        let expected = format!(
            "https://github.com/{REPOSITORY}/releases/download/{}/{name}",
            self.tag
        );
        let asset = self.assets.iter().find(|asset| asset.name == name).ok_or(
            if name == MANIFEST_NAME || name == SIGNATURE_NAME {
                NO_MANIFEST
            } else {
                MISMATCHED_RELEASE
            },
        )?;
        if asset.browser_download_url != expected {
            return Err(MISMATCHED_RELEASE.into());
        }
        Ok((expected, asset.size))
    }
}

/// Parses `releases/latest`; drafts and prereleases are ignored.
pub fn parse_release(body: &[u8]) -> Result<Option<Release>, String> {
    let release: ReleaseBody = serde_json::from_slice(body).map_err(|_| UNREADABLE_RELEASE)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .and_then(|version| semver::Version::parse(version).ok())
        .filter(|version| version.pre.is_empty() && version.build.is_empty())
        .ok_or(UNREADABLE_RELEASE)?;
    Ok(Some(Release {
        tag: release.tag_name,
        version,
        assets: release.assets,
    }))
}

/// Delay before the first automatic check: one minute after start, at most once per day.
pub fn first_check_delay(
    since_start: Duration,
    checked_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Duration {
    let startup = STARTUP_DELAY.saturating_sub(since_start);
    let daily = checked_at.map_or(Duration::ZERO, |checked| {
        DAY.saturating_sub((now - checked).to_std().unwrap_or(Duration::ZERO))
    });
    startup.max(daily)
}

/// True for executables built inside a Cargo `target` directory.
pub fn development_build(executable: &Path) -> bool {
    executable.ancestors().skip(1).any(|directory| {
        directory
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("target"))
            && directory
                .parent()
                .is_some_and(|parent| parent.join("Cargo.toml").is_file())
    })
}

/// Confirms Tomlook can replace itself and returns its staging directory.
pub fn staging_root(executable: &Path) -> Result<PathBuf, String> {
    if !executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(EXECUTABLE))
    {
        return Err(RENAMED.into());
    }
    let directory = executable.parent().ok_or(NOT_WRITABLE)?;
    let probe = directory.join(format!(".tomlook-write-test-{}", std::process::id()));
    let _ = fs::remove_file(&probe);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|_| NOT_WRITABLE)?;
    fs::remove_file(&probe).map_err(|_| NOT_WRITABLE)?;
    let root = directory.join(STAGING);
    fs::create_dir_all(&root).map_err(|_| NOT_WRITABLE)?;
    Ok(root)
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("Read {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    pub version: String,
    pub notes_url: String,
    /// Verified extracted files: Tomlook.exe and optional notices.
    pub directory: PathBuf,
    /// The executable that will be replaced.
    pub target: PathBuf,
    pub executable_sha256: String,
}

/// Verifies a downloaded archive against the signed manifest and extracts it into `directory`.
pub fn stage(
    archive: &Path,
    directory: &Path,
    manifest: &Manifest,
    target: &Path,
) -> Result<Staged, String> {
    let size = fs::metadata(archive).map_err(|_| UNREADABLE_ARCHIVE)?.len();
    if size != manifest.archive.size || sha256_file(archive)? != manifest.archive.sha256 {
        return Err(ARCHIVE_MISMATCH.into());
    }
    if let Some(parent) = directory.parent() {
        fs::create_dir_all(parent).map_err(|_| STAGING_FAILED)?;
    }
    fs::create_dir(directory).map_err(|_| STAGING_FAILED)?;
    if let Err(error) = extract(archive, directory, manifest) {
        let _ = fs::remove_dir_all(directory);
        return Err(error);
    }
    Ok(Staged {
        version: manifest.version.clone(),
        notes_url: manifest.notes_url.clone(),
        directory: directory.into(),
        target: target.into(),
        executable_sha256: manifest.executable.sha256.clone(),
    })
}

fn extract(archive: &Path, directory: &Path, manifest: &Manifest) -> Result<(), String> {
    let file = File::open(archive).map_err(|_| UNREADABLE_ARCHIVE)?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(|_| UNREADABLE_ARCHIVE)?;
    if zip.len() > 1 + NOTICES.len() {
        return Err(UNEXPECTED_ENTRY.into());
    }
    let mut seen = BTreeSet::new();
    let mut executable = None;
    let mut buffer = vec![0; 64 * 1024];
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|_| UNREADABLE_ARCHIVE)?;
        let name = entry.name().to_owned();
        if name.contains(['/', '\\', ':', '\0'])
            || name.contains("..")
            || entry.enclosed_name().is_none()
        {
            return Err(UNSAFE_PATH.into());
        }
        if !entry.is_file()
            || !(name == EXECUTABLE || NOTICES.contains(&name.as_str()))
            || !seen.insert(name.clone())
        {
            return Err(UNEXPECTED_ENTRY.into());
        }
        let limit = if name == EXECUTABLE {
            ARCHIVE_LIMIT
        } else {
            NOTICE_LIMIT
        };
        if entry.size() > limit {
            return Err(UNEXPECTED_ENTRY.into());
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(&name))
            .map_err(|_| STAGING_FAILED)?;
        let mut hasher = Sha256::new();
        let mut written = 0_u64;
        loop {
            let read = entry.read(&mut buffer).map_err(|_| UNREADABLE_ARCHIVE)?;
            if read == 0 {
                break;
            }
            written += read as u64;
            if written > limit {
                return Err(UNEXPECTED_ENTRY.into());
            }
            hasher.update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .map_err(|_| STAGING_FAILED)?;
        }
        output.sync_all().map_err(|_| STAGING_FAILED)?;
        if name == EXECUTABLE {
            executable = Some(format!("{:x}", hasher.finalize()));
        }
    }
    match executable {
        None => Err(MISSING_EXECUTABLE.into()),
        Some(hash) if hash != manifest.executable.sha256 => Err(EXECUTABLE_MISMATCH.into()),
        Some(_) => Ok(()),
    }
}

/// `Tomlook.exe` becomes `Tomlook.exe.old` while the new executable takes its place.
pub fn old_executable(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| EXECUTABLE.into());
    name.push(".old");
    target.with_file_name(name)
}

fn missing(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

/// Replaces the (possibly running) executable with a verified staged update; rolls back on failure.
pub fn apply(staged: &Staged) -> Result<(), String> {
    let source = staged.directory.join(EXECUTABLE);
    if sha256_file(&source).ok().as_deref() != Some(staged.executable_sha256.as_str()) {
        return Err(CHANGED.into());
    }
    let target = &staged.target;
    let directory = target.parent().ok_or(STAGING_FAILED)?;
    if sha256_file(target).ok().as_deref() == Some(staged.executable_sha256.as_str()) {
        let _ = fs::remove_dir_all(&staged.directory);
        return Ok(());
    }
    let old = old_executable(target);
    match fs::remove_file(&old) {
        Ok(()) => {}
        Err(error) if missing(&error) => {}
        Err(_) => return Err(IN_USE.into()),
    }
    fs::rename(target, &old)
        .map_err(|error| format!("Could not move the current Tomlook.exe aside: {error}"))?;
    if let Err(error) = fs::rename(&source, target) {
        return Err(rolled_back(
            format!("Could not move the new Tomlook.exe into place: {error}"),
            rollback(target, &old, None, &[]),
        ));
    }
    let mut replaced: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    for name in NOTICES {
        let update = staged.directory.join(name);
        if !update.is_file() {
            continue;
        }
        let destination = directory.join(name);
        let result = match fs::read(&destination) {
            Ok(previous) => Ok(Some(previous)),
            Err(error) if missing(&error) => Ok(None),
            Err(error) => Err(error),
        }
        .and_then(|previous| {
            replaced.push((destination.clone(), previous));
            fs::copy(&update, &destination)
        });
        if let Err(error) = result {
            return Err(rolled_back(
                format!("Could not update {name}: {error}"),
                rollback(target, &old, Some(&source), &replaced),
            ));
        }
    }
    let _ = fs::remove_dir_all(&staged.directory);
    if let Some(root) = staged.directory.parent() {
        let _ = fs::remove_dir(root);
    }
    Ok(())
}

fn rollback(
    target: &Path,
    old: &Path,
    source: Option<&Path>,
    replaced: &[(PathBuf, Option<Vec<u8>>)],
) -> Result<(), String> {
    let mut problems = Vec::new();
    for (destination, previous) in replaced.iter().rev() {
        let result = match previous {
            Some(bytes) => fs::write(destination, bytes),
            None => fs::remove_file(destination)
                .or_else(|error| if missing(&error) { Ok(()) } else { Err(error) }),
        };
        if let Err(error) = result {
            problems.push(format!("{}: {error}", destination.display()));
        }
    }
    if let Some(source) = source
        && let Err(error) = fs::rename(target, source)
    {
        problems.push(format!("staged executable: {error}"));
    }
    if let Err(error) = fs::rename(old, target) {
        problems.push(format!("previous executable: {error}"));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn rolled_back(error: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => format!("{error}; the previous version was restored."),
        Err(problem) => format!(
            "{error}; restoring the previous version also failed ({problem}). Download Tomlook again from the release page."
        ),
    }
}

/// Best-effort startup cleanup of a replaced executable and stale staging data.
pub fn cleanup(executable: &Path, current: &semver::Version) {
    let _ = fs::remove_file(old_executable(executable));
    let Some(root) = executable.parent().map(|directory| directory.join(STAGING)) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
        let pending = directory
            && entry
                .file_name()
                .to_str()
                .and_then(|name| semver::Version::parse(name).ok())
                .is_some_and(|version| version > *current)
            && entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age < DAY);
        if !pending {
            let _ = if directory {
                fs::remove_dir_all(entry.path())
            } else {
                fs::remove_file(entry.path())
            };
        }
    }
    let _ = fs::remove_dir(&root);
}

/// Command-line arguments for a relaunch, without a previous `--after-update` handover.
pub fn relaunch_arguments(arguments: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut arguments = arguments.into_iter();
    let mut kept = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == AFTER_UPDATE {
            arguments.next();
        } else {
            kept.push(argument);
        }
    }
    kept
}

/// Starts the replaced executable, which waits for this process before taking the profile.
pub fn relaunch(executable: &Path, arguments: Vec<OsString>) -> Result<(), String> {
    std::process::Command::new(executable)
        .args(arguments)
        .arg(AFTER_UPDATE)
        .arg(std::process::id().to_string())
        .spawn()
        .map(drop)
        .map_err(|error| format!("Could not restart Tomlook: {error}"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activity {
    Checking,
    Downloading {
        version: String,
        received: u64,
        total: u64,
    },
    Verifying {
        version: String,
    },
}

/// A verified newer release that cannot be installed automatically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub notes_url: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub revision: u64,
    pub disabled: Option<String>,
    pub activity: Option<Activity>,
    /// Last successful check.
    pub checked_at: Option<DateTime<Utc>>,
    pub latest: Option<String>,
    pub offer: Option<Offer>,
    pub staged: Option<Staged>,
    pub error: Option<String>,
}

enum Command {
    Configure {
        automatic: bool,
        checked_at: Option<DateTime<Utc>>,
    },
    Check,
}

#[derive(Default)]
struct Shared {
    status: Mutex<Status>,
    stop: AtomicBool,
    restart: AtomicBool,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Background update checker; network and disk work never run on the UI thread.
pub struct Updater {
    commands: mpsc::SyncSender<Command>,
    shared: Arc<Shared>,
}

/// Lets the process apply a staged update after the window and workers have stopped.
#[derive(Clone)]
pub struct ExitHandle(Arc<Shared>);

impl ExitHandle {
    /// Stops further update work and returns the verified update plus whether a restart was requested.
    pub fn finish(&self) -> Option<(Staged, bool)> {
        self.0.stop.store(true, Ordering::Release);
        let staged = self.0.lock().staged.clone()?;
        Some((staged, self.0.restart.load(Ordering::Acquire)))
    }
}

impl Updater {
    pub fn start(executable: PathBuf, notify: impl Fn() + Send + 'static) -> Result<Self, String> {
        let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map_err(|error| format!("Tomlook version is invalid: {error}"))?;
        let (commands, inbox) = mpsc::sync_channel(8);
        let shared = Arc::new(Shared::default());
        let worker = Worker {
            shared: shared.clone(),
            notify: Box::new(notify),
            executable,
            current,
        };
        std::thread::Builder::new()
            .name("tomlook-updater".into())
            .spawn(move || worker.run(inbox))
            .map_err(|error| format!("Start the update checker: {error}"))?;
        Ok(Self { commands, shared })
    }

    /// Returns the current status when it differs from revision `seen`; never blocks.
    pub fn status(&self, seen: u64) -> Option<Status> {
        let status = self.shared.status.try_lock().ok()?;
        (status.revision != seen).then(|| status.clone())
    }

    pub fn configure(
        &self,
        automatic: bool,
        checked_at: Option<DateTime<Utc>>,
    ) -> Result<(), String> {
        self.send(Command::Configure {
            automatic,
            checked_at,
        })
    }

    pub fn check_now(&self) -> Result<(), String> {
        self.send(Command::Check)
    }

    pub fn request_restart(&self) {
        self.shared.restart.store(true, Ordering::Release);
    }

    pub fn exit_handle(&self) -> ExitHandle {
        ExitHandle(self.shared.clone())
    }

    fn send(&self, command: Command) -> Result<(), String> {
        match self.commands.try_send(command) {
            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                Err("The update checker is busy; try again in a moment.".into())
            }
        }
    }
}

impl Drop for Updater {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

enum Discovery {
    UpToDate(Option<String>),
    Available {
        manifest: Box<Manifest>,
        archive_url: String,
    },
}

struct Failure {
    message: String,
    retry: Duration,
}

impl Failure {
    fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry: RETRY,
        }
    }

    fn rejected(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry: DAY,
        }
    }
}

struct Worker {
    shared: Arc<Shared>,
    notify: Box<dyn Fn() + Send>,
    executable: PathBuf,
    current: semver::Version,
}

impl Worker {
    fn update(&self, change: impl FnOnce(&mut Status)) {
        {
            let mut status = self.shared.lock();
            change(&mut status);
            status.revision += 1;
        }
        (self.notify)();
    }

    fn stopping(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    fn run(self, inbox: mpsc::Receiver<Command>) {
        if development_build(&self.executable) {
            self.update(|status| status.disabled = Some(DEVELOPMENT.into()));
            return;
        }
        cleanup(&self.executable, &self.current);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let (Ok(runtime), Ok(client)) = (runtime, client()) else {
            self.update(|status| status.disabled = Some(UNAVAILABLE.into()));
            return;
        };
        let started = Instant::now();
        let mut automatic = false;
        let mut due: Option<Instant> = None;
        loop {
            let received = match due {
                Some(due) => inbox.recv_timeout(due.saturating_duration_since(Instant::now())),
                None => inbox
                    .recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            };
            if self.stopping() {
                return;
            }
            match received {
                Ok(Command::Configure {
                    automatic: enabled,
                    checked_at,
                }) => {
                    automatic = enabled;
                    due = enabled.then(|| {
                        Instant::now()
                            + first_check_delay(started.elapsed(), checked_at, Utc::now())
                    });
                    continue;
                }
                Ok(Command::Check) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            let retry = self.check(&runtime, &client);
            due = automatic.then(|| Instant::now() + retry);
        }
    }

    fn check(&self, runtime: &tokio::runtime::Runtime, client: &reqwest::Client) -> Duration {
        self.update(|status| {
            status.activity = Some(Activity::Checking);
            status.error = None;
        });
        let (manifest, archive_url) = match runtime.block_on(discover(client, &self.current)) {
            Err(failure) => {
                self.update(|status| {
                    status.activity = None;
                    status.error = Some(failure.message);
                });
                return failure.retry;
            }
            Ok(Discovery::UpToDate(latest)) => {
                self.update(|status| {
                    status.activity = None;
                    status.checked_at = Some(Utc::now());
                    status.latest = latest;
                    status.offer = None;
                });
                return DAY;
            }
            Ok(Discovery::Available {
                manifest,
                archive_url,
            }) => (manifest, archive_url),
        };
        let version = manifest.version.clone();
        let staged = self.shared.lock().staged.clone();
        if staged
            .is_some_and(|staged| is_newer(&version, &staged.version).is_ok_and(|newer| !newer))
        {
            self.update(|status| {
                status.activity = None;
                status.checked_at = Some(Utc::now());
                status.latest = Some(version);
            });
            return DAY;
        }
        let root = match staging_root(&self.executable) {
            Ok(root) => root,
            Err(reason) => {
                self.update(|status| {
                    status.activity = None;
                    status.checked_at = Some(Utc::now());
                    status.latest = Some(version.clone());
                    status.offer = Some(Offer {
                        version,
                        notes_url: manifest.notes_url.clone(),
                        reason,
                    });
                });
                return DAY;
            }
        };
        self.update(|status| {
            status.checked_at = Some(Utc::now());
            status.latest = Some(version.clone());
            status.offer = None;
            status.activity = Some(Activity::Downloading {
                version: version.clone(),
                received: 0,
                total: manifest.archive.size,
            });
        });
        let result = runtime.block_on(self.download(client, &archive_url, &manifest, &root));
        let retry = result.as_ref().err().map_or(DAY, |failure| failure.retry);
        self.update(|status| {
            status.activity = None;
            match result {
                Ok(staged) => status.staged = Some(staged),
                Err(failure) => status.error = Some(failure.message),
            }
        });
        retry
    }

    async fn download(
        &self,
        client: &reqwest::Client,
        url: &str,
        manifest: &Manifest,
        root: &Path,
    ) -> Result<Staged, Failure> {
        let directory = root.join(&manifest.version);
        let partial = root.join(format!("{}.zip.part", manifest.version));
        let _ = fs::remove_dir_all(&directory);
        let _ = fs::remove_file(&partial);
        let result = async {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&partial)
                .map_err(|_| Failure::transient(STAGING_FAILED))?;
            let mut response =
                send(client, url, "application/octet-stream", ARCHIVE_TIMEOUT).await?;
            let total = manifest.archive.size;
            if response
                .content_length()
                .is_some_and(|length| length != total)
            {
                return Err(Failure::rejected(ARCHIVE_MISMATCH));
            }
            let mut received = 0_u64;
            let mut reported = 0_u64;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| Failure::transient(NETWORK))?
            {
                if self.stopping() {
                    return Err(Failure::transient(STOPPED));
                }
                received += chunk.len() as u64;
                if received > total {
                    return Err(Failure::rejected(ARCHIVE_MISMATCH));
                }
                file.write_all(&chunk)
                    .map_err(|_| Failure::transient(STAGING_FAILED))?;
                if received - reported >= 1024 * 1024 {
                    reported = received;
                    self.update(|status| {
                        status.activity = Some(Activity::Downloading {
                            version: manifest.version.clone(),
                            received,
                            total,
                        })
                    });
                }
            }
            file.sync_all()
                .map_err(|_| Failure::transient(STAGING_FAILED))?;
            drop(file);
            self.update(|status| {
                status.activity = Some(Activity::Verifying {
                    version: manifest.version.clone(),
                })
            });
            stage(&partial, &directory, manifest, &self.executable).map_err(Failure::rejected)
        }
        .await;
        let _ = fs::remove_file(&partial);
        if result.is_err() {
            let _ = fs::remove_dir_all(&directory);
        }
        result
    }
}

fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(format!("Tomlook/{}", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let trusted = attempt.url().scheme() == "https"
                && attempt.url().host_str().is_some_and(|host| {
                    host == "github.com"
                        || host == "api.github.com"
                        || host.ends_with(".githubusercontent.com")
                });
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if trusted {
                attempt.follow()
            } else {
                attempt.error("redirect left GitHub")
            }
        }))
        .build()
}

async fn send(
    client: &reqwest::Client,
    url: &str,
    accept: &str,
    timeout: Duration,
) -> Result<reqwest::Response, Failure> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, accept)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(timeout)
        .send()
        .await
        .map_err(|_| Failure::transient(NETWORK))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let exhausted = response
        .headers()
        .get("x-ratelimit-remaining")
        .is_some_and(|remaining| remaining == "0");
    Err(match status.as_u16() {
        429 => Failure::transient(RATE_LIMITED),
        403 if exhausted => Failure::transient(RATE_LIMITED),
        404 => Failure::rejected(NO_RELEASE),
        code => Failure::transient(format!("GitHub returned HTTP {code}.")),
    })
}

async fn fetch(
    client: &reqwest::Client,
    url: &str,
    accept: &str,
    limit: usize,
) -> Result<Vec<u8>, Failure> {
    let mut response = send(client, url, accept, API_TIMEOUT).await?;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Failure::rejected(OVERSIZED));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Failure::transient(NETWORK))?
    {
        if body.len() + chunk.len() > limit {
            return Err(Failure::rejected(OVERSIZED));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn discover(
    client: &reqwest::Client,
    current: &semver::Version,
) -> Result<Discovery, Failure> {
    let body = fetch(client, LATEST, "application/vnd.github+json", RELEASE_LIMIT).await?;
    let Some(release) = parse_release(&body).map_err(Failure::rejected)? else {
        return Ok(Discovery::UpToDate(None));
    };
    if release.version <= *current {
        return Ok(Discovery::UpToDate(Some(release.version.to_string())));
    }
    let (manifest_url, _) = release.asset(MANIFEST_NAME).map_err(Failure::rejected)?;
    let (signature_url, _) = release.asset(SIGNATURE_NAME).map_err(Failure::rejected)?;
    let manifest = fetch(
        client,
        &manifest_url,
        "application/octet-stream",
        MANIFEST_LIMIT,
    )
    .await?;
    let signature = fetch(
        client,
        &signature_url,
        "application/octet-stream",
        MANIFEST_LIMIT,
    )
    .await?;
    let manifest =
        verify_manifest(&manifest, &signature, TRUSTED_KEYS).map_err(Failure::rejected)?;
    if manifest.tag != release.tag
        || !is_newer(&manifest.version, &current.to_string()).unwrap_or(false)
    {
        return Err(Failure::rejected(MISMATCHED_RELEASE));
    }
    let (archive_url, size) = release
        .asset(&manifest.archive.name)
        .map_err(Failure::rejected)?;
    if size != manifest.archive.size {
        return Err(Failure::rejected(ARCHIVE_MISMATCH));
    }
    Ok(Discovery::Available {
        manifest: Box::new(manifest),
        archive_url,
    })
}

/// Release-signing key management for `release-sign`; private key material never leaves DPAPI protection on disk.
pub mod signing {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const BINDING: &str = "tomlook-release-signing-key-v1";
    const KEY_LIMIT: u64 = 16 * 1024;

    fn protect(data: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
        crate::credentials::protect(data, BINDING, encrypt).map_err(|_| {
            "Windows DPAPI could not protect or unlock the release signing key for this Windows user"
                .to_string()
        })
    }

    fn load(path: &Path) -> Result<Ed25519KeyPair, String> {
        let metadata = fs::metadata(path).map_err(|error| {
            format!(
                "Release signing key {} is unavailable: {error}",
                path.display()
            )
        })?;
        if metadata.len() > KEY_LIMIT {
            return Err("Release signing key file is too large".into());
        }
        let protected = fs::read(path)
            .map_err(|error| format!("Read release signing key {}: {error}", path.display()))?;
        let mut pkcs8 = protect(&protected, false)?;
        let pair = Ed25519KeyPair::from_pkcs8(&pkcs8)
            .map_err(|_| "Release signing key is not a valid Ed25519 key".to_string());
        pkcs8.fill(0);
        pair
    }

    fn encode(pair: &Ed25519KeyPair) -> String {
        STANDARD.encode(pair.public_key().as_ref())
    }

    /// Creates a new DPAPI-protected key; refuses to replace an existing file.
    pub fn generate(path: &Path) -> Result<String, String> {
        if path.exists() {
            return Err(format!(
                "Release signing key {} already exists; it was not replaced",
                path.display()
            ));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Create {}: {error}", parent.display()))?;
        }
        let random = ring::rand::SystemRandom::new();
        let protected = {
            let document = Ed25519KeyPair::generate_pkcs8(&random)
                .map_err(|_| "Generate an Ed25519 release signing key failed".to_string())?;
            protect(document.as_ref(), true)?
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| format!("Create {}: {error}", path.display()))?;
        if let Err(error) = file.write_all(&protected).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(format!("Write {}: {error}", path.display()));
        }
        public_key(path)
    }

    pub fn public_key(path: &Path) -> Result<String, String> {
        load(path).map(|pair| encode(&pair))
    }

    /// Signs the exact bytes of a valid manifest; refuses to replace an existing signature.
    pub fn sign(key: &Path, input: &Path, output: &Path) -> Result<String, String> {
        let manifest =
            fs::read(input).map_err(|error| format!("Read {}: {error}", input.display()))?;
        if manifest.len() > MANIFEST_LIMIT {
            return Err("Manifest exceeds the 64 KiB update limit".into());
        }
        serde_json::from_slice::<Manifest>(&manifest)
            .map_err(|error| format!("Manifest is not readable: {error}"))?
            .validate()?;
        let pair = load(key)?;
        let signature = STANDARD.encode(pair.sign(&manifest).as_ref());
        verify_signature(&manifest, signature.as_bytes(), &[&encode(&pair)])?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|error| format!("Create {}: {error}", output.display()))?;
        file.write_all(format!("{signature}\n").as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("Write {}: {error}", output.display()))?;
        Ok(encode(&pair))
    }
}
