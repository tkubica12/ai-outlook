//! Pre-meeting reminders shown as native Windows toast notifications.
//!
//! A dedicated thread owns all notification work: it registers the notification identity,
//! computes the next due reminder from the cached calendar and waits on a channel until then.
//! The UI thread only hands over calendar snapshots and preferences.

use crate::{
    calendar::{Calendar, Event},
    i18n::Language,
    storage::{Preferences, Store},
};
use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, TimeZone, Timelike, Utc};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

pub const AUMID: &str = "Tomlook.Calendar";
pub const PREVIEW_AUMID: &str = "Tomlook.Calendar.Demo";
pub const MINUTES: [u8; 5] = [1, 2, 5, 10, 15];
pub const DEFAULT_MINUTES: u8 = 5;
pub const FIRED_LIMIT: usize = 4096;
/// A reminder may still be shown this long after its meeting started.
pub const GRACE_SECONDS: i64 = 120;
const RETENTION_HOURS: i64 = 48;
const BULLETS: usize = 3;
const BULLET_CHARS: usize = 80;
const TITLE_CHARS: usize = 120;
const LOCATION_CHARS: usize = 80;
const JOIN_URL_BYTES: usize = 2048;
const GROUP: &str = "meeting-reminders";
const TEST_GROUP: &str = "test";
const TEST_TAG: &str = "test";
const ICON_FILE: &str = "notification-icon.png";
const ICON_SIZE: u32 = 256;
/// Windows wait timeouts stop while the PC sleeps; waking at least this often keeps
/// reminders that became due during sleep or a clock change at most this late.
const MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
const SAMPLE_JOIN_URL: &str = "https://www.microsoft.com/microsoft-teams/join-a-meeting";
const SAMPLE_BULLETS: [&str; 3] = [
    "Prepare: Review the sample agenda",
    "Talking point: Confirm the next step",
    "Question: Who owns the follow-up?",
];
pub const PREVIEW_DISABLED: &str = "Meeting reminders are disabled in the offline demo; the test notification uses fictional content.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Live,
    Preview,
}

impl Mode {
    pub fn aumid(self) -> &'static str {
        match self {
            Self::Live => AUMID,
            Self::Preview => PREVIEW_AUMID,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Live => "Tomlook",
            Self::Preview => "Tomlook (demo)",
        }
    }

    /// COM class Windows calls when a toast is clicked in the notification center.
    fn activator(self) -> u128 {
        match self {
            Self::Live => 0x4B0F56E6_AACA_47CF_8521_CEF0E3AAFDB0,
            Self::Preview => 0x287091C5_040C_40ED_BFAE_43F10A953813,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub minutes: u8,
    pub tentative: bool,
    pub language: Language,
}

impl Settings {
    pub fn from_preferences(preferences: &Preferences) -> Self {
        Self {
            enabled: preferences.meeting_reminders,
            minutes: normalize_minutes(preferences.reminder_minutes),
            tentative: preferences.remind_tentative,
            language: preferences.language,
        }
    }
}

pub fn normalize_minutes(minutes: u8) -> u8 {
    if MINUTES.contains(&minutes) {
        minutes
    } else {
        DEFAULT_MINUTES
    }
}

/// Timed meetings the user organizes or accepted; tentative ones only when requested.
/// Following, declined, unanswered, cancelled and free items never remind.
pub fn eligible(event: &Event, tentative: bool) -> bool {
    let flag = |key: &str| event.extra.get(key).and_then(Value::as_bool) == Some(true);
    let show_as = event
        .extra
        .get("showAs")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let response = event.status.to_ascii_lowercase();
    if event.is_all_day
        || flag("isCancelled")
        || matches!(response.as_str(), "cancelled" | "canceled" | "declined")
        || show_as.eq_ignore_ascii_case("free")
        || event.following()
    {
        return false;
    }
    let confirmed = flag("isOrganizer") || matches!(response.as_str(), "organizer" | "accepted");
    let tentative_response = matches!(response.as_str(), "tentativelyaccepted" | "tentative");
    if !confirmed && !tentative_response {
        return false;
    }
    let uncertain = tentative_response
        || show_as.eq_ignore_ascii_case("tentative")
        || event
            .tags()
            .iter()
            .any(|tag| tag.trim().to_lowercase() == "nejisté");
    !uncertain || tentative
}

/// Stable identity of one occurrence: the event and its start time.
pub fn occurrence_key(event: &Event) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(event.id.as_bytes());
    digest.update([0]);
    digest.update(event.start.with_timezone(&Utc).to_rfc3339().as_bytes());
    format!("{:x}", digest.finalize())
}

fn tag(key: &str) -> &str {
    &key[..32]
}

fn grace() -> Duration {
    Duration::seconds(GRACE_SECONDS)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    pub key: String,
    pub id: String,
    pub start: DateTime<Utc>,
    pub at: DateTime<Utc>,
}

/// Reminders of eligible meetings that have not started more than the grace period ago,
/// ordered by the time they are due.
pub fn schedule(calendar: &Calendar, settings: &Settings, now: DateTime<Utc>) -> Vec<Due> {
    if !settings.enabled {
        return Vec::new();
    }
    let lead = Duration::minutes(i64::from(normalize_minutes(settings.minutes)));
    let mut plan: Vec<Due> = calendar
        .events
        .iter()
        .filter(|event| eligible(event, settings.tentative))
        .filter_map(|event| {
            let start = event.start.with_timezone(&Utc);
            (start + grace() > now).then(|| Due {
                key: occurrence_key(event),
                id: event.id.clone(),
                start,
                at: start - lead,
            })
        })
        .collect();
    plan.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.key.cmp(&b.key)));
    plan
}

/// The current reminder plan plus the bounded set of occurrences already shown.
#[derive(Clone, Debug, Default)]
pub struct Planner {
    plan: Vec<Due>,
    fired: BTreeMap<String, DateTime<Utc>>,
}

impl Planner {
    pub fn with_fired(fired: BTreeMap<String, DateTime<Utc>>, now: DateTime<Utc>) -> Self {
        let mut planner = Self {
            plan: Vec::new(),
            fired,
        };
        planner.prune(now);
        planner
    }

    pub fn fired(&self) -> &BTreeMap<String, DateTime<Utc>> {
        &self.fired
    }

    pub fn replace(&mut self, plan: Vec<Due>) {
        self.plan = plan;
    }

    fn pending(&self, now: DateTime<Utc>) -> impl Iterator<Item = &Due> {
        self.plan
            .iter()
            .filter(move |due| now < due.start + grace() && !self.fired.contains_key(&due.key))
    }

    /// Reminders to show now, including ones whose time passed while the PC slept.
    pub fn due(&self, now: DateTime<Utc>) -> Vec<Due> {
        self.pending(now)
            .take_while(|due| due.at <= now)
            .cloned()
            .collect()
    }

    /// When the next reminder becomes due; `now` when one is already due.
    pub fn next_wake(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.pending(now).next().map(|due| due.at.max(now))
    }

    pub fn mark(&mut self, due: &Due, now: DateTime<Utc>) {
        self.fired.insert(due.key.clone(), due.start);
        self.prune(now);
    }

    fn prune(&mut self, now: DateTime<Utc>) {
        let cutoff = now - Duration::hours(RETENTION_HOURS);
        self.fired.retain(|_, start| *start > cutoff);
        while self.fired.len() > FIRED_LIMIT {
            let Some(oldest) = self
                .fired
                .iter()
                .min_by_key(|(_, start)| **start)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.fired.remove(&oldest);
        }
    }
}

/// Collapses whitespace, removes control characters and truncates to `limit` characters.
pub fn shorten(text: &str, limit: usize) -> String {
    let text = text
        .chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    let mut short: String = text.chars().take(limit.saturating_sub(1)).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    short
}

/// Up to three short points from saved Insights: next steps first, then talking points,
/// then open questions. Czech uses the saved Czech narrative and falls back to English.
pub fn bullets(insights: &Value, language: Language) -> Vec<String> {
    let narrative = language.insight(insights).unwrap_or(insights);
    let mut result: Vec<String> = Vec::new();
    for field in ["preparation", "talking_points", "open_questions"] {
        for item in narrative[field]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let item = shorten(item, BULLET_CHARS);
            if !item.is_empty() && !result.contains(&item) {
                result.push(item);
                if result.len() == BULLETS {
                    return result;
                }
            }
        }
    }
    result
}

fn time_range<Tz: TimeZone>(
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    language: Language,
    zone: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let last = (end - Duration::nanoseconds(1)).with_timezone(zone);
    let start = start.with_timezone(zone);
    let end = end.with_timezone(zone);
    if start.date_naive() == last.date_naive() {
        return format!("{}–{}", start.format("%H:%M"), end.format("%H:%M"));
    }
    match language {
        Language::En => format!(
            "{}–{}",
            start.format("%H:%M"),
            end.format("%a %-d %b %H:%M")
        ),
        Language::Cs => format!(
            "{}–{}. {}. {}",
            start.format("%H:%M"),
            end.day(),
            end.month(),
            end.format("%H:%M")
        ),
    }
}

/// "In 5 min · 09:15–10:45"; the actual remaining minutes when late, "Now" at the start.
pub fn title<Tz: TimeZone>(
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    now: DateTime<Utc>,
    language: Language,
    zone: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let range = time_range(start, end, language, zone);
    let seconds = (start.with_timezone(&Utc) - now).num_seconds();
    if seconds < 30 {
        language.text(&format!("Now · {range}")).into_owned()
    } else {
        let minutes = ((seconds + 30) / 60).max(1);
        language
            .text(&format!("In {minutes} min · {range}"))
            .into_owned()
    }
}

/// A safe join link short enough for the notification payload.
pub fn join_url(event: &Event) -> Option<&str> {
    event.join_url().filter(|url| url.len() <= JOIN_URL_BYTES)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub title: String,
    pub meeting: String,
    pub location: Option<String>,
    pub bullets: Vec<String>,
    pub join: Option<String>,
    pub arguments: String,
    pub snooze: u8,
    pub language: Language,
}

pub fn reminder<Tz: TimeZone>(
    event: &Event,
    insights: Option<&Value>,
    settings: &Settings,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Toast
where
    Tz::Offset: std::fmt::Display,
{
    Toast {
        title: title(event.start, event.end, now, settings.language, zone),
        meeting: shorten(&event.title, TITLE_CHARS),
        location: Some(shorten(&event.location, LOCATION_CHARS))
            .filter(|location| !location.is_empty()),
        bullets: insights
            .map(|insights| bullets(insights, settings.language))
            .unwrap_or_default(),
        join: join_url(event).map(str::to_owned),
        arguments: format!("open:{}", tag(&occurrence_key(event))),
        snooze: if settings.minutes <= 2 { 1 } else { 5 },
        language: settings.language,
    }
}

/// Clearly labelled sample content for Settings → Notifications → Send test notification.
pub fn sample<Tz: TimeZone>(now: DateTime<Utc>, language: Language, zone: &Tz) -> Toast
where
    Tz::Offset: std::fmt::Display,
{
    let start = (now + Duration::minutes(5))
        .with_second(0)
        .and_then(|time| time.with_nanosecond(0))
        .unwrap_or(now)
        .fixed_offset();
    let end = start + Duration::minutes(30);
    let lead = title(
        start,
        end,
        start.to_utc() - Duration::minutes(5),
        language,
        zone,
    );
    Toast {
        title: format!("{} · {lead}", language.text("Test notification")),
        meeting: language.text("Sample meeting (fictional)").into_owned(),
        location: Some(language.text("Room 2 (sample)").into_owned()),
        bullets: SAMPLE_BULLETS
            .iter()
            .map(|bullet| language.text(bullet).into_owned())
            .collect(),
        join: Some(SAMPLE_JOIN_URL.into()),
        arguments: format!("open:{TEST_TAG}"),
        snooze: 5,
        language,
    }
}

/// The toast tag addressed by `open:<tag>` activation arguments; empty for a body click.
pub fn activation_tag(arguments: &str) -> Option<&str> {
    if arguments.is_empty() {
        Some("")
    } else {
        arguments.strip_prefix("open:")
    }
}

/// Escapes XML text and attribute values and drops characters XML 1.0 forbids.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t' | '\n' | '\r' => escaped.push(character),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => {}
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Toast XML for a reminder; every dynamic value is escaped.
pub fn toast_xml(toast: &Toast) -> String {
    let language = toast.language;
    let arguments = escape(&toast.arguments);
    let mut xml = format!(
        r#"<toast scenario="reminder" activationType="foreground" useButtonStyle="true" launch="{arguments}"><visual><binding template="ToastGeneric"><text hint-maxLines="2">{}</text><text>{}</text>"#,
        escape(&toast.title),
        escape(&toast.meeting)
    );
    if let Some(location) = &toast.location {
        xml.push_str(&format!(
            r#"<text placement="attribution">{}</text>"#,
            escape(location)
        ));
    }
    for bullet in &toast.bullets {
        xml.push_str(&format!(
            r#"<group><subgroup><text hint-style="captionSubtle" hint-wrap="true" hint-maxLines="2">• {}</text></subgroup></group>"#,
            escape(bullet)
        ));
    }
    xml.push_str(&format!(
        r#"</binding></visual><actions><input id="snoozeTime" type="selection" defaultInput="{}">"#,
        normalize_minutes(toast.snooze)
    ));
    for (minutes, label) in [
        (1, "1 minute"),
        (2, "2 minutes"),
        (5, "5 minutes"),
        (10, "10 minutes"),
        (15, "15 minutes"),
    ] {
        xml.push_str(&format!(
            r#"<selection id="{minutes}" content="{}"/>"#,
            escape(&language.text(label))
        ));
    }
    xml.push_str("</input>");
    if let Some(url) = &toast.join {
        xml.push_str(&format!(
            r#"<action content="{}" activationType="protocol" arguments="{}" hint-buttonStyle="Success"/>"#,
            escape(&language.text("Join")),
            escape(url)
        ));
    }
    // At most three buttons fit without wrapping; with a join link the toast's own × dismisses it.
    xml.push_str(&format!(
        r#"<action content="{}" activationType="foreground" arguments="{arguments}"/><action content="" activationType="system" arguments="snooze" hint-inputId="snoozeTime"/>"#,
        escape(&language.text("Open"))
    ));
    if toast.join.is_none() {
        xml.push_str(r#"<action content="" activationType="system" arguments="dismiss"/>"#);
    }
    xml.push_str(r#"</actions><audio src="ms-winsoundevent:Notification.Reminder"/></toast>"#);
    xml
}

/// The brand mark as a PNG for the registered notification identity.
pub fn icon_png() -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &crate::brand::rgba(ICON_SIZE as usize),
            ICON_SIZE,
            ICON_SIZE,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("Encode the notification icon: {error}"))?;
    Ok(png)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowsSetting {
    Enabled,
    DisabledForApplication,
    DisabledForUser,
    DisabledByGroupPolicy,
    DisabledByManifest,
    Unknown,
}

impl WindowsSetting {
    /// Explanation when Windows will not show Tomlook notifications.
    pub fn message(self) -> Option<&'static str> {
        match self {
            Self::Enabled => None,
            Self::DisabledForApplication => Some(
                "Windows notifications are turned off for Tomlook. Turn them on in Windows Settings > System > Notifications.",
            ),
            Self::DisabledForUser => Some("Windows notifications are turned off for this user."),
            Self::DisabledByGroupPolicy => {
                Some("Windows notifications are blocked by group policy.")
            }
            Self::DisabledByManifest => Some("Windows does not allow notifications for Tomlook."),
            Self::Unknown => Some("Windows reported an unknown notification setting."),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub revision: u64,
    /// Why scheduled reminders do not run in this session; the test may still work.
    pub disabled: Option<String>,
    pub windows: Option<WindowsSetting>,
    pub error: Option<String>,
    pub next: Option<DateTime<Utc>>,
    pub last_reminder: Option<DateTime<Utc>>,
    pub last_test: Option<DateTime<Utc>>,
}

type OpenHandler = Arc<dyn Fn(Option<String>) + Send + Sync>;

#[derive(Default)]
struct Pending {
    update: Option<(Arc<Calendar>, Settings)>,
    // Holding the last snapshot keeps its address from being reused by a newer one.
    sent: Option<(Arc<Calendar>, Settings)>,
    test: Option<Language>,
    refresh: bool,
}

struct Shared {
    status: Mutex<Status>,
    pending: Mutex<Pending>,
    stop: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
    /// Shown toast tags and the meeting each one opens (`None` for the test).
    activations: Mutex<BTreeMap<String, Option<String>>>,
}

impl Shared {
    /// Opens Tomlook for a toast click; unknown tags (for example after a restart) only restore it.
    fn activate(&self, open: &OpenHandler, arguments: &str) {
        let Some(tag) = activation_tag(arguments) else {
            return;
        };
        let meeting = self
            .activations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(tag)
            .cloned()
            .flatten();
        open(meeting);
    }

    fn update(&self, change: impl FnOnce(&mut Status)) {
        let changed = {
            let mut status = self
                .status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let before = status.clone();
            change(&mut status);
            let changed = *status != before;
            if changed {
                status.revision += 1;
            }
            changed
        };
        if changed {
            (self.notify)();
        }
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Handle owned by the UI; every method returns immediately.
pub struct Reminders {
    shared: Arc<Shared>,
    wake: mpsc::SyncSender<()>,
    done: Option<mpsc::Receiver<()>>,
}

/// Stops the reminder thread after the native window has closed.
pub struct Shutdown {
    shared: Arc<Shared>,
    wake: mpsc::SyncSender<()>,
    done: mpsc::Receiver<()>,
}

impl Shutdown {
    pub fn stop(self, timeout: std::time::Duration) -> Result<(), String> {
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
        match self.done.recv_timeout(timeout) {
            Ok(()) => Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err("Meeting reminder shutdown could not be verified".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("Meeting reminders stopped unexpectedly".into())
            }
        }
    }
}

impl Reminders {
    /// Starts the reminder thread. `notify` requests a repaint after status changes;
    /// `open` runs on a Windows thread when a toast asks to open Tomlook or a meeting.
    pub fn start(
        state: PathBuf,
        mode: Mode,
        notify: impl Fn() + Send + Sync + 'static,
        open: impl Fn(Option<String>) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            status: Mutex::new(Status::default()),
            pending: Mutex::new(Pending::default()),
            stop: AtomicBool::new(false),
            notify: Box::new(notify),
            activations: Mutex::new(BTreeMap::new()),
        });
        let (wake, inbox) = mpsc::sync_channel(1);
        let (finished, done) = mpsc::sync_channel(1);
        let runner = Runner {
            shared: shared.clone(),
            open: Arc::new(open),
            state,
            mode,
        };
        std::thread::Builder::new()
            .name("tomlook-reminders".into())
            .spawn(move || {
                runner.run(inbox);
                let _ = finished.send(());
            })
            .map_err(|error| format!("Start meeting reminders: {error}"))?;
        Ok(Self {
            shared,
            wake,
            done: Some(done),
        })
    }

    pub fn shutdown(&mut self) -> Option<Shutdown> {
        Some(Shutdown {
            shared: self.shared.clone(),
            wake: self.wake.clone(),
            done: self.done.take()?,
        })
    }

    fn wake(&self) {
        // A full slot already wakes the thread; it then reads every pending request.
        let _ = self.wake.try_send(());
    }

    /// Hands over the current calendar snapshot and preferences; unchanged input is ignored.
    pub fn update(&self, calendar: &Arc<Calendar>, settings: Settings) {
        {
            let mut pending = self.shared.pending();
            if pending.sent.as_ref().is_some_and(|(sent, sent_settings)| {
                Arc::ptr_eq(sent, calendar) && *sent_settings == settings
            }) {
                return;
            }
            pending.sent = Some((calendar.clone(), settings));
            pending.update = Some((calendar.clone(), settings));
        }
        self.wake();
    }

    pub fn test(&self, language: Language) {
        self.shared.pending().test = Some(language);
        self.wake();
    }

    /// Re-reads whether Windows allows Tomlook notifications.
    pub fn refresh(&self) {
        self.shared.pending().refresh = true;
        self.wake();
    }

    /// Returns the status when it differs from revision `seen`; never blocks.
    pub fn status(&self, seen: u64) -> Option<Status> {
        let status = self.shared.status.try_lock().ok()?;
        (status.revision != seen).then(|| status.clone())
    }
}

impl Drop for Reminders {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.wake();
    }
}

/// Sets the process notification identity; call before any window is created.
pub fn set_process_identity(mode: Mode) -> Result<(), String> {
    platform::set_process_identity(mode)
}

struct Runner {
    shared: Arc<Shared>,
    open: OpenHandler,
    state: PathBuf,
    mode: Mode,
}

struct Live {
    store: Option<Store>,
    planner: Planner,
    calendar: Arc<Calendar>,
    settings: Option<Settings>,
}

impl Runner {
    fn run(self, inbox: mpsc::Receiver<()>) {
        let mut backend = platform::Backend::new(
            self.mode,
            &self.state,
            self.open.clone(),
            self.shared.clone(),
        );
        let registered = backend.register().and_then(|()| backend.listen());
        let setting = backend.setting();
        self.shared.update(|status| {
            status.disabled = (self.mode == Mode::Preview).then(|| PREVIEW_DISABLED.into());
            status.windows = setting.as_ref().ok().copied().flatten();
            status.error = registered.err().or(setting.err());
        });
        let mut live: Option<Live> = None;
        loop {
            let now = Utc::now();
            let wait = live
                .as_ref()
                .and_then(|live| live.planner.next_wake(now))
                .map(|at| (at - now).to_std().unwrap_or_default().min(MAX_WAIT));
            let received = match wait {
                Some(wait) => inbox.recv_timeout(wait),
                None => inbox
                    .recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            };
            if self.shared.stop.load(Ordering::Acquire)
                || received == Err(mpsc::RecvTimeoutError::Disconnected)
            {
                break;
            }
            let (update, test, refresh) = {
                let mut pending = self.shared.pending();
                (
                    pending.update.take(),
                    pending.test.take(),
                    std::mem::take(&mut pending.refresh),
                )
            };
            if refresh {
                let setting = backend.setting();
                self.shared.update(|status| match setting {
                    Ok(setting) => status.windows = setting.or(status.windows),
                    Err(error) => status.error = Some(error),
                });
            }
            if let Some(language) = test {
                self.test(&mut backend, language);
            }
            if let Some((calendar, settings)) = update
                && self.mode == Mode::Live
            {
                let live = live.get_or_insert_with(|| self.open_live());
                if settings.enabled {
                    backend.retract(|id, start| {
                        calendar.event_index(id).is_some_and(|index| {
                            let event = &calendar.events[index];
                            event.start.with_timezone(&Utc) == start
                                && eligible(event, settings.tentative)
                        })
                    });
                }
                live.planner
                    .replace(schedule(&calendar, &settings, Utc::now()));
                live.calendar = calendar;
                live.settings = Some(settings);
            }
            if let Some(live) = &mut live {
                self.fire_due(&mut backend, live);
                let next = live.planner.next_wake(Utc::now());
                self.shared.update(|status| status.next = next);
            }
            backend.prune(Utc::now());
        }
        if self.mode == Mode::Preview {
            backend.cleanup();
        }
    }

    fn open_live(&self) -> Live {
        let now = Utc::now();
        let (store, fired) = match Store::open(&self.state).and_then(|store| {
            let fired = store.fired_reminders()?;
            Ok((store, fired))
        }) {
            Ok((store, fired)) => (Some(store), fired),
            Err(error) => {
                self.shared.update(|status| {
                    status.error = Some(format!("Meeting reminder history is unavailable: {error}"))
                });
                (None, BTreeMap::new())
            }
        };
        Live {
            store,
            planner: Planner::with_fired(fired, now),
            calendar: Arc::new(Calendar::default()),
            settings: None,
        }
    }

    fn fire_due(&self, backend: &mut platform::Backend, live: &mut Live) {
        let Some(settings) = live.settings else {
            return;
        };
        let now = Utc::now();
        for due in live.planner.due(now) {
            // Recorded before showing: each occurrence is shown at most once, even after a crash.
            live.planner.mark(&due, now);
            let saved = live
                .store
                .as_ref()
                .map(|store| store.save_fired_reminders(live.planner.fired()));
            let Some(event) = live
                .calendar
                .event_index(&due.id)
                .map(|index| &live.calendar.events[index])
            else {
                continue;
            };
            let insights = live
                .store
                .as_ref()
                .and_then(|store| store.briefing(&event.id).ok().flatten())
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
            let toast = reminder(event, insights.as_ref(), &settings, now, &Local);
            let shown = backend.show(platform::Request {
                tag: tag(&due.key).into(),
                group: GROUP,
                xml: toast_xml(&toast),
                expires: event
                    .end
                    .with_timezone(&Utc)
                    .max(now + Duration::minutes(15)),
                event: Some((event.id.clone(), due.start)),
            });
            let setting = backend.setting().ok().flatten();
            self.shared.update(|status| {
                status.windows = setting.or(status.windows);
                match (&shown, &saved) {
                    (Err(error), _) => status.error = Some(error.clone()),
                    (Ok(()), Some(Err(error))) => {
                        status.last_reminder = Some(now);
                        status.error = Some(format!(
                            "Meeting reminder history could not be saved: {error}"
                        ));
                    }
                    (Ok(()), _) => {
                        status.error = None;
                        status.last_reminder = Some(now);
                    }
                }
            });
        }
    }

    fn test(&self, backend: &mut platform::Backend, language: Language) {
        let now = Utc::now();
        let toast = sample(now, language, &Local);
        let shown = backend.show(platform::Request {
            tag: TEST_TAG.into(),
            group: TEST_GROUP,
            xml: toast_xml(&toast),
            expires: now + Duration::hours(1),
            event: None,
        });
        let setting = backend.setting().ok().flatten();
        self.shared.update(|status| {
            status.windows = setting.or(status.windows);
            match shown {
                Ok(()) => {
                    status.error = None;
                    status.last_test = Some(now);
                }
                Err(error) => status.error = Some(error),
            }
        });
    }
}

#[cfg(windows)]
mod platform {
    use super::{ICON_FILE, Mode, OpenHandler, Shared, WindowsSetting};
    use chrono::{DateTime, Utc};
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
        sync::Arc,
    };
    use windows::{
        Data::Xml::Dom::XmlDocument,
        Foundation::{DateTime as WindowsTime, IReference, PropertyValue, TypedEventHandler},
        UI::Notifications::{
            NotificationSetting, ToastActivatedEventArgs, ToastFailedEventArgs, ToastNotification,
            ToastNotificationManager, ToastNotifier,
        },
        Win32::{
            Foundation::{CLASS_E_NOAGGREGATION, E_POINTER, ERROR_NOT_FOUND, ERROR_SUCCESS},
            System::{
                Com::{
                    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoInitializeEx,
                    CoRegisterClassObject, CoRevokeClassObject, CoUninitialize, IClassFactory,
                    IClassFactory_Impl, REGCLS_MULTIPLEUSE,
                },
                Registry::{
                    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyW, RegGetValueW,
                    RegSetKeyValueW,
                },
            },
            UI::{
                Notifications::{
                    INotificationActivationCallback, INotificationActivationCallback_Impl,
                    NOTIFICATION_USER_INPUT_DATA,
                },
                Shell::SetCurrentProcessExplicitAppUserModelID,
            },
        },
        core::{BOOL, GUID, HSTRING, IInspectable, IUnknown, Interface, PCWSTR, Ref, implement},
    };

    /// Receives notification-center clicks through COM while Tomlook runs.
    #[implement(INotificationActivationCallback)]
    struct Activator {
        open: OpenHandler,
        shared: Arc<Shared>,
    }

    impl INotificationActivationCallback_Impl for Activator_Impl {
        fn Activate(
            &self,
            _: &PCWSTR,
            arguments: &PCWSTR,
            _: *const NOTIFICATION_USER_INPUT_DATA,
            _: u32,
        ) -> windows::core::Result<()> {
            let arguments = if arguments.is_null() {
                String::new()
            } else {
                // Windows passes a null-terminated argument string valid for this call.
                unsafe { arguments.to_string() }.unwrap_or_default()
            };
            self.shared.activate(&self.open, &arguments);
            Ok(())
        }
    }

    #[implement(IClassFactory)]
    struct Factory {
        open: OpenHandler,
        shared: Arc<Shared>,
    }

    impl IClassFactory_Impl for Factory_Impl {
        fn CreateInstance(
            &self,
            outer: Ref<IUnknown>,
            iid: *const GUID,
            object: *mut *mut core::ffi::c_void,
        ) -> windows::core::Result<()> {
            if object.is_null() || iid.is_null() {
                return Err(E_POINTER.into());
            }
            // COM provides a writable out pointer that must be cleared on failure.
            unsafe { *object = std::ptr::null_mut() };
            if !outer.is_null() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let callback: INotificationActivationCallback = Activator {
                open: self.open.clone(),
                shared: self.shared.clone(),
            }
            .into();
            // Standard QueryInterface into the caller's out pointer.
            unsafe { callback.query(iid, object) }.ok()
        }

        fn LockServer(&self, _: BOOL) -> windows::core::Result<()> {
            Ok(())
        }
    }

    struct ClassObject(u32);

    impl Drop for ClassObject {
        fn drop(&mut self) {
            // Revokes this thread's own registration before COM is uninitialized.
            let _ = unsafe { CoRevokeClassObject(self.0) };
        }
    }

    fn clsid(value: u128) -> String {
        let hex = format!("{value:032X}");
        format!(
            "{{{}-{}-{}-{}-{}}}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        )
    }

    pub(super) fn set_process_identity(mode: Mode) -> Result<(), String> {
        // The HSTRING outlives the synchronous call.
        unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(mode.aumid())) }
            .map_err(|error| format!("Set the Windows notification identity: {error}"))
    }

    fn registry_key(mode: Mode) -> HSTRING {
        HSTRING::from(format!(
            "Software\\Classes\\AppUserModelId\\{}",
            mode.aumid()
        ))
    }

    fn read_string(key: &HSTRING, name: &str) -> Option<String> {
        let name = HSTRING::from(name);
        let mut size = 0_u32;
        // Size query only; both strings stay alive for the call.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key,
                &name,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS || size == 0 || size > 64 * 1024 {
            return None;
        }
        let mut buffer = vec![0_u16; (size as usize).div_ceil(2)];
        // The buffer holds `size` bytes as reported by the previous call.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key,
                &name,
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16(&buffer[..length]).ok()
    }

    fn write_string(key: &HSTRING, name: &str, value: &str) -> Result<(), String> {
        let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        let bytes = u32::try_from(data.len() * 2).map_err(|_| "Registry value is too long")?;
        // RegSetKeyValueW creates the per-user key when missing; data includes the terminator.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key,
                &HSTRING::from(name),
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                bytes,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "Register Tomlook for Windows notifications: {}",
                std::io::Error::from_raw_os_error(status.0 as i32)
            ))
        }
    }

    fn windows_time(at: DateTime<Utc>) -> Result<IReference<WindowsTime>, String> {
        let ticks = at
            .timestamp()
            .checked_mul(10_000_000)
            .and_then(|ticks| ticks.checked_add(i64::from(at.timestamp_subsec_nanos() / 100)))
            .and_then(|ticks| ticks.checked_add(116_444_736_000_000_000))
            .ok_or("Notification expiry is outside the Windows time range")?;
        PropertyValue::CreateDateTime(WindowsTime {
            UniversalTime: ticks,
        })
        .and_then(|value| value.cast())
        .map_err(|error| format!("Set notification expiry: {error}"))
    }

    struct Com(bool);

    impl Com {
        fn init() -> Self {
            // Multithreaded apartment for this worker thread only.
            Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // Balanced with the successful CoInitializeEx on the same thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    pub(super) struct Request {
        pub tag: String,
        pub group: &'static str,
        pub xml: String,
        pub expires: DateTime<Utc>,
        /// Event id and start for meeting reminders; `None` for the test notification.
        pub event: Option<(String, DateTime<Utc>)>,
    }

    struct Shown {
        group: &'static str,
        event: Option<(String, DateTime<Utc>)>,
        expires: DateTime<Utc>,
        _toast: ToastNotification,
    }

    pub(super) struct Backend {
        mode: Mode,
        aumid: HSTRING,
        icon: PathBuf,
        open: OpenHandler,
        shared: Arc<Shared>,
        notifier: Option<ToastNotifier>,
        // Toasts stay referenced so their in-process activation handlers remain registered.
        shown: BTreeMap<String, Shown>,
        class: Option<ClassObject>,
        // Declared last: COM is released only after every toast reference above.
        _com: Com,
    }

    impl Backend {
        pub(super) fn new(
            mode: Mode,
            state: &Path,
            open: OpenHandler,
            shared: Arc<Shared>,
        ) -> Self {
            let com = Com::init();
            Self {
                mode,
                aumid: HSTRING::from(mode.aumid()),
                icon: state.join(ICON_FILE),
                open,
                shared,
                notifier: None,
                shown: BTreeMap::new(),
                class: None,
                _com: com,
            }
        }

        /// Accepts notification-center clicks while this thread runs; nothing is launched otherwise.
        pub(super) fn listen(&mut self) -> Result<(), String> {
            let factory: IClassFactory = Factory {
                open: self.open.clone(),
                shared: self.shared.clone(),
            }
            .into();
            // COM keeps its own factory reference until the cookie is revoked.
            let cookie = unsafe {
                CoRegisterClassObject(
                    &GUID::from_u128(self.mode.activator()),
                    &factory,
                    CLSCTX_LOCAL_SERVER,
                    REGCLS_MULTIPLEUSE,
                )
            }
            .map_err(|error| {
                format!(
                    "Register Tomlook for Windows notifications: {}",
                    error.message()
                )
            })?;
            self.class = Some(ClassObject(cookie));
            Ok(())
        }

        /// Writes the icon and the per-user AppUserModelId entry only when they differ.
        pub(super) fn register(&self) -> Result<(), String> {
            let png = super::icon_png()?;
            if std::fs::read(&self.icon).ok().as_deref() != Some(png.as_slice()) {
                if let Some(parent) = self.icon.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        format!("Register Tomlook for Windows notifications: {error}")
                    })?;
                }
                std::fs::write(&self.icon, &png).map_err(|error| {
                    format!("Register Tomlook for Windows notifications: {error}")
                })?;
            }
            let icon = self.icon.to_str().ok_or(
                "Register Tomlook for Windows notifications: the icon path is not valid Unicode",
            )?;
            let key = registry_key(self.mode);
            let activator = clsid(self.mode.activator());
            for (name, value) in [
                ("DisplayName", self.mode.display_name()),
                ("IconUri", icon),
                ("CustomActivator", activator.as_str()),
            ] {
                if read_string(&key, name).as_deref() != Some(value) {
                    write_string(&key, name, value)?;
                }
            }
            Ok(())
        }

        fn notifier(&mut self) -> Result<&ToastNotifier, String> {
            if self.notifier.is_none() {
                self.notifier = Some(
                    ToastNotificationManager::CreateToastNotifierWithId(&self.aumid).map_err(
                        |error| format!("Show the Windows notification: {}", error.message()),
                    )?,
                );
            }
            Ok(self.notifier.as_ref().expect("notifier was created"))
        }

        /// `None` until Windows has seen this identity's first notification.
        pub(super) fn setting(&mut self) -> Result<Option<WindowsSetting>, String> {
            let setting = match self.notifier()?.Setting() {
                Ok(setting) => setting,
                Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => return Ok(None),
                Err(error) => {
                    return Err(format!(
                        "Show the Windows notification: {}",
                        error.message()
                    ));
                }
            };
            Ok(Some(match setting {
                NotificationSetting::Enabled => WindowsSetting::Enabled,
                NotificationSetting::DisabledForApplication => {
                    WindowsSetting::DisabledForApplication
                }
                NotificationSetting::DisabledForUser => WindowsSetting::DisabledForUser,
                NotificationSetting::DisabledByGroupPolicy => WindowsSetting::DisabledByGroupPolicy,
                NotificationSetting::DisabledByManifest => WindowsSetting::DisabledByManifest,
                _ => WindowsSetting::Unknown,
            }))
        }

        pub(super) fn show(&mut self, request: Request) -> Result<(), String> {
            self.register()?;
            let failure = |error: windows::core::Error| {
                format!("Show the Windows notification: {}", error.message())
            };
            let document = XmlDocument::new().map_err(failure)?;
            document
                .LoadXml(&HSTRING::from(request.xml.as_str()))
                .map_err(failure)?;
            let toast = ToastNotification::CreateToastNotification(&document).map_err(failure)?;
            toast
                .SetTag(&HSTRING::from(request.tag.as_str()))
                .map_err(failure)?;
            toast
                .SetGroup(&HSTRING::from(request.group))
                .map_err(failure)?;
            toast
                .SetExpirationTime(&windows_time(request.expires)?)
                .map_err(failure)?;
            let open = self.open.clone();
            let shared = self.shared.clone();
            toast
                .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
                    move |_, arguments| {
                        let arguments = arguments
                            .as_ref()
                            .and_then(|arguments| arguments.cast::<ToastActivatedEventArgs>().ok())
                            .and_then(|arguments| arguments.Arguments().ok())
                            .map(|arguments| arguments.to_string())
                            .unwrap_or_default();
                        shared.activate(&open, &arguments);
                        Ok(())
                    },
                ))
                .map_err(failure)?;
            let shared = self.shared.clone();
            toast
                .Failed(
                    &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(
                        move |_, arguments| {
                            let detail = arguments
                                .as_ref()
                                .and_then(|arguments| arguments.ErrorCode().ok())
                                .map(|code| code.message())
                                .unwrap_or_default();
                            shared.update(|status| {
                                status.error =
                                    Some(format!("Windows notification failed: {detail}"))
                            });
                            Ok(())
                        },
                    ),
                )
                .map_err(failure)?;
            self.activations().insert(
                request.tag.clone(),
                request.event.as_ref().map(|(id, _)| id.clone()),
            );
            self.notifier()?.Show(&toast).map_err(failure)?;
            self.shown.insert(
                request.tag,
                Shown {
                    group: request.group,
                    event: request.event,
                    expires: request.expires,
                    _toast: toast,
                },
            );
            Ok(())
        }

        /// Removes shown reminders whose meeting moved, disappeared or is no longer eligible.
        pub(super) fn retract(&mut self, keep: impl Fn(&str, DateTime<Utc>) -> bool) {
            let stale: Vec<String> = self
                .shown
                .iter()
                .filter(|(_, shown)| {
                    shown
                        .event
                        .as_ref()
                        .is_some_and(|(id, start)| !keep(id, *start))
                })
                .map(|(tag, _)| tag.clone())
                .collect();
            if stale.is_empty() {
                return;
            }
            let history = ToastNotificationManager::History();
            for tag in stale {
                self.activations().remove(&tag);
                if let Some(shown) = self.shown.remove(&tag)
                    && let Ok(history) = &history
                {
                    let _ = history.RemoveGroupedTagWithId(
                        &HSTRING::from(tag.as_str()),
                        &HSTRING::from(shown.group),
                        &self.aumid,
                    );
                }
            }
        }

        fn activations(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Option<String>>> {
            self.shared
                .activations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }

        pub(super) fn prune(&mut self, now: DateTime<Utc>) {
            self.shown.retain(|_, shown| shown.expires > now);
            let shown = &self.shown;
            self.activations().retain(|tag, _| shown.contains_key(tag));
        }

        /// Demo sessions leave no notifications or registration behind.
        pub(super) fn cleanup(&mut self) {
            self.shown.clear();
            self.activations().clear();
            self.class = None;
            if let Ok(history) = ToastNotificationManager::History() {
                let _ = history.ClearWithId(&self.aumid);
            }
            // Removes only the demo identity's own value-only key.
            let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &registry_key(self.mode)) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Mode, OpenHandler, Shared, WindowsSetting};
    use chrono::{DateTime, Utc};
    use std::{path::Path, sync::Arc};

    const UNAVAILABLE: &str = "Windows notifications require Windows";

    pub(super) fn set_process_identity(_: Mode) -> Result<(), String> {
        Ok(())
    }

    pub(super) struct Request {
        pub tag: String,
        pub group: &'static str,
        pub xml: String,
        pub expires: DateTime<Utc>,
        pub event: Option<(String, DateTime<Utc>)>,
    }

    pub(super) struct Backend;

    impl Backend {
        pub(super) fn new(_: Mode, _: &Path, _: OpenHandler, _: Arc<Shared>) -> Self {
            Self
        }

        pub(super) fn register(&self) -> Result<(), String> {
            Err(UNAVAILABLE.into())
        }

        pub(super) fn listen(&mut self) -> Result<(), String> {
            Err(UNAVAILABLE.into())
        }

        pub(super) fn setting(&mut self) -> Result<Option<WindowsSetting>, String> {
            Err(UNAVAILABLE.into())
        }

        pub(super) fn show(&mut self, _: Request) -> Result<(), String> {
            Err(UNAVAILABLE.into())
        }

        pub(super) fn retract(&mut self, _: impl Fn(&str, DateTime<Utc>) -> bool) {}

        pub(super) fn prune(&mut self, _: DateTime<Utc>) {}

        pub(super) fn cleanup(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn zone() -> FixedOffset {
        FixedOffset::east_opt(3600).unwrap()
    }

    fn at(hour: u32, minute: u32) -> DateTime<FixedOffset> {
        zone()
            .with_ymd_and_hms(2026, 11, 2, hour, minute, 0)
            .unwrap()
    }

    fn meeting(id: &str, start: DateTime<FixedOffset>, status: &str) -> Event {
        Event {
            id: id.into(),
            title: "Customer sync".into(),
            start,
            end: start + Duration::minutes(90),
            category: String::new(),
            attendees: vec![],
            organizer: String::new(),
            location: String::new(),
            status: status.into(),
            briefing_status: String::new(),
            is_all_day: false,
            source_url: None,
            extra: BTreeMap::new(),
        }
    }

    fn calendar(events: Vec<Event>) -> Calendar {
        Calendar::build(events, BTreeSet::new(), Vec::new()).unwrap()
    }

    fn settings(minutes: u8) -> Settings {
        Settings {
            enabled: true,
            minutes,
            tentative: false,
            language: Language::En,
        }
    }

    #[test]
    fn only_timed_confirmed_meetings_are_eligible_and_tentative_is_optional() {
        let start = at(9, 15);
        let mut organizer = meeting("organizer", start, "none");
        organizer.extra.insert("isOrganizer".into(), json!(true));
        assert!(eligible(&organizer, false));
        assert!(eligible(&meeting("accepted", start, "accepted"), false));
        assert!(eligible(
            &meeting("organizer-status", start, "organizer"),
            false
        ));
        for status in ["none", "notResponded", "declined", ""] {
            assert!(!eligible(&meeting(status, start, status), true), "{status}");
        }
        let tentative = meeting("tentative", start, "tentativelyAccepted");
        assert!(!eligible(&tentative, false));
        assert!(eligible(&tentative, true));
        let mut shown_tentative = meeting("shown-tentative", start, "accepted");
        shown_tentative
            .extra
            .insert("showAs".into(), json!("tentative"));
        assert!(!eligible(&shown_tentative, false));
        assert!(eligible(&shown_tentative, true));
        let mut uncertain = meeting("uncertain", start, "accepted");
        uncertain
            .extra
            .insert("categories".into(), json!(["Nejisté"]));
        assert!(!eligible(&uncertain, false));
        let mut all_day = meeting("all-day", start, "accepted");
        all_day.is_all_day = true;
        let mut cancelled = meeting("cancelled", start, "accepted");
        cancelled.extra.insert("isCancelled".into(), json!(true));
        let mut free = meeting("free", start, "accepted");
        free.extra.insert("showAs".into(), json!("free"));
        let mut followed = meeting("followed", start, "accepted");
        followed
            .extra
            .insert("categories".into(), json!(["Follow"]));
        let mut following = meeting("following", start, "organizer");
        following.extra.insert("isFollowing".into(), json!(true));
        for event in [all_day, cancelled, free, followed, following] {
            assert!(!eligible(&event, true), "{}", event.id);
        }
    }

    #[test]
    fn next_due_respects_lead_time_late_window_and_grace_period() {
        let calendar = calendar(vec![
            meeting("first", at(9, 15), "accepted"),
            meeting("second", at(11, 0), "accepted"),
            meeting("declined", at(10, 0), "declined"),
        ]);
        let now = at(8, 0).to_utc();
        let mut planner = Planner::default();
        planner.replace(schedule(&calendar, &settings(5), now));
        assert_eq!(planner.next_wake(now), Some(at(9, 10).to_utc()));
        assert!(planner.due(now).is_empty());
        assert!(planner.due(at(9, 9).to_utc()).is_empty());
        let due = planner.due(at(9, 10).to_utc());
        assert_eq!(
            due.iter().map(|due| due.id.as_str()).collect::<Vec<_>>(),
            ["first"]
        );
        // Woken late after sleep, but before the start: still shown once.
        let late = at(9, 13).to_utc();
        assert_eq!(planner.due(late).len(), 1);
        planner.mark(&due[0], late);
        assert!(planner.due(late).is_empty());
        assert_eq!(planner.next_wake(late), Some(at(10, 55).to_utc()));
        // More than two minutes after the start the reminder is skipped.
        let mut skipped = Planner::default();
        skipped.replace(schedule(&calendar, &settings(5), at(8, 0).to_utc()));
        assert_eq!(skipped.due(at(9, 16).to_utc()).len(), 1);
        assert!(
            skipped
                .due(at(9, 17).to_utc())
                .iter()
                .all(|due| due.id != "first")
        );
        assert!(
            schedule(&calendar, &settings(5), at(9, 18).to_utc())
                .iter()
                .all(|due| due.id != "first")
        );
        let disabled = Settings {
            enabled: false,
            ..settings(5)
        };
        assert!(schedule(&calendar, &disabled, now).is_empty());
        assert_eq!(
            schedule(&calendar, &settings(15), now)[0].at,
            at(9, 0).to_utc()
        );
    }

    #[test]
    fn moved_meetings_are_new_occurrences_and_history_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let original = meeting("weekly", at(9, 15), "accepted");
        let now = at(9, 11).to_utc();
        let mut planner = Planner::default();
        planner.replace(schedule(
            &calendar(vec![original.clone()]),
            &settings(5),
            now,
        ));
        let due = planner.due(now);
        planner.mark(&due[0], now);
        Store::open(root.path())
            .unwrap()
            .save_fired_reminders(planner.fired())
            .unwrap();
        let saved = Store::open(root.path()).unwrap().fired_reminders().unwrap();
        let mut restarted = Planner::with_fired(saved, now);
        restarted.replace(schedule(
            &calendar(vec![original.clone()]),
            &settings(5),
            now,
        ));
        assert!(
            restarted.due(now).is_empty(),
            "an occurrence is shown at most once"
        );
        let mut moved = original;
        moved.start = at(9, 14);
        moved.end = moved.start + Duration::minutes(30);
        restarted.replace(schedule(&calendar(vec![moved]), &settings(5), now));
        assert_eq!(
            restarted.due(now).len(),
            1,
            "a moved meeting gets its own reminder"
        );
        let pruned = Planner::with_fired(
            restarted.fired().clone(),
            now + Duration::hours(RETENTION_HOURS + 1),
        );
        assert!(pruned.fired().is_empty());
    }

    #[test]
    fn titles_show_remaining_minutes_or_now_in_both_languages() {
        let (start, end) = (at(9, 15), at(10, 45));
        let title = |now: DateTime<FixedOffset>, language| {
            title(start, end, now.to_utc(), language, &zone())
        };
        assert_eq!(title(at(9, 10), Language::En), "In 5 min · 09:15–10:45");
        assert_eq!(title(at(9, 10), Language::Cs), "Za 5 min · 09:15–10:45");
        assert_eq!(title(at(9, 12), Language::En), "In 3 min · 09:15–10:45");
        assert_eq!(title(at(9, 15), Language::En), "Now · 09:15–10:45");
        assert_eq!(title(at(9, 16), Language::Cs), "Právě začíná · 09:15–10:45");
        let multi_day = time_range(start, start + Duration::days(1), Language::Cs, &zone());
        assert_eq!(multi_day, "09:15–3. 11. 09:15");
        assert_eq!(
            time_range(
                at(23, 0),
                at(23, 0) + Duration::hours(1),
                Language::En,
                &zone()
            ),
            "23:00–00:00"
        );
    }

    #[test]
    fn bullets_prefer_next_steps_and_use_the_selected_language() {
        let insights = json!({
            "summary": "Overview",
            "preparation": ["Demo: Open the prepared demo and check   the\nrecording"],
            "talking_points": ["Pricing: Explain the pilot", "Pricing: Explain the pilot"],
            "open_questions": ["Owner: Who signs off?", "Unused question"],
            "cs": {
                "preparation": ["Ukázka: Otevřete připravenou ukázku"],
                "talking_points": [],
                "open_questions": ["Vlastník: Kdo schvaluje?"]
            }
        });
        assert_eq!(
            bullets(&insights, Language::En),
            [
                "Demo: Open the prepared demo and check the recording",
                "Pricing: Explain the pilot",
                "Owner: Who signs off?"
            ]
        );
        assert_eq!(
            bullets(&insights, Language::Cs),
            [
                "Ukázka: Otevřete připravenou ukázku",
                "Vlastník: Kdo schvaluje?"
            ]
        );
        let english_only = json!({"preparation": ["Agenda: Confirm scope"]});
        assert_eq!(
            bullets(&english_only, Language::Cs),
            ["Agenda: Confirm scope"]
        );
        assert!(bullets(&json!({"summary": "Only a summary"}), Language::En).is_empty());
        let long = "Příprava: ".to_string() + &"žluťoučký kůň ".repeat(20);
        let short = &bullets(&json!({"preparation": [long]}), Language::En)[0];
        assert_eq!(short.chars().count(), BULLET_CHARS);
        assert!(short.ends_with('…') && !short.ends_with(" …"));
    }

    #[test]
    fn reminder_content_uses_original_title_location_insights_and_safe_join_link() {
        let mut event = meeting("teams", at(9, 15), "accepted");
        event.title = "Zákazník <Contoso> & \"partneři\"".into();
        event.location = "Microsoft Teams Meeting".into();
        event.extra.insert(
            "onlineMeeting".into(),
            json!({"joinUrl": "https://teams.microsoft.com/l/meetup-join/19%3a1?context=%7b%7d&x=1"}),
        );
        let insights = json!({"preparation": ["Demo: Bring the <prototype>"]});
        let toast = reminder(
            &event,
            Some(&insights),
            &settings(5),
            at(9, 10).to_utc(),
            &zone(),
        );
        assert_eq!(toast.meeting, event.title);
        assert_eq!(toast.location.as_deref(), Some("Microsoft Teams Meeting"));
        assert_eq!(toast.bullets, ["Demo: Bring the <prototype>"]);
        assert!(toast.arguments.starts_with("open:") && toast.arguments.len() == 37);
        assert_eq!(
            activation_tag(&toast.arguments),
            Some(&toast.arguments[5..])
        );
        assert_eq!(activation_tag(""), Some(""));
        let xml = toast_xml(&toast);
        assert!(xml.contains("Zákazník &lt;Contoso&gt; &amp; &quot;partneři&quot;"));
        assert!(xml.contains("• Demo: Bring the &lt;prototype&gt;"));
        assert!(xml.contains(
            r#"arguments="https://teams.microsoft.com/l/meetup-join/19%3a1?context=%7b%7d&amp;x=1""#
        ));
        assert!(!xml.contains("<Contoso>") && !xml.contains("<prototype>"));
        assert!(xml.contains(r#"scenario="reminder""#));
        assert!(xml.contains(r#"arguments="snooze" hint-inputId="snoozeTime""#));
        assert!(
            !xml.contains(r#"arguments="dismiss""#),
            "the toast's own × dismisses it"
        );
        assert_eq!(xml.matches("<action ").count(), 3);
        let czech = Settings {
            language: Language::Cs,
            ..settings(5)
        };
        let xml = toast_xml(&reminder(&event, None, &czech, at(9, 10).to_utc(), &zone()));
        assert!(xml.contains(r#"content="Připojit""#) && xml.contains(r#"content="Otevřít""#));
        assert!(xml.contains("Za 5 min · 09:15–10:45") && !xml.contains("<group>"));
        event.extra.insert(
            "onlineMeeting".into(),
            json!({"joinUrl": "javascript:alert(1)"}),
        );
        let xml = toast_xml(&reminder(
            &event,
            None,
            &settings(5),
            at(9, 10).to_utc(),
            &zone(),
        ));
        assert!(!xml.contains("activationType=\"protocol\""));
        assert!(xml.contains(r#"arguments="dismiss""#));
        assert_eq!(xml.matches("<action ").count(), 3);
    }

    #[test]
    fn toast_clicks_open_the_shown_meeting_or_only_restore_tomlook() {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let seen = opened.clone();
        let open: OpenHandler = Arc::new(move |meeting| seen.lock().unwrap().push(meeting));
        let shared = Shared {
            status: Mutex::new(Status::default()),
            pending: Mutex::new(Pending::default()),
            stop: AtomicBool::new(false),
            notify: Box::new(|| {}),
            activations: Mutex::new(BTreeMap::from([
                ("a".repeat(32), Some("meeting-id".to_string())),
                (TEST_TAG.to_string(), None),
            ])),
        };
        for arguments in [
            format!("open:{}", "a".repeat(32)),
            "open:test".into(),
            format!("open:{}", "b".repeat(32)),
            String::new(),
            "snooze".into(),
        ] {
            shared.activate(&open, &arguments);
        }
        assert_eq!(
            *opened.lock().unwrap(),
            [Some("meeting-id".to_string()), None, None, None]
        );
        assert_eq!(activation_tag("open:abc"), Some("abc"));
        assert_eq!(activation_tag("dismiss"), None);
    }

    #[test]
    fn escaping_removes_invalid_xml_characters_and_sample_is_labelled() {
        assert_eq!(escape("a\u{1}b\u{fffe}<'>"), "ab&lt;&apos;&gt;");
        assert_eq!(shorten(" Line\u{7}one\n two ", 80), "Lineone two");
        for language in [Language::En, Language::Cs] {
            let toast = sample(at(9, 10).to_utc(), language, &zone());
            assert!(
                toast
                    .title
                    .starts_with(language.text("Test notification").as_ref())
            );
            assert!(toast.title.ends_with("· 09:15–09:45"), "{}", toast.title);
            assert_eq!(toast.bullets.len(), 3);
            assert_eq!(toast.arguments, "open:test");
            assert_eq!(activation_tag(&toast.arguments), Some(TEST_TAG));
        }
        let czech = sample(at(9, 10).to_utc(), Language::Cs, &zone());
        assert!(czech.title.contains("Za 5 min"));
        assert_eq!(czech.meeting, "Ukázková schůzka (smyšlená)");
        for text in [
            "Join",
            "Open",
            "1 minute",
            "2 minutes",
            "5 minutes",
            "10 minutes",
            "15 minutes",
        ]
        .into_iter()
        .chain(SAMPLE_BULLETS)
        {
            assert_ne!(Language::Cs.text(text), text, "Missing Czech text: {text}");
        }
    }

    #[test]
    fn preferences_default_to_five_minute_reminders_for_confirmed_meetings() {
        let old: Preferences = serde_json::from_str(r#"{"dark":false,"language":"Cs"}"#).unwrap();
        assert!(old.meeting_reminders);
        assert_eq!(old.reminder_minutes, 5);
        assert!(!old.remind_tentative);
        let invalid: Preferences = serde_json::from_str(r#"{"reminder_minutes":7}"#).unwrap();
        let settings = Settings::from_preferences(&invalid);
        assert_eq!(settings.minutes, DEFAULT_MINUTES);
        assert_eq!(normalize_minutes(15), 15);
    }

    #[test]
    fn notification_icon_is_a_decodable_brand_png() {
        let png = icon_png().unwrap();
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        assert_eq!(image.dimensions(), (ICON_SIZE, ICON_SIZE));
        assert_eq!(image.into_raw(), crate::brand::rgba(ICON_SIZE as usize));
    }

    #[test]
    #[cfg(windows)]
    fn toast_xml_is_well_formed_for_windows_without_showing_it() {
        use windows::{Data::Xml::Dom::XmlDocument, core::HSTRING};
        let mut event = meeting("xml", at(9, 15), "accepted");
        event.title = "A & B <C> \"D\" 'E' \u{1}".into();
        event.location = "Room <1>".into();
        event.extra.insert(
            "onlineMeeting".into(),
            json!({"joinUrl": "https://teams.microsoft.com/l/meetup-join/a?b=1&c=2"}),
        );
        let insights =
            json!({"preparation": ["One & <two>", "Three"], "open_questions": ["Four?"]});
        for language in [Language::En, Language::Cs] {
            let settings = Settings {
                language,
                ..settings(5)
            };
            for toast in [
                reminder(
                    &event,
                    Some(&insights),
                    &settings,
                    at(9, 10).to_utc(),
                    &zone(),
                ),
                sample(at(9, 10).to_utc(), language, &zone()),
            ] {
                let document = XmlDocument::new().unwrap();
                document.LoadXml(&HSTRING::from(toast_xml(&toast))).unwrap();
                let texts = document
                    .GetElementsByTagName(&HSTRING::from("text"))
                    .unwrap();
                assert_eq!(
                    texts.Item(1).unwrap().InnerText().unwrap().to_string(),
                    toast.meeting
                );
                assert!(texts.Length().unwrap() >= 3);
            }
        }
    }
}
