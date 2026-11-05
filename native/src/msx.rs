//! MSX (Dataverse) and Azure consumption (Power BI) read-only data layer.
//!
//! Tokens come from the user's Azure CLI sign-in and stay in memory. Background work never
//! starts an interactive sign-in; only [`Client::sign_in`] does, after an explicit user action.
//! Only allowlisted read tools are ever called, and every response is bounded.
pub mod acr;
pub mod auth;
pub mod dataverse;
pub mod mcp;
pub mod tools;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::watch;

pub const DEFAULT_TENANT: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
pub const DATAVERSE_ORIGIN: &str = "https://microsoftsales.crm.dynamics.com";
pub const DATAVERSE_MCP: &str = "https://microsoftsales.crm.dynamics.com/api/mcp";
pub const POWERBI_RESOURCE: &str = "https://analysis.windows.net/powerbi/api";
pub const POWERBI_MCP: &str = "https://api.fabric.microsoft.com/v1/mcp/powerbi";
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
const PAYLOAD_LIMIT: usize = 2 * 1024 * 1024;

pub const NOT_CONNECTED: &str = "Not connected. MSX stays idle until you sign in.";
pub const CHECKING: &str = "Checking the MSX connection...";
pub const CONNECTED: &str = "Connected read-only to Dataverse and Power BI.";
pub const DATAVERSE_ONLY: &str =
    "Dataverse connected; Azure consumption (Power BI) is unavailable.";
pub const POWERBI_ONLY: &str = "Power BI connected; Dataverse is unavailable.";
pub const DISABLED: &str = "MSX is turned off in Settings.";
pub const DEMO: &str = "Offline demo never connects MSX.";
pub const DISCONNECTED: &str =
    "Disconnected. In-memory MSX tokens were forgotten; your Azure CLI sign-in is unchanged.";
pub const BROWSER_PROMPT: &str =
    "Complete the Microsoft sign-in that Azure CLI opened (browser or Windows prompt).";
pub const DEVICE_REQUEST: &str = "Requesting a device code from Azure CLI (up to 60 seconds)...";
pub const DEVICE_PROMPT: &str = "Enter this code on Microsoft's device sign-in page.";
pub const CHECK_CANCELLED: &str = "MSX check cancelled.";
pub const INVALID_TENANT: &str = "The MSX tenant must be a Microsoft Entra tenant ID (GUID).";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Resource {
    Dataverse,
    PowerBi,
}

impl Resource {
    pub const ALL: [Self; 2] = [Self::Dataverse, Self::PowerBi];

    pub fn uri(self) -> &'static str {
        match self {
            Self::Dataverse => DATAVERSE_ORIGIN,
            Self::PowerBi => POWERBI_RESOURCE,
        }
    }

    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Dataverse => DATAVERSE_MCP,
            Self::PowerBi => POWERBI_MCP,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dataverse => "Dataverse",
            Self::PowerBi => "Power BI",
        }
    }

    fn required_tool(self) -> &'static str {
        match self {
            Self::Dataverse => "read_query",
            Self::PowerBi => "ExecuteQuery",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Dataverse => 0,
            Self::PowerBi => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub tenant: String,
}

pub fn validate_tenant(tenant: &str) -> Result<(), String> {
    if auth::uuid(tenant) {
        Ok(())
    } else {
        Err(INVALID_TENANT.into())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    NotConnected,
    Checking,
    SigningIn,
    Connected,
    SignInRequired,
    Unavailable,
    Disabled,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotConnected => "Not connected",
            Self::Checking => "Checking...",
            Self::SigningIn => "Signing in...",
            Self::Connected => "Connected",
            Self::SignInRequired => "Sign-in required",
            Self::Unavailable => "Unavailable",
            Self::Disabled => "Off",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Service {
    pub phase: Phase,
    pub message: String,
}

impl Service {
    fn new(phase: Phase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub phase: Phase,
    pub message: String,
    pub tenant: String,
    /// Azure sign-in identity from the token; not an MSX customer account.
    pub account: Option<String>,
    pub dataverse: Service,
    pub powerbi: Service,
    pub device_code: Option<String>,
    /// Allowlisted read tools found on the connected services.
    pub tools: Vec<String>,
}

impl Status {
    pub fn new(tenant: &str) -> Self {
        let mut status = Self {
            tenant: tenant.into(),
            ..Default::default()
        };
        status.summarize();
        status
    }

    pub fn disabled() -> Self {
        Self {
            phase: Phase::Disabled,
            message: DISABLED.into(),
            dataverse: Service::new(Phase::Disabled, ""),
            powerbi: Service::new(Phase::Disabled, ""),
            ..Default::default()
        }
    }

    pub fn demo() -> Self {
        Self {
            phase: Phase::Unavailable,
            message: DEMO.into(),
            dataverse: Service::new(Phase::Unavailable, DEMO),
            powerbi: Service::new(Phase::Unavailable, DEMO),
            ..Default::default()
        }
    }

    pub fn label(&self) -> &'static str {
        match self.phase {
            Phase::NotConnected => "MSX: not connected",
            Phase::Checking => "MSX: checking...",
            Phase::SigningIn => "MSX: signing in...",
            Phase::Connected => "MSX: connected",
            Phase::SignInRequired => "MSX: sign-in required",
            Phase::Unavailable => "MSX: unavailable",
            Phase::Disabled => "MSX: off",
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Checking | Phase::SigningIn)
    }

    pub fn service(&self, resource: Resource) -> &Service {
        match resource {
            Resource::Dataverse => &self.dataverse,
            Resource::PowerBi => &self.powerbi,
        }
    }

    fn service_mut(&mut self, resource: Resource) -> &mut Service {
        match resource {
            Resource::Dataverse => &mut self.dataverse,
            Resource::PowerBi => &mut self.powerbi,
        }
    }

    /// Derives the indicator state from both services.
    fn summarize(&mut self) {
        use Phase::*;
        if self.phase == Disabled {
            return;
        }
        let (dataverse, powerbi) = (self.dataverse.phase, self.powerbi.phase);
        let failed = [&self.dataverse, &self.powerbi]
            .into_iter()
            .find(|service| matches!(service.phase, SignInRequired | Unavailable))
            .map(|service| service.message.clone());
        let (phase, message) = if dataverse == SigningIn || powerbi == SigningIn {
            (SigningIn, self.message.clone())
        } else if dataverse == Checking || powerbi == Checking {
            (Checking, CHECKING.into())
        } else if dataverse == Connected && powerbi == Connected {
            (Connected, CONNECTED.into())
        } else if dataverse == Connected {
            (Connected, DATAVERSE_ONLY.into())
        } else if dataverse == SignInRequired || powerbi == SignInRequired {
            let message = [&self.dataverse, &self.powerbi]
                .into_iter()
                .find(|service| service.phase == SignInRequired)
                .map(|service| service.message.clone())
                .unwrap_or_default();
            (SignInRequired, message)
        } else if powerbi == Connected {
            (Connected, POWERBI_ONLY.into())
        } else if let Some(message) = failed {
            (Unavailable, message)
        } else {
            let message = [&self.dataverse, &self.powerbi]
                .into_iter()
                .map(|service| service.message.clone())
                .find(|message| !message.is_empty())
                .unwrap_or_else(|| NOT_CONNECTED.into());
            (NotConnected, message)
        };
        self.phase = phase;
        self.message = message;
        if phase != SigningIn {
            self.device_code = None;
        }
    }
}

/// Where a typed result came from.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Source {
    pub system: &'static str,
    pub table: String,
    pub query: String,
    pub retrieved_at: DateTime<Utc>,
    pub rows: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sourced<T> {
    pub value: T,
    pub source: Source,
}

/// A failed check or sign-in; `retry` marks a transient failure for backoff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub retry: bool,
}

impl From<auth::AuthError> for Failure {
    fn from(error: auth::AuthError) -> Self {
        Self {
            retry: matches!(
                error,
                auth::AuthError::Unavailable {
                    transient: true,
                    ..
                }
            ),
            message: error.message().into(),
        }
    }
}

pub type Publish = Arc<dyn Fn(Status) + Send + Sync>;

struct Live {
    session: Arc<dyn mcp::Session>,
    tools: Arc<BTreeMap<String, mcp::ToolInfo>>,
    discovered: Vec<String>,
    expires: i64,
    claims: auth::Claims,
}

pub(crate) struct Connection {
    session: Arc<dyn mcp::Session>,
    tools: Arc<BTreeMap<String, mcp::ToolInfo>>,
    claims: auth::Claims,
}

pub struct Client {
    tokens: auth::Tokens,
    connector: Arc<dyn mcp::Connector>,
    live: [tokio::sync::Mutex<Option<Live>>; 2],
    schemas: std::sync::Mutex<BTreeMap<String, Arc<dataverse::TableSchema>>>,
    model: tokio::sync::Mutex<Option<Arc<acr::ModelSchema>>>,
    /// The resolved MSX user and the token identity it was resolved for.
    user: tokio::sync::Mutex<Option<(auth::Claims, Sourced<dataverse::SystemUser>)>>,
    /// The user's account-team accounts and the token identity they were read for.
    territory: tokio::sync::Mutex<Option<(auth::Claims, Sourced<Vec<dataverse::AccountCandidate>>)>>,
    status: std::sync::Mutex<Status>,
    publish: Publish,
}

fn phase_of(error: &auth::AuthError) -> Phase {
    match error {
        auth::AuthError::SignInRequired(_) => Phase::SignInRequired,
        auth::AuthError::Unavailable { .. } => Phase::Unavailable,
    }
}

fn mcp_error(resource: Resource, error: mcp::CallError) -> auth::AuthError {
    match error {
        mcp::CallError::Unauthorized => auth::AuthError::SignInRequired(format!(
            "{} rejected the Azure CLI token; sign in to MSX again.",
            resource.label()
        )),
        mcp::CallError::Transport(reason) => auth::AuthError::Unavailable {
            message: format!("{} is unreachable: {reason}", resource.label()),
            transient: true,
        },
        mcp::CallError::Rejected(reason) => auth::AuthError::Unavailable {
            message: format!("{} refused the connection: {reason}", resource.label()),
            transient: false,
        },
    }
}

impl Client {
    /// A client that uses the installed Azure CLI and the live MCP endpoints.
    pub fn new(settings: &Settings, publish: Publish) -> Self {
        Self::with(
            &settings.tenant,
            Arc::new(auth::CliRunner::installed()),
            Arc::new(mcp::RmcpConnector),
            publish,
        )
    }

    pub fn with(
        tenant: &str,
        runner: Arc<dyn auth::Runner>,
        connector: Arc<dyn mcp::Connector>,
        publish: Publish,
    ) -> Self {
        Self {
            tokens: auth::Tokens::new(runner, tenant),
            connector,
            live: [tokio::sync::Mutex::new(None), tokio::sync::Mutex::new(None)],
            schemas: std::sync::Mutex::new(BTreeMap::new()),
            model: tokio::sync::Mutex::new(None),
            user: tokio::sync::Mutex::new(None),
            territory: tokio::sync::Mutex::new(None),
            status: std::sync::Mutex::new(Status::new(tenant)),
            publish,
        }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    pub fn ready(&self, resource: Resource) -> bool {
        self.status().service(resource).phase == Phase::Connected
    }

    fn update(&self, change: impl FnOnce(&mut Status)) {
        let changed = {
            let Ok(mut status) = self.status.lock() else {
                return;
            };
            let before = status.clone();
            change(&mut status);
            status.summarize();
            (*status != before).then(|| status.clone())
        };
        if let Some(status) = changed {
            (self.publish)(status);
        }
    }

    fn mark(&self, resource: Resource, error: &auth::AuthError) {
        self.update(|status| {
            *status.service_mut(resource) = Service::new(phase_of(error), error.message());
            status
                .tools
                .retain(|tool| !tool.starts_with(resource.label()));
        });
    }

    /// The connected session, reconnecting with a silent token refresh when needed.
    pub(crate) async fn connection(
        &self,
        resource: Resource,
    ) -> Result<Connection, auth::AuthError> {
        let mut slot = self.live[resource.index()].lock().await;
        let now = Utc::now().timestamp();
        if let Some(live) = slot
            .as_ref()
            .filter(|live| live.expires > now + auth::REFRESH_MARGIN)
        {
            return Ok(Connection {
                session: live.session.clone(),
                tools: live.tools.clone(),
                claims: live.claims.clone(),
            });
        }
        if let Some(old) = slot.take() {
            old.session.close().await;
        }
        match self.open(resource).await {
            Ok(live) => {
                let connection = Connection {
                    session: live.session.clone(),
                    tools: live.tools.clone(),
                    claims: live.claims.clone(),
                };
                let names = live
                    .tools
                    .keys()
                    .map(|name| format!("{}: {name}", resource.label()))
                    .collect::<Vec<_>>();
                let account = live.claims.account.clone();
                *slot = Some(live);
                self.update(|status| {
                    *status.service_mut(resource) =
                        Service::new(Phase::Connected, "Connected (read-only).");
                    if account.is_some() {
                        status.account = account;
                    }
                    status
                        .tools
                        .retain(|tool| !tool.starts_with(resource.label()));
                    status.tools.extend(names);
                });
                Ok(connection)
            }
            Err(error) => {
                self.mark(resource, &error);
                Err(error)
            }
        }
    }

    async fn open(&self, resource: Resource) -> Result<Live, auth::AuthError> {
        let token = self.tokens.token(resource).await?;
        let session = tokio::time::timeout(
            CONNECT_TIMEOUT,
            self.connector.connect(resource, token.secret()),
        )
        .await
        .map_err(|_| auth::AuthError::Unavailable {
            message: format!("{} connection timed out", resource.label()),
            transient: true,
        })?
        .map_err(|error| mcp_error(resource, error))?;
        let discovered = match tokio::time::timeout(CONNECT_TIMEOUT, session.list_tools()).await {
            Ok(Ok(tools)) => tools,
            Ok(Err(error)) => {
                session.close().await;
                return Err(mcp_error(resource, error));
            }
            Err(_) => {
                session.close().await;
                return Err(auth::AuthError::Unavailable {
                    message: format!("{} tool discovery timed out", resource.label()),
                    transient: true,
                });
            }
        };
        let mut names = discovered
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        let tools = discovered
            .into_iter()
            .filter(|tool| mcp::allowed(resource, tool))
            .map(|tool| (tool.name.clone(), tool))
            .collect::<BTreeMap<_, _>>();
        if !tools.contains_key(resource.required_tool()) {
            session.close().await;
            return Err(auth::AuthError::Unavailable {
                message: format!(
                    "{} does not offer its read tool {} to this account.",
                    resource.label(),
                    resource.required_tool()
                ),
                transient: false,
            });
        }
        Ok(Live {
            session,
            tools: Arc::new(tools),
            discovered: names,
            expires: token.expires,
            claims: token.claims,
        })
    }

    async fn drop_session(&self, resource: Resource) {
        if let Some(live) = self.live[resource.index()].lock().await.take() {
            live.session.close().await;
        }
    }

    /// Calls one allowlisted read tool. A rejected token is refreshed silently once; transient
    /// transport failures drop the session so the next read reconnects.
    pub(crate) async fn call(
        &self,
        resource: Resource,
        tool: &str,
        arguments: Map<String, Value>,
        detail: bool,
    ) -> Result<Value, String> {
        for attempt in 0..2 {
            let connection = self
                .connection(resource)
                .await
                .map_err(|error| error.message().to_owned())?;
            if !connection.tools.contains_key(tool) {
                return Err(format!(
                    "{} does not offer the read tool {tool} on this connection",
                    resource.label()
                ));
            }
            let result = tokio::time::timeout(
                CALL_TIMEOUT,
                connection.session.call(tool, arguments.clone()),
            )
            .await;
            let error = match result {
                Ok(Ok(result)) => {
                    return mcp::payload(&result, PAYLOAD_LIMIT).map_err(|error| {
                        match mcp::rejection_detail(&result).filter(|_| detail) {
                            Some(detail) => format!("{error}: {detail}"),
                            None => error,
                        }
                    });
                }
                Ok(Err(mcp::CallError::Unauthorized)) if attempt == 0 => {
                    self.drop_session(resource).await;
                    self.tokens.forget(resource).await;
                    continue;
                }
                Ok(Err(mcp::CallError::Rejected(reason))) => {
                    return Err(format!("{} read was rejected: {reason}", resource.label()));
                }
                Ok(Err(error)) => mcp_error(resource, error),
                Err(_) => auth::AuthError::Unavailable {
                    message: format!("{} read timed out", resource.label()),
                    transient: true,
                },
            };
            self.drop_session(resource).await;
            // A token the service keeps rejecting needs a new sign-in; later reads fail fast.
            self.tokens.latch(&error);
            self.mark(resource, &error);
            return Err(error.message().into());
        }
        Err(format!("{} read was not completed", resource.label()))
    }

    /// Discovered (all) and allowlisted tool names of a live session, for diagnostics.
    pub async fn tool_names(&self, resource: Resource) -> Option<(Vec<String>, Vec<String>)> {
        let slot = self.live[resource.index()].lock().await;
        slot.as_ref().map(|live| {
            (
                live.discovered.clone(),
                live.tools.keys().cloned().collect(),
            )
        })
    }

    /// Quiet or explicit connection check. Never interactive; a background token read waits
    /// at most 20 seconds and a sign-in-required result is not retried.
    pub async fn check(
        &self,
        explicit: bool,
        mut cancelled: watch::Receiver<bool>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), Failure> {
        if explicit {
            self.tokens.unlatch();
        }
        self.update(|status| {
            status.dataverse = Service::new(Phase::Checking, "");
            status.powerbi = Service::new(Phase::Checking, "");
        });
        let operation = async {
            let dataverse = self.connection(Resource::Dataverse).await.map(|_| ());
            if dataverse.is_ok() {
                let _ = self.whoami().await;
            }
            let powerbi = self.connection(Resource::PowerBi).await.map(|_| ());
            dataverse.and(powerbi)
        };
        let result = if *cancelled.borrow() || *shutdown.borrow() {
            None
        } else {
            tokio::select! {
                biased;
                _ = cancelled.changed() => None,
                _ = shutdown.changed() => None,
                result = operation => Some(result),
            }
        };
        match result {
            Some(result) => result.map_err(Failure::from),
            None => {
                self.update(|status| {
                    for resource in Resource::ALL {
                        let service = status.service_mut(resource);
                        if service.phase == Phase::Checking {
                            *service = Service::new(Phase::NotConnected, CHECK_CANCELLED);
                        }
                    }
                });
                Err(Failure {
                    message: CHECK_CANCELLED.into(),
                    retry: false,
                })
            }
        }
    }

    /// Explicit interactive sign-in (browser/WAM, or a device code), then a connection check.
    pub async fn sign_in(
        self: &Arc<Self>,
        device_code: bool,
        cancelled: watch::Receiver<bool>,
        shutdown: watch::Receiver<bool>,
    ) -> Result<(), Failure> {
        for resource in Resource::ALL {
            self.drop_session(resource).await;
        }
        // The sign-in may switch accounts; the next read resolves the user again.
        *self.user.lock().await = None;
        self.update(|status| {
            status.message = if device_code {
                DEVICE_REQUEST
            } else {
                BROWSER_PROMPT
            }
            .into();
            status.device_code = None;
            status.dataverse = Service::new(Phase::SigningIn, "");
            status.powerbi = Service::new(Phase::SigningIn, "");
        });
        let client = Arc::downgrade(self);
        let progress: auth::Progress = Arc::new(move |code: String| {
            let client = client.upgrade().ok_or("MSX sign-in ended")?;
            client.update(|status| {
                status.message = DEVICE_PROMPT.into();
                status.device_code = Some(code);
            });
            Ok(())
        });
        match self
            .tokens
            .sign_in(device_code, progress, cancelled.clone(), shutdown.clone())
            .await
        {
            Ok(()) => self.check(false, cancelled, shutdown).await,
            Err(error) => {
                self.update(|status| {
                    status.dataverse = Service::new(phase_of(&error), error.message());
                    status.powerbi = Service::new(phase_of(&error), error.message());
                });
                Err(error.into())
            }
        }
    }

    /// Forgets in-memory tokens, sessions and caches. The Azure CLI sign-in is not changed, and
    /// nothing reconnects until the user checks the connection or signs in again.
    pub async fn disconnect(&self) {
        for resource in Resource::ALL {
            self.drop_session(resource).await;
        }
        self.tokens.clear().await;
        self.tokens
            .latch(&auth::AuthError::SignInRequired(DISCONNECTED.into()));
        if let Ok(mut schemas) = self.schemas.lock() {
            schemas.clear();
        }
        *self.model.lock().await = None;
        *self.user.lock().await = None;
        self.update(|status| {
            status.account = None;
            status.tools.clear();
            status.dataverse = Service::new(Phase::NotConnected, DISCONNECTED);
            status.powerbi = Service::new(Phase::NotConnected, DISCONNECTED);
        });
    }
}

#[cfg(test)]
mod tests;
