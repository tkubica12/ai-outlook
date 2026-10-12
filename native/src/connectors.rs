use crate::copilot::{Connection, READ_ONLY};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientConfig, ProtocolVersion},
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::watch;

pub(crate) const GITHUB_READ_TOOLSETS: &str = "context,repos,issues,pull_requests";

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    Webiq,
    Github,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Webiq => "WebIQ",
            Self::Github => "GitHub",
        }
    }
    pub fn server(self) -> &'static str {
        match self {
            Self::Webiq => "WebIQ-MCP",
            Self::Github => "github",
        }
    }
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Webiq => "https://api.microsoft.ai/v3/mcp",
            Self::Github => "https://api.githubcopilot.com/mcp/",
        }
    }
}

pub enum Input {
    WebiqKey(String),
    GithubApp(String),
    GithubToken(String),
}

impl Input {
    pub fn kind(&self) -> Kind {
        match self {
            Self::WebiqKey(_) => Kind::Webiq,
            Self::GithubApp(_) | Self::GithubToken(_) => Kind::Github,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Saved {
    credential: String,
    pub client_id: Option<String>,
    pub login: Option<String>,
    expires_at: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    refresh_expires_at: Option<i64>,
}

impl Saved {
    pub fn available(&self) -> bool {
        self.expires_at
            .is_none_or(|time| time > chrono::Utc::now().timestamp() + 60)
    }
    pub fn refresh_due(&self) -> bool {
        self.refresh_token.is_some()
            && self
                .expires_at
                .is_some_and(|time| time <= chrono::Utc::now().timestamp() + 300)
    }
    pub fn can_refresh(&self) -> bool {
        self.refresh_token.is_some()
            && self
                .refresh_expires_at
                .is_some_and(|time| time > chrono::Utc::now().timestamp() + 60)
    }
    fn validate(&self, kind: Kind) -> Result<(), String> {
        validate_secret(&self.credential)?;
        if kind == Kind::Webiq
            && (self.client_id.is_some()
                || self.login.is_some()
                || self.expires_at.is_some()
                || self.refresh_token.is_some()
                || self.refresh_expires_at.is_some())
        {
            return Err("Saved WebIQ identity has unexpected account fields".into());
        }
        if kind == Kind::Github {
            if self.refresh_token.is_some() != self.refresh_expires_at.is_some()
                || (self.refresh_token.is_some()
                    && (self.client_id.is_none() || self.expires_at.is_none()))
            {
                return Err(
                    "Saved GitHub refresh credentials are incomplete; sign in again".into(),
                );
            }
            if let Some(token) = &self.refresh_token {
                validate_secret(token)?;
            }
            match self.client_id.as_deref() {
                Some(client_id) => validate_client_id(client_id)?,
                None => validate_github_token(&self.credential)?,
            }
            if self.login.as_deref().is_none_or(|login| {
                login.is_empty()
                    || login.len() > 100
                    || !login
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            }) {
                return Err("GitHub account identity is invalid".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct Registry {
    pub records: BTreeMap<Kind, Saved>,
}

impl Registry {
    pub fn context_revision(&self) -> String {
        use sha2::{Digest, Sha256};
        if self.records.is_empty() {
            return String::new();
        }
        let mut digest = Sha256::new();
        for (kind, saved) in &self.records {
            digest.update(kind.server());
            if *kind == Kind::Github && saved.client_id.is_some() {
                for field in [saved.client_id.as_deref(), saved.login.as_deref()] {
                    let value = field.unwrap_or_default();
                    digest.update((value.len() as u64).to_le_bytes());
                    digest.update(value);
                }
            } else {
                digest.update((saved.credential.len() as u64).to_le_bytes());
                digest.update(&saved.credential);
            }
        }
        format!("{:x}", digest.finalize())
    }
    pub fn revision(&self) -> String {
        use sha2::{Digest, Sha256};
        if self.records.is_empty() {
            return String::new();
        }
        let mut digest = Sha256::new();
        for (kind, saved) in &self.records {
            digest.update(kind.server());
            digest.update((saved.credential.len() as u64).to_le_bytes());
            digest.update(&saved.credential);
        }
        format!("{:x}", digest.finalize())
    }
    pub fn secrets(&self) -> impl Iterator<Item = String> + '_ {
        self.records.values().flat_map(|saved| {
            std::iter::once(saved.credential.clone()).chain(saved.refresh_token.clone())
        })
    }
    pub fn expires_in(&self) -> Option<Duration> {
        let now = chrono::Utc::now().timestamp();
        self.records
            .values()
            .filter_map(|saved| saved.expires_at)
            .map(|expiry| {
                Duration::from_secs(expiry.saturating_sub(now).saturating_sub(60).max(0) as u64)
            })
            .min()
    }
    pub fn apply(&self, config: &mut crate::copilot::Config) -> BTreeMap<String, String> {
        let mut credentials = BTreeMap::new();
        config.servers.remove(Kind::Webiq.server());
        config.servers.remove(Kind::Github.server());
        for (kind, saved) in &self.records {
            // Registered app credentials are only sent to the fixed provider, never a configured alternate host.
            config.servers.remove(kind.server());
            if saved.available() {
                config.servers.insert(
                    kind.server().into(),
                    Connection {
                        url: kind.endpoint().into(),
                        credential_env: None,
                    },
                );
                credentials.insert(kind.server().into(), saved.credential.clone());
            }
        }
        credentials
    }
}

pub struct Vault {
    path: PathBuf,
    kind: Kind,
    generation: i64,
}

impl Vault {
    pub async fn bind(root: &Path, kind: Kind) -> Result<Self, String> {
        let path = root.join("connector-login.db");
        tokio::task::spawn_blocking(move || {
            let db = database(&path)?;
            let generation = generation(&db, kind)?;
            Ok(Self {
                path,
                kind,
                generation,
            })
        })
        .await
        .map_err(|_| "Connector credential worker stopped")?
    }
    pub async fn load(&self) -> Result<Option<Saved>, String> {
        let path = self.path.clone();
        let kind = self.kind;
        let expected = self.generation;
        tokio::task::spawn_blocking(move || {
            let db = database(&path)?;
            check_generation(&db, kind, expected)?;
            let encrypted: Option<Vec<u8>> = db
                .query_row(
                    "SELECT payload FROM credentials WHERE provider=?1",
                    [kind.server()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| "Read connector credentials failed")?;
            let Some(encrypted) = encrypted else {
                return Ok(None);
            };
            if encrypted.len() > 131_072 {
                return Err("Connector credential record exceeds its bound".into());
            }
            let mut plain = crate::credentials::protect(&encrypted, &binding(kind), false)?;
            let decoded = serde_json::from_slice::<Saved>(&plain);
            plain.fill(0);
            let saved = decoded.map_err(|_| "Saved connector credentials are invalid")?;
            saved.validate(kind)?;
            Ok(Some(saved))
        })
        .await
        .map_err(|_| "Connector credential worker stopped")?
    }
    pub async fn save(&self, saved: Saved) -> Result<(), String> {
        let path = self.path.clone();
        let kind = self.kind;
        let expected = self.generation;
        tokio::task::spawn_blocking(move || {
            saved.validate(kind)?;
            let mut plain =
                serde_json::to_vec(&saved).map_err(|_| "Encode connector credentials failed")?;
            let encrypted = crate::credentials::protect(&plain, &binding(kind), true);
            plain.fill(0);
            let encrypted = encrypted?;
            let mut db = database(&path)?;
            let transaction = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| "Lock connector credentials failed")?;
            check_generation(&transaction, kind, expected)?;
            transaction
                .execute(
                    "INSERT OR REPLACE INTO credentials VALUES(?1,?2)",
                    rusqlite::params![kind.server(), encrypted],
                )
                .map_err(|_| "Save connector credentials failed")?;
            transaction
                .commit()
                .map_err(|_| "Commit connector credentials failed".into())
        })
        .await
        .map_err(|_| "Connector credential worker stopped")?
    }
    pub async fn forget(root: &Path, kind: Kind) -> Result<(), String> {
        let path = root.join("connector-login.db");
        tokio::task::spawn_blocking(move || {
            let mut db = database(&path)?;
            db.execute_batch("PRAGMA secure_delete=ON").map_err(|_| "Configure credential removal failed")?;
            let transaction = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_| "Lock credential removal failed")?;
            transaction.execute("DELETE FROM credentials WHERE provider=?1", [kind.server()]).map_err(|_| "Remove connector credentials failed")?;
            transaction.execute("INSERT INTO generations VALUES(?1,1) ON CONFLICT(provider) DO UPDATE SET generation=generation+1", [kind.server()]).map_err(|_| "Revoke connector sign-in failed")?;
            transaction.commit().map_err(|_| "Commit credential removal failed".into())
        }).await.map_err(|_| "Connector credential worker stopped")?
    }
}

fn binding(kind: Kind) -> String {
    format!("Tomlook connector v1:{}:{}", kind.server(), kind.endpoint())
}
fn database(path: &Path) -> Result<rusqlite::Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "Create connector credential directory failed")?;
    }
    let db = rusqlite::Connection::open(path).map_err(|_| "Open connector credentials failed")?;
    db.busy_timeout(Duration::from_secs(2))
        .map_err(|_| "Set credential deadline failed")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS credentials(provider TEXT PRIMARY KEY,payload BLOB NOT NULL); CREATE TABLE IF NOT EXISTS generations(provider TEXT PRIMARY KEY,generation INTEGER NOT NULL);").map_err(|_| "Initialize connector credentials failed")?;
    Ok(db)
}
fn generation(db: &rusqlite::Connection, kind: Kind) -> Result<i64, String> {
    db.query_row(
        "SELECT generation FROM generations WHERE provider=?1",
        [kind.server()],
        |row| row.get(0),
    )
    .optional()
    .map(|value| value.unwrap_or(0))
    .map_err(|_| "Read connector credential generation failed".into())
}
fn check_generation(db: &rusqlite::Connection, kind: Kind, expected: i64) -> Result<(), String> {
    if generation(db, kind)? != expected {
        return Err("This connector sign-in was removed; reconnect".into());
    }
    Ok(())
}
pub fn validate_secret(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 8000
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("The key is empty, too long or contains whitespace".into());
    }
    Ok(())
}
pub fn validate_client_id(value: &str) -> Result<(), String> {
    if !(value.starts_with("Iv1.") || value.starts_with("Iv23"))
        || value.len() > 100
        || value.len() <= 4
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
    {
        return Err(
            "Enter the Client ID of a registered GitHub App, with Device flow enabled".into(),
        );
    }
    Ok(())
}
fn validate_github_token(value: &str) -> Result<(), String> {
    validate_secret(value)?;
    if !["github_pat_", "ghp_"]
        .iter()
        .any(|prefix| value.starts_with(prefix) && value.len() > prefix.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(
            "Enter a GitHub personal access token, not a Client ID or Copilot token".into(),
        );
    }
    Ok(())
}
fn headers(kind: Kind, credential: &str) -> Result<reqwest::header::HeaderMap, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    if !credential.is_empty() {
        validate_secret(credential)?;
        let name = if kind == Kind::Webiq {
            "x-apikey"
        } else {
            "authorization"
        };
        let value = if kind == Kind::Webiq {
            credential.into()
        } else {
            format!("Bearer {credential}")
        };
        let mut value = reqwest::header::HeaderValue::from_str(&value)
            .map_err(|_| "Invalid connector credential")?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    if kind == Kind::Github {
        headers.insert(
            "x-mcp-readonly",
            reqwest::header::HeaderValue::from_static("true"),
        );
        headers.insert(
            "x-mcp-toolsets",
            reqwest::header::HeaderValue::from_static(GITHUB_READ_TOOLSETS),
        );
    }
    Ok(headers)
}
fn client(kind: Kind, credential: &str) -> Result<reqwest::Client, String> {
    client_with_timeout(kind, credential, Duration::from_secs(30))
}

fn client_with_timeout(
    kind: Kind,
    credential: &str,
    timeout: Duration,
) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .default_headers(headers(kind, credential)?)
        .user_agent("Tomlook/0.1")
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .build()
        .map_err(|_| "Create connector HTTP client failed".into())
}
async fn json_response<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Connector returned HTTP {}. Check sign-in, app installation or organization approval.",
            response.status().as_u16()
        ));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Read connector response failed")?
    {
        if data.len() + chunk.len() > 32_768 {
            return Err("Connector authentication response exceeds its bound".into());
        }
        data.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&data).map_err(|_| "Connector authentication response is invalid".into())
}
async fn verify(
    kind: Kind,
    saved: &Saved,
    mut cancelled: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    if *cancelled.borrow() || *shutdown.borrow() {
        return Err("Connector verification cancelled before initialization".into());
    }
    let http = client_with_timeout(kind, &saved.credential, Duration::from_secs(60))?;
    let initialization = tokio::time::timeout(
        Duration::from_secs(30),
        verification_client_config().serve(StreamableHttpClientTransport::with_client(
            http,
            verification_transport_config(kind.endpoint()),
        )),
    );
    let service = tokio::select! {
        biased;
        _ = cancelled.changed() => return Err("Connector initialization cancelled".into()),
        _ = shutdown.changed() => return Err("Connector initialization stopped".into()),
        result = initialization => result,
    }
    .map_err(|_| "Connector initialization timed out")?
    .map_err(|error| initialization_error(kind, error))?;
    verify_service(kind, saved.login.as_deref(), service, cancelled, shutdown).await
}

fn verification_client_config() -> ClientConfig {
    // These providers use initialize, not the newer discover handshake.
    ClientConfig::default().with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
}

fn verification_transport_config(endpoint: &str) -> StreamableHttpClientTransportConfig {
    let mut transport = StreamableHttpClientTransportConfig::with_uri(endpoint);
    transport.allow_stateless = true;
    transport.max_concurrent_requests = 1;
    transport
}

fn http_reason(status: u16) -> String {
    let guidance = match status {
        401 => "credential rejected; check the key or sign-in",
        403 => "access denied; check provider permissions",
        429 => "provider rate limit; retry later",
        500..=599 => "provider unavailable; retry later",
        _ => "provider request failed",
    };
    format!("HTTP {status}: {guidance}")
}

fn transport_reason(error: &rmcp::transport::DynamicTransportError) -> String {
    use rmcp::transport::streamable_http_client::StreamableHttpError;
    let Some(error) = error
        .error
        .downcast_ref::<StreamableHttpError<reqwest::Error>>()
    else {
        return "transport failed".into();
    };
    match error {
        StreamableHttpError::Client(error) if error.is_timeout() => "HTTP request timed out".into(),
        StreamableHttpError::Client(error) if error.status().is_some() => {
            http_reason(error.status().expect("status checked").as_u16())
        }
        StreamableHttpError::Client(error) if error.is_connect() => {
            "cannot reach the provider".into()
        }
        StreamableHttpError::AuthRequired(_) => http_reason(401),
        StreamableHttpError::InsufficientScope(_) => http_reason(403),
        StreamableHttpError::UnexpectedServerResponse(detail) => {
            // Only retain the transport's numeric HTTP prefix, never its response body.
            detail
                .strip_prefix("HTTP ")
                .and_then(|text| text.get(..3))
                .filter(|code| code.bytes().all(|byte| byte.is_ascii_digit()))
                .and_then(|code| code.parse::<u16>().ok())
                .filter(|code| (100..=599).contains(code))
                .map(http_reason)
                .unwrap_or_else(|| "provider returned an invalid MCP response".into())
        }
        StreamableHttpError::UnexpectedContentType(_) => {
            "provider returned an unsupported response type".into()
        }
        StreamableHttpError::SessionExpired => "MCP session expired; reconnect".into(),
        StreamableHttpError::Deserialize(_) => "provider returned invalid MCP data".into(),
        _ => "transport/protocol failed".into(),
    }
}

fn initialization_error(kind: Kind, error: rmcp::service::ClientInitializeError) -> String {
    use rmcp::service::ClientInitializeError;
    let reason = match error {
        ClientInitializeError::TransportError { error, .. } => transport_reason(&error),
        ClientInitializeError::JsonRpcError(error) => format!("MCP code {}", error.code.0),
        ClientInitializeError::NoCompatibleProtocolVersion { .. } => {
            "no compatible MCP protocol".into()
        }
        _ => "invalid MCP initialization response".into(),
    };
    format!("{} initialization failed: {reason}", kind.label())
}

fn request_error(kind: Kind, phase: &str, error: rmcp::ServiceError) -> String {
    let reason = match error {
        rmcp::ServiceError::McpError(error) if error.code.0 == -32602 => {
            "invalid tool arguments (MCP code -32602); not a rejected key".into()
        }
        rmcp::ServiceError::McpError(error) => format!("MCP code {}", error.code.0),
        rmcp::ServiceError::TransportSend(error) => transport_reason(&error),
        rmcp::ServiceError::TransportClosed => "MCP transport closed before the response".into(),
        rmcp::ServiceError::Timeout { .. } => "MCP request timed out".into(),
        rmcp::ServiceError::UnexpectedResponse => "provider returned unexpected MCP data".into(),
        _ => "MCP request failed".into(),
    };
    format!("{} {phase} failed: {reason}", kind.label())
}

async fn verify_service(
    kind: Kind,
    expected_login: Option<&str>,
    service: RunningService<RoleClient, ClientConfig>,
    mut cancelled: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let peer = service.peer().clone();
    let check = async {
        let allowed = READ_ONLY
            .iter()
            .find(|(name, _)| *name == kind.server())
            .ok_or("Connector is not registered")?
            .1;
        let mut cursor = None;
        let mut seen = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        let mut probe_schema = None;
        let probe = if kind == Kind::Webiq { "web" } else { "get_me" };
        for page in 0..8 {
            let tools = peer
                .list_tools(cursor)
                .await
                .map_err(|error| request_error(kind, "tool discovery", error))?;
            for tool in tools.tools {
                if tool.name == probe {
                    probe_schema = Some(tool.input_schema.clone());
                }
                if names.len() >= 256 || !names.insert(tool.name.to_string()) {
                    return Err(
                        "Connector tool discovery exceeded its bound or repeated a tool".into(),
                    );
                }
                if allowed.contains(&tool.name.as_ref())
                    && tool
                        .input_schema
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        != Some("object")
                {
                    return Err("Connector read tool has an unsupported schema".into());
                }
            }
            cursor = match tools.next_cursor {
                None => break,
                Some(next) if page < 7 && seen.insert(next.clone()) => {
                    Some(rmcp::model::PaginatedRequestParams::default().with_cursor(Some(next)))
                }
                Some(_) => {
                    return Err("Connector tool discovery exceeded its pagination bound".into());
                }
            };
        }
        let required = if kind == Kind::Webiq {
            "web"
        } else {
            "get_file_contents"
        };
        if !names.contains(required) {
            return Err("Connector has no supported read tool".into());
        }
        let schema = probe_schema.ok_or(if kind == Kind::Github {
            "GitHub MCP did not expose the account verification tool (get_me). Check the context toolset; this is not a rejected sign-in."
        } else {
            "Connector has no supported verification read"
        })?;
        let arguments = if kind == Kind::Webiq {
            if schema
                .get("properties")
                .and_then(|value| value.get("query"))
                .and_then(|value| value.get("type"))
                .and_then(serde_json::Value::as_str)
                != Some("string")
            {
                return Err("WebIQ search has an unsupported query schema".into());
            }
            serde_json::json!({"query":"Microsoft", "maxResults":1, "maxLength":256, "contentFormat":"passage"})
        } else {
            serde_json::json!({})
        };
        let result = peer
            .call_tool(
                CallToolRequestParams::new(probe).with_arguments(
                    arguments
                        .as_object()
                        .cloned()
                        .ok_or("Invalid fixed verification arguments")?,
                ),
            )
            .await
            .map_err(|error| request_error(kind, "verification read", error))?;
        validate_verification_read(&result, expected_login)?;
        Ok(())
    };
    let result = if *cancelled.borrow() || *shutdown.borrow() {
        Err("Connector discovery cancelled before reading".into())
    } else {
        tokio::select! {
            biased;
            _ = cancelled.changed() => Err("Connector discovery cancelled".into()),
            _ = shutdown.changed() => Err("Connector discovery stopped".into()),
            result = tokio::time::timeout(Duration::from_secs(60), check) =>
                result.map_err(|_| "Connector discovery timed out".to_owned()).and_then(|result| result),
        }
    };
    tokio::time::timeout(Duration::from_secs(5), service.cancel())
        .await
        .map_err(|_| "Connector verification cleanup timed out")?
        .map_err(|_| "Connector verification cleanup failed")?;
    result
}

fn validate_verification_read(
    result: &CallToolResult,
    expected_login: Option<&str>,
) -> Result<(), String> {
    if serde_json::to_vec(result)
        .map_err(|_| "Encode connector verification read failed")?
        .len()
        > 1_048_576
    {
        return Err("Connector verification read exceeded its data bound".into());
    }
    if result.is_error == Some(true) {
        return Err("Connector verification read was rejected; provider content withheld".into());
    }
    let mut payloads = Vec::new();
    if let Some(payload) = &result.structured_content {
        payloads.push(payload.clone());
    }
    for content in &result.content {
        let text = content
            .as_text()
            .ok_or("Connector verification read returned unsupported content")?;
        if !text.text.trim().is_empty() {
            // Plain text is permitted for public search; GitHub identity needs structured JSON.
            let payload = serde_json::from_str::<serde_json::Value>(&text.text)
                .unwrap_or_else(|_| serde_json::Value::String(text.text.clone()));
            payloads.push(payload);
        }
    }
    if payloads.is_empty() || payloads.iter().all(|value| value.is_null()) {
        return Err("Connector verification read was empty; no sign-in saved".into());
    }
    if payloads.iter().any(provider_error) {
        return Err(
            "Connector verification read reported an error; provider content withheld".into(),
        );
    }
    if let Some(expected) = expected_login {
        let identities: Vec<_> = payloads
            .iter()
            .filter_map(|payload| payload.get("login").and_then(serde_json::Value::as_str))
            .collect();
        if identities.is_empty()
            || identities
                .iter()
                .any(|login| !login.eq_ignore_ascii_case(expected))
        {
            return Err("GitHub MCP identity does not match the authorized account".into());
        }
    }
    Ok(())
}

fn provider_error(value: &serde_json::Value) -> bool {
    value
        .get("statusCode")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|status| status >= 400)
        || value.get("success") == Some(&serde_json::Value::Bool(false))
        || value.get("error").is_some_and(|error| !error.is_null())
}

#[derive(Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}
impl Device {
    fn validate(&self) -> Result<(), String> {
        if self.verification_uri != "https://github.com/login/device"
            || self.device_code.len() != 40
            || !self
                .device_code
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.user_code.len() != 9
            || self.user_code.as_bytes()[4] != b'-'
            || !self.user_code.bytes().enumerate().all(|(index, byte)| {
                index == 4 || byte.is_ascii_uppercase() || byte.is_ascii_digit()
            })
            || !(1..=900).contains(&self.expires_in)
            || !(1..=60).contains(&self.interval)
        {
            return Err("GitHub returned invalid device authorization data".into());
        }
        Ok(())
    }
}
#[derive(Deserialize)]
struct Token {
    access_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    refresh_token_expires_in: Option<u64>,
    error: Option<String>,
    interval: Option<u64>,
}
impl Token {
    fn credentials(self, client_id: &str, login: Option<String>) -> Result<Saved, String> {
        let received_at = chrono::Utc::now().timestamp();
        if self.token_type.as_deref() != Some("bearer") || self.error.is_some() {
            return Err("GitHub token renewal was rejected; sign in again".into());
        }
        if self
            .expires_in
            .is_some_and(|seconds| !(1..=31_536_000).contains(&seconds))
            || self
                .refresh_token_expires_in
                .is_some_and(|seconds| !(1..=31_536_000).contains(&seconds))
            || self.refresh_token.is_some() != self.refresh_token_expires_in.is_some()
            || (self.refresh_token.is_some() && self.expires_in.is_none())
        {
            return Err("GitHub returned an invalid token lifetime".into());
        }
        let saved = Saved {
            credential: self
                .access_token
                .ok_or("GitHub did not return an access token")?,
            client_id: Some(client_id.into()),
            login,
            expires_at: self.expires_in.map(|seconds| received_at + seconds as i64),
            refresh_token: self.refresh_token,
            refresh_expires_at: self
                .refresh_token_expires_in
                .map(|seconds| received_at + seconds as i64),
        };
        validate_secret(&saved.credential)?;
        if let Some(token) = &saved.refresh_token {
            validate_secret(token)?;
        }
        Ok(saved)
    }
    fn poll_interval(&self, previous: u64) -> Result<Option<u64>, String> {
        match self.error.as_deref() {
            Some("authorization_pending") => Ok(Some(previous)),
            Some("slow_down") => {
                let interval = previous.saturating_add(5).max(self.interval.unwrap_or(0));
                if interval > 900 {
                    return Err(
                        "GitHub requested a polling delay beyond the sign-in deadline".into(),
                    );
                }
                Ok(Some(interval))
            }
            Some("access_denied") => Err("GitHub sign-in was declined".into()),
            Some("expired_token" | "token_expired") => Err("GitHub sign-in expired".into()),
            Some("incorrect_client_credentials" | "device_flow_disabled") => Err(
                "Check the GitHub App Client ID and enable Device flow in its registration".into(),
            ),
            Some(_) => Err("GitHub device sign-in failed; provider details withheld".into()),
            None if self.token_type.as_deref() == Some("bearer") => Ok(None),
            None => Err("GitHub returned an unsupported token type".into()),
        }
    }
}
async fn github_login(client_id: &str, progress: impl Fn(String)) -> Result<Saved, String> {
    validate_client_id(client_id)?;
    let http = client(Kind::Github, "")?;
    let device: Device = json_response(
        http.post("https://github.com/login/device/code")
            .header("accept", "application/json")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form(&[("client_id", client_id)]))
            .send()
            .await
            .map_err(|_| "GitHub device sign-in could not start")?,
    )
    .await?;
    device.validate()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(device.expires_in);
    progress(format!(
        "Enter {} at github.com/login/device",
        device.user_code
    ));
    let address = device.verification_uri.clone();
    tokio::task::spawn_blocking(move || crate::browser::open(&address))
        .await
        .map_err(|_| "GitHub browser opener stopped")??;
    let mut interval = device.interval;
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if tokio::time::Instant::now() >= deadline {
            return Err("GitHub sign-in expired; sign in again".into());
        }
        let token: Token = json_response(
            http.post("https://github.com/login/oauth/access_token")
                .header("accept", "application/json")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(form(&[
                    ("client_id", client_id),
                    ("device_code", &device.device_code),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ]))
                .send()
                .await
                .map_err(|_| "GitHub device sign-in polling failed")?,
        )
        .await?;
        if let Some(next) = token.poll_interval(interval)? {
            interval = next;
            continue;
        }
        return github_account_saved(token.credentials(client_id, None)?).await;
    }
}

async fn github_account(
    credential: String,
    client_id: Option<String>,
    expires_at: Option<i64>,
) -> Result<Saved, String> {
    #[derive(Deserialize)]
    struct Identity {
        login: String,
    }
    let identity: Identity = json_response(
        client(Kind::Github, &credential)?
            .get("https://api.github.com/user")
            .header("accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|_| "GitHub account verification failed")?,
    )
    .await?;
    let saved = Saved {
        credential,
        client_id,
        login: Some(identity.login),
        expires_at,
        refresh_token: None,
        refresh_expires_at: None,
    };
    saved.validate(Kind::Github)?;
    Ok(saved)
}

async fn github_account_saved(mut saved: Saved) -> Result<Saved, String> {
    let account = github_account(
        saved.credential.clone(),
        saved.client_id.clone(),
        saved.expires_at,
    )
    .await?;
    saved.login = account.login;
    saved.validate(Kind::Github)?;
    Ok(saved)
}

async fn renew(vault: &Vault, saved: Saved) -> Result<Saved, String> {
    renew_at(vault, saved, "https://github.com/login/oauth/access_token").await
}

async fn renew_at(vault: &Vault, saved: Saved, endpoint: &str) -> Result<Saved, String> {
    let client_id = saved
        .client_id
        .as_deref()
        .ok_or("Saved GitHub sign-in cannot be renewed; sign in again")?;
    let refresh_token = saved.refresh_token.as_deref().ok_or(
        "Saved GitHub sign-in has no refresh token; sign in once to enable automatic renewal",
    )?;
    if saved
        .refresh_expires_at
        .is_none_or(|time| time <= chrono::Utc::now().timestamp() + 60)
    {
        return Err("GitHub refresh token expired; sign in again".into());
    }
    let token: Token = json_response(
        client(Kind::Github, "")?
            .post(endpoint)
            .header("accept", "application/json")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form(&[
                ("client_id", client_id),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ]))
            .send()
            .await
            .map_err(|_| "GitHub token renewal could not reach the provider; retrying shortly")?,
    )
    .await?;
    let rotated = token.credentials(client_id, saved.login)?;
    if rotated.refresh_token.is_none() {
        return Err("GitHub token renewal returned no refresh token; sign in again".into());
    }
    rotated.validate(Kind::Github)?;
    // Rotation invalidates the old pair. Commit the encrypted replacement before cancellable MCP verification.
    vault.save(rotated.clone()).await?;
    Ok(rotated)
}

fn form(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish()
}

pub async fn remembered_client_id(root: &Path) -> Result<Option<String>, String> {
    Ok(Vault::bind(root, Kind::Github)
        .await?
        .load()
        .await?
        .and_then(|saved| saved.client_id))
}

pub async fn connect(
    root: &Path,
    kind: Kind,
    input: Option<Input>,
    mut cancelled: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
    progress: impl Fn(String),
) -> Result<Option<Saved>, String> {
    if input.as_ref().is_some_and(|input| input.kind() != kind) {
        return Err("Connector input does not match the selected provider".into());
    }
    let vault = Vault::bind(root, kind).await?;
    let interactive = input.is_some();
    let operation = async {
        let saved = match input {
            Some(Input::WebiqKey(credential)) => {
                validate_secret(&credential)?;
                Saved {
                    credential,
                    client_id: None,
                    login: None,
                    expires_at: None,
                    refresh_token: None,
                    refresh_expires_at: None,
                }
            }
            Some(Input::GithubApp(client_id)) => github_login(&client_id, &progress).await?,
            Some(Input::GithubToken(credential)) => {
                validate_github_token(&credential)?;
                github_account(credential, None, None).await?
            }
            None => match vault.load().await? {
                None => return Ok(None),
                Some(saved) => saved,
            },
        };
        Ok(Some(saved))
    };
    if *cancelled.borrow() || *shutdown.borrow() {
        return Err("Connector sign-in was cancelled".into());
    }
    let mut result = tokio::select! {
        biased;
        _ = cancelled.changed() => Err("Connector sign-in cancelled".into()),
        _ = shutdown.changed() => Err("Connector sign-in stopped during shutdown".into()),
        result = tokio::time::timeout(Duration::from_secs(960), operation) => result.map_err(|_| "Connector sign-in timed out".to_owned()).and_then(|result| result),
    }?;
    if *cancelled.borrow() || *shutdown.borrow() {
        return Err("Connector sign-in was cancelled before saving".into());
    }
    if kind == Kind::Github
        && result
            .as_ref()
            .is_some_and(|saved| saved.refresh_due() || !saved.available())
    {
        let saved = result.take().expect("saved sign-in checked");
        // Once dispatched, finish rotation and its encrypted commit even if shutdown is requested.
        result = Some(renew(&vault, saved).await?);
    }
    if let Some(saved) = &result {
        progress(format!(
            "Verifying {} read-only connection...",
            kind.label()
        ));
        verify(kind, saved, cancelled.clone(), shutdown.clone()).await?;
        if !saved.available() {
            return Err("GitHub sign-in expired during verification; sign in again".into());
        }
        if *cancelled.borrow() || *shutdown.borrow() {
            return Err("Connector sign-in was cancelled before saving".into());
        }
        if interactive {
            // A successful local commit completes sign-in; a later Cancel cannot undo it.
            vault.save(saved.clone()).await?;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    enum MockResponse {
        Read(serde_json::Value),
        HttpReadError(u16),
        RpcReadError(i32),
        Hold(&'static str),
        DuplicateTools,
        MissingVerificationRead,
    }

    struct Mock {
        service: RunningService<RoleClient, ClientConfig>,
        requests: tokio::sync::mpsc::Receiver<serde_json::Value>,
        stop: watch::Sender<bool>,
        task: tokio::task::JoinHandle<()>,
    }

    async fn mock(kind: Kind, response: MockResponse) -> Mock {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let (stop, mut stopped) = watch::channel(false);
        let (requests, received) = tokio::sync::mpsc::channel(16);
        let task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = stopped.changed() => break,
                    connection = listener.accept() => connection.unwrap(),
                };
                let (mut stream, _) = accepted;
                let mut bytes = Vec::new();
                let header_end = loop {
                    let byte = tokio::time::timeout(Duration::from_secs(5), stream.read_u8())
                        .await
                        .unwrap()
                        .unwrap();
                    bytes.push(byte);
                    assert!(bytes.len() <= 65_536);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break bytes.len();
                    }
                };
                let headers = std::str::from_utf8(&bytes).unwrap();
                let github_context = headers.lines().any(|line| {
                    line.split_once(':').is_some_and(|(name, value)| {
                        name.eq_ignore_ascii_case("x-mcp-toolsets")
                            && value.split(',').any(|toolset| toolset.trim() == "context")
                    })
                });
                if kind == Kind::Webiq {
                    assert!(headers.lines().any(|line| {
                        line.split_once(':').is_some_and(|(name, value)| {
                            name.eq_ignore_ascii_case("x-apikey")
                                && value.trim() == "synthetic-webiq-key"
                        })
                    }));
                    assert!(!headers.lines().any(|line| {
                        line.split_once(':')
                            .is_some_and(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    }));
                }
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(length <= 65_536);
                bytes.resize(header_end + length, 0);
                stream.read_exact(&mut bytes[header_end..]).await.unwrap();
                let request = if length == 0 {
                    None
                } else {
                    Some(serde_json::from_slice::<serde_json::Value>(&bytes[header_end..]).unwrap())
                };
                let method = request
                    .as_ref()
                    .and_then(|value| value["method"].as_str())
                    .unwrap_or("");
                if let Some(request) = &request {
                    requests.send(request.clone()).await.unwrap();
                }
                if method == "tools/call" {
                    let failure = match &response {
                        MockResponse::HttpReadError(status) => Some((
                            format!("{status} Fixture error"),
                            "text/plain",
                            "sensitive-provider-body synthetic-webiq-key".to_owned(),
                        )),
                        MockResponse::RpcReadError(code) => Some((
                            "200 OK".into(),
                            "application/json",
                            serde_json::json!({
                                "jsonrpc":"2.0", "id":request.as_ref().unwrap()["id"],
                                "error":{"code":code,"message":"sensitive-provider-body synthetic-webiq-key"}
                            }).to_string(),
                        )),
                        _ => None,
                    };
                    if let Some((status, content_type, body)) = failure {
                        stream.write_all(format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                        ).as_bytes()).await.unwrap();
                        stream.shutdown().await.unwrap();
                        continue;
                    }
                }
                if matches!(&response, MockResponse::Hold(phase) if *phase == method) {
                    tokio::select! {
                        _ = stopped.changed() => break,
                        closed = stream.read_u8() => {
                            assert!(closed.is_err(), "Cancelled verification left its request open");
                            requests.send(serde_json::json!({"method":"fixture/closed"})).await.unwrap();
                        }
                    }
                    continue;
                }
                let result = match method {
                    "initialize" => Some(serde_json::json!({
                        "protocolVersion":"2025-06-18", "capabilities":{"tools":{}},
                        "serverInfo":{"name":"tomlook-fixture","version":"1"}
                    })),
                    "tools/list" => {
                        let query_schema = serde_json::json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]});
                        let mut tools = if kind == Kind::Webiq {
                            vec![serde_json::json!({"name":"web","inputSchema":query_schema})]
                        } else {
                            let mut tools = vec![
                                serde_json::json!({"name":"get_file_contents","inputSchema":{"type":"object"}}),
                            ];
                            if github_context
                                && !matches!(response, MockResponse::MissingVerificationRead)
                            {
                                tools.push(serde_json::json!({"name":"get_me","inputSchema":{"type":"object"}}));
                            }
                            tools
                        };
                        if matches!(response, MockResponse::DuplicateTools) {
                            tools.push(tools[0].clone());
                        }
                        Some(serde_json::json!({"tools":tools}))
                    }
                    "tools/call" => match &response {
                        MockResponse::Read(value) => Some(value.clone()),
                        _ => {
                            Some(serde_json::json!({"content":[{"type":"text","text":"fixture"}]}))
                        }
                    },
                    _ => None,
                };
                let (status, body) = match result {
                    Some(result) => (
                        "200 OK",
                        serde_json::to_string(&serde_json::json!({
                            "jsonrpc":"2.0", "id":request.unwrap()["id"], "result":result
                        }))
                        .unwrap(),
                    ),
                    None if method.starts_with("notifications/") => ("202 Accepted", String::new()),
                    None => ("405 Method Not Allowed", String::new()),
                };
                stream.write_all(format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                ).as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        });
        let service = tokio::time::timeout(
            Duration::from_secs(5),
            verification_client_config().serve(StreamableHttpClientTransport::with_client(
                client(
                    kind,
                    if kind == Kind::Webiq {
                        "synthetic-webiq-key"
                    } else {
                        "synthetic-github-token"
                    },
                )
                .unwrap(),
                verification_transport_config(&endpoint),
            )),
        )
        .await
        .unwrap()
        .unwrap();
        Mock {
            service,
            requests: received,
            stop,
            task,
        }
    }

    async fn wait_method(
        requests: &mut tokio::sync::mpsc::Receiver<serde_json::Value>,
        method: &str,
    ) -> serde_json::Value {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let request = requests
                    .recv()
                    .await
                    .expect("Fixture ended before expected request");
                if request["method"] == method {
                    return request;
                }
            }
        })
        .await
        .unwrap()
    }

    fn web_key() -> Saved {
        Saved {
            credential: "synthetic-webiq-key".into(),
            client_id: None,
            login: None,
            expires_at: None,
            refresh_token: None,
            refresh_expires_at: None,
        }
    }

    fn github_token() -> Saved {
        Saved {
            credential: "synthetic-github-token".into(),
            client_id: Some("Iv23fixture".into()),
            login: Some("fixture-owner".into()),
            expires_at: Some(chrono::Utc::now().timestamp() + 3600),
            refresh_token: None,
            refresh_expires_at: None,
        }
    }

    fn github_personal_token() -> Saved {
        Saved {
            credential: "github_pat_synthetic_token".into(),
            client_id: None,
            login: Some("fixture-owner".into()),
            expires_at: None,
            refresh_token: None,
            refresh_expires_at: None,
        }
    }

    #[test]
    fn refresh_credentials_are_backward_compatible_bounded_and_do_not_invalidate_context() {
        let legacy: Saved = serde_json::from_value(serde_json::json!({
            "credential":"synthetic-legacy", "client_id":"Iv23fixture",
            "login":"fixture-owner", "expires_at":chrono::Utc::now().timestamp()+3600
        }))
        .unwrap();
        assert!(legacy.refresh_token.is_none());
        assert!(legacy.validate(Kind::Github).is_ok());
        let response: Token = serde_json::from_value(serde_json::json!({
            "access_token":"synthetic-new-access", "token_type":"bearer", "expires_in":28800,
            "refresh_token":"synthetic-new-refresh", "refresh_token_expires_in":15897600
        }))
        .unwrap();
        let mut saved = response
            .credentials("Iv23fixture", Some("fixture-owner".into()))
            .unwrap();
        assert!(saved.validate(Kind::Github).is_ok());
        assert!(saved.can_refresh());
        assert!(!saved.refresh_due());
        let first = Registry {
            records: [(Kind::Github, saved.clone())].into(),
        };
        saved.credential = "synthetic-rotated-access".into();
        saved.refresh_token = Some("synthetic-rotated-refresh".into());
        let second = Registry {
            records: [(Kind::Github, saved.clone())].into(),
        };
        assert_ne!(first.revision(), second.revision());
        assert_eq!(first.context_revision(), second.context_revision());
        let redactor = crate::evidence::Redactor::default().extended(second.secrets());
        assert!(
            !redactor
                .text("synthetic-rotated-access synthetic-rotated-refresh")
                .contains("synthetic-rotated")
        );
        saved.expires_at = Some(chrono::Utc::now().timestamp() + 240);
        assert!(saved.refresh_due());
        saved.refresh_expires_at = Some(chrono::Utc::now().timestamp() - 1);
        assert!(!saved.can_refresh());
        saved.refresh_expires_at = None;
        assert!(saved.validate(Kind::Github).is_err());
        for fields in [
            serde_json::json!({"expires_in":0}),
            serde_json::json!({"expires_in":28800, "refresh_token":"synthetic"}),
            serde_json::json!({"refresh_token":"synthetic", "refresh_token_expires_in":15897600}),
            serde_json::json!({"expires_in":28800, "refresh_token":"synthetic", "refresh_token_expires_in":0}),
        ] {
            let mut payload =
                serde_json::json!({"access_token":"synthetic", "token_type":"bearer"});
            payload
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let token: Token = serde_json::from_value(payload).unwrap();
            assert!(
                token
                    .credentials("Iv23fixture", Some("fixture-owner".into()))
                    .is_err()
            );
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn refresh_rotation_uses_device_flow_without_secret_and_persists_both_new_tokens() {
        let root = tempfile::tempdir().unwrap();
        let vault = Vault::bind(root.path(), Kind::Github).await.unwrap();
        let mut saved = github_token();
        saved.expires_at = Some(chrono::Utc::now().timestamp() - 1);
        saved.refresh_token = Some("synthetic-old-refresh".into());
        saved.refresh_expires_at = Some(chrono::Utc::now().timestamp() + 3600);
        vault.save(saved.clone()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(stream.read_u8().await.unwrap());
                assert!(bytes.len() < 32768);
            }
            let headers = String::from_utf8(bytes).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let values = url::form_urlencoded::parse(&body)
                .into_owned()
                .collect::<BTreeMap<_, _>>();
            assert_eq!(values["client_id"], "Iv23fixture");
            assert_eq!(values["grant_type"], "refresh_token");
            assert_eq!(values["refresh_token"], "synthetic-old-refresh");
            assert!(!values.contains_key("client_secret"));
            assert!(!headers.to_ascii_lowercase().contains("authorization:"));
            let body = serde_json::json!({
                "access_token":"synthetic-rotated-access", "token_type":"bearer", "expires_in":28800,
                "refresh_token":"synthetic-rotated-refresh", "refresh_token_expires_in":15897600
            }).to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let rotated =
            tokio::time::timeout(Duration::from_secs(3), renew_at(&vault, saved, &endpoint))
                .await
                .unwrap()
                .unwrap();
        server.await.unwrap();
        assert!(rotated.available());
        let reopened = Vault::bind(root.path(), Kind::Github)
            .await
            .unwrap()
            .load()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reopened.credential, "synthetic-rotated-access");
        assert_eq!(
            reopened.refresh_token.as_deref(),
            Some("synthetic-rotated-refresh")
        );
        assert_eq!(reopened.login.as_deref(), Some("fixture-owner"));
        let encrypted: Vec<u8> = database(&vault.path)
            .unwrap()
            .query_row(
                "SELECT payload FROM credentials WHERE provider=?1",
                [Kind::Github.server()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !encrypted
                .windows(b"synthetic-".len())
                .any(|window| window == b"synthetic-")
        );
    }

    #[test]
    fn personal_tokens_are_explicit_and_keep_legacy_device_credentials_valid() {
        for value in ["github_pat_fixture", "ghp_fixture"] {
            assert!(validate_github_token(value).is_ok());
        }
        for value in [
            "",
            "github_pat_",
            "ghp_",
            "Iv23fixture",
            "gho_fixture",
            "ghu_fixture",
            "github_pat_fixture\n",
            "github_pat_fixture.invalid",
        ] {
            let error = validate_github_token(value).unwrap_err();
            assert!(!error.contains(value) || value.is_empty());
        }
        let pat = github_personal_token();
        assert!(pat.validate(Kind::Github).is_ok());
        assert!(pat.validate(Kind::Webiq).is_err());
        assert!(pat.available());
        let restored: Saved = serde_json::from_slice(&serde_json::to_vec(&pat).unwrap()).unwrap();
        assert_eq!(restored.login, pat.login);
        assert!(restored.client_id.is_none());
        assert!(restored.validate(Kind::Github).is_ok());
        let mut invalid = restored;
        invalid.login = None;
        assert!(invalid.validate(Kind::Github).is_err());
        let mut old = github_token();
        assert!(old.validate(Kind::Github).is_ok());
        old.client_id = None;
        assert!(old.validate(Kind::Github).is_err());
        assert_eq!(Input::GithubToken(pat.credential).kind(), Kind::Github);
    }

    #[tokio::test]
    async fn provider_mismatch_and_invalid_personal_tokens_never_contact_or_save() {
        for (kind, input) in [
            (Kind::Webiq, Input::GithubToken("ghp_fixture".into())),
            (Kind::Github, Input::WebiqKey("synthetic-key".into())),
            (Kind::Github, Input::GithubToken("Iv23fixture".into())),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (_cancel, cancelled) = watch::channel(false);
            let (_stop, shutdown) = watch::channel(false);
            let error = connect(root.path(), kind, Some(input), cancelled, shutdown, |_| {
                panic!("Invalid admission must not contact the provider");
            })
            .await
            .err()
            .expect("Invalid admission must fail");
            assert!(error.contains("selected provider") || error.contains("personal access token"));
            assert!(
                Vault::bind(root.path(), kind)
                    .await
                    .unwrap()
                    .load()
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn credentials_and_device_data_are_bounded_and_provider_bound() {
        for value in ["", "a b", "line\nbreak", "tab\tvalue"] {
            assert!(validate_secret(value).is_err());
        }
        assert!(validate_secret(&"a".repeat(8001)).is_err());
        assert!(validate_secret("fixture-key").is_ok());
        assert!(validate_client_id("Iv1.fixture").is_ok());
        assert!(validate_client_id("Iv23fixture").is_ok());
        for value in [
            "",
            "ghp_fixture",
            "Ov23fixture",
            "Iv23 fixture",
            "https://github.com",
        ] {
            assert!(validate_client_id(value).is_err());
        }
        let valid = serde_json::json!({
            "device_code": "a".repeat(40), "user_code": "ABCD-EF12",
            "verification_uri": "https://github.com/login/device", "expires_in": 900, "interval": 5
        });
        assert!(
            serde_json::from_value::<Device>(valid.clone())
                .unwrap()
                .validate()
                .is_ok()
        );
        for (field, value) in [
            (
                "verification_uri",
                serde_json::json!("https://github.com.evil.example/login/device"),
            ),
            (
                "verification_uri",
                serde_json::json!("https://github.com/login/device?code=fixture"),
            ),
            ("device_code", serde_json::json!("z".repeat(40))),
            ("user_code", serde_json::json!("ABCD_EF12")),
            ("expires_in", serde_json::json!(901)),
            ("interval", serde_json::json!(0)),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(
                serde_json::from_value::<Device>(invalid)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut saved = github_token();
        saved.login = Some("identity\ninjection".into());
        assert!(saved.validate(Kind::Github).is_err());
        assert_ne!(binding(Kind::Github), binding(Kind::Webiq));
    }

    #[tokio::test]
    async fn client_id_is_recovered_from_an_expired_legacy_sign_in_without_provider_reads() {
        let root = tempfile::tempdir().unwrap();
        assert!(remembered_client_id(root.path()).await.unwrap().is_none());
        let vault = Vault::bind(root.path(), Kind::Github).await.unwrap();
        let mut saved = github_token();
        saved.expires_at = Some(chrono::Utc::now().timestamp() - 1);
        vault.save(saved).await.unwrap();
        assert_eq!(
            remembered_client_id(root.path()).await.unwrap().as_deref(),
            Some("Iv23fixture")
        );
    }

    #[test]
    fn verification_reads_reject_provider_errors_empty_data_and_wrong_github_identity() {
        let read = |value| serde_json::from_value::<CallToolResult>(value).unwrap();
        assert!(
            validate_verification_read(
                &read(
                    serde_json::json!({"content":[{"type":"text","text":"Public search fixture"}]})
                ),
                None
            )
            .is_ok()
        );
        assert!(
            validate_verification_read(
                &read(
                    serde_json::json!({"content":[],"structuredContent":{"login":"FIXTURE-OWNER"}})
                ),
                Some("fixture-owner")
            )
            .is_ok()
        );
        for value in [
            serde_json::json!({"content":[]}),
            serde_json::json!({"content":[{"type":"text","text":"   "}]}),
            serde_json::json!({"isError":true,"content":[{"type":"text","text":"sensitive-error"}]}),
            serde_json::json!({"content":[{"type":"text","text":"{\"statusCode\":401}"}]}),
            serde_json::json!({"content":[],"structuredContent":{"error":"sensitive-error"}}),
            serde_json::json!({"content":[],"structuredContent":{"success":false}}),
            serde_json::json!({"content":[{"type":"text","text":"x".repeat(1_048_576)}]}),
        ] {
            assert!(validate_verification_read(&read(value), None).is_err());
        }
        for value in [
            serde_json::json!({"content":[{"type":"text","text":"unstructured success"}]}),
            serde_json::json!({"content":[],"structuredContent":{"login":"other-owner"}}),
            serde_json::json!({"content":[{"type":"text","text":"{\"login\":\"other-owner\"}"}],"structuredContent":{"login":"fixture-owner"}}),
        ] {
            assert!(validate_verification_read(&read(value), Some("fixture-owner")).is_err());
        }
    }

    #[tokio::test]
    async fn verification_failures_report_safe_status_and_never_provider_body_or_key() {
        for (response, expected) in [
            (MockResponse::HttpReadError(401), "HTTP 401"),
            (MockResponse::HttpReadError(403), "HTTP 403"),
            (MockResponse::HttpReadError(429), "HTTP 429"),
            (MockResponse::HttpReadError(503), "HTTP 503"),
            (MockResponse::RpcReadError(-32602), "invalid tool arguments"),
        ] {
            let Mock {
                service,
                requests: _requests,
                stop,
                task,
            } = mock(Kind::Webiq, response).await;
            let (_cancel, cancelled) = watch::channel(false);
            let (_stop, shutdown) = watch::channel(false);
            let error = verify_service(Kind::Webiq, None, service, cancelled, shutdown)
                .await
                .unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(error.contains("WebIQ verification read"));
            assert!(!error.contains("sensitive-provider-body"));
            assert!(!error.contains("synthetic-webiq-key"));
            stop.send(true).unwrap();
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn stateless_http_verification_initializes_reads_and_closes_without_model_or_browser() {
        for kind in [Kind::Webiq, Kind::Github] {
            let payload = if kind == Kind::Github {
                serde_json::json!({"content":[{"type":"text","text":"{\"login\":\"fixture-owner\"}"}]})
            } else {
                serde_json::json!({"content":[{"type":"text","text":"Public search fixture"}]})
            };
            let Mock {
                service,
                mut requests,
                stop,
                task,
            } = mock(kind, MockResponse::Read(payload)).await;
            let peer = service.peer().clone();
            let (_cancel, cancelled) = watch::channel(false);
            let (_stop, shutdown) = watch::channel(false);
            verify_service(
                kind,
                (kind == Kind::Github).then_some("fixture-owner"),
                service,
                cancelled,
                shutdown,
            )
            .await
            .unwrap();
            let request = wait_method(&mut requests, "tools/call").await;
            assert_eq!(
                request["params"]["name"],
                if kind == Kind::Webiq { "web" } else { "get_me" }
            );
            if kind == Kind::Webiq {
                assert_eq!(request["params"]["arguments"]["query"], "Microsoft");
                assert_eq!(request["params"]["arguments"]["maxResults"], 1);
            }
            assert!(
                tokio::time::timeout(Duration::from_secs(1), peer.list_tools(None))
                    .await
                    .unwrap()
                    .is_err()
            );
            stop.send(true).unwrap();
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn missing_github_identity_tool_never_reads_or_reports_a_rejected_sign_in() {
        let Mock {
            service,
            mut requests,
            stop,
            task,
        } = mock(Kind::Github, MockResponse::MissingVerificationRead).await;
        let peer = service.peer().clone();
        let (_cancel, cancelled) = watch::channel(false);
        let (_stop, shutdown) = watch::channel(false);
        let error = verify_service(
            Kind::Github,
            Some("fixture-owner"),
            service,
            cancelled,
            shutdown,
        )
        .await
        .unwrap_err();
        assert!(error.contains("get_me"));
        assert!(error.contains("context toolset"));
        assert!(error.contains("not a rejected sign-in"));
        while let Ok(request) = requests.try_recv() {
            assert_ne!(request["method"], "tools/call");
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(1), peer.list_tools(None))
                .await
                .unwrap()
                .is_err()
        );
        stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn cancel_and_shutdown_close_pending_discovery_and_verification_reads() {
        for method in ["tools/list", "tools/call"] {
            for stop_app in [false, true] {
                let Mock {
                    service,
                    mut requests,
                    stop: server_stop,
                    task: server,
                } = mock(Kind::Webiq, MockResponse::Hold(method)).await;
                let peer = service.peer().clone();
                let (cancel, cancelled) = watch::channel(false);
                let (stop, shutdown) = watch::channel(false);
                let verification = tokio::spawn(verify_service(
                    Kind::Webiq,
                    None,
                    service,
                    cancelled,
                    shutdown,
                ));
                wait_method(&mut requests, method).await;
                if stop_app {
                    stop.send(true).unwrap();
                } else {
                    cancel.send(true).unwrap();
                }
                let result = tokio::time::timeout(Duration::from_secs(2), verification)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    matches!(result, Err(error) if error.contains("cancelled") || error.contains("stopped"))
                );
                assert!(
                    tokio::time::timeout(Duration::from_secs(1), peer.list_tools(None))
                        .await
                        .unwrap()
                        .is_err()
                );
                wait_method(&mut requests, "fixture/closed").await;
                server_stop.send(true).unwrap();
                tokio::time::timeout(Duration::from_secs(1), server)
                    .await
                    .unwrap()
                    .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn discovery_and_provider_failures_close_the_service_without_ready_fallback() {
        for response in [
            MockResponse::DuplicateTools,
            MockResponse::Read(
                serde_json::json!({"content":[{"type":"text","text":"{\"error\":\"sensitive-provider-error\"}"}]}),
            ),
        ] {
            let Mock {
                service,
                mut requests,
                stop,
                task,
            } = mock(Kind::Webiq, response).await;
            let peer = service.peer().clone();
            let (_cancel, cancelled) = watch::channel(false);
            let (_stop, shutdown) = watch::channel(false);
            let error = verify_service(Kind::Webiq, None, service, cancelled, shutdown)
                .await
                .unwrap_err();
            assert!(!error.contains("sensitive-provider-error"));
            assert!(
                tokio::time::timeout(Duration::from_secs(1), peer.list_tools(None))
                    .await
                    .unwrap()
                    .is_err()
            );
            if error.contains("repeated") {
                while let Ok(request) = requests.try_recv() {
                    assert_ne!(
                        request["method"], "tools/call",
                        "Malformed discovery must not send a read"
                    );
                }
            }
            stop.send(true).unwrap();
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[test]
    fn device_poll_respects_pending_slowdown_and_explicit_failures() {
        let token = |value| serde_json::from_value::<Token>(value).unwrap();
        assert_eq!(
            token(serde_json::json!({"error":"authorization_pending"}))
                .poll_interval(5)
                .unwrap(),
            Some(5)
        );
        assert_eq!(
            token(serde_json::json!({"error":"slow_down"}))
                .poll_interval(60)
                .unwrap(),
            Some(65)
        );
        assert_eq!(
            token(serde_json::json!({"error":"slow_down","interval":75}))
                .poll_interval(5)
                .unwrap(),
            Some(75)
        );
        assert!(
            token(serde_json::json!({"error":"slow_down","interval":901}))
                .poll_interval(5)
                .is_err()
        );
        for error in [
            "access_denied",
            "expired_token",
            "device_flow_disabled",
            "incorrect_client_credentials",
            "unrecognized-sensitive-error",
        ] {
            let result = token(serde_json::json!({"error":error}))
                .poll_interval(5)
                .unwrap_err();
            assert!(!result.contains("unrecognized-sensitive-error"));
        }
        assert!(
            token(serde_json::json!({"token_type":"unknown"}))
                .poll_interval(5)
                .is_err()
        );
        assert_eq!(
            token(serde_json::json!({"token_type":"bearer"}))
                .poll_interval(5)
                .unwrap(),
            None
        );
        assert_eq!(
            form(&[("client_id", "Iv1.fixture"), ("value", "a+b&c")]),
            "client_id=Iv1.fixture&value=a%2Bb%26c"
        );
    }

    #[test]
    fn registry_uses_fixed_hosts_and_expiry_changes_available_tools_and_identity() {
        let mut registry = Registry::default();
        assert!(registry.revision().is_empty());
        assert!(registry.expires_in().is_none());
        let mut config: crate::copilot::Config = serde_json::from_value(serde_json::json!({
            "runtime":"fixture.exe", "model":"fixture-model", "use_stored_login":true,
            "servers":{"github":{"url":"https://other.example/mcp","credential_env":"TOMLOOK_GITHUB_TOKEN"}}
        })).unwrap();
        registry.records.insert(Kind::Webiq, web_key());
        registry.records.insert(Kind::Github, github_token());
        let first = registry.revision();
        let credentials = registry.apply(&mut config);
        assert_eq!(credentials.len(), 2);
        for kind in [Kind::Webiq, Kind::Github] {
            assert_eq!(config.servers[kind.server()].url, kind.endpoint());
            assert!(config.servers[kind.server()].credential_env.is_none());
        }
        assert!(registry.expires_in().unwrap() > Duration::from_secs(3000));
        registry.records.get_mut(&Kind::Github).unwrap().credential = "replacement-fixture".into();
        assert_ne!(first, registry.revision());
        registry.records.get_mut(&Kind::Github).unwrap().expires_at =
            Some(chrono::Utc::now().timestamp() - 1);
        assert_eq!(registry.expires_in(), Some(Duration::ZERO));
        let credentials = registry.apply(&mut config);
        assert!(!credentials.contains_key("github"));
        assert!(!config.servers.contains_key("github"));
        assert_eq!(credentials.len(), 1);
        let redactor = crate::evidence::Redactor::default().extended(registry.secrets());
        assert!(
            !redactor
                .text("synthetic-webiq-key replacement-fixture")
                .contains("synthetic-webiq-key")
        );
        assert!(
            !redactor
                .partial("replacement-fixture")
                .contains("replacement-fixture")
        );
    }

    #[test]
    fn http_headers_are_read_only_sensitive_and_do_not_contain_ambient_credentials() {
        for kind in [Kind::Webiq, Kind::Github] {
            let headers = headers(kind, "synthetic-key").unwrap();
            let header = headers
                .get(if kind == Kind::Webiq {
                    "x-apikey"
                } else {
                    "authorization"
                })
                .unwrap();
            assert!(header.is_sensitive());
            assert_eq!(
                header.to_str().unwrap(),
                if kind == Kind::Webiq {
                    "synthetic-key"
                } else {
                    "Bearer synthetic-key"
                }
            );
            if kind == Kind::Github {
                assert_eq!(headers["x-mcp-readonly"], "true");
                assert_eq!(
                    headers["x-mcp-toolsets"],
                    "context,repos,issues,pull_requests"
                );
            }
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn vault_survives_restart_and_forgetting_fences_old_saves_per_provider() {
        let root = tempfile::tempdir().unwrap();
        let web = Vault::bind(root.path(), Kind::Webiq).await.unwrap();
        let github = Vault::bind(root.path(), Kind::Github).await.unwrap();
        web.save(web_key()).await.unwrap();
        github.save(github_token()).await.unwrap();
        let reopened = Vault::bind(root.path(), Kind::Webiq).await.unwrap();
        assert_eq!(
            reopened.load().await.unwrap().unwrap().credential,
            web_key().credential
        );
        let db = database(&web.path).unwrap();
        let encrypted: Vec<u8> = db
            .query_row(
                "SELECT payload FROM credentials WHERE provider=?1",
                [Kind::Webiq.server()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !encrypted
                .windows(web_key().credential.len())
                .any(|window| window == web_key().credential.as_bytes())
        );
        assert!(crate::credentials::protect(&encrypted, &binding(Kind::Github), false).is_err());
        drop(db);
        Vault::forget(root.path(), Kind::Webiq).await.unwrap();
        assert!(web.save(web_key()).await.is_err());
        assert!(web.load().await.is_err());
        let fresh = Vault::bind(root.path(), Kind::Webiq).await.unwrap();
        assert!(fresh.load().await.unwrap().is_none());
        assert!(github.load().await.unwrap().is_some());
        fresh.save(web_key()).await.unwrap();
        assert!(fresh.load().await.unwrap().is_some());
        let db = database(&fresh.path).unwrap();
        db.execute(
            "UPDATE credentials SET payload=X'000102' WHERE provider=?1",
            [Kind::Webiq.server()],
        )
        .unwrap();
        assert!(fresh.load().await.is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn personal_token_vault_restores_encrypts_replaces_and_revokes_without_app_id() {
        let root = tempfile::tempdir().unwrap();
        let vault = Vault::bind(root.path(), Kind::Github).await.unwrap();
        vault.save(github_token()).await.unwrap();
        let before = Registry {
            records: [(Kind::Github, vault.load().await.unwrap().unwrap())].into(),
        }
        .revision();
        vault.save(github_personal_token()).await.unwrap();
        let restored = Vault::bind(root.path(), Kind::Github)
            .await
            .unwrap()
            .load()
            .await
            .unwrap()
            .unwrap();
        assert!(restored.client_id.is_none());
        assert_eq!(restored.login.as_deref(), Some("fixture-owner"));
        let registry = Registry {
            records: [(Kind::Github, restored)].into(),
        };
        assert_ne!(before, registry.revision());
        assert!(registry.expires_in().is_none());
        let redactor = crate::evidence::Redactor::default().extended(registry.secrets());
        assert!(
            !redactor
                .text("github_pat_synthetic_token")
                .contains("github_pat_")
        );
        let db = database(&vault.path).unwrap();
        let encrypted: Vec<u8> = db
            .query_row(
                "SELECT payload FROM credentials WHERE provider=?1",
                [Kind::Github.server()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !encrypted
                .windows(b"github_pat_".len())
                .any(|window| window == b"github_pat_")
        );
        drop(db);
        Vault::forget(root.path(), Kind::Github).await.unwrap();
        assert!(vault.save(github_personal_token()).await.is_err());
        assert!(
            Vault::bind(root.path(), Kind::Github)
                .await
                .unwrap()
                .load()
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn cancel_and_shutdown_before_admission_never_save_a_key_or_contact_provider() {
        for stopping in [false, true] {
            for input in [
                Input::WebiqKey(web_key().credential),
                Input::GithubApp("Iv23fixture".into()),
                Input::GithubToken(github_personal_token().credential),
            ] {
                let root = tempfile::tempdir().unwrap();
                let kind = input.kind();
                let (_cancel, cancelled) = watch::channel(!stopping);
                let (_stop, shutdown) = watch::channel(stopping);
                let result = connect(root.path(), kind, Some(input), cancelled, shutdown, |_| {
                    panic!("No provider progress before admission")
                })
                .await;
                assert!(matches!(result, Err(error) if error.contains("cancelled")));
                let vault = Vault::bind(root.path(), kind).await.unwrap();
                assert!(vault.load().await.unwrap().is_none());
            }
        }
    }
}
