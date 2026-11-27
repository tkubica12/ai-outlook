//! Browser sign-in for the MSX Dataverse MCP endpoint.
//!
//! Dataverse admits only allowlisted MCP clients, so Tomlook signs in with the public client
//! that GitHub Copilot and VS Code use (authorization code with PKCE). Microsoft redirects to
//! vscode.dev, which forwards the result to a one-time loopback listener named in `state`.
//! The refresh token is saved encrypted for the current Windows user (DPAPI) in
//! `msx-login.db`; access tokens stay in memory. A browser opens only after an explicit user
//! action, and nothing here writes to MSX.
use super::auth::{self, AuthError, Token};
use base64::Engine as _;
use rusqlite::OptionalExtension;
use serde_json::Value;
use sha2::Digest as _;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

pub const CLIENT_ID: &str = "aebc6443-996d-45c2-90f0-388ff96faa56";
const REDIRECT: &str = "https://vscode.dev/redirect";
const SCOPE: &str =
    "openid profile offline_access https://microsoftsales.crm.dynamics.com/api/mcp/mcp.tools";
const AUTHORITY: &str = "https://login.microsoftonline.com";
const VAULT_FILE: &str = "msx-login.db";
const LOGIN_DEADLINE: Duration = Duration::from_secs(600);
const CONNECTION_DEADLINE: Duration = Duration::from_secs(5);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CALLBACK_BYTES: usize = 16_384;
const MAX_RESPONSE_BYTES: usize = 65_536;
const MAX_ACCESS_TOKEN: usize = 16_000;
const MAX_REFRESH_TOKEN: usize = 32_768;
const MAX_LIFETIME: i64 = 172_800;

pub const NETWORK: &str =
    "MSX could not reach Microsoft sign-in; check connectivity. Retrying automatically.";
pub const INVALID_RESPONSE: &str =
    "Microsoft sign-in returned an unusable response; MSX was not connected.";
pub const TENANT_MISMATCH: &str =
    "Microsoft sign-in returned a token for a different tenant; MSX was not connected.";
pub const HTTP_FAILED: &str = "Tomlook could not prepare a secure connection to Microsoft sign-in.";
pub const LISTENER_FAILED: &str =
    "Tomlook could not start its local sign-in listener; no browser was opened.";
pub const BROWSER_FAILED: &str = "Tomlook could not open your browser for the MSX sign-in.";
pub const LOGIN_FAILED: &str = "MSX sign-in failed or was blocked by tenant policy. Try again.";
pub const SAVE_FAILED: &str =
    "MSX sign-in succeeded, but Tomlook could not save it securely; MSX was not connected.";
pub const CLEAR_FAILED: &str = "The saved MSX sign-in could not be removed.";
const RECEIVED: &str = "Tomlook received the Microsoft sign-in. You can close this tab.";
const REJECTED: &str = "Invalid sign-in callback.";

/// Obtains Dataverse tokens. Only [`Grant::sign_in`] may open a browser.
#[async_trait::async_trait]
pub trait Grant: Send + Sync {
    /// Silent renewal from the saved sign-in; without one, an explicit sign-in is required.
    async fn refresh(&self) -> Result<Token, AuthError>;
    /// Explicit interactive browser sign-in; the result replaces the saved sign-in.
    async fn sign_in(&self) -> Result<Token, AuthError>;
    /// Removes the saved sign-in; a refresh still in flight cannot save it again.
    async fn clear(&self) -> Result<(), AuthError>;
}

/// Whether an MSX sign-in is saved in this profile; read-only, nothing is decrypted.
pub fn saved_sign_in_exists(root: &Path) -> bool {
    crate::credentials::has_records(&root.join(VAULT_FILE), "login")
}

pub struct BrowserGrant {
    tenant: String,
    authority: String,
    vault: Vault,
    /// Serializes refreshes and the final exchange of a sign-in, so a rotated refresh token
    /// is never overwritten by an older one.
    gate: tokio::sync::Mutex<()>,
}

impl BrowserGrant {
    pub fn new(root: &Path, tenant: &str) -> Self {
        Self::with_authority(root, tenant, AUTHORITY)
    }

    fn with_authority(root: &Path, tenant: &str, authority: &str) -> Self {
        Self {
            tenant: tenant.to_ascii_lowercase(),
            authority: authority.into(),
            vault: Vault::new(root, tenant),
            gate: tokio::sync::Mutex::new(()),
        }
    }

    async fn redeem(&self, form: &[(&str, &str)], stage: Stage) -> Result<Issued, AuthError> {
        let http = reqwest::Client::builder()
            .user_agent("Tomlook/0.1")
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|_| AuthError::unavailable(HTTP_FAILED))?;
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let mut response = http
            .post(format!(
                "{}/{}/oauth2/v2.0/token",
                self.authority, self.tenant
            ))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(|_| network())?;
        let status = response.status().as_u16();
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| network())? {
            if data.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(AuthError::unavailable(INVALID_RESPONSE));
            }
            data.extend_from_slice(&chunk);
        }
        issued(
            status,
            &data,
            &self.tenant,
            stage,
            chrono::Utc::now().timestamp(),
        )
    }
}

#[async_trait::async_trait]
impl Grant for BrowserGrant {
    async fn refresh(&self) -> Result<Token, AuthError> {
        let _gate = self.gate.lock().await;
        let saved = self
            .vault
            .load()
            .await
            .map_err(|_| AuthError::SignInRequired(auth::SIGN_IN_REQUIRED.into()))?
            .ok_or_else(|| AuthError::SignInRequired(auth::SIGN_IN_REQUIRED.into()))?;
        let issued = self
            .redeem(
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", CLIENT_ID),
                    ("refresh_token", &saved),
                    ("scope", SCOPE),
                ],
                Stage::Refresh,
            )
            .await?;
        if let Some(rotated) = issued.refresh {
            // The previous refresh token stays valid, so a failed local save only matters
            // when the user removed the sign-in meanwhile.
            if self.vault.save(rotated).await == Err(VaultError::Removed) {
                return Err(AuthError::SignInRequired(auth::SIGN_IN_REQUIRED.into()));
            }
        }
        Ok(issued.token)
    }

    async fn sign_in(&self) -> Result<Token, AuthError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| AuthError::unavailable(LISTENER_FAILED))?;
        let port = listener
            .local_addr()
            .map_err(|_| AuthError::unavailable(LISTENER_FAILED))?
            .port();
        let expected = Expected::new(port, random()?, &self.tenant);
        let verifier = random()?;
        let url = authorize_url(&self.authority, &self.tenant, &expected.state, &verifier);
        tokio::task::spawn_blocking(move || crate::browser::open(&url))
            .await
            .map_err(|_| AuthError::unavailable(BROWSER_FAILED))?
            .map_err(|_| AuthError::unavailable(BROWSER_FAILED))?;
        let callback = tokio::time::timeout(LOGIN_DEADLINE, await_callback(&listener, &expected))
            .await
            .map_err(|_| AuthError::SignInRequired(auth::LOGIN_TIMED_OUT.into()))??;
        drop(listener);
        let code = match callback {
            Callback::Code(code) => code,
            Callback::Error(provider) => return Err(failed(provider.as_deref())),
        };
        let _gate = self.gate.lock().await;
        self.vault
            .rebind()
            .await
            .map_err(|_| AuthError::unavailable(SAVE_FAILED))?;
        let issued = self
            .redeem(
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", CLIENT_ID),
                    ("code", &code),
                    ("redirect_uri", REDIRECT),
                    ("code_verifier", &verifier),
                    ("scope", SCOPE),
                ],
                Stage::Exchange,
            )
            .await?;
        let refresh = issued
            .refresh
            .ok_or_else(|| AuthError::unavailable(INVALID_RESPONSE))?;
        match self.vault.save(refresh).await {
            Ok(()) => Ok(issued.token),
            Err(VaultError::Removed) => {
                Err(AuthError::SignInRequired(auth::LOGIN_CANCELLED.into()))
            }
            Err(VaultError::Failed) => Err(AuthError::unavailable(SAVE_FAILED)),
        }
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.vault
            .clear()
            .await
            .map_err(|_| AuthError::unavailable(CLEAR_FAILED))
    }
}

fn network() -> AuthError {
    AuthError::Unavailable {
        message: NETWORK.into(),
        transient: true,
    }
}

fn random() -> Result<String, AuthError> {
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| AuthError::unavailable(LISTENER_FAILED))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn authorize_url(authority: &str, tenant: &str, state: &str, verifier: &str) -> String {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT),
            ("state", state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("scope", SCOPE),
            ("prompt", "select_account"),
        ])
        .finish();
    format!("{authority}/{tenant}/oauth2/v2.0/authorize?{query}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Exchange,
    Refresh,
}

fn failed(provider: Option<&str>) -> AuthError {
    AuthError::SignInRequired(match provider {
        Some(code) => format!("MSX sign-in failed (Microsoft sign-in reported {code})."),
        None => LOGIN_FAILED.into(),
    })
}

struct Issued {
    token: Token,
    refresh: Option<String>,
}

fn secret(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

/// Maps a token endpoint response to a token or a safe status; provider text is never shown.
fn issued(
    status: u16,
    body: &[u8],
    tenant: &str,
    stage: Stage,
    now: i64,
) -> Result<Issued, AuthError> {
    match status {
        200 => {}
        408 | 429 | 500..=599 => {
            return Err(AuthError::Unavailable {
                message: format!(
                    "Microsoft sign-in returned HTTP {status}; retrying automatically."
                ),
                transient: true,
            });
        }
        400..=499 => {
            let text = String::from_utf8_lossy(body);
            let provider = auth::aadsts(&text);
            return Err(match stage {
                Stage::Exchange => failed(provider),
                Stage::Refresh => AuthError::SignInRequired(match provider {
                    Some(code) => {
                        format!("MSX sign-in required (Microsoft sign-in reported {code}).")
                    }
                    None => auth::SIGN_IN_REQUIRED.into(),
                }),
            });
        }
        _ => return Err(AuthError::unavailable(INVALID_RESPONSE)),
    }
    let invalid = || AuthError::unavailable(INVALID_RESPONSE);
    let value: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let access = value["access_token"]
        .as_str()
        .filter(|token| secret(token, MAX_ACCESS_TOKEN))
        .ok_or_else(invalid)?;
    if !value["token_type"]
        .as_str()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("bearer"))
    {
        return Err(invalid());
    }
    let lifetime = value["expires_in"]
        .as_i64()
        .or_else(|| value["expires_in"].as_str()?.parse().ok())
        .filter(|lifetime| *lifetime <= MAX_LIFETIME)
        .ok_or_else(invalid)?;
    if lifetime <= 60 {
        return Err(AuthError::SignInRequired(auth::SIGN_IN_REQUIRED.into()));
    }
    let refresh = match value.get("refresh_token") {
        None | Some(Value::Null) => None,
        Some(Value::String(token)) if secret(token, MAX_REFRESH_TOKEN) => Some(token.clone()),
        Some(_) => return Err(invalid()),
    };
    let claims =
        auth::claims(access, tenant).map_err(|_| AuthError::unavailable(TENANT_MISMATCH))?;
    Ok(Issued {
        token: Token::new(access.into(), now + lifetime, claims),
        refresh,
    })
}

struct Expected {
    port: u16,
    nonce: String,
    state: String,
    issuer: String,
}

impl Expected {
    fn new(port: u16, nonce: String, tenant: &str) -> Self {
        Self {
            port,
            state: format!("http://127.0.0.1:{port}/?nonce={nonce}"),
            nonce,
            issuer: format!("{AUTHORITY}/{tenant}/v2.0"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Callback {
    Code(String),
    /// The provider's error; only an AADSTS code is kept.
    Error(Option<String>),
}

fn parse_callback(request: &str, expected: &Expected) -> Result<Callback, &'static str> {
    let mut lines = request.split("\r\n");
    let request_line = lines.next().ok_or("Missing callback request line")?;
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "GET"
        || !matches!(parts[2], "HTTP/1.1" | "HTTP/1.0")
        || !parts[1].starts_with("/?")
        || parts[1].contains('#')
    {
        return Err("Callback method or target rejected");
    }
    let host = format!("127.0.0.1:{}", expected.port);
    let hosts = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
    if hosts != [host.as_str()] {
        return Err("Callback Host header rejected");
    }
    let url = url::Url::parse(&format!("http://{host}{}", parts[1]))
        .map_err(|_| "Invalid callback URL")?;
    if url.path() != "/" {
        return Err("Callback path rejected");
    }
    let (mut code, mut error, mut provider) = (None, None, None);
    let (mut state, mut nonce, mut issuer) = (None, None, None);
    let mut seen = std::collections::BTreeSet::new();
    for (name, value) in url.query_pairs() {
        if matches!(
            name.as_ref(),
            "code" | "state" | "iss" | "error" | "error_description" | "nonce"
        ) && !seen.insert(name.to_string())
        {
            return Err("Duplicate OAuth callback parameter rejected");
        }
        match name.as_ref() {
            "code" if !value.is_empty() && value.len() <= 8192 => code = Some(value.into_owned()),
            "error"
                if !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') =>
            {
                error = Some(value.into_owned());
            }
            "code" | "error" => return Err("Invalid OAuth callback parameter"),
            "error_description" => provider = auth::aadsts(&value).map(str::to_owned),
            "state" => state = Some(value.into_owned()),
            "nonce" => nonce = Some(value.into_owned()),
            "iss" => issuer = Some(value.into_owned()),
            _ => {}
        }
    }
    if nonce.as_deref() != Some(expected.nonce.as_str())
        || state.as_deref() != Some(expected.state.as_str())
    {
        return Err("OAuth callback state rejected");
    }
    if issuer
        .as_deref()
        .is_some_and(|issuer| !issuer.eq_ignore_ascii_case(&expected.issuer))
    {
        return Err("OAuth callback issuer rejected");
    }
    match (code, error) {
        (Some(code), None) => Ok(Callback::Code(code)),
        (None, Some(_)) => Ok(Callback::Error(provider)),
        _ => Err("Callback must contain either a code or an error"),
    }
}

async fn read_callback(stream: &mut TcpStream) -> Result<String, &'static str> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 2048];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream
            .read(&mut buffer)
            .await
            .map_err(|_| "Read loopback callback failed")?;
        if count == 0 || bytes.len() + count > MAX_CALLBACK_BYTES {
            return Err("Incomplete or oversized loopback callback");
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).map_err(|_| "Callback is not valid UTF-8")
}

async fn reply(stream: &mut TcpStream, status: &str, message: &str) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Tomlook sign-in</title></head><body><p>{message}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await
}

/// Waits for the one valid callback. Invalid or slow requests are answered and ignored, so
/// another local process cannot end the sign-in.
async fn await_callback(
    listener: &TcpListener,
    expected: &Expected,
) -> Result<Callback, AuthError> {
    loop {
        let (mut stream, peer) = listener
            .accept()
            .await
            .map_err(|_| AuthError::unavailable(LISTENER_FAILED))?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let parsed = match tokio::time::timeout(CONNECTION_DEADLINE, read_callback(&mut stream))
            .await
        {
            Ok(Ok(request)) => parse_callback(&request, expected),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("Loopback callback exceeded its deadline"),
        };
        let (status, message) = match parsed {
            Ok(_) => ("200 OK", RECEIVED),
            Err(_) => ("400 Bad Request", REJECTED),
        };
        let _ = tokio::time::timeout(CONNECTION_DEADLINE, reply(&mut stream, status, message)).await;
        if let Ok(callback) = parsed {
            return Ok(callback);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VaultError {
    /// The user removed the saved sign-in after this vault bound to it.
    Removed,
    Failed,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    client_id: String,
    tenant: String,
    refresh_token: String,
}

#[derive(Clone)]
struct Vault {
    path: PathBuf,
    tenant: String,
    generation: Arc<StdMutex<Option<i64>>>,
}

impl Vault {
    fn new(root: &Path, tenant: &str) -> Self {
        Self {
            path: root.join(VAULT_FILE),
            tenant: tenant.to_ascii_lowercase(),
            generation: Arc::new(StdMutex::new(None)),
        }
    }

    fn database(&self) -> Result<rusqlite::Connection, VaultError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| VaultError::Failed)?;
        }
        let db = rusqlite::Connection::open(&self.path).map_err(|_| VaultError::Failed)?;
        db.busy_timeout(Duration::from_secs(2))
            .map_err(|_| VaultError::Failed)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS login(account TEXT PRIMARY KEY, payload BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS login_generation(account TEXT PRIMARY KEY, generation INTEGER NOT NULL);",
        )
        .map_err(|_| VaultError::Failed)?;
        Ok(db)
    }

    fn current(&self, db: &rusqlite::Connection) -> Result<i64, VaultError> {
        db.query_row(
            "SELECT generation FROM login_generation WHERE account=?1",
            [&self.tenant],
            |row| row.get(0),
        )
        .optional()
        .map(|generation| generation.unwrap_or(0))
        .map_err(|_| VaultError::Failed)
    }

    fn check_generation(&self, db: &rusqlite::Connection) -> Result<(), VaultError> {
        let current = self.current(db)?;
        let mut bound = self.generation.lock().map_err(|_| VaultError::Failed)?;
        if bound.is_some_and(|generation| generation != current) {
            return Err(VaultError::Removed);
        }
        *bound = Some(current);
        Ok(())
    }

    /// Binds to the current saved sign-in; only an explicit sign-in may do so after removal.
    async fn rebind(&self) -> Result<(), VaultError> {
        self.run(|vault| {
            let current = vault.current(&vault.database()?)?;
            *vault.generation.lock().map_err(|_| VaultError::Failed)? = Some(current);
            Ok(())
        })
        .await
    }

    async fn load(&self) -> Result<Option<String>, VaultError> {
        self.run(|vault| {
            let db = vault.database()?;
            vault.check_generation(&db)?;
            let encrypted: Option<Vec<u8>> = db
                .query_row(
                    "SELECT payload FROM login WHERE account=?1",
                    [&vault.tenant],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| VaultError::Failed)?;
            let Some(encrypted) = encrypted else {
                return Ok(None);
            };
            if encrypted.len() > 131_072 {
                return Err(VaultError::Failed);
            }
            let mut plain = vault.protect(&encrypted, false)?;
            let saved: Result<Saved, _> = serde_json::from_slice(&plain);
            plain.fill(0);
            let saved = saved.map_err(|_| VaultError::Failed)?;
            if saved.client_id != CLIENT_ID
                || saved.tenant != vault.tenant
                || !secret(&saved.refresh_token, MAX_REFRESH_TOKEN)
            {
                return Err(VaultError::Failed);
            }
            Ok(Some(saved.refresh_token))
        })
        .await
    }

    async fn save(&self, refresh_token: String) -> Result<(), VaultError> {
        self.run(move |vault| {
            let saved = Saved {
                client_id: CLIENT_ID.into(),
                tenant: vault.tenant.clone(),
                refresh_token,
            };
            let mut plain = serde_json::to_vec(&saved).map_err(|_| VaultError::Failed)?;
            drop(saved);
            if plain.len() > 65_536 {
                plain.fill(0);
                return Err(VaultError::Failed);
            }
            let encrypted = vault.protect(&plain, true);
            plain.fill(0);
            let encrypted = encrypted?;
            let mut db = vault.database()?;
            let transaction = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| VaultError::Failed)?;
            vault.check_generation(&transaction)?;
            transaction
                .execute(
                    "INSERT OR REPLACE INTO login(account,payload) VALUES(?1,?2)",
                    rusqlite::params![vault.tenant, encrypted],
                )
                .map_err(|_| VaultError::Failed)?;
            transaction.commit().map_err(|_| VaultError::Failed)
        })
        .await
    }

    async fn clear(&self) -> Result<(), VaultError> {
        self.run(|vault| {
            let mut db = vault.database()?;
            db.execute_batch("PRAGMA secure_delete=ON;")
                .map_err(|_| VaultError::Failed)?;
            let transaction = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| VaultError::Failed)?;
            transaction
                .execute("DELETE FROM login WHERE account=?1", [&vault.tenant])
                .map_err(|_| VaultError::Failed)?;
            transaction
                .execute(
                    "INSERT INTO login_generation VALUES(?1,1) ON CONFLICT(account) DO UPDATE SET generation=generation+1",
                    [&vault.tenant],
                )
                .map_err(|_| VaultError::Failed)?;
            transaction.commit().map_err(|_| VaultError::Failed)
        })
        .await
    }

    fn protect(&self, data: &[u8], encrypt: bool) -> Result<Vec<u8>, VaultError> {
        let binding = format!("Tomlook MSX v1:{}:{CLIENT_ID}", self.tenant);
        crate::credentials::protect(data, &binding, encrypt).map_err(|_| VaultError::Failed)
    }

    async fn run<T: Send + 'static>(
        &self,
        action: impl FnOnce(Self) -> Result<T, VaultError> + Send + 'static,
    ) -> Result<T, VaultError> {
        let vault = self.clone();
        tokio::task::spawn_blocking(move || action(vault))
            .await
            .map_err(|_| VaultError::Failed)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msx::auth::tests::{TENANT, jwt};
    use serde_json::json;

    fn access(tenant: &str) -> String {
        jwt(json!({
            "tid": tenant,
            "upn": "someone@example.test",
            "oid": "11111111-2222-3333-4444-555555555555"
        }))
    }

    fn body(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    fn expected() -> Expected {
        Expected::new(50123, "fixture-nonce".into(), TENANT)
    }

    fn request(query: &str) -> String {
        format!("GET /?{query} HTTP/1.1\r\nHost: 127.0.0.1:50123\r\nAccept: */*\r\n\r\n")
    }

    fn query(pairs: &[(&str, &str)]) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish()
    }

    fn callback_query(extra: &[(&str, &str)]) -> String {
        let expected = expected();
        let mut pairs = vec![
            ("nonce", "fixture-nonce"),
            ("state", expected.state.as_str()),
        ];
        pairs.extend_from_slice(extra);
        query(&pairs)
    }

    #[test]
    fn the_authorize_url_uses_pkce_and_names_the_loopback_listener_in_state() {
        let expected = expected();
        assert_eq!(expected.state, "http://127.0.0.1:50123/?nonce=fixture-nonce");
        let address = url::Url::parse(&authorize_url(AUTHORITY, TENANT, &expected.state, "verifier"))
            .unwrap();
        assert_eq!(
            address.as_str().split('?').next().unwrap(),
            format!("https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/authorize")
        );
        let pairs = address
            .query_pairs()
            .into_owned()
            .collect::<std::collections::BTreeMap<_, _>>();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(b"verifier"));
        assert_eq!(pairs["client_id"], CLIENT_ID);
        assert_eq!(pairs["redirect_uri"], "https://vscode.dev/redirect");
        assert_eq!(pairs["state"], expected.state);
        assert_eq!(pairs["code_challenge"], challenge);
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(pairs["scope"], SCOPE);
        assert_eq!(pairs["response_type"], "code");
        assert!(!pairs.contains_key("code_verifier"));
        let (first, second) = (random().unwrap(), random().unwrap());
        assert_eq!(first.len(), 43);
        assert_ne!(first, second);
    }

    #[test]
    fn a_valid_callback_yields_its_code_or_only_the_provider_error_code() {
        let expected = expected();
        let issuer = format!("https://login.microsoftonline.com/{TENANT}/v2.0");
        assert_eq!(
            parse_callback(
                &request(&callback_query(&[
                    ("code", "fixture-code"),
                    ("session_state", "ignored"),
                    ("iss", &issuer)
                ])),
                &expected
            ),
            Ok(Callback::Code("fixture-code".into()))
        );
        assert_eq!(
            parse_callback(
                &request(&callback_query(&[
                    ("error", "access_denied"),
                    ("error_description", "AADSTS53003: Access has been blocked by secret policy text")
                ])),
                &expected
            ),
            Ok(Callback::Error(Some("AADSTS53003".into())))
        );
    }

    #[test]
    fn forged_or_malformed_callbacks_are_rejected() {
        let expected = expected();
        let valid = callback_query(&[("code", "fixture-code")]);
        let rejected = [
            request(&query(&[("nonce", "other"), ("state", &expected.state), ("code", "c")])),
            request(&query(&[("nonce", "fixture-nonce"), ("state", "http://127.0.0.1:50123/?nonce=other"), ("code", "c")])),
            request(&callback_query(&[("code", "a"), ("code", "b")])),
            request(&callback_query(&[("code", "a"), ("error", "access_denied")])),
            request(&callback_query(&[])),
            request(&callback_query(&[("error", "bad value")])),
            request(&callback_query(&[("code", "c"), ("iss", "https://login.microsoftonline.com/other/v2.0")])),
            request(&format!("{valid}#fragment")),
            format!("POST /?{valid} HTTP/1.1\r\nHost: 127.0.0.1:50123\r\n\r\n"),
            format!("GET /other?{valid} HTTP/1.1\r\nHost: 127.0.0.1:50123\r\n\r\n"),
            format!("GET /?{valid} HTTP/1.1\r\nHost: localhost:50123\r\n\r\n"),
            format!("GET /?{valid} HTTP/1.1\r\nHost: 127.0.0.1:50123\r\nHost: 127.0.0.1:50123\r\n\r\n"),
        ];
        for request in rejected {
            assert!(parse_callback(&request, &expected).is_err(), "{request}");
        }
    }

    #[tokio::test]
    async fn invalid_loopback_requests_are_answered_without_ending_the_sign_in() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected = Expected::new(port, "fixture-nonce".into(), TENANT);
        let state = expected.state.clone();
        let client = tokio::spawn(async move {
            let send = |request: String| async move {
                let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                response
            };
            let forged = send(format!(
                "GET /?nonce=wrong&code=c HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
            ))
            .await;
            let valid = send(format!(
                "GET /?{} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n",
                query(&[("nonce", "fixture-nonce"), ("state", &state), ("code", "fixture-code")])
            ))
            .await;
            (forged, valid)
        });
        let callback = await_callback(&listener, &expected).await.unwrap();
        assert_eq!(callback, Callback::Code("fixture-code".into()));
        let (forged, valid) = client.await.unwrap();
        assert!(forged.starts_with("HTTP/1.1 400"));
        assert!(valid.starts_with("HTTP/1.1 200") && valid.contains("You can close this tab"));
        assert!(!valid.contains("fixture-code"));
    }

    #[test]
    fn token_responses_require_a_bearer_token_for_the_tenant() {
        let now = 1_000;
        let first = issued(
            200,
            &body(json!({
                "access_token": access(TENANT), "token_type": "Bearer",
                "expires_in": "3599", "refresh_token": "fixture-refresh"
            })),
            TENANT,
            Stage::Exchange,
            now,
        )
        .unwrap();
        assert_eq!(first.token.expires, now + 3599);
        assert_eq!(
            first.token.claims.account.as_deref(),
            Some("someone@example.test")
        );
        assert_eq!(first.refresh.as_deref(), Some("fixture-refresh"));
        let without = issued_body(json!({
            "access_token": access(TENANT), "token_type": "bearer", "expires_in": 3600
        }))
        .unwrap();
        assert!(without.refresh.is_none());
        let invalid = AuthError::unavailable(INVALID_RESPONSE);
        for value in [
            json!({"access_token": access(TENANT), "token_type": "pop", "expires_in": 3600}),
            json!({"access_token": "a b", "token_type": "Bearer", "expires_in": 3600}),
            json!({"access_token": access(TENANT), "token_type": "Bearer"}),
            json!({"access_token": access(TENANT), "token_type": "Bearer", "expires_in": 3600, "refresh_token": 5}),
            json!({"access_token": access(TENANT), "token_type": "Bearer", "expires_in": 9_999_999}),
        ] {
            assert_eq!(issued_body(value).err(), Some(invalid.clone()));
        }
        assert_eq!(
            issued_body(json!({
                "access_token": access("00000000-0000-0000-0000-000000000009"),
                "token_type": "Bearer", "expires_in": 3600
            }))
            .err(),
            Some(AuthError::unavailable(TENANT_MISMATCH))
        );
        assert!(matches!(
            issued_body(json!({"access_token": access(TENANT), "token_type": "Bearer", "expires_in": 30})),
            Err(AuthError::SignInRequired(_))
        ));
        assert_eq!(
            issued(200, b"not json", TENANT, Stage::Refresh, now).err(),
            Some(invalid)
        );
    }

    fn issued_body(value: Value) -> Result<Issued, AuthError> {
        issued(200, &body(value), TENANT, Stage::Refresh, 1_000)
    }

    #[test]
    fn token_endpoint_failures_map_to_safe_statuses() {
        let rejected = body(json!({
            "error": "invalid_grant",
            "error_description": "AADSTS700082: The refresh token has expired. secret detail"
        }));
        assert_eq!(
            issued(400, &rejected, TENANT, Stage::Refresh, 0).err(),
            Some(AuthError::SignInRequired(
                "MSX sign-in required (Microsoft sign-in reported AADSTS700082).".into()
            ))
        );
        assert_eq!(
            issued(400, &rejected, TENANT, Stage::Exchange, 0).err(),
            Some(AuthError::SignInRequired(
                "MSX sign-in failed (Microsoft sign-in reported AADSTS700082).".into()
            ))
        );
        assert_eq!(
            issued(401, b"{}", TENANT, Stage::Refresh, 0).err(),
            Some(AuthError::SignInRequired(auth::SIGN_IN_REQUIRED.into()))
        );
        assert_eq!(
            issued(400, b"{}", TENANT, Stage::Exchange, 0).err(),
            Some(AuthError::SignInRequired(LOGIN_FAILED.into()))
        );
        for status in [408, 429, 503] {
            let error = issued(status, b"", TENANT, Stage::Refresh, 0).err().unwrap();
            assert!(matches!(error, AuthError::Unavailable { transient: true, .. }));
            assert!(crate::retry::transient(error.message()), "{error:?}");
        }
        assert_eq!(
            issued(302, b"", TENANT, Stage::Refresh, 0).err(),
            Some(AuthError::unavailable(INVALID_RESPONSE))
        );
        assert!(crate::retry::transient(NETWORK));
        assert!(!crate::retry::transient(LOGIN_FAILED));
    }

    #[cfg(windows)]
    mod vault {
        use super::*;

        #[tokio::test]
        async fn the_saved_sign_in_survives_reopen_and_removal_blocks_a_late_save() {
            let root = tempfile::tempdir().unwrap();
            assert!(!saved_sign_in_exists(root.path()));
            let vault = Vault::new(root.path(), TENANT);
            assert_eq!(vault.load().await, Ok(None));
            vault.save("fixture-refresh".into()).await.unwrap();
            assert!(saved_sign_in_exists(root.path()));
            let reopened = Vault::new(root.path(), &TENANT.to_ascii_uppercase());
            assert_eq!(reopened.load().await, Ok(Some("fixture-refresh".into())));
            let payload: Vec<u8> = vault
                .database()
                .unwrap()
                .query_row("SELECT payload FROM login", [], |row| row.get(0))
                .unwrap();
            assert!(!payload.windows(15).any(|window| window == b"fixture-refresh"));
            reopened.clear().await.unwrap();
            assert!(!saved_sign_in_exists(root.path()));
            assert_eq!(
                vault.save("late-rotation".into()).await,
                Err(VaultError::Removed)
            );
            assert_eq!(vault.load().await, Err(VaultError::Removed));
            vault.rebind().await.unwrap();
            vault.save("signed-in-again".into()).await.unwrap();
            assert_eq!(
                Vault::new(root.path(), TENANT).load().await,
                Ok(Some("signed-in-again".into()))
            );
        }

        #[tokio::test]
        async fn damaged_or_foreign_records_are_not_restored() {
            let root = tempfile::tempdir().unwrap();
            let vault = Vault::new(root.path(), TENANT);
            vault.save("fixture-refresh".into()).await.unwrap();
            let other = Vault::new(root.path(), "00000000-0000-0000-0000-000000000009");
            assert_eq!(other.load().await, Ok(None));
            let db = vault.database().unwrap();
            db.execute(
                "INSERT INTO login SELECT ?1,payload FROM login WHERE account=?2",
                ["00000000-0000-0000-0000-000000000009", TENANT],
            )
            .unwrap();
            assert_eq!(other.load().await, Err(VaultError::Failed));
            db.execute("UPDATE login SET payload=X'000102' WHERE account=?1", [TENANT])
                .unwrap();
            assert_eq!(vault.load().await, Err(VaultError::Failed));
        }

        async fn token_server(
            responses: Vec<(u16, Value)>,
        ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let authority = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let mut forms = Vec::new();
                for (status, value) in responses {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut data = Vec::new();
                    let mut buffer = [0; 4096];
                    loop {
                        let count = stream.read(&mut buffer).await.unwrap();
                        data.extend_from_slice(&buffer[..count]);
                        let text = String::from_utf8_lossy(&data).to_string();
                        if let Some((head, form)) = text.split_once("\r\n\r\n") {
                            let length = head
                                .lines()
                                .find_map(|line| {
                                    line.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|value| value.trim().parse::<usize>().unwrap())
                                })
                                .unwrap();
                            if form.len() >= length {
                                forms.push(form.to_owned());
                                break;
                            }
                        }
                    }
                    let payload = value.to_string();
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                        payload.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
                forms
            });
            (authority, server)
        }

        #[tokio::test]
        async fn refresh_rotates_the_saved_sign_in_and_needs_one_to_start() {
            let root = tempfile::tempdir().unwrap();
            let (authority, server) = token_server(vec![
                (
                    200,
                    json!({"access_token": access(TENANT), "token_type": "Bearer",
                        "expires_in": 3600, "refresh_token": "rotated-refresh"}),
                ),
                (
                    400,
                    json!({"error": "invalid_grant", "error_description": "AADSTS50173: revoked"}),
                ),
            ])
            .await;
            let grant = BrowserGrant::with_authority(root.path(), TENANT, &authority);
            assert_eq!(
                grant.refresh().await.unwrap_err(),
                AuthError::SignInRequired(auth::SIGN_IN_REQUIRED.into())
            );
            grant.vault.save("first-refresh".into()).await.unwrap();
            let token = grant.refresh().await.unwrap();
            assert_eq!(token.claims.account.as_deref(), Some("someone@example.test"));
            assert_eq!(
                grant.vault.load().await,
                Ok(Some("rotated-refresh".into()))
            );
            assert_eq!(
                grant.refresh().await.unwrap_err(),
                AuthError::SignInRequired(
                    "MSX sign-in required (Microsoft sign-in reported AADSTS50173).".into()
                )
            );
            let forms = server.await.unwrap();
            let first = url::form_urlencoded::parse(forms[0].as_bytes())
                .into_owned()
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(first["grant_type"], "refresh_token");
            assert_eq!(first["client_id"], CLIENT_ID);
            assert_eq!(first["refresh_token"], "first-refresh");
            assert_eq!(first["scope"], SCOPE);
            assert!(forms[1].contains("refresh_token=rotated-refresh"));
            grant.clear().await.unwrap();
            assert_eq!(
                grant.refresh().await.unwrap_err(),
                AuthError::SignInRequired(auth::SIGN_IN_REQUIRED.into())
            );
            assert!(!saved_sign_in_exists(root.path()));
        }
    }
}
