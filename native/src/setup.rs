//! Setup checklist and first-run guidance. Every item is derived from the existing connection,
//! notification and update states; deriving it never starts a sign-in, a check or a network call.

use crate::calendar_sync::{Progress, Window};
use chrono::{DateTime, Local, NaiveDate, Utc};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Official installation guide of the GitHub Copilot CLI runtime that live AI requires.
pub const COPILOT_CLI_INSTALL_URL: &str =
    "https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli";

/// Shown instead of a status in the offline demo, which never connects anything.
pub const DEMO: &str = "Not available in the offline demo.";

/// Largest resource section read to find an executable's version.
const RESOURCE_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Step {
    Calendar,
    Copilot,
    Insights,
    Notifications,
    Updates,
    Webiq,
    Github,
    Foundry,
    Msx,
    Runtime,
}

impl Step {
    pub const ALL: [Self; 10] = [
        Self::Calendar,
        Self::Copilot,
        Self::Insights,
        Self::Notifications,
        Self::Updates,
        Self::Webiq,
        Self::Github,
        Self::Foundry,
        Self::Msx,
        Self::Runtime,
    ];

    pub fn required(self) -> bool {
        matches!(
            self,
            Self::Calendar | Self::Copilot | Self::Insights | Self::Notifications | Self::Updates
        )
    }

    pub fn optional(self) -> bool {
        matches!(self, Self::Webiq | Self::Github | Self::Foundry | Self::Msx)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Calendar => "Microsoft 365 calendar (WorkIQ)",
            Self::Copilot => "GitHub Copilot",
            Self::Insights => "Identity match",
            Self::Notifications => "Windows notifications",
            Self::Updates => "Updates",
            Self::Webiq => "WebIQ",
            Self::Github => "GitHub connector",
            Self::Foundry => "Foundry / Auto mode",
            Self::Msx => "MSX and Azure consumption",
            Self::Runtime => "Runtime prerequisites",
        }
    }

    /// One line about what the step enables.
    pub fn purpose(self) -> &'static str {
        match self {
            Self::Calendar => {
                "Loads your calendar, people and availability; keeps one month back and six months ahead in sync."
            }
            Self::Copilot => {
                "Creates Insights, answers and meeting notes. Live AI needs an installed GitHub Copilot CLI runtime."
            }
            Self::Insights => {
                "Automatic Insights require matching connected identities and verified calendar coverage."
            }
            Self::Notifications => {
                "Meeting reminders, My day and the weekly review appear as Windows notifications."
            }
            Self::Updates => {
                "Checks GitHub Releases once a day and installs only signed Tomlook releases."
            }
            Self::Webiq => "Public web search for meeting and customer research.",
            Self::Github => "Read-only access to selected repositories for research.",
            Self::Foundry => {
                "Microsoft-Decision-1 chooses Quick or Thorough in Auto mode, using your Azure CLI sign-in."
            }
            Self::Msx => {
                "Read-only customer context from MSX (Dataverse) and Azure consumption (Power BI)."
            }
            Self::Runtime => "What Tomlook itself needs on this computer.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Done,
    /// A required step needs the user.
    Action,
    /// An optional connection is not set up.
    NotSet,
    Checking,
    Error,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Done => "Done",
            Self::Action => "Action needed",
            Self::NotSet => "Not set up",
            Self::Checking => "Checking...",
            Self::Error => "Error",
        }
    }
}

/// What an item's button does. Every action opens or calls an existing flow; none signs in
/// without the click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The WorkIQ connection dialog, where the work e-mail is entered.
    SignInWorkiq,
    OpenWorkiq,
    RetrySync,
    /// The AI dialog's isolated browser sign-in.
    SignInCopilot,
    OpenAi,
    InstallCopilot,
    OpenInsights,
    TestNotification,
    EnableUpdates,
    CheckUpdates,
    EnterWebiqKey,
    ConnectGithub,
    OpenFoundry,
    OpenMsx,
    OpenMsxSettings,
    CheckAgain,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::SignInWorkiq | Self::SignInCopilot => "Sign in",
            Self::OpenWorkiq => "Open WorkIQ connection",
            Self::RetrySync => "Retry sync",
            Self::OpenAi => "Open AI connection",
            Self::InstallCopilot => "Install Copilot CLI",
            Self::OpenInsights => "Open Insights activity",
            Self::TestNotification => "Send test notification",
            Self::EnableUpdates => "Turn on automatic checks",
            Self::CheckUpdates => "Check now",
            Self::EnterWebiqKey => "Enter key",
            Self::ConnectGithub => "Connect",
            Self::OpenFoundry => "Open Foundry connection",
            Self::OpenMsx => "Open MSX connection",
            Self::OpenMsxSettings => "Open MSX settings",
            Self::CheckAgain => "Check again",
        }
    }
}

/// One line of an item's details: a catalog label and the honest current value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail {
    pub label: &'static str,
    pub value: String,
}

impl Detail {
    fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub step: Step,
    pub state: State,
    pub summary: String,
    pub details: Vec<Detail>,
    pub action: Option<Action>,
}

impl Item {
    fn new(
        step: Step,
        state: State,
        summary: impl Into<String>,
        details: Vec<Detail>,
        action: Option<Action>,
    ) -> Self {
        Self {
            step,
            state,
            summary: summary.into(),
            details,
            action,
        }
    }
}

/// A connection's state as the status bar shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Connection<'a> {
    pub busy: bool,
    pub ready: bool,
    pub message: &'a str,
}

/// The Copilot runtime configured in the profile's `connections.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub path: PathBuf,
    pub version: Option<String>,
}

impl Runtime {
    pub fn label(&self) -> String {
        match &self.version {
            Some(version) => format!("{} (version {version})", self.path.display()),
            None => self.path.display().to_string(),
        }
    }
}

/// Filesystem facts gathered once off the UI thread and again after **Check again**.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// The machine-wide Azure CLI used by Foundry and MSX sign-ins.
    pub azure_cli: Option<PathBuf>,
    /// The configured Copilot runtime, or why it cannot be used.
    pub copilot: Result<Runtime, String>,
    /// Saved configuration or sign-ins from earlier use of this profile.
    pub saved_connections: bool,
}

/// Reads the profile and machine without starting anything; may block, so call it off the UI
/// thread.
pub fn probe(root: &Path) -> Probe {
    let copilot = crate::copilot::Config::load(root).map(|config| Runtime {
        version: executable_version(&config.runtime),
        path: config.runtime,
    });
    let saved_connections = ["connections.json", "decision-router.json", "copilot"]
        .iter()
        .any(|name| root.join(name).exists())
        || crate::workiq::saved_sign_in_exists(root)
        || crate::connectors::saved_sign_in_exists(root);
    Probe {
        azure_cli: crate::azure_cli::installed(),
        copilot,
        saved_connections,
    }
}

/// The product version in a Windows executable's version resource, when it has one.
pub fn executable_version(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut read = |offset: u64, length: usize| -> Option<Vec<u8>> {
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut buffer = vec![0; length];
        file.read_exact(&mut buffer).ok()?;
        Some(buffer)
    };
    let dos = read(0, 64)?;
    if !dos.starts_with(b"MZ") {
        return None;
    }
    let header_at = u64::from(u32::from_le_bytes(dos[60..64].try_into().ok()?));
    let header = read(header_at, 24)?;
    if !header.starts_with(b"PE\0\0") {
        return None;
    }
    let sections = usize::from(u16::from_le_bytes([header[6], header[7]]));
    let optional = u64::from(u16::from_le_bytes([header[20], header[21]]));
    if sections == 0 || sections > 96 {
        return None;
    }
    let table = read(header_at + 24 + optional, sections * 40)?;
    let resources = table
        .as_chunks::<40>()
        .0
        .iter()
        .find(|section| section.starts_with(b".rsrc\0"))?;
    let size = u32::from_le_bytes(resources[16..20].try_into().ok()?) as usize;
    let offset = u64::from(u32::from_le_bytes(resources[20..24].try_into().ok()?));
    if size == 0 || size > RESOURCE_LIMIT {
        return None;
    }
    fixed_version(&read(offset, size)?)
}

/// Reads `VS_FIXEDFILEINFO` after the `VS_VERSION_INFO` key of a resource section.
fn fixed_version(resources: &[u8]) -> Option<String> {
    let key: Vec<u8> = "VS_VERSION_INFO"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let start = resources
        .windows(key.len())
        .position(|window| window == key.as_slice())?;
    let tail = resources.get(start + key.len()..)?;
    let signature = 0xFEEF_04BD_u32.to_le_bytes();
    // Only the key's terminator and 32-bit alignment padding precede the signature.
    let at = tail
        .get(..tail.len().min(16))?
        .windows(4)
        .position(|window| window == signature)?;
    let info = tail.get(at..at + 24)?;
    let word = |index: usize| {
        u32::from_le_bytes([
            info[index],
            info[index + 1],
            info[index + 2],
            info[index + 3],
        ])
    };
    let (high, low) = (word(16), word(20));
    let parts = [high >> 16, high & 0xffff, low >> 16, low & 0xffff];
    if parts == [0; 4] {
        return None;
    }
    Some(if parts[3] == 0 {
        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    } else {
        format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3])
    })
}

/// Everything the checklist reads, borrowed from the states the status bar already shows.
pub struct Inputs<'a> {
    pub demo: bool,
    pub today: NaiveDate,
    pub workiq: Connection<'a>,
    /// The connected WorkIQ sign-in, or the saved work e-mail while disconnected.
    pub workiq_account: Option<&'a str>,
    pub calendar_owner: Option<&'a str>,
    /// Days with synchronized calendar data.
    pub coverage: &'a BTreeSet<NaiveDate>,
    pub sync: &'a Progress,
    pub copilot: Connection<'a>,
    pub prepare_enabled: bool,
    pub paused: bool,
    /// The reminder thread runs; a test notification can be sent.
    pub reminders_running: bool,
    pub reminders: &'a crate::reminders::Status,
    pub updates: &'a crate::updater::Status,
    pub automatic_updates: bool,
    /// Saved result of the last check, used before the updater reports.
    pub update_checked_at: Option<DateTime<Utc>>,
    pub latest_version: &'a str,
    pub webiq: Connection<'a>,
    pub github: Connection<'a>,
    pub foundry: &'a crate::decision::Status,
    pub msx_enabled: bool,
    pub msx: &'a crate::msx::Status,
    /// `None` until the first probe finishes.
    pub probe: Option<&'a Probe>,
}

/// The ordered checklist: required steps, optional connections, then runtime prerequisites.
pub fn checklist(inputs: &Inputs) -> Vec<Item> {
    if inputs.demo {
        return Step::ALL
            .into_iter()
            .map(|step| Item::new(step, State::NotSet, DEMO, Vec::new(), None))
            .collect();
    }
    vec![
        calendar(inputs),
        copilot(inputs),
        insights(inputs),
        notifications(inputs),
        updates(inputs),
        external(Step::Webiq, inputs.webiq, Action::EnterWebiqKey),
        external(Step::Github, inputs.github, Action::ConnectGithub),
        foundry(inputs),
        msx(inputs),
        runtime(inputs),
    ]
}

fn local(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local)
        .format("%d.%m.%Y %H:%M")
        .to_string()
}

fn progress(message: &str) -> &str {
    if message.is_empty() {
        "Checking..."
    } else {
        message
    }
}

fn calendar(inputs: &Inputs) -> Item {
    let step = Step::Calendar;
    let link = inputs.workiq;
    let mut details = vec![Detail::new(
        "Status",
        progress_or(link.message, "Not connected"),
    )];
    if let Some(account) = inputs.workiq_account {
        details.push(Detail::new(
            if link.ready {
                "Signed-in account"
            } else {
                "Work email"
            },
            account,
        ));
    }
    if link.busy {
        return Item::new(step, State::Checking, progress(link.message), details, None);
    }
    if !link.ready {
        return Item::new(
            step,
            State::Action,
            "Not connected",
            details,
            Some(Action::SignInWorkiq),
        );
    }
    if let Some(owner) = inputs.calendar_owner {
        details.push(Detail::new("Calendar owner", owner));
    }
    let window = match Window::new(inputs.today) {
        Ok(window) => window,
        Err(error) => {
            details.push(Detail::new("Error", error));
            return Item::new(
                step,
                State::Error,
                "Calendar sync failed; retrying automatically",
                details,
                Some(Action::RetrySync),
            );
        }
    };
    let total = (window.last - window.first).num_days().max(0) as usize + 1;
    let synced = inputs.coverage.range(window.first..=window.last).count();
    details.push(Detail::new(
        "Calendar sync",
        format!("{synced} of {total} days"),
    ));
    if let Some(updated) = inputs.sync.updated {
        details.push(Detail::new("Last saved refresh", local(updated)));
    }
    if let Some(error) = &inputs.sync.error {
        details.push(Detail::new("Error", error));
        Item::new(
            step,
            State::Error,
            "Calendar sync failed; retrying automatically",
            details,
            Some(Action::RetrySync),
        )
    } else if synced >= total {
        Item::new(
            step,
            State::Done,
            "Synced: 1 month back – 6 months ahead",
            details,
            None,
        )
    } else {
        let summary = format!(
            "Synced {synced} of {total} days; {} not synced yet",
            total - synced
        );
        Item::new(step, State::Checking, summary, details, None)
    }
}

fn progress_or<'a>(message: &'a str, fallback: &'a str) -> &'a str {
    if message.is_empty() {
        fallback
    } else {
        message
    }
}

fn copilot(inputs: &Inputs) -> Item {
    let step = Step::Copilot;
    let link = inputs.copilot;
    let runtime = inputs.probe.map(|probe| &probe.copilot);
    let mut details = vec![Detail::new(
        "Status",
        progress_or(link.message, "Not connected"),
    )];
    details.push(Detail::new(
        "Copilot runtime",
        match runtime {
            Some(Ok(runtime)) => runtime.label(),
            Some(Err(error)) => error.clone(),
            None => "Checking...".into(),
        },
    ));
    if link.ready {
        Item::new(
            step,
            State::Done,
            "Signed in to the isolated Copilot runtime",
            details,
            None,
        )
    } else if link.busy {
        Item::new(step, State::Checking, progress(link.message), details, None)
    } else if matches!(runtime, Some(Err(_))) {
        Item::new(
            step,
            State::Action,
            "Copilot runtime is not set up",
            details,
            Some(Action::InstallCopilot),
        )
    } else {
        Item::new(
            step,
            State::Action,
            "Not signed in",
            details,
            Some(Action::SignInCopilot),
        )
    }
}

fn insights(inputs: &Inputs) -> Item {
    use crate::scheduler::Blocker;
    let readiness = crate::scheduler::Readiness {
        enabled: inputs.prepare_enabled,
        paused: inputs.paused,
        ai_ready: inputs.copilot.ready,
        workiq_ready: inputs.workiq.ready,
        workiq_account: inputs.workiq_account.filter(|_| inputs.workiq.ready),
        calendar_owner: inputs.calendar_owner,
        coverage_verified: inputs.coverage.contains(&inputs.today),
    };
    let mut details = vec![
        Detail::new(
            "GitHub Copilot",
            if inputs.copilot.ready {
                "Connected"
            } else {
                "Not connected"
            },
        ),
        Detail::new(
            "WorkIQ account",
            readiness.workiq_account.unwrap_or("Not connected"),
        ),
        Detail::new(
            "Calendar owner",
            inputs.calendar_owner.unwrap_or("Not synced yet"),
        ),
    ];
    let Some(blocker) = readiness.blocker() else {
        return Item::new(
            Step::Insights,
            State::Done,
            "Automatic Insights can run",
            details,
            None,
        );
    };
    details.push(Detail::new("Reason", blocker.message()));
    let waiting = inputs.copilot.busy
        || inputs.workiq.busy
        || (blocker == Blocker::Coverage && inputs.sync.error.is_none());
    let action = match blocker {
        Blocker::Disabled | Blocker::Paused => Action::OpenInsights,
        Blocker::Ai => Action::OpenAi,
        Blocker::Workiq | Blocker::ForeignCalendar => Action::OpenWorkiq,
        Blocker::Coverage => Action::RetrySync,
    };
    Item::new(
        Step::Insights,
        if waiting {
            State::Checking
        } else {
            State::Action
        },
        "Automatic Insights cannot run yet",
        details,
        (!waiting).then_some(action),
    )
}

fn notifications(inputs: &Inputs) -> Item {
    let step = Step::Notifications;
    let status = inputs.reminders;
    let test = inputs.reminders_running.then_some(Action::TestNotification);
    let mut details = Vec::new();
    if let Some(at) = status.last_test {
        details.push(Detail::new("Last test notification", local(at)));
    }
    if !inputs.reminders_running {
        if let Some(reason) = status.disabled.as_ref().or(status.error.as_ref()) {
            details.push(Detail::new("Reason", reason));
        }
        return Item::new(
            step,
            State::Error,
            "Notifications are unavailable",
            details,
            None,
        );
    }
    if let Some(error) = &status.error {
        details.push(Detail::new("Error", error));
        return Item::new(
            step,
            State::Error,
            "Windows reported a notification error",
            details,
            test,
        );
    }
    match status.windows {
        None => Item::new(
            step,
            State::Checking,
            "Checking the Windows notification setting...",
            details,
            None,
        ),
        Some(setting) => match setting.message() {
            None => Item::new(
                step,
                State::Done,
                "Windows notifications are on for Tomlook.",
                details,
                test,
            ),
            Some(message) => Item::new(step, State::Action, message, details, test),
        },
    }
}

fn updates(inputs: &Inputs) -> Item {
    let step = Step::Updates;
    let status = inputs.updates;
    let mut details = vec![
        Detail::new("Current version", env!("CARGO_PKG_VERSION")),
        Detail::new(
            "Last check",
            status
                .checked_at
                .or(inputs.update_checked_at)
                .map(local)
                .unwrap_or_else(|| "Not checked yet.".into()),
        ),
    ];
    let latest = status
        .latest
        .as_deref()
        .or(Some(inputs.latest_version))
        .filter(|latest| !latest.is_empty());
    if let Some(staged) = &status.staged {
        details.push(Detail::new(
            "Latest release",
            format!("Tomlook {} is downloaded and verified.", staged.version),
        ));
    } else if let Some(offer) = &status.offer {
        details.push(Detail::new(
            "Latest release",
            format!("Tomlook {} is available.", offer.version),
        ));
    } else if latest.is_some_and(|latest| {
        !crate::updater::is_newer(latest, env!("CARGO_PKG_VERSION")).unwrap_or(true)
    }) {
        details.push(Detail::new("Latest release", "Tomlook is up to date."));
    }
    if let Some(reason) = &status.disabled {
        return Item::new(step, State::Error, reason.clone(), details, None);
    }
    if status.activity.is_some() {
        return Item::new(
            step,
            State::Checking,
            "Checking GitHub Releases...",
            details,
            None,
        );
    }
    if let Some(error) = &status.error {
        details.push(Detail::new("Error", error));
        return Item::new(
            step,
            State::Error,
            "The last update check failed",
            details,
            Some(Action::CheckUpdates),
        );
    }
    if inputs.automatic_updates {
        Item::new(
            step,
            State::Done,
            "Automatic update checks are on",
            details,
            None,
        )
    } else {
        Item::new(
            step,
            State::Action,
            "Automatic update checks are off",
            details,
            Some(Action::EnableUpdates),
        )
    }
}

fn external(step: Step, link: Connection, action: Action) -> Item {
    let details = if link.message.is_empty() {
        Vec::new()
    } else {
        vec![Detail::new("Status", link.message)]
    };
    if link.busy {
        Item::new(step, State::Checking, progress(link.message), details, None)
    } else if link.ready {
        Item::new(
            step,
            State::Done,
            progress_or(link.message, "Connected"),
            details,
            None,
        )
    } else if matches!(
        link.message,
        "" | "Not connected" | "Disconnected; saved sign-in removed"
    ) {
        Item::new(step, State::NotSet, "Not set up", details, Some(action))
    } else {
        Item::new(
            step,
            State::Error,
            "Connection failed",
            details,
            Some(action),
        )
    }
}

fn azure_cli(probe: Option<&Probe>) -> Detail {
    Detail::new(
        "Azure CLI",
        match probe {
            None => "Checking...".into(),
            Some(Probe {
                azure_cli: Some(path),
                ..
            }) => path.display().to_string(),
            Some(_) => "Not found".into(),
        },
    )
}

fn foundry(inputs: &Inputs) -> Item {
    use crate::decision::Phase;
    let status = inputs.foundry;
    let mut details = vec![azure_cli(inputs.probe)];
    if !status.message.is_empty() {
        details.push(Detail::new("Status", &status.message));
    }
    let (state, summary) = match status.phase {
        Phase::Checking => (State::Checking, "Checking..."),
        Phase::SigningIn => (State::Checking, "Signing in..."),
        Phase::SignedIn => (State::Done, "Signed in"),
        Phase::Unknown => (State::NotSet, "Not checked yet."),
        Phase::Unconfigured => (State::NotSet, "Not configured"),
        Phase::SignInRequired => (State::NotSet, "Sign-in required"),
        Phase::Failed => (State::Error, "Unavailable"),
    };
    let action = matches!(state, State::NotSet | State::Error).then_some(Action::OpenFoundry);
    Item::new(Step::Foundry, state, summary, details, action)
}

fn msx(inputs: &Inputs) -> Item {
    use crate::msx::Phase;
    let status = inputs.msx;
    let mut details = vec![azure_cli(inputs.probe)];
    if !inputs.msx_enabled || status.phase == Phase::Disabled {
        return Item::new(
            Step::Msx,
            State::NotSet,
            crate::msx::DISABLED,
            details,
            Some(Action::OpenMsxSettings),
        );
    }
    if !status.message.is_empty() {
        details.push(Detail::new("Status", &status.message));
    }
    if let Some(account) = &status.account {
        details.push(Detail::new("Signed-in Azure account", account));
    }
    let state = match status.phase {
        Phase::Connected => State::Done,
        Phase::Checking | Phase::SigningIn => State::Checking,
        Phase::Unavailable => State::Error,
        Phase::NotConnected | Phase::SignInRequired | Phase::Disabled => State::NotSet,
    };
    let action = matches!(state, State::NotSet | State::Error).then_some(Action::OpenMsx);
    Item::new(Step::Msx, state, status.phase.label(), details, action)
}

fn runtime(inputs: &Inputs) -> Item {
    let step = Step::Runtime;
    let mut details = vec![
        Detail::new("Visual C++ runtime", "Available (Tomlook is running)"),
        azure_cli(inputs.probe),
    ];
    let Some(probe) = inputs.probe else {
        return Item::new(
            step,
            State::Checking,
            "Checking prerequisites...",
            details,
            None,
        );
    };
    details.push(Detail::new(
        "Copilot runtime",
        match &probe.copilot {
            Ok(runtime) => runtime.label(),
            Err(error) => error.clone(),
        },
    ));
    let (state, summary) = match (probe.copilot.is_ok(), probe.azure_cli.is_some()) {
        (true, true) => (State::Done, "All prerequisites found"),
        (false, true) => (State::NotSet, "Copilot runtime is not set up"),
        (true, false) => (
            State::NotSet,
            "Azure CLI not found (needed only for Foundry and MSX)",
        ),
        (false, false) => (
            State::NotSet,
            "Copilot runtime is not set up and Azure CLI was not found",
        ),
    };
    Item::new(step, state, summary, details, Some(Action::CheckAgain))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub required_done: usize,
    pub required: usize,
    pub optional_done: usize,
    pub optional: usize,
    /// A required step is still being checked, so its outcome is not known yet.
    pub checking: bool,
}

impl Summary {
    pub fn complete(&self) -> bool {
        self.required_done == self.required
    }

    pub fn text(&self) -> String {
        format!(
            "{} of {} required steps done · {} optional connected",
            self.required_done, self.required, self.optional_done
        )
    }

    /// The status-bar indicator.
    pub fn badge(&self) -> String {
        format!("Setup {}/{}", self.required_done, self.required)
    }

    /// The indicator appears only while a required step is known to be incomplete.
    pub fn show_hint(&self, hidden: bool) -> bool {
        !hidden && !self.complete() && !self.checking
    }
}

pub fn summarize(items: &[Item]) -> Summary {
    let mut summary = Summary::default();
    for item in items {
        if item.step.required() {
            summary.required += 1;
            summary.required_done += usize::from(item.state == State::Done);
            summary.checking |= item.state == State::Checking;
        } else if item.step.optional() {
            summary.optional += 1;
            summary.optional_done += usize::from(item.state == State::Done);
        }
    }
    summary
}

/// The first required step the user can act on now.
pub fn next_step(items: &[Item]) -> Option<&Item> {
    items.iter().find(|item| {
        item.step.required()
            && matches!(item.state, State::Action | State::Error)
            && item.action.is_some()
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstRun {
    /// Preferences or the probe are not known yet.
    Wait,
    Nothing,
    /// A new profile: open the welcome window once.
    Show,
    /// An existing profile with saved connections: record the welcome as shown without opening it.
    MarkShown,
}

/// Decides the first-run welcome. `eligible` is a normal launch, never a demo, stress,
/// measurement or acceptance run.
pub fn first_run(
    preferences: &crate::storage::Preferences,
    eligible: bool,
    loaded: bool,
    calendar_owner: Option<&str>,
    probe: Option<&Probe>,
) -> FirstRun {
    if !eligible {
        return FirstRun::Nothing;
    }
    if !loaded {
        return FirstRun::Wait;
    }
    if preferences.welcome_shown {
        return FirstRun::Nothing;
    }
    let Some(probe) = probe else {
        return FirstRun::Wait;
    };
    if prior_setup(preferences, calendar_owner, probe) {
        FirstRun::MarkShown
    } else {
        FirstRun::Show
    }
}

/// Any saved connection of an earlier Tomlook version.
pub fn prior_setup(
    preferences: &crate::storage::Preferences,
    calendar_owner: Option<&str>,
    probe: &Probe,
) -> bool {
    !preferences.work_email.trim().is_empty()
        || !preferences.github_client_id.trim().is_empty()
        || calendar_owner.is_some()
        || probe.saved_connections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    struct Fixture {
        today: NaiveDate,
        coverage: BTreeSet<NaiveDate>,
        sync: Progress,
        reminders: crate::reminders::Status,
        updates: crate::updater::Status,
        foundry: crate::decision::Status,
        msx: crate::msx::Status,
        probe: Option<Probe>,
    }

    impl Fixture {
        /// A new profile: nothing connected, no Copilot runtime or Azure CLI.
        fn new() -> Self {
            Self {
                today: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
                coverage: BTreeSet::new(),
                sync: Progress::default(),
                reminders: crate::reminders::Status {
                    windows: Some(crate::reminders::WindowsSetting::Enabled),
                    ..Default::default()
                },
                updates: Default::default(),
                foundry: Default::default(),
                msx: crate::msx::Status {
                    phase: crate::msx::Phase::NotConnected,
                    message: crate::msx::NOT_CONNECTED.into(),
                    ..Default::default()
                },
                probe: Some(Probe {
                    azure_cli: None,
                    copilot: Err("Read Tomlook connections.json: missing".into()),
                    saved_connections: false,
                }),
            }
        }

        /// Everything connected and synchronized.
        fn connected() -> Self {
            let mut fixture = Self::new();
            let window = Window::new(fixture.today).unwrap();
            let mut day = window.first;
            while day <= window.last {
                fixture.coverage.insert(day);
                day = day.succ_opt().unwrap();
            }
            fixture.foundry = crate::decision::Status::signed_in();
            fixture.msx = crate::msx::Status {
                phase: crate::msx::Phase::Connected,
                message: crate::msx::CONNECTED.into(),
                account: Some("owner@contoso.example".into()),
                ..Default::default()
            };
            fixture.probe = Some(Probe {
                azure_cli: Some(PathBuf::from(
                    r"C:\Program Files\Microsoft SDKs\Azure\CLI2\wbin\az.cmd",
                )),
                copilot: Ok(Runtime {
                    path: PathBuf::from(r"C:\Tools\copilot.exe"),
                    version: Some("1.0.93".into()),
                }),
                saved_connections: true,
            });
            fixture
        }

        fn inputs(&self) -> Inputs<'_> {
            Inputs {
                demo: false,
                today: self.today,
                workiq: Connection {
                    busy: false,
                    ready: false,
                    message: "Not connected",
                },
                workiq_account: None,
                calendar_owner: None,
                coverage: &self.coverage,
                sync: &self.sync,
                copilot: Connection {
                    busy: false,
                    ready: false,
                    message: "Not connected",
                },
                prepare_enabled: true,
                paused: false,
                reminders_running: true,
                reminders: &self.reminders,
                updates: &self.updates,
                automatic_updates: true,
                update_checked_at: None,
                latest_version: "",
                webiq: Connection {
                    busy: false,
                    ready: false,
                    message: "Not connected",
                },
                github: Connection::default(),
                foundry: &self.foundry,
                msx_enabled: true,
                msx: &self.msx,
                probe: self.probe.as_ref(),
            }
        }

        fn connected_inputs(&self) -> Inputs<'_> {
            Inputs {
                workiq: Connection {
                    busy: false,
                    ready: true,
                    message: "Connected",
                },
                workiq_account: Some("owner@contoso.example"),
                calendar_owner: Some("owner@contoso.example"),
                copilot: Connection {
                    busy: false,
                    ready: true,
                    message: "Connected",
                },
                webiq: Connection {
                    busy: false,
                    ready: true,
                    message: "Connected",
                },
                github: Connection {
                    busy: false,
                    ready: true,
                    message: "Connected as octocat",
                },
                ..self.inputs()
            }
        }
    }

    fn item(items: &[Item], step: Step) -> &Item {
        items.iter().find(|item| item.step == step).unwrap()
    }

    fn states(items: &[Item]) -> Vec<(Step, State, Option<Action>)> {
        items
            .iter()
            .map(|item| (item.step, item.state, item.action))
            .collect()
    }

    #[test]
    fn a_new_profile_lists_every_step_in_order_with_the_action_that_reuses_its_flow() {
        let fixture = Fixture::new();
        let items = checklist(&fixture.inputs());
        assert_eq!(
            items.iter().map(|item| item.step).collect::<Vec<_>>(),
            Step::ALL
        );
        assert_eq!(
            states(&items),
            vec![
                (Step::Calendar, State::Action, Some(Action::SignInWorkiq)),
                (Step::Copilot, State::Action, Some(Action::InstallCopilot)),
                (Step::Insights, State::Action, Some(Action::OpenAi)),
                (
                    Step::Notifications,
                    State::Done,
                    Some(Action::TestNotification)
                ),
                (Step::Updates, State::Done, None),
                (Step::Webiq, State::NotSet, Some(Action::EnterWebiqKey)),
                (Step::Github, State::NotSet, Some(Action::ConnectGithub)),
                (Step::Foundry, State::NotSet, Some(Action::OpenFoundry)),
                (Step::Msx, State::NotSet, Some(Action::OpenMsx)),
                (Step::Runtime, State::NotSet, Some(Action::CheckAgain)),
            ]
        );
        assert_eq!(
            item(&items, Step::Copilot).summary,
            "Copilot runtime is not set up"
        );
        assert!(
            item(&items, Step::Copilot)
                .details
                .iter()
                .any(|detail| detail.label == "Copilot runtime"
                    && detail.value.contains("connections.json"))
        );
        let reason = item(&items, Step::Insights)
            .details
            .iter()
            .find(|detail| detail.label == "Reason")
            .unwrap();
        assert_eq!(reason.value, crate::scheduler::AI_NOT_CONNECTED);
        assert_eq!(
            item(&items, Step::Runtime).summary,
            "Copilot runtime is not set up and Azure CLI was not found"
        );
        let summary = summarize(&items);
        assert_eq!(
            summary,
            Summary {
                required_done: 2,
                required: 5,
                optional_done: 0,
                optional: 4,
                checking: false
            }
        );
        assert_eq!(
            summary.text(),
            "2 of 5 required steps done · 0 optional connected"
        );
        assert_eq!(summary.badge(), "Setup 2/5");
        assert!(summary.show_hint(false) && !summary.show_hint(true));
        assert_eq!(
            next_step(&items).map(|item| (item.step, item.action)),
            Some((Step::Calendar, Some(Action::SignInWorkiq)))
        );
    }

    #[test]
    fn a_fully_connected_profile_is_complete_and_hides_the_hint() {
        let fixture = Fixture::connected();
        let items = checklist(&fixture.connected_inputs());
        for item in &items {
            assert_eq!(item.state, State::Done, "{:?}: {}", item.step, item.summary);
        }
        assert_eq!(
            item(&items, Step::Calendar).summary,
            "Synced: 1 month back – 6 months ahead"
        );
        assert_eq!(
            item(&items, Step::Insights).summary,
            "Automatic Insights can run"
        );
        assert_eq!(item(&items, Step::Github).summary, "Connected as octocat");
        assert!(
            item(&items, Step::Copilot)
                .details
                .iter()
                .any(|detail| detail.value == r"C:\Tools\copilot.exe (version 1.0.93)")
        );
        let summary = summarize(&items);
        assert!(summary.complete() && !summary.show_hint(false));
        assert_eq!((summary.optional_done, summary.optional), (4, 4));
        assert!(next_step(&items).is_none());
    }

    #[test]
    fn calendar_coverage_progress_errors_and_connection_progress_are_honest() {
        let mut fixture = Fixture::connected();
        let window = Window::new(fixture.today).unwrap();
        fixture.coverage.retain(|day| *day <= fixture.today);
        let total = (window.last - window.first).num_days() as usize + 1;
        let synced = (fixture.today - window.first).num_days() as usize + 1;
        let items = checklist(&fixture.connected_inputs());
        let calendar = item(&items, Step::Calendar);
        assert_eq!(calendar.state, State::Checking);
        assert_eq!(
            calendar.summary,
            format!(
                "Synced {synced} of {total} days; {} not synced yet",
                total - synced
            )
        );
        assert!(
            summarize(&items).checking && !summarize(&items).show_hint(false),
            "No hint while a required step is still being checked"
        );
        fixture.sync.error = Some(
            "Calendar synchronization acknowledgement timed out; retrying in one minute".into(),
        );
        let items = checklist(&fixture.connected_inputs());
        let calendar = item(&items, Step::Calendar);
        assert_eq!(
            (calendar.state, calendar.action),
            (State::Error, Some(Action::RetrySync))
        );
        assert!(
            calendar
                .details
                .iter()
                .any(|detail| detail.label == "Error" && detail.value.contains("timed out"))
        );
        let busy = Inputs {
            workiq: Connection {
                busy: true,
                ready: false,
                message: "WorkIQ connection queued...",
            },
            ..fixture.inputs()
        };
        let items = checklist(&busy);
        assert_eq!(
            (
                item(&items, Step::Calendar).state,
                item(&items, Step::Calendar).summary.as_str()
            ),
            (State::Checking, "WorkIQ connection queued...")
        );
        let copilot = Inputs {
            workiq: Connection {
                busy: true,
                ..Default::default()
            },
            copilot: Connection {
                busy: true,
                ready: false,
                message: "Connecting isolated SDK...",
            },
            ..fixture.inputs()
        };
        let items = checklist(&copilot);
        assert_eq!(item(&items, Step::Copilot).state, State::Checking);
        assert_eq!(item(&items, Step::Insights).state, State::Checking);
        assert_eq!(
            next_step(&items).map(|item| item.step),
            None,
            "Checking steps are not offered as the next step"
        );
        let configured = Fixture::connected();
        let items = checklist(&configured.inputs());
        assert_eq!(
            states(&items)[1],
            (Step::Copilot, State::Action, Some(Action::SignInCopilot))
        );
    }

    #[test]
    fn identity_match_exposes_the_preparation_blocker() {
        use crate::scheduler::Blocker;
        let fixture = Fixture::connected();
        let reason = |inputs: &Inputs| {
            let items = checklist(inputs);
            let item = item(&items, Step::Insights).clone();
            (
                item.state,
                item.action,
                item.details
                    .iter()
                    .find(|detail| detail.label == "Reason")
                    .map(|detail| detail.value.clone()),
            )
        };
        let foreign = Inputs {
            workiq_account: Some("someone@contoso.example"),
            ..fixture.connected_inputs()
        };
        assert_eq!(
            reason(&foreign),
            (
                State::Action,
                Some(Action::OpenWorkiq),
                Some(Blocker::ForeignCalendar.message().into())
            )
        );
        let paused = Inputs {
            paused: true,
            ..fixture.connected_inputs()
        };
        assert_eq!(
            reason(&paused),
            (
                State::Action,
                Some(Action::OpenInsights),
                Some(Blocker::Paused.message().into())
            )
        );
        let disabled = Inputs {
            prepare_enabled: false,
            paused: true,
            ..fixture.connected_inputs()
        };
        assert_eq!(
            reason(&disabled).2.as_deref(),
            Some(Blocker::Disabled.message())
        );
        let workiq = Inputs {
            workiq: Connection {
                busy: false,
                ready: false,
                message: "Not connected",
            },
            ..fixture.connected_inputs()
        };
        assert_eq!(
            reason(&workiq),
            (
                State::Action,
                Some(Action::OpenWorkiq),
                Some(Blocker::Workiq.message().into())
            )
        );
        let empty = Fixture::new();
        let unsynced = Inputs {
            calendar_owner: None,
            ..empty.connected_inputs()
        };
        assert_eq!(
            reason(&unsynced),
            (
                State::Checking,
                None,
                Some(crate::scheduler::UNVERIFIED_COVERAGE.into())
            )
        );
        let readiness = crate::scheduler::Readiness {
            enabled: true,
            ai_ready: true,
            workiq_ready: true,
            workiq_account: Some("a@contoso.example"),
            calendar_owner: Some("a@contoso.example"),
            coverage_verified: true,
            ..Default::default()
        };
        assert!(readiness.automatic() && readiness.connected() && readiness.blocker().is_none());
        assert_eq!(
            crate::scheduler::Readiness {
                ai_ready: false,
                ..readiness
            }
            .blocker(),
            Some(Blocker::Ai)
        );
        assert_eq!(
            crate::scheduler::Readiness {
                coverage_verified: false,
                ..readiness
            }
            .blocker(),
            Some(Blocker::Coverage)
        );
        assert!(
            !crate::scheduler::Readiness {
                paused: true,
                ..readiness
            }
            .automatic()
        );
    }

    #[test]
    fn notifications_updates_and_optional_connections_map_their_states() {
        use crate::reminders::WindowsSetting;
        let mut fixture = Fixture::new();
        fixture.reminders.windows = None;
        let items = checklist(&fixture.inputs());
        assert_eq!(
            (
                item(&items, Step::Notifications).state,
                item(&items, Step::Notifications).action
            ),
            (State::Checking, None)
        );
        fixture.reminders.windows = Some(WindowsSetting::DisabledForApplication);
        let items = checklist(&fixture.inputs());
        let notifications = item(&items, Step::Notifications);
        assert_eq!(
            (notifications.state, notifications.action),
            (State::Action, Some(Action::TestNotification))
        );
        assert_eq!(
            notifications.summary,
            WindowsSetting::DisabledForApplication.message().unwrap()
        );
        fixture.reminders.error = Some("Show the Windows notification: Access is denied.".into());
        assert_eq!(
            item(&checklist(&fixture.inputs()), Step::Notifications).state,
            State::Error
        );
        let stopped = Inputs {
            reminders_running: false,
            ..fixture.inputs()
        };
        let items = checklist(&stopped);
        assert_eq!(
            (
                item(&items, Step::Notifications).state,
                item(&items, Step::Notifications).action
            ),
            (State::Error, None)
        );

        let fixture = Fixture::new();
        let off = Inputs {
            automatic_updates: false,
            ..fixture.inputs()
        };
        assert_eq!(
            states(&checklist(&off))[4],
            (Step::Updates, State::Action, Some(Action::EnableUpdates))
        );
        let mut failed = Fixture::new();
        failed.updates.error = Some("GitHub Releases did not answer".into());
        assert_eq!(
            states(&checklist(&failed.inputs()))[4],
            (Step::Updates, State::Error, Some(Action::CheckUpdates))
        );
        failed.updates.activity = Some(crate::updater::Activity::Checking);
        failed.updates.error = None;
        assert_eq!(
            states(&checklist(&failed.inputs()))[4],
            (Step::Updates, State::Checking, None)
        );
        failed.updates.disabled =
            Some("Updates are disabled in measurement and diagnostic modes.".into());
        let items = checklist(&failed.inputs());
        assert_eq!(
            (
                item(&items, Step::Updates).state,
                item(&items, Step::Updates).summary.as_str()
            ),
            (
                State::Error,
                "Updates are disabled in measurement and diagnostic modes."
            )
        );

        let fixture = Fixture::new();
        let failing = Inputs {
            webiq: Connection {
                busy: false,
                ready: false,
                message: "WebIQ verification read failed: HTTP 401",
            },
            github: Connection {
                busy: true,
                ready: false,
                message: "Checking saved sign-in...",
            },
            msx_enabled: false,
            ..fixture.inputs()
        };
        let items = checklist(&failing);
        assert_eq!(
            states(&items)[5..9],
            [
                (Step::Webiq, State::Error, Some(Action::EnterWebiqKey)),
                (Step::Github, State::Checking, None),
                (Step::Foundry, State::NotSet, Some(Action::OpenFoundry)),
                (Step::Msx, State::NotSet, Some(Action::OpenMsxSettings)),
            ]
        );
        assert_eq!(item(&items, Step::Msx).summary, crate::msx::DISABLED);
    }

    #[test]
    fn the_demo_explains_instead_of_offering_actions() {
        let fixture = Fixture::connected();
        let items = checklist(&Inputs {
            demo: true,
            ..fixture.connected_inputs()
        });
        assert_eq!(items.len(), Step::ALL.len());
        assert!(items.iter().all(|item| item.state == State::NotSet
            && item.summary == DEMO
            && item.action.is_none()
            && item.details.is_empty()));
        assert!(next_step(&items).is_none());
    }

    #[test]
    fn welcome_opens_once_for_new_profiles_and_never_for_upgraded_or_diagnostic_runs() {
        let fresh = Probe {
            azure_cli: None,
            copilot: Err("missing".into()),
            saved_connections: false,
        };
        let saved = Probe {
            saved_connections: true,
            ..fresh.clone()
        };
        let new = crate::storage::Preferences::default();
        assert!(!new.welcome_shown && !new.setup_hint_hidden);
        assert_eq!(
            first_run(&new, false, true, None, Some(&fresh)),
            FirstRun::Nothing,
            "Demo, stress and measurement runs never welcome"
        );
        assert_eq!(
            first_run(&new, true, false, None, Some(&fresh)),
            FirstRun::Wait
        );
        assert_eq!(first_run(&new, true, true, None, None), FirstRun::Wait);
        assert_eq!(
            first_run(&new, true, true, None, Some(&fresh)),
            FirstRun::Show
        );
        assert_eq!(
            first_run(&new, true, true, None, Some(&saved)),
            FirstRun::MarkShown
        );
        assert_eq!(
            first_run(
                &new,
                true,
                true,
                Some("owner@contoso.example"),
                Some(&fresh)
            ),
            FirstRun::MarkShown
        );
        for upgraded in [
            crate::storage::Preferences {
                work_email: "owner@contoso.example".into(),
                ..Default::default()
            },
            crate::storage::Preferences {
                github_client_id: "Iv23liExample".into(),
                ..Default::default()
            },
        ] {
            assert_eq!(
                first_run(&upgraded, true, true, None, Some(&fresh)),
                FirstRun::MarkShown
            );
        }
        let shown = crate::storage::Preferences {
            welcome_shown: true,
            ..Default::default()
        };
        assert_eq!(
            first_run(&shown, true, true, None, Some(&fresh)),
            FirstRun::Nothing
        );
    }

    #[test]
    fn old_preferences_default_to_no_welcome_record_and_a_visible_hint_and_dismissal_persists() {
        let old: crate::storage::Preferences =
            serde_json::from_str(r#"{"dark":false,"work_email":"owner@contoso.example"}"#).unwrap();
        assert!(!old.welcome_shown && !old.setup_hint_hidden);
        let root = tempfile::tempdir().unwrap();
        let store = crate::storage::Store::open(root.path()).unwrap();
        store
            .save_preferences(&crate::storage::Preferences {
                welcome_shown: true,
                setup_hint_hidden: true,
                ..old
            })
            .unwrap();
        drop(store);
        let restored = crate::storage::Store::open(root.path())
            .unwrap()
            .preferences()
            .unwrap();
        assert!(restored.welcome_shown && restored.setup_hint_hidden);
    }

    #[test]
    fn probe_finds_configured_runtime_and_saved_sign_ins_without_creating_credentials() {
        let root = tempfile::tempdir().unwrap();
        let empty = probe(root.path());
        assert!(!empty.saved_connections);
        assert!(
            empty
                .copilot
                .as_ref()
                .is_err_and(|error| error.contains("connections.json"))
        );
        assert!(
            !root.path().join("connector-login.db").exists()
                && !root.path().join("workiq-login.db").exists(),
            "The probe never creates credential databases"
        );
        let created = rusqlite::Connection::open(root.path().join("connector-login.db")).unwrap();
        created
            .execute_batch(
                "CREATE TABLE credentials(provider TEXT PRIMARY KEY,payload BLOB NOT NULL)",
            )
            .unwrap();
        assert!(
            !probe(root.path()).saved_connections,
            "An empty credential database is created at every startup"
        );
        created
            .execute("INSERT INTO credentials VALUES('WebIQ-MCP',X'00')", [])
            .unwrap();
        assert!(probe(root.path()).saved_connections);

        let root = tempfile::tempdir().unwrap();
        let vault = rusqlite::Connection::open(root.path().join("workiq-login.db")).unwrap();
        vault
            .execute_batch(
                "CREATE TABLE login(account TEXT PRIMARY KEY, payload BLOB NOT NULL);
            INSERT INTO login VALUES('owner@contoso.example',X'00');",
            )
            .unwrap();
        assert!(probe(root.path()).saved_connections);

        let root = tempfile::tempdir().unwrap();
        let runtime = std::env::current_exe().unwrap();
        std::fs::write(
            root.path().join("connections.json"),
            serde_json::json!({
                "runtime": runtime, "use_stored_login": true, "servers": {}
            })
            .to_string(),
        )
        .unwrap();
        let configured = probe(root.path());
        assert!(configured.saved_connections);
        assert_eq!(configured.copilot.unwrap().path, runtime);
    }

    fn synthetic_executable(version: Option<[u16; 4]>) -> Vec<u8> {
        let mut bytes = vec![0_u8; 64];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
        bytes.extend_from_slice(b"PE\0\0");
        let mut coff = [0_u8; 20];
        coff[2..4].copy_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&coff);
        let mut resources = vec![0_u8; 6];
        resources.extend(
            "VS_VERSION_INFO\0"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        resources.extend_from_slice(&[0, 0]);
        if let Some([major, minor, patch, build]) = version {
            let words = [
                0xFEEF_04BD,
                0x0001_0000,
                0,
                0,
                (u32::from(major) << 16) | u32::from(minor),
                (u32::from(patch) << 16) | u32::from(build),
            ];
            resources.extend(words.iter().flat_map(|word| word.to_le_bytes()));
        }
        let mut section = [0_u8; 40];
        section[..6].copy_from_slice(b".rsrc\0");
        section[16..20].copy_from_slice(&(resources.len() as u32).to_le_bytes());
        section[20..24].copy_from_slice(&128_u32.to_le_bytes());
        bytes.extend_from_slice(&section);
        bytes.resize(128, 0);
        bytes.extend_from_slice(&resources);
        bytes
    }

    #[test]
    fn runtime_version_comes_from_the_executable_version_resource() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("copilot.exe");
        std::fs::write(&path, synthetic_executable(Some([1, 0, 93, 1]))).unwrap();
        assert_eq!(executable_version(&path).as_deref(), Some("1.0.93.1"));
        std::fs::write(&path, synthetic_executable(Some([1, 0, 94, 0]))).unwrap();
        assert_eq!(executable_version(&path).as_deref(), Some("1.0.94"));
        std::fs::write(&path, synthetic_executable(None)).unwrap();
        assert_eq!(executable_version(&path), None);
        std::fs::write(&path, b"not an executable").unwrap();
        assert_eq!(executable_version(&path), None);
        assert_eq!(executable_version(&root.path().join("missing.exe")), None);
        let _ = executable_version(&std::env::current_exe().unwrap());
        let runtime = Runtime {
            path: path.clone(),
            version: Some("1.0.93".into()),
        };
        assert_eq!(
            Language::Cs.text(&runtime.label()),
            format!("{} (verze 1.0.93)", path.display())
        );
    }

    fn localized(text: &str) -> bool {
        Language::Cs.text(text) != text || crate::i18n::TEXTS.iter().any(|(en, _)| *en == text)
    }

    #[test]
    fn every_checklist_text_is_in_the_czech_catalog() {
        for step in Step::ALL {
            assert!(
                localized(step.title()),
                "Missing Czech title: {}",
                step.title()
            );
            assert!(
                localized(step.purpose()),
                "Missing Czech purpose: {}",
                step.purpose()
            );
        }
        for state in [
            State::Done,
            State::Action,
            State::NotSet,
            State::Checking,
            State::Error,
        ] {
            assert!(
                localized(state.label()),
                "Missing Czech state: {}",
                state.label()
            );
        }
        use crate::scheduler::Blocker;
        for blocker in [
            Blocker::Disabled,
            Blocker::Paused,
            Blocker::Ai,
            Blocker::Workiq,
            Blocker::ForeignCalendar,
            Blocker::Coverage,
        ] {
            assert!(
                localized(blocker.message()),
                "Missing Czech blocker: {}",
                blocker.message()
            );
        }
        let new = Fixture::new();
        let connected = Fixture::connected();
        let mut partial = Fixture::connected();
        partial.coverage.retain(|day| *day <= partial.today);
        let mut failed = Fixture::new();
        failed.updates.error = Some("x".into());
        failed.reminders.windows = None;
        failed.probe = None;
        let mut unavailable = Fixture::new();
        unavailable.probe = Some(Probe {
            azure_cli: Some("az.cmd".into()),
            copilot: Err("missing".into()),
            saved_connections: false,
        });
        unavailable.foundry =
            crate::decision::Status::new(crate::decision::Phase::Unconfigured, "");
        unavailable.msx.phase = crate::msx::Phase::Unavailable;
        let mut runtime = Fixture::new();
        runtime.probe = Some(Probe {
            azure_cli: None,
            copilot: Ok(Runtime {
                path: "copilot.exe".into(),
                version: None,
            }),
            saved_connections: false,
        });
        runtime.updates.activity = Some(crate::updater::Activity::Checking);
        let sets = [
            checklist(&new.inputs()),
            checklist(&Inputs {
                automatic_updates: false,
                prepare_enabled: false,
                ..new.inputs()
            }),
            checklist(&connected.connected_inputs()),
            checklist(&partial.connected_inputs()),
            checklist(&failed.inputs()),
            checklist(&unavailable.inputs()),
            checklist(&runtime.inputs()),
            checklist(&Inputs {
                demo: true,
                ..new.inputs()
            }),
        ];
        for items in &sets {
            for item in items {
                assert!(
                    localized(&item.summary),
                    "Missing Czech summary: {}",
                    item.summary
                );
                if let Some(action) = item.action {
                    assert!(
                        localized(action.label()),
                        "Missing Czech action: {}",
                        action.label()
                    );
                }
                for detail in &item.details {
                    assert!(
                        localized(detail.label),
                        "Missing Czech detail: {}",
                        detail.label
                    );
                }
            }
            assert!(localized(&summarize(items).text()));
            assert!(localized(&summarize(items).badge()));
        }
        assert_eq!(
            Language::Cs.text("Synced 120 of 214 days; 94 not synced yet"),
            "Synchronizováno 120 z 214 dní, zbývá 94"
        );
        assert_eq!(Language::Cs.text("Setup 2/5"), "Nastavení 2/5");
        assert_eq!(
            Language::Cs.text("4 of 5 required steps done · 2 optional connected"),
            "Povinné kroky: hotovo 4 z 5 · nepovinná připojení: 2"
        );
        assert_eq!(Language::Cs.text("120 of 214 days"), "120 z 214 dní");
        assert_eq!(Language::Cs.text(Step::Insights.title()), "Shoda účtů");
    }
}
