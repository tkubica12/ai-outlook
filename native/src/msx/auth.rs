//! Memory-only tokens for the MSX resources.
//!
//! Dataverse tokens come from Tomlook's saved browser sign-in ([`super::oauth`]); Power BI
//! tokens come from the Azure CLI. Background reads use a short hard deadline and never retry:
//! without a fresh cache a `get-access-token` can wait indefinitely for interactive (WAM)
//! authentication. Only an explicit sign-in may open a browser or start `az login`.
use super::{Resource, oauth::Grant};
use base64::Engine as _;
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::{Mutex, watch},
};

pub const BACKGROUND_TIMEOUT: Duration = Duration::from_secs(20);
const BROWSER_LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const DEVICE_LOGIN_TIMEOUT: Duration = Duration::from_secs(960);
const DEVICE_CODE_WAIT: Duration = Duration::from_secs(60);
/// Tokens are renewed this long before their expiry.
pub const REFRESH_MARGIN: i64 = 300;

pub const SIGN_IN_REQUIRED: &str =
    "MSX sign-in required. Choose Sign in to MSX; Tomlook never opens a sign-in prompt on its own.";
pub const TOKEN_TIMEOUT: &str =
    "MSX sign-in required: Azure CLI did not return a cached token within 20 seconds.";
pub const CLI_MISSING: &str =
    "Azure CLI is not installed. Install the Azure CLI (MSI) to connect MSX.";
pub const CLI_FAILED: &str = "Azure CLI could not start. Check its installation.";
pub const TENANT_MISMATCH: &str =
    "Azure CLI returned a token for a different tenant; MSX was not connected.";
pub const INVALID_TOKEN: &str =
    "Azure CLI returned an unusable token response; MSX was not connected.";
pub const INVALID_ACCOUNTS: &str =
    "Azure CLI returned an unusable account list; MSX was not connected.";
pub const AMBIGUOUS_ACCOUNTS: &str =
    "Multiple Azure CLI identities are available for the MSX tenant. Sign in to MSX to choose one.";
pub const NETWORK: &str =
    "Azure CLI could not reach Microsoft sign-in; check connectivity. Retrying automatically.";
pub const LOGIN_FAILED: &str = "MSX sign-in failed or was blocked by tenant policy. Try again or use the other sign-in method.";
pub const LOGIN_CANCELLED: &str = "MSX sign-in cancelled.";
pub const LOGIN_TIMED_OUT: &str = "MSX sign-in timed out. Start sign-in again.";
pub const REFRESH_TIMEOUT: &str = "MSX sign-in refresh timed out; retrying automatically.";
pub const DEVICE_CODE_TIMEOUT: &str = "Azure CLI did not provide a device code within 60 seconds. Your tenant may block device-code sign-in; use Sign in to MSX instead.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// An explicit user sign-in is needed; never retried automatically.
    SignInRequired(String),
    /// Local CLI or network problem; only `transient` failures are retried, with backoff.
    Unavailable { message: String, transient: bool },
}

impl AuthError {
    pub fn message(&self) -> &str {
        match self {
            Self::SignInRequired(message) | Self::Unavailable { message, .. } => message,
        }
    }

    pub(super) fn unavailable(message: &str) -> Self {
        Self::Unavailable {
            message: message.into(),
            transient: false,
        }
    }
}

pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    Missing,
    Spawn,
    Timeout,
    Io,
}

pub type Progress = Arc<dyn Fn(String) -> Result<(), String> + Send + Sync>;

/// Runs the Azure CLI; the fake in tests never starts a process.
#[async_trait::async_trait]
pub trait Runner: Send + Sync {
    /// Non-interactive command with bounded output and a hard deadline.
    async fn run(&self, args: Vec<String>, timeout: Duration) -> Result<Output, RunError>;
    /// Explicit sign-in. Dropping the future ends the CLI process.
    async fn login(
        &self,
        args: Vec<String>,
        device_code: bool,
        progress: Progress,
    ) -> Result<(), String>;
}

pub struct CliRunner {
    cli: Option<PathBuf>,
}

impl CliRunner {
    pub fn installed() -> Self {
        Self {
            cli: crate::azure_cli::installed(),
        }
    }

    fn command(&self, args: &[String]) -> Result<tokio::process::Command, RunError> {
        let cli = self.cli.as_ref().ok_or(RunError::Missing)?;
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        crate::azure_cli::command(cli, &args).map_err(|()| RunError::Spawn)
    }
}

async fn read_bounded(reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, RunError> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| RunError::Io)?;
    if bytes.len() > limit {
        return Err(RunError::Io);
    }
    Ok(bytes)
}

async fn login_output(
    mut reader: impl AsyncRead + Unpin,
    progress: &(dyn Fn(String) -> Result<(), String> + Send + Sync),
) -> Result<(), String> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    let mut reported = false;
    loop {
        let count = reader.read(&mut chunk).await.map_err(|_| LOGIN_FAILED)?;
        if count == 0 {
            return Ok(());
        }
        if bytes.len() + count > 16_384 {
            return Err(LOGIN_FAILED.into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if !reported && let Some(code) = crate::azure_cli::device_code(&bytes) {
            reported = true;
            progress(code)?;
        }
    }
}

#[async_trait::async_trait]
impl Runner for CliRunner {
    async fn run(&self, args: Vec<String>, timeout: Duration) -> Result<Output, RunError> {
        let account_list = args.first().is_some_and(|arg| arg == "account")
            && args.get(1).is_some_and(|arg| arg == "list");
        let stdout_limit = if account_list { 1_048_576 } else { 65_536 };
        let mut child = self.command(&args)?.spawn().map_err(|_| RunError::Spawn)?;
        let stdout = child.stdout.take().ok_or(RunError::Io)?;
        let stderr = child.stderr.take().ok_or(RunError::Io)?;
        let collect = async {
            let (stdout, stderr) = tokio::try_join!(
                read_bounded(stdout, stdout_limit),
                read_bounded(stderr, 16_384)
            )?;
            let status = child.wait().await.map_err(|_| RunError::Io)?;
            Ok::<_, RunError>(Output {
                success: status.success(),
                stdout,
                stderr,
            })
        };
        let result = tokio::time::timeout(timeout, collect).await;
        match result {
            Ok(result) => result,
            Err(_) => {
                crate::azure_cli::kill_tree(&mut child).await;
                Err(RunError::Timeout)
            }
        }
    }

    async fn login(
        &self,
        args: Vec<String>,
        device_code: bool,
        progress: Progress,
    ) -> Result<(), String> {
        let mut command = self.command(&args).map_err(|error| {
            if error == RunError::Missing {
                CLI_MISSING
            } else {
                CLI_FAILED
            }
        })?;
        // The subscription picker would wait for console input that never arrives.
        command.env("AZURE_CORE_LOGIN_EXPERIENCE_V2", "off");
        if device_code {
            command.env("AZURE_CORE_ENABLE_BROKER_ON_WINDOWS", "false");
        }
        let mut child = command.spawn().map_err(|_| CLI_FAILED)?;
        let stdout = child.stdout.take().ok_or(LOGIN_FAILED)?;
        let stderr = child.stderr.take().ok_or(LOGIN_FAILED)?;
        let (sent, mut received) = watch::channel(false);
        let forward = move |code: String| {
            if !sent.send_replace(true) {
                progress(code)?;
            }
            Ok::<(), String>(())
        };
        let process = async {
            let wait = async { child.wait().await.map_err(|_| LOGIN_FAILED.to_owned()) };
            let (_, _, status) = tokio::try_join!(
                login_output(stdout, &forward),
                login_output(stderr, &forward),
                wait,
            )?;
            if !status.success() {
                return Err(LOGIN_FAILED.to_owned());
            }
            Ok(())
        };
        if !device_code {
            return process.await;
        }
        tokio::pin!(process);
        let code = async {
            tokio::time::timeout(DEVICE_CODE_WAIT, received.wait_for(|code| *code))
                .await
                .map(|received| received.is_ok())
        };
        tokio::select! {
            result = &mut process => result,
            waited = code => {
                match waited {
                    Ok(true) => process.await,
                    Ok(false) => Err(LOGIN_FAILED.into()),
                    Err(_) => Err(DEVICE_CODE_TIMEOUT.into()),
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Claims {
    /// Sign-in identity (UPN) shown in the connection dialog.
    pub account: Option<String>,
    /// Entra object ID; resolved to a Dataverse `systemuser`, never used as one.
    pub object_id: Option<String>,
}

#[derive(Clone)]
pub struct Token {
    secret: String,
    pub expires: i64,
    pub claims: Claims,
}

impl Token {
    pub(super) fn new(secret: String, expires: i64, claims: Claims) -> Self {
        Self {
            secret,
            expires,
            claims,
        }
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub fn fresh(&self, now: i64) -> bool {
        self.expires > now + REFRESH_MARGIN
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Token")
            .field("secret", &"<withheld>")
            .field("expires", &self.expires)
            .field("claims", &self.claims)
            .finish()
    }
}

pub(crate) fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

pub(super) fn claims(token: &str, tenant: &str) -> Result<Claims, AuthError> {
    let mut parts = token.split('.');
    let (Some(_), Some(payload), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Ok(Claims::default());
    };
    let Some(value) = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()
        .filter(|bytes| bytes.len() <= 16_000)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return Ok(Claims::default());
    };
    if value["tid"]
        .as_str()
        .is_some_and(|tid| !tid.eq_ignore_ascii_case(tenant))
    {
        return Err(AuthError::unavailable(TENANT_MISMATCH));
    }
    let account = ["upn", "unique_name", "preferred_username"]
        .iter()
        .filter_map(|claim| value[*claim].as_str())
        .find(|name| {
            name.len() <= 254
                && name.contains('@')
                && !name
                    .chars()
                    .any(|character| character.is_control() || character.is_whitespace())
        })
        .map(str::to_owned);
    let object_id = value["oid"]
        .as_str()
        .filter(|oid| uuid(oid))
        .map(str::to_ascii_lowercase);
    Ok(Claims { account, object_id })
}

/// Parses `az account get-access-token` JSON for the configured tenant. The token is never logged.
pub fn parse_token(bytes: &[u8], tenant: &str, now: i64) -> Result<Token, AuthError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| AuthError::unavailable(INVALID_TOKEN))?;
    let token = value["accessToken"]
        .as_str()
        .ok_or_else(|| AuthError::unavailable(INVALID_TOKEN))?;
    let expires = value["expires_on"]
        .as_i64()
        .or_else(|| value["expires_on"].as_str()?.parse().ok())
        .ok_or_else(|| AuthError::unavailable(INVALID_TOKEN))?;
    if !value["tenant"]
        .as_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(tenant))
    {
        return Err(AuthError::unavailable(TENANT_MISMATCH));
    }
    if token.is_empty()
        || token.len() > 16_000
        || token
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(AuthError::unavailable(INVALID_TOKEN));
    }
    if expires <= now + 60 {
        return Err(AuthError::SignInRequired(SIGN_IN_REQUIRED.into()));
    }
    Ok(Token {
        secret: token.into(),
        expires,
        claims: claims(token, tenant)?,
    })
}

pub(super) fn aadsts(text: &str) -> Option<&str> {
    let start = text.find("AADSTS")?;
    let digits = text[start + 6..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    (4..=7)
        .contains(&digits)
        .then(|| &text[start..start + 6 + digits])
}

/// Maps a failed CLI token read to a safe status. Provider output is never shown verbatim.
pub fn classify_failure(stderr: &[u8]) -> AuthError {
    let text = String::from_utf8_lossy(stderr);
    let lower = text.to_ascii_lowercase();
    if [
        "max retries exceeded",
        "connectionerror",
        "failed to establish a new connection",
        "getaddrinfo",
        "name resolution",
        "connection aborted",
        "connection reset",
        "read timed out",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return AuthError::Unavailable {
            message: NETWORK.into(),
            transient: true,
        };
    }
    match aadsts(&text) {
        Some(code) => {
            AuthError::SignInRequired(format!("MSX sign-in required (Azure CLI reported {code})."))
        }
        None => AuthError::SignInRequired(SIGN_IN_REQUIRED.into()),
    }
}

fn account_args() -> Vec<String> {
    [
        "account",
        "list",
        "--all",
        "--query",
        "[].{id:id,tenantId:tenantId,isDefault:isDefault,state:state,userName:user.name,userType:user.type}",
        "--output",
        "json",
        "--only-show-errors",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliAccount {
    id: Option<String>,
    tenant_id: Option<String>,
    is_default: Option<bool>,
    state: Option<String>,
    user_name: Option<String>,
    user_type: Option<String>,
}

// Azure CLI's `--tenant` uses its globally default identity; a subscription pins the tenant's
// identity without changing that default.
fn select_subscription(bytes: &[u8], tenant: &str) -> Result<String, AuthError> {
    let accounts: Vec<CliAccount> =
        serde_json::from_slice(bytes).map_err(|_| AuthError::unavailable(INVALID_ACCOUNTS))?;
    let mut first = None;
    let mut default = None;
    let mut identity: Option<(&str, &str)> = None;
    let mut ambiguous = false;
    for account in &accounts {
        let account_tenant = account
            .tenant_id
            .as_deref()
            .ok_or_else(|| AuthError::unavailable(INVALID_ACCOUNTS))?;
        if !account_tenant.eq_ignore_ascii_case(tenant) {
            continue;
        }
        let state = account
            .state
            .as_deref()
            .ok_or_else(|| AuthError::unavailable(INVALID_ACCOUNTS))?;
        if !state.eq_ignore_ascii_case("Enabled") {
            continue;
        }
        let id = account
            .id
            .as_deref()
            .filter(|id| uuid(id))
            .ok_or_else(|| AuthError::unavailable(INVALID_ACCOUNTS))?;
        let valid_field = |value: &&str| {
            !value.trim().is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
        };
        let name = account
            .user_name
            .as_deref()
            .filter(valid_field)
            .ok_or_else(|| AuthError::unavailable(INVALID_ACCOUNTS))?;
        let kind = account
            .user_type
            .as_deref()
            .filter(valid_field)
            .ok_or_else(|| AuthError::unavailable(INVALID_ACCOUNTS))?;
        let is_default = account
            .is_default
            .ok_or_else(|| AuthError::unavailable(INVALID_ACCOUNTS))?;
        if is_default && default.replace(id).is_some() {
            return Err(AuthError::unavailable(INVALID_ACCOUNTS));
        }
        first.get_or_insert(id);
        if let Some((previous_name, previous_kind)) = identity {
            ambiguous |= !name.eq_ignore_ascii_case(previous_name)
                || !kind.eq_ignore_ascii_case(previous_kind);
        } else {
            identity = Some((name, kind));
        }
    }
    if let Some(id) = default {
        return Ok(id.to_owned());
    }
    if ambiguous {
        return Err(AuthError::SignInRequired(AMBIGUOUS_ACCOUNTS.into()));
    }
    first
        .map(str::to_owned)
        .ok_or_else(|| AuthError::SignInRequired(SIGN_IN_REQUIRED.into()))
}

pub fn token_args(resource: Resource, subscription: &str) -> Vec<String> {
    [
        "account",
        "get-access-token",
        "--resource",
        resource.uri(),
        "--subscription",
        subscription,
        "--output",
        "json",
        "--only-show-errors",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn run_failure(error: RunError, invalid: &str) -> AuthError {
    match error {
        RunError::Timeout => AuthError::SignInRequired(TOKEN_TIMEOUT.into()),
        RunError::Missing => AuthError::unavailable(CLI_MISSING),
        RunError::Spawn => AuthError::unavailable(CLI_FAILED),
        RunError::Io => AuthError::unavailable(invalid),
    }
}

pub fn login_args(resource: Resource, tenant: &str, device_code: bool) -> Vec<String> {
    let mut args = vec![
        "login".to_owned(),
        "--tenant".into(),
        tenant.into(),
        "--scope".into(),
        format!("{}/.default", resource.uri()),
        "--allow-no-subscriptions".into(),
        "--output".into(),
        "none".into(),
        "--only-show-errors".into(),
    ];
    if device_code {
        args.push("--use-device-code".into());
    }
    args
}

pub struct Tokens {
    runner: Arc<dyn Runner>,
    grant: Arc<dyn Grant>,
    tenant: String,
    cache: Mutex<BTreeMap<Resource, Token>>,
    /// The last sign-in failure per resource. Background reads fail fast on it instead of
    /// asking again; only an explicit check or sign-in clears it.
    latched: std::sync::Mutex<BTreeMap<Resource, AuthError>>,
}

impl Tokens {
    pub fn new(runner: Arc<dyn Runner>, grant: Arc<dyn Grant>, tenant: &str) -> Self {
        Self {
            runner,
            grant,
            tenant: tenant.into(),
            cache: Mutex::new(BTreeMap::new()),
            latched: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    fn latched(&self, resource: Resource) -> Option<AuthError> {
        self.latched
            .lock()
            .ok()
            .and_then(|latched| latched.get(&resource).cloned())
    }

    pub(super) fn latch(&self, resource: Resource, error: &AuthError) {
        if let (AuthError::SignInRequired(_), Ok(mut latched)) = (error, self.latched.lock()) {
            latched.insert(resource, error.clone());
        }
    }

    pub fn unlatch(&self) {
        if let Ok(mut latched) = self.latched.lock() {
            latched.clear();
        }
    }

    fn unlatch_one(&self, resource: Resource) {
        if let Ok(mut latched) = self.latched.lock() {
            latched.remove(&resource);
        }
    }

    /// Forgets every in-memory token; saved sign-ins are left unchanged.
    pub async fn clear(&self) {
        self.cache.lock().await.clear();
        self.unlatch();
    }

    /// Forgets in-memory tokens and removes Tomlook's saved MSX sign-in. The Azure CLI
    /// sign-in used for Power BI is left unchanged.
    pub async fn sign_out(&self) -> Result<(), AuthError> {
        self.clear().await;
        self.grant.clear().await
    }

    pub async fn forget(&self, resource: Resource) {
        self.cache.lock().await.remove(&resource);
    }

    /// A cached token, or one silent bounded read; never interactive and never retried.
    pub async fn token(&self, resource: Resource) -> Result<Token, AuthError> {
        if let Some(error) = self.latched(resource) {
            return Err(error);
        }
        let mut cache = self.cache.lock().await;
        let now = chrono::Utc::now().timestamp();
        if let Some(token) = cache.get(&resource).filter(|token| token.fresh(now)) {
            return Ok(token.clone());
        }
        cache.remove(&resource);
        match self.fetch(resource).await {
            Ok(token) => {
                cache.insert(resource, token.clone());
                Ok(token)
            }
            Err(error) => {
                self.latch(resource, &error);
                Err(error)
            }
        }
    }

    async fn fetch(&self, resource: Resource) -> Result<Token, AuthError> {
        if resource == Resource::Dataverse {
            return tokio::time::timeout(BACKGROUND_TIMEOUT, self.grant.refresh())
                .await
                .unwrap_or_else(|_| {
                    Err(AuthError::Unavailable {
                        message: REFRESH_TIMEOUT.into(),
                        transient: true,
                    })
                });
        }
        let started = tokio::time::Instant::now();
        let accounts = self
            .runner
            .run(account_args(), BACKGROUND_TIMEOUT)
            .await
            .map_err(|error| run_failure(error, INVALID_ACCOUNTS))?;
        if !accounts.success {
            return Err(classify_failure(&accounts.stderr));
        }
        let subscription = select_subscription(&accounts.stdout, &self.tenant)?;
        let remaining = BACKGROUND_TIMEOUT.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(AuthError::SignInRequired(TOKEN_TIMEOUT.into()));
        }
        let output = self
            .runner
            .run(token_args(resource, &subscription), remaining)
            .await
            .map_err(|error| run_failure(error, INVALID_TOKEN))?;
        if !output.success {
            return Err(classify_failure(&output.stderr));
        }
        parse_token(&output.stdout, &self.tenant, chrono::Utc::now().timestamp())
    }

    /// Explicit, user-started sign-in: the browser sign-in for Dataverse, then Power BI through
    /// the Azure CLI only when it has no usable token. Bounded and cancellable; dropping the
    /// operation ends the listener and the CLI.
    pub async fn sign_in(
        &self,
        device_code: bool,
        progress: Progress,
        mut cancelled: watch::Receiver<bool>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), AuthError> {
        if *cancelled.borrow() || *shutdown.borrow() {
            return Err(AuthError::SignInRequired(LOGIN_CANCELLED.into()));
        }
        self.clear().await;
        let deadline = BROWSER_LOGIN_TIMEOUT
            + if device_code {
                DEVICE_LOGIN_TIMEOUT
            } else {
                BROWSER_LOGIN_TIMEOUT
            };
        let operation = async {
            let token = self.grant.sign_in().await?;
            self.cache.lock().await.insert(Resource::Dataverse, token);
            // Power BI is optional for MSX; its failure is reported for that resource only.
            if let Err(AuthError::SignInRequired(_)) = self.token(Resource::PowerBi).await {
                self.unlatch_one(Resource::PowerBi);
                match self
                    .runner
                    .login(
                        login_args(Resource::PowerBi, &self.tenant, device_code),
                        device_code,
                        progress.clone(),
                    )
                    .await
                {
                    Ok(()) => {
                        let _ = self.token(Resource::PowerBi).await;
                    }
                    Err(message) => {
                        self.latch(Resource::PowerBi, &AuthError::SignInRequired(message));
                    }
                }
            }
            Ok(())
        };
        let result = tokio::select! {
            biased;
            _ = cancelled.changed() => Err(AuthError::SignInRequired(LOGIN_CANCELLED.into())),
            _ = shutdown.changed() => Err(AuthError::SignInRequired(LOGIN_CANCELLED.into())),
            result = tokio::time::timeout(deadline, operation) =>
                result.unwrap_or_else(|_| Err(AuthError::SignInRequired(LOGIN_TIMED_OUT.into()))),
        };
        if result.is_err() {
            self.cache.lock().await.clear();
        }
        result
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    pub(crate) const TENANT: &str = "00000000-0000-0000-0000-000000000001";
    const SUBSCRIPTION: &str = "00000000-0000-0000-0000-000000000002";

    pub(crate) fn jwt(claims: Value) -> String {
        let encode = |value: &Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(value).unwrap())
        };
        format!(
            "{}.{}.synthetic-signature",
            encode(&serde_json::json!({"alg":"none"})),
            encode(&claims)
        )
    }

    pub(crate) fn token_json(tenant: &str, expires: i64) -> Vec<u8> {
        token_json_as(
            tenant,
            expires,
            (
                "someone@example.test",
                "11111111-2222-3333-4444-555555555555",
            ),
        )
    }

    /// A token of the given (UPN, object ID) identity.
    pub(crate) fn token_json_as(tenant: &str, expires: i64, identity: (&str, &str)) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "accessToken": jwt(serde_json::json!({
                "tid": tenant,
                "upn": identity.0,
                "oid": identity.1
            })),
            "expires_on": expires,
            "tenant": tenant,
            "tokenType": "Bearer"
        }))
        .unwrap()
    }

    pub(crate) enum Reply {
        Token,
        Timeout,
        Fail(&'static str),
    }

    #[derive(Default)]
    pub(crate) struct FakeRunner {
        pub replies: StdMutex<Vec<Reply>>,
        pub account_data: StdMutex<Option<Vec<u8>>>,
        pub account_calls: StdMutex<Vec<Vec<String>>>,
        pub account_delay: StdMutex<Duration>,
        pub timeouts: StdMutex<Vec<Duration>>,
        pub calls: StdMutex<Vec<Vec<String>>>,
        pub logins: StdMutex<Vec<Vec<String>>>,
        pub login_result: StdMutex<Option<Result<(), String>>>,
        /// The (UPN, object ID) of issued tokens; the default fixture identity when unset.
        pub identity: StdMutex<Option<(&'static str, &'static str)>>,
    }

    impl FakeRunner {
        pub(crate) fn with(replies: Vec<Reply>) -> Arc<Self> {
            Arc::new(Self {
                replies: StdMutex::new(replies),
                ..Default::default()
            })
        }
    }

    #[async_trait::async_trait]
    impl Runner for FakeRunner {
        async fn run(&self, args: Vec<String>, timeout: Duration) -> Result<Output, RunError> {
            assert!(!timeout.is_zero() && timeout <= BACKGROUND_TIMEOUT);
            self.timeouts.lock().unwrap().push(timeout);
            if args.get(1).is_some_and(|arg| arg == "list") {
                self.account_calls.lock().unwrap().push(args);
                let delay = *self.account_delay.lock().unwrap();
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let data = self
                    .account_data
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| {
                        let identity = self.identity.lock().unwrap().unwrap_or((
                            "someone@example.test",
                            "11111111-2222-3333-4444-555555555555",
                        ));
                        serde_json::to_vec(&serde_json::json!([{
                            "id": SUBSCRIPTION,
                            "tenantId": TENANT,
                            "isDefault": true,
                            "state": "Enabled",
                            "userName": identity.0,
                            "userType": "user"
                        }]))
                        .unwrap()
                    });
                return Ok(Output {
                    success: true,
                    stdout: data,
                    stderr: Vec::new(),
                });
            }
            self.calls.lock().unwrap().push(args);
            let reply = {
                let mut replies = self.replies.lock().unwrap();
                if replies.is_empty() {
                    Reply::Token
                } else {
                    replies.remove(0)
                }
            };
            match reply {
                Reply::Token => Ok(Output {
                    success: true,
                    stdout: match *self.identity.lock().unwrap() {
                        Some(identity) => {
                            token_json_as(TENANT, chrono::Utc::now().timestamp() + 3600, identity)
                        }
                        None => token_json(TENANT, chrono::Utc::now().timestamp() + 3600),
                    },
                    stderr: Vec::new(),
                }),
                Reply::Timeout => Err(RunError::Timeout),
                Reply::Fail(stderr) => Ok(Output {
                    success: false,
                    stdout: Vec::new(),
                    stderr: stderr.as_bytes().to_vec(),
                }),
            }
        }

        async fn login(
            &self,
            args: Vec<String>,
            device_code: bool,
            progress: Progress,
        ) -> Result<(), String> {
            self.logins.lock().unwrap().push(args);
            if device_code {
                progress("ABCD-EF12".into())?;
            }
            self.login_result.lock().unwrap().clone().unwrap_or(Ok(()))
        }
    }

    /// Dataverse sign-in without a browser or network; signed out unless `saved` is set.
    #[derive(Default)]
    pub(crate) struct FakeGrant {
        pub saved: StdMutex<bool>,
        pub refreshes: StdMutex<usize>,
        pub sign_ins: StdMutex<usize>,
        pub cleared: StdMutex<usize>,
        pub refresh_error: StdMutex<Option<AuthError>>,
        pub sign_in_error: StdMutex<Option<AuthError>>,
        /// The (UPN, object ID) of issued tokens; the default fixture identity when unset.
        pub identity: StdMutex<Option<(&'static str, &'static str)>>,
    }

    impl FakeGrant {
        pub(crate) fn signed_in() -> Arc<Self> {
            let grant = Self::default();
            *grant.saved.lock().unwrap() = true;
            Arc::new(grant)
        }

        fn issue(&self) -> Token {
            let (account, object_id) = self.identity.lock().unwrap().unwrap_or((
                "someone@example.test",
                "11111111-2222-3333-4444-555555555555",
            ));
            let secret = jwt(serde_json::json!({"tid": TENANT, "upn": account, "oid": object_id}));
            let claims = claims(&secret, TENANT).unwrap();
            Token::new(secret, chrono::Utc::now().timestamp() + 3600, claims)
        }
    }

    #[async_trait::async_trait]
    impl Grant for FakeGrant {
        async fn refresh(&self) -> Result<Token, AuthError> {
            *self.refreshes.lock().unwrap() += 1;
            if let Some(error) = self.refresh_error.lock().unwrap().clone() {
                return Err(error);
            }
            if !*self.saved.lock().unwrap() {
                return Err(AuthError::SignInRequired(SIGN_IN_REQUIRED.into()));
            }
            Ok(self.issue())
        }

        async fn sign_in(&self) -> Result<Token, AuthError> {
            *self.sign_ins.lock().unwrap() += 1;
            if let Some(error) = self.sign_in_error.lock().unwrap().clone() {
                return Err(error);
            }
            *self.saved.lock().unwrap() = true;
            *self.refresh_error.lock().unwrap() = None;
            Ok(self.issue())
        }

        async fn clear(&self) -> Result<(), AuthError> {
            *self.cleared.lock().unwrap() += 1;
            *self.saved.lock().unwrap() = false;
            Ok(())
        }
    }

    fn tokens(runner: Arc<FakeRunner>) -> Tokens {
        Tokens::new(runner, FakeGrant::signed_in(), TENANT)
    }

    #[test]
    fn tokens_require_the_configured_tenant_a_future_expiry_and_stay_out_of_debug_output() {
        let now = chrono::Utc::now().timestamp();
        let token = parse_token(&token_json(TENANT, now + 3600), TENANT, now).unwrap();
        assert_eq!(
            token.claims.account.as_deref(),
            Some("someone@example.test")
        );
        assert_eq!(
            token.claims.object_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert!(token.fresh(now) && !token.fresh(token.expires - 10));
        let debug = format!("{token:?}");
        assert!(!debug.contains(token.secret()) && debug.contains("withheld"));
        assert_eq!(
            parse_token(
                &token_json(TENANT, now + 3600),
                "00000000-0000-0000-0000-000000000009",
                now
            )
            .unwrap_err(),
            AuthError::unavailable(TENANT_MISMATCH)
        );
        assert!(matches!(
            parse_token(&token_json(TENANT, now + 30), TENANT, now).unwrap_err(),
            AuthError::SignInRequired(_)
        ));
        let mut value: Value = serde_json::from_slice(&token_json(TENANT, now + 3600)).unwrap();
        value["accessToken"] =
            jwt(serde_json::json!({"tid":"00000000-0000-0000-0000-000000000009"})).into();
        assert_eq!(
            parse_token(&serde_json::to_vec(&value).unwrap(), TENANT, now).unwrap_err(),
            AuthError::unavailable(TENANT_MISMATCH),
            "The token's own tenant claim must match too"
        );
        value["accessToken"] = "token\ninjection".into();
        assert_eq!(
            parse_token(&serde_json::to_vec(&value).unwrap(), TENANT, now).unwrap_err(),
            AuthError::unavailable(INVALID_TOKEN)
        );
        value["accessToken"] = "opaque-token".into();
        value["expires_on"] = (now + 3600).to_string().into();
        let opaque = parse_token(&serde_json::to_vec(&value).unwrap(), TENANT, now).unwrap();
        assert_eq!(opaque.claims, Claims::default());
        assert!(parse_token(b"not json", TENANT, now).is_err());
    }

    #[test]
    fn failures_map_to_sign_in_or_transient_network_without_provider_text() {
        assert_eq!(
            classify_failure(b"ERROR: AADSTS50076: Due to a configuration change... secret detail"),
            AuthError::SignInRequired(
                "MSX sign-in required (Azure CLI reported AADSTS50076).".into()
            )
        );
        assert_eq!(
            classify_failure(b"Please run 'az login' to setup account."),
            AuthError::SignInRequired(SIGN_IN_REQUIRED.into())
        );
        assert_eq!(
            classify_failure(
                b"HTTPSConnectionPool(host='login.microsoftonline.com'): Max retries exceeded"
            ),
            AuthError::Unavailable {
                message: NETWORK.into(),
                transient: true
            }
        );
        assert!(crate::retry::transient(NETWORK));
        assert!(!crate::retry::transient(TOKEN_TIMEOUT));
    }

    #[test]
    fn token_and_login_commands_are_scoped_to_the_selected_identity_and_resource() {
        let args = token_args(Resource::Dataverse, SUBSCRIPTION);
        assert_eq!(
            args,
            [
                "account",
                "get-access-token",
                "--resource",
                "https://microsoftsales.crm.dynamics.com",
                "--subscription",
                SUBSCRIPTION,
                "--output",
                "json",
                "--only-show-errors"
            ]
        );
        assert!(!args.iter().any(|arg| arg == "set" || arg == "--tenant"));
        let login = login_args(Resource::PowerBi, TENANT, false);
        assert!(login.windows(2).any(|pair| pair[0] == "--scope"
            && pair[1] == "https://analysis.windows.net/powerbi/api/.default"));
        assert!(!login.contains(&"--use-device-code".to_owned()));
        assert!(
            login_args(Resource::Dataverse, TENANT, true).contains(&"--use-device-code".to_owned())
        );
    }

    #[tokio::test]
    async fn a_different_default_tenant_uses_the_matching_identity_without_switching_it() {
        let runner = FakeRunner::with(vec![]);
        *runner.account_data.lock().unwrap() = Some(
            serde_json::to_vec(&serde_json::json!([
                {
                    "id": "00000000-0000-0000-0000-000000000003",
                    "tenantId": "00000000-0000-0000-0000-000000000004",
                    "isDefault": true, "state": "Enabled",
                    "userName": "personal@example.test", "userType": "user"
                },
                {
                    "id": SUBSCRIPTION, "tenantId": TENANT,
                    "isDefault": false, "state": "Enabled",
                    "userName": "work@example.test", "userType": "user"
                },
                {
                    "id": "00000000-0000-0000-0000-000000000005", "tenantId": TENANT,
                    "isDefault": false, "state": "Enabled",
                    "userName": "WORK@example.test", "userType": "user"
                }
            ]))
            .unwrap(),
        );
        let tokens = tokens(runner.clone());
        tokens.token(Resource::PowerBi).await.unwrap();
        let account_calls = runner.account_calls.lock().unwrap();
        assert_eq!(account_calls.len(), 1);
        assert_eq!(&account_calls[0][..2], ["account", "list"]);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0]
                .windows(2)
                .any(|pair| pair == ["--subscription", SUBSCRIPTION])
        );
        assert!(!calls[0].iter().any(|arg| arg == "--tenant" || arg == "set"));
    }

    #[test]
    fn account_selection_rejects_ambiguous_or_invalid_target_identities() {
        let account = |id, name, default| {
            serde_json::json!({
                "id": id, "tenantId": TENANT, "isDefault": default, "state": "Enabled",
                "userName": name, "userType": "user"
            })
        };
        let first = account(SUBSCRIPTION, "first@example.test", false);
        let second = account(
            "00000000-0000-0000-0000-000000000003",
            "second@example.test",
            false,
        );
        let list = |accounts: Vec<Value>| serde_json::to_vec(&accounts).unwrap();
        assert_eq!(
            select_subscription(&list(vec![first.clone(), second.clone()]), TENANT),
            Err(AuthError::SignInRequired(AMBIGUOUS_ACCOUNTS.into()))
        );
        let mut selected = second;
        selected["isDefault"] = true.into();
        assert_eq!(
            select_subscription(&list(vec![first.clone(), selected]), TENANT).unwrap(),
            "00000000-0000-0000-0000-000000000003"
        );
        let mut invalid = first;
        invalid["userName"] = Value::Null;
        assert_eq!(
            select_subscription(&list(vec![invalid]), TENANT),
            Err(AuthError::unavailable(INVALID_ACCOUNTS))
        );
        assert_eq!(
            select_subscription(b"[]", TENANT),
            Err(AuthError::SignInRequired(SIGN_IN_REQUIRED.into()))
        );
    }

    #[tokio::test]
    async fn account_lookup_and_token_fetch_share_the_background_deadline() {
        let runner = FakeRunner::with(vec![]);
        *runner.account_delay.lock().unwrap() = Duration::from_millis(40);
        tokens(runner.clone())
            .token(Resource::PowerBi)
            .await
            .unwrap();
        let timeouts = runner.timeouts.lock().unwrap();
        assert_eq!(timeouts.len(), 2);
        assert_eq!(timeouts[0], BACKGROUND_TIMEOUT);
        assert!(timeouts[1] <= BACKGROUND_TIMEOUT - Duration::from_millis(30));
    }

    #[tokio::test]
    async fn a_background_timeout_requires_sign_in_once_and_never_spins_the_cli_again() {
        let runner = FakeRunner::with(vec![Reply::Timeout]);
        let grant = FakeGrant::signed_in();
        let tokens = Tokens::new(runner.clone(), grant.clone(), TENANT);
        assert_eq!(
            tokens.token(Resource::PowerBi).await.unwrap_err(),
            AuthError::SignInRequired(TOKEN_TIMEOUT.into())
        );
        for _ in 0..5 {
            assert_eq!(
                tokens.token(Resource::PowerBi).await.unwrap_err(),
                AuthError::SignInRequired(TOKEN_TIMEOUT.into())
            );
        }
        assert_eq!(
            runner.calls.lock().unwrap().len(),
            1,
            "No busy retry after interaction is required"
        );
        tokens.token(Resource::Dataverse).await.unwrap();
        *grant.refresh_error.lock().unwrap() =
            Some(AuthError::SignInRequired(SIGN_IN_REQUIRED.into()));
        tokens.forget(Resource::Dataverse).await;
        for _ in 0..3 {
            assert!(tokens.token(Resource::Dataverse).await.is_err());
        }
        assert_eq!(
            *grant.refreshes.lock().unwrap(),
            2,
            "A Dataverse latch is independent of Power BI"
        );
        tokens.unlatch();
        tokens.token(Resource::PowerBi).await.unwrap();
        tokens.token(Resource::PowerBi).await.unwrap();
        assert_eq!(
            runner.calls.lock().unwrap().len(),
            2,
            "A fresh token is served from memory"
        );
    }

    #[tokio::test]
    async fn dataverse_uses_the_saved_browser_sign_in_and_power_bi_the_cli() {
        let runner = FakeRunner::with(vec![]);
        let grant = FakeGrant::signed_in();
        let tokens = Tokens::new(runner.clone(), grant.clone(), TENANT);
        let token = tokens.token(Resource::Dataverse).await.unwrap();
        tokens.token(Resource::Dataverse).await.unwrap();
        assert_eq!(
            token.claims.account.as_deref(),
            Some("someone@example.test")
        );
        assert_eq!(*grant.refreshes.lock().unwrap(), 1);
        assert!(runner.calls.lock().unwrap().is_empty());
        assert!(runner.account_calls.lock().unwrap().is_empty());
        tokens.token(Resource::PowerBi).await.unwrap();
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
        assert_eq!(*grant.refreshes.lock().unwrap(), 1);
        tokens.sign_out().await.unwrap();
        assert_eq!(*grant.cleared.lock().unwrap(), 1);
        assert_eq!(
            tokens.token(Resource::Dataverse).await.unwrap_err(),
            AuthError::SignInRequired(SIGN_IN_REQUIRED.into())
        );
        assert!(runner.logins.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transient_failures_are_not_latched_and_missing_cli_is_unavailable() {
        let runner = FakeRunner::with(vec![Reply::Fail("Max retries exceeded")]);
        let tokens = tokens(runner.clone());
        assert!(matches!(
            tokens.token(Resource::PowerBi).await.unwrap_err(),
            AuthError::Unavailable {
                transient: true,
                ..
            }
        ));
        tokens.token(Resource::PowerBi).await.unwrap();
        let missing = Tokens::new(
            Arc::new(CliRunner { cli: None }),
            FakeGrant::signed_in(),
            TENANT,
        );
        assert_eq!(
            missing.token(Resource::PowerBi).await.unwrap_err(),
            AuthError::unavailable(CLI_MISSING)
        );
        missing.token(Resource::Dataverse).await.unwrap();
    }

    fn collect_codes() -> (Arc<StdMutex<Vec<String>>>, Progress) {
        let codes = Arc::new(StdMutex::new(Vec::new()));
        let seen = codes.clone();
        (
            codes,
            Arc::new(move |code| {
                seen.lock().unwrap().push(code);
                Ok(())
            }),
        )
    }

    #[tokio::test]
    async fn explicit_sign_in_uses_the_browser_for_dataverse_and_keeps_a_working_power_bi() {
        let runner = FakeRunner::with(vec![]);
        let grant = Arc::new(FakeGrant::default());
        let tokens = Tokens::new(runner.clone(), grant.clone(), TENANT);
        let (codes, progress) = collect_codes();
        let (_cancel, cancelled) = watch::channel(false);
        let (_stop, shutdown) = watch::channel(false);
        tokens
            .sign_in(true, progress, cancelled, shutdown)
            .await
            .unwrap();
        assert_eq!(*grant.sign_ins.lock().unwrap(), 1);
        assert!(runner.logins.lock().unwrap().is_empty());
        assert!(codes.lock().unwrap().is_empty());
        tokens.token(Resource::Dataverse).await.unwrap();
        tokens.token(Resource::PowerBi).await.unwrap();
        assert_eq!(*grant.refreshes.lock().unwrap(), 0);
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn explicit_sign_in_starts_the_cli_only_for_a_signed_out_power_bi() {
        let runner = FakeRunner::with(vec![Reply::Fail("Please run 'az login'"), Reply::Token]);
        let grant = Arc::new(FakeGrant::default());
        let tokens = Tokens::new(runner.clone(), grant.clone(), TENANT);
        let (codes, progress) = collect_codes();
        let (_cancel, cancelled) = watch::channel(false);
        let (_stop, shutdown) = watch::channel(false);
        tokens
            .sign_in(true, progress.clone(), cancelled.clone(), shutdown.clone())
            .await
            .unwrap();
        assert_eq!(*codes.lock().unwrap(), ["ABCD-EF12"]);
        {
            let logins = runner.logins.lock().unwrap();
            assert_eq!(logins.len(), 1);
            assert!(
                logins[0]
                    .contains(&"https://analysis.windows.net/powerbi/api/.default".to_owned())
            );
            assert!(logins[0].contains(&"--use-device-code".to_owned()));
        }
        tokens.token(Resource::PowerBi).await.unwrap();
        assert_eq!(runner.calls.lock().unwrap().len(), 2);

        *runner.replies.lock().unwrap() = vec![Reply::Fail("Please run 'az login'")];
        *runner.login_result.lock().unwrap() = Some(Err(LOGIN_FAILED.into()));
        tokens
            .sign_in(false, progress, cancelled, shutdown)
            .await
            .unwrap();
        tokens.token(Resource::Dataverse).await.unwrap();
        assert_eq!(
            tokens.token(Resource::PowerBi).await.unwrap_err(),
            AuthError::SignInRequired(LOGIN_FAILED.into()),
            "A failed Power BI sign-in leaves Dataverse connected"
        );
    }

    #[tokio::test]
    async fn cancelled_or_failed_sign_in_launches_nothing_or_reports_sign_in_required() {
        let runner = FakeRunner::with(vec![]);
        let grant = Arc::new(FakeGrant::default());
        let tokens = Tokens::new(runner.clone(), grant.clone(), TENANT);
        let (_cancel, cancelled) = watch::channel(true);
        let (_stop, shutdown) = watch::channel(false);
        assert_eq!(
            tokens
                .sign_in(false, Arc::new(|_| Ok(())), cancelled, shutdown.clone())
                .await
                .unwrap_err(),
            AuthError::SignInRequired(LOGIN_CANCELLED.into())
        );
        assert_eq!(*grant.sign_ins.lock().unwrap(), 0);
        let failure = AuthError::SignInRequired("MSX sign-in failed.".into());
        *grant.sign_in_error.lock().unwrap() = Some(failure.clone());
        let (_cancel, cancelled) = watch::channel(false);
        assert_eq!(
            tokens
                .sign_in(false, Arc::new(|_| Ok(())), cancelled, shutdown)
                .await
                .unwrap_err(),
            failure
        );
        assert!(runner.logins.lock().unwrap().is_empty());
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_real_runner_kills_a_hung_cli_at_its_deadline() {
        #[cfg(windows)]
        let (program, args) = (
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join(r"System32\ping.exe"),
            vec!["-n".to_owned(), "30".into(), "127.0.0.1".into()],
        );
        #[cfg(not(windows))]
        let (program, args) = (PathBuf::from("/bin/sleep"), vec!["30".to_owned()]);
        let runner = CliRunner { cli: Some(program) };
        let started = std::time::Instant::now();
        assert_eq!(
            runner.run(args, Duration::from_millis(300)).await.err(),
            Some(RunError::Timeout)
        );
        assert!(started.elapsed() < Duration::from_secs(15));
    }
}
