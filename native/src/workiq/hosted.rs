use super::validate_tenant_metadata;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientConfig, ProtocolVersion, Tool},
    transport::{
        StreamableHttpClientTransport,
        auth::{
            AuthClient, AuthError, AuthorizationManager, AuthorizationMetadataSource,
            OAuthClientConfig, WWWAuthenticateParams,
        },
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

pub const ENDPOINT: &str = "https://workiq.svc.cloud.microsoft/mcp";
const RESOURCE_METADATA: &str =
    "https://workiq.svc.cloud.microsoft/.well-known/oauth-protected-resource/mcp";
// Microsoft publishes this public client in its host-neutral WorkIQ plugin.
pub(super) const CLIENT_ID: &str = "ba081686-5d24-4bc6-a0d6-d034ecffed87";
pub(super) const TENANT: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
const REDIRECT: &str = "http://localhost:8080/callback";
const NETWORK_DEADLINE: Duration = Duration::from_secs(60);
const LOGIN_DEADLINE: Duration = Duration::from_secs(600);
const MAX_CALLBACK_BYTES: usize = 16_384;

pub(super) fn client_config() -> ClientConfig {
    // `serve` uses initialize; the newest protocol revision uses discover instead.
    ClientConfig::default().with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
}

pub(super) fn transport_config() -> StreamableHttpClientTransportConfig {
    let mut config = StreamableHttpClientTransportConfig::with_uri(ENDPOINT);
    config.allow_stateless = true;
    config.max_concurrent_requests = 1;
    config
}

#[derive(Debug, Serialize)]
pub struct HostedReport {
    pub endpoint: &'static str,
    pub tenant: &'static str,
    pub outcome: &'static str,
    pub issuer: String,
    pub resource_scope: String,
    pub redirect: &'static str,
    pub token_storage: &'static str,
    pub account_verified: bool,
    pub authenticated_read_verified: bool,
    pub calendar_records_returned: Option<usize>,
    pub tools_discovered: Option<usize>,
    pub elapsed_ms: u128,
    pub model_calls: u32,
    pub mutation_calls: u32,
}

impl HostedReport {
    pub fn summary(&self) -> String {
        if let (true, true, Some(tools), Some(records)) = (
            self.authenticated_read_verified,
            self.account_verified,
            self.tools_discovered,
            self.calendar_records_returned,
        ) {
            format!(
                "WorkIQ read verified: {tools} tools, {records} calendar records, {} ms. Tokens discarded; not connected to the assistant.",
                self.elapsed_ms
            )
        } else {
            "WorkIQ discovery verified; authentication and calendar access are not verified.".into()
        }
    }
}

#[derive(Clone, Copy)]
pub enum Phase {
    BrowserSignIn,
    VerifyingRead,
}

impl Phase {
    pub fn message(self) -> &'static str {
        match self {
            Self::BrowserSignIn => {
                "Sign in with the selected work account in your browser. Waiting up to 10 minutes; tokens stay in memory."
            }
            Self::VerifyingRead => {
                "Token acquired. Verifying your account and read-only access; no AI or writes."
            }
        }
    }
}

#[derive(Deserialize)]
struct ResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    scopes_supported: Vec<String>,
}

fn validate_resource(metadata: &ResourceMetadata) -> Result<&str, String> {
    if metadata.resource != ENDPOINT
        || metadata.authorization_servers
            != ["https://login.microsoftonline.com/organizations/v2.0"]
        || metadata.scopes_supported.len() != 1
        || !matches!(
            metadata.scopes_supported[0].as_str(),
            "fdcc1f02-fc51-4226-8753-f668596af7f7/WorkIQAgent.Ask"
                | "api://workiq.svc.cloud.microsoft/WorkIQAgent.Ask"
        )
    {
        return Err("Hosted WorkIQ resource, authority or requested scope differs from the approved configuration; no authentication was attempted".into());
    }
    Ok(&metadata.scopes_supported[0])
}

fn aadsts(detail: &str) -> Option<&str> {
    let start = detail.find("AADSTS")?;
    let tail = &detail[start + 6..];
    let length = tail.bytes().take_while(u8::is_ascii_digit).count();
    (length > 0).then_some(&detail[start..start + 6 + length])
}

fn auth_error(error: AuthError) -> String {
    let (category, provider_code) = match &error {
        AuthError::TokenExchangeFailed(detail) => ("token exchange rejected", aadsts(detail)),
        AuthError::TokenRefreshFailed(detail) => ("token refresh failed", aadsts(detail)),
        AuthError::TokenRefreshRejected(detail) => ("refresh token rejected", aadsts(detail)),
        AuthError::AuthorizationFailed(detail) => ("authorization failed", aadsts(detail)),
        AuthError::AuthorizationRequired => ("authorization required", None),
        AuthError::AuthorizationServerMismatch { .. }
        | AuthError::AuthorizationServerMissingIssuer { .. } => {
            ("authorization-server issuer validation failed", None)
        }
        AuthError::PkceUnsupported => ("server does not support S256 PKCE", None),
        AuthError::HttpError(_) => ("OAuth HTTP request failed", None),
        AuthError::MetadataError(_) => ("OAuth metadata rejected", None),
        _ => ("OAuth protocol operation failed", None),
    };
    match provider_code {
        Some(code) => format!("Hosted WorkIQ {category} ({code}); credential details withheld"),
        None => format!("Hosted WorkIQ {category}; credential details withheld"),
    }
}

pub(super) fn initialization_error(error: rmcp::service::ClientInitializeError) -> String {
    use rmcp::service::ClientInitializeError;
    let authorization_required = error.is_authorization_required();
    let reason = match &error {
        ClientInitializeError::JsonRpcError(data) => {
            format!("JSON-RPC error code {:?}", data.code)
        }
        ClientInitializeError::NoCompatibleProtocolVersion { .. } => {
            "no compatible MCP protocol version".into()
        }
        ClientInitializeError::ExpectedInitResponse(_) => {
            "unexpected initialization response".into()
        }
        ClientInitializeError::ExpectedInitResult(_) => "unexpected initialization result".into(),
        ClientInitializeError::TransportError { error, .. } => {
            let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error.error.as_ref());
            let mut status = None;
            while let Some(current) = source {
                if let Some(http) = current.downcast_ref::<reqwest::Error>() {
                    status = http.status().map(|status| status.as_u16());
                    break;
                }
                source = current.source();
            }
            if let Some(status) = status {
                format!("HTTP {status}")
            } else if authorization_required {
                "HTTP authorization required".into()
            } else {
                let detail = error.to_string();
                if detail.contains("Missing session id") {
                    "transport missing session identifier".into()
                } else if detail.contains("could not parse JSON response") {
                    "transport received unexpected JSON".into()
                } else if detail.contains("Content type") || detail.contains("content type") {
                    "transport received an unsupported content type".into()
                } else {
                    "transport/protocol failure".into()
                }
            }
        }
        _ => "connection/lifecycle failure".into(),
    };
    format!(
        "Authenticated hosted WorkIQ initialization failed: {reason}; provider content withheld"
    )
}

async fn discovery(
    client: &reqwest::Client,
) -> Result<(AuthorizationManager, String, String), String> {
    let transport = StreamableHttpClientTransport::with_client(client.clone(), transport_config());
    let challenge = match client_config().serve(transport).await {
        Ok(service) => {
            tokio::time::timeout(Duration::from_secs(5), service.cancel())
                .await
                .map_err(|_| "Close unexpected unauthenticated WorkIQ session timed out")?
                .map_err(|_| "Close unexpected unauthenticated WorkIQ session failed")?;
            return Err("Hosted WorkIQ accepted initialization without authentication; refusing to read an unverified user context".into());
        }
        Err(error) => error
            .auth_challenge()
            .ok_or("Hosted WorkIQ initialization failed without an OAuth challenge; no credentials or tools were used")?
            .to_owned(),
    };
    let resource_url = url::Url::parse(ENDPOINT).map_err(|_| "Invalid fixed WorkIQ endpoint")?;
    let params = WWWAuthenticateParams::parse(&challenge, &resource_url);
    if params.resource_metadata_url.as_ref().map(url::Url::as_str) != Some(RESOURCE_METADATA) {
        return Err(
            "Hosted WorkIQ challenge did not advertise the expected same-origin resource metadata"
                .into(),
        );
    }
    let response = client
        .get(RESOURCE_METADATA)
        .send()
        .await
        .map_err(|error| format!("Fetch WorkIQ resource metadata: {}", error.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "WorkIQ resource metadata returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Read WorkIQ resource metadata failed")?
    {
        if body.len() + chunk.len() > 65_536 {
            return Err("WorkIQ resource metadata exceeded its 64 KiB limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    let resource: ResourceMetadata =
        serde_json::from_slice(&body).map_err(|_| "Invalid WorkIQ resource metadata JSON")?;
    let scope = validate_resource(&resource)?.to_owned();
    let authority = format!("https://login.microsoftonline.com/{TENANT}/v2.0");
    let mut tenant_manager = AuthorizationManager::new(&authority)
        .await
        .map_err(auth_error)?;
    tenant_manager
        .with_client(client.clone())
        .map_err(auth_error)?;
    let resolution = tenant_manager
        .resolve_metadata()
        .await
        .map_err(auth_error)?;
    if resolution.source != AuthorizationMetadataSource::AuthorizationServerMetadata {
        return Err("Entra discovery did not return published tenant-specific metadata".into());
    }
    validate_tenant_metadata(TENANT, &resolution.metadata)?;
    let issuer = resolution
        .metadata
        .issuer
        .clone()
        .ok_or("Validated Entra metadata has no issuer")?;
    let mut manager = AuthorizationManager::new(ENDPOINT)
        .await
        .map_err(auth_error)?;
    manager.with_client(client.clone()).map_err(auth_error)?;
    manager.set_metadata(resolution.metadata);
    manager
        .configure_client(OAuthClientConfig::new(CLIENT_ID, REDIRECT))
        .map_err(auth_error)?;
    Ok((manager, scope, issuer))
}

struct Callback {
    code: Option<String>,
    state: String,
    issuer: Option<String>,
    error: Option<String>,
    provider_code: Option<String>,
}

fn parse_callback(request: &str, expected_state: &str) -> Result<Callback, String> {
    let mut lines = request.split("\r\n");
    let request_line = lines.next().ok_or("Missing callback request line")?;
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "GET"
        || !matches!(parts[2], "HTTP/1.1" | "HTTP/1.0")
        || !parts[1].starts_with("/callback?")
        || parts[1].contains('#')
    {
        return Err("Callback method or target rejected".into());
    }
    let hosts = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
    if hosts != ["localhost:8080"] {
        return Err("Callback Host header rejected".into());
    }
    let url = url::Url::parse(&format!("http://localhost:8080{}", parts[1]))
        .map_err(|_| "Invalid callback URL")?;
    if url.path() != "/callback" {
        return Err("Callback path rejected".into());
    }
    let mut callback = Callback {
        code: None,
        state: String::new(),
        issuer: None,
        error: None,
        provider_code: None,
    };
    let mut seen = std::collections::BTreeSet::new();
    for (name, value) in url.query_pairs() {
        if matches!(
            name.as_ref(),
            "code" | "state" | "iss" | "error" | "error_description"
        ) && !seen.insert(name.to_string())
        {
            return Err("Duplicate OAuth callback parameter rejected".into());
        }
        match name.as_ref() {
            "code" if !value.is_empty() && value.len() <= 8192 => {
                callback.code = Some(value.into_owned());
            }
            "state" => callback.state = value.into_owned(),
            "iss" => callback.issuer = Some(value.into_owned()),
            "error"
                if value.len() <= 128
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_') =>
            {
                callback.error = Some(value.into_owned());
            }
            "error_description" => callback.provider_code = aadsts(&value).map(str::to_owned),
            "code" | "error" => return Err("Invalid OAuth callback parameter".into()),
            _ => {}
        }
    }
    if callback.state.is_empty() || callback.state != expected_state {
        return Err("OAuth callback state rejected".into());
    }
    if callback
        .issuer
        .as_deref()
        .is_some_and(|issuer| issuer != format!("https://login.microsoftonline.com/{TENANT}/v2.0"))
    {
        return Err("OAuth callback issuer rejected".into());
    }
    if callback.code.is_some() == callback.error.is_some() {
        return Err("Callback must contain either a code or an error".into());
    }
    Ok(callback)
}

async fn read_callback(stream: &mut TcpStream) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 2048];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream
            .read(&mut buffer)
            .await
            .map_err(|_| "Read loopback callback failed")?;
        if count == 0 || bytes.len() + count > MAX_CALLBACK_BYTES {
            return Err("Incomplete or oversized loopback callback".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).map_err(|_| "Callback is not valid UTF-8".into())
}

async fn reply(stream: &mut TcpStream, status: &str, message: &str) -> Result<(), String> {
    let message = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Tomlook sign-in response</title></head><body><p>{message}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|_| "Write loopback callback response failed".into())
}

async fn await_callback(listener: &TcpListener, state: &str) -> Result<Callback, String> {
    loop {
        let (mut stream, peer) = listener
            .accept()
            .await
            .map_err(|_| "Accept loopback callback failed")?;
        if !peer.ip().is_loopback() {
            return Err("Non-loopback callback peer rejected".into());
        }
        let callback = tokio::time::timeout(Duration::from_secs(5), async {
            match read_callback(&mut stream)
                .await
                .and_then(|request| parse_callback(&request, state))
            {
                Ok(callback) => {
                    reply(
                        &mut stream,
                        "200 OK",
                        "Tomlook received the sign-in response. You can close this tab. Authentication and the read experiment are still being verified.",
                    )
                    .await?;
                    Ok::<Option<Callback>, String>(Some(callback))
                }
                Err(_) => {
                    eprintln!("Rejected an invalid loopback callback; still waiting for sign-in.");
                    reply(&mut stream, "400 Bad Request", "Invalid sign-in callback.")
                        .await?;
                    Ok(None)
                }
            }
        })
        .await
        .map_err(|_| "Loopback callback connection exceeded its 5-second deadline")??;
        if let Some(callback) = callback {
            return Ok(callback);
        }
    }
}

pub(super) fn validate_fetch_schema(tool: &Tool) -> Result<(), String> {
    let properties = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object);
    let entity_urls = properties
        .and_then(|properties| properties.get("entityUrls"))
        .ok_or("WorkIQ fetch schema has no entityUrls parameter")?;
    let required = tool.input_schema.get("required").and_then(Value::as_array);
    if entity_urls.get("type").and_then(Value::as_str) != Some("array")
        || entity_urls.pointer("/items/type").and_then(Value::as_str) != Some("string")
        || !required.is_some_and(|required| {
            required
                .iter()
                .any(|name| name.as_str() == Some("entityUrls"))
                && required
                    .iter()
                    .all(|name| name.as_str() == Some("entityUrls"))
        })
    {
        return Err("WorkIQ fetch schema differs from the documented read contract".into());
    }
    Ok(())
}

pub(super) fn read_json(result: &CallToolResult) -> Result<Value, String> {
    if result.is_error == Some(true) {
        return Err("Hosted WorkIQ fetch returned a tool error; remote content withheld".into());
    }
    read_json_payload(result)
}

pub(super) fn read_json_payload(result: &CallToolResult) -> Result<Value, String> {
    match &result.structured_content {
        Some(payload) => {
            if serde_json::to_vec(payload)
                .map_err(|_| "Encode WorkIQ result failed")?
                .len()
                > 1_048_576
            {
                return Err(super::connection::READ_TOO_LARGE.into());
            }
            Ok(payload.clone())
        }
        None => {
            if result.content.len() != 1 {
                return Err("WorkIQ read has no unambiguous structured JSON result".into());
            }
            let text = result.content[0]
                .as_text()
                .ok_or("WorkIQ read did not return JSON text")?;
            if text.text.len() > 1_048_576 {
                return Err(super::connection::READ_TOO_LARGE.into());
            }
            serde_json::from_str(&text.text).map_err(|_| "Invalid WorkIQ read JSON".into())
        }
    }
}

pub(super) fn validate_ask_schema(tool: &Tool) -> Result<(), String> {
    let question = tool
        .input_schema
        .get("properties")
        .and_then(|properties| properties.get("question"));
    let required = tool.input_schema.get("required").and_then(Value::as_array);
    if question
        .and_then(|question| question.get("type"))
        .and_then(Value::as_str)
        != Some("string")
        || !required.is_some_and(|required| {
            required
                .iter()
                .any(|name| name.as_str() == Some("question"))
                && required
                    .iter()
                    .all(|name| name.as_str() == Some("question"))
        })
    {
        return Err("WorkIQ ask schema differs from the supported context contract".into());
    }
    Ok(())
}

pub(super) fn verify_context(value: &Value) -> Result<(), String> {
    if !value
        .get("answer")
        .and_then(Value::as_str)
        .is_some_and(|answer| !answer.trim().is_empty())
    {
        return Err("WorkIQ returned no usable context answer; response contents withheld".into());
    }
    Ok(())
}

pub(super) fn context_data(value: Value) -> Result<Value, String> {
    verify_context(&value)?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| "Encode WorkIQ context failed")?
        .len();
    if bytes <= 60_000 {
        return Ok(value);
    }
    let projected = json!({
        "answer": value["answer"],
        "supporting_metadata_omitted": true,
        "original_response_bytes": bytes,
        "coverage_note": "Response metadata was omitted to fit the model limit. The context answer is preserved in full."
    });
    if serde_json::to_vec(&projected)
        .map_err(|_| "Encode bounded WorkIQ context failed")?
        .len()
        > 60_000
    {
        return Err(
            "WorkIQ context answer exceeds its 60,000-byte model limit; narrow the query".into(),
        );
    }
    Ok(projected)
}

pub(super) fn fetch_data(result: &CallToolResult) -> Result<Value, String> {
    let payload = read_json(result)?;
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .ok_or("WorkIQ fetch did not return the documented results array")?;
    if results.len() != 1 {
        return Err("WorkIQ fetch returned an unexpected number of resource results".into());
    }
    let status = results[0]
        .get("statusCode")
        .and_then(Value::as_u64)
        .ok_or("WorkIQ fetch result has no HTTP status")?;
    if status != 200 {
        return Err(format!("Hosted WorkIQ read returned HTTP {status}"));
    }
    results[0]
        .get("data")
        .cloned()
        .ok_or_else(|| "WorkIQ fetch result has no data".into())
}

pub(super) fn verify_account(result: &CallToolResult, expected: &str) -> Result<(), String> {
    let data = fetch_data(result)?;
    let account = data
        .get("userPrincipalName")
        .and_then(Value::as_str)
        .ok_or("WorkIQ user read did not return an account identity")?;
    if !account.eq_ignore_ascii_case(expected) {
        return Err("WorkIQ signed-in account does not match the explicitly selected work account; no calendar read was attempted".into());
    }
    Ok(())
}

fn verify_calendar(result: &CallToolResult) -> Result<usize, String> {
    let data = fetch_data(result)?;
    let records = data
        .get("value")
        .and_then(Value::as_array)
        .ok_or("WorkIQ calendar read did not return a collection")?;
    if records.len() > 1
        || records.iter().any(|record| {
            record.get("id").and_then(Value::as_str).is_none()
                || record
                    .pointer("/start/dateTime")
                    .and_then(Value::as_str)
                    .is_none()
                || record
                    .pointer("/end/dateTime")
                    .and_then(Value::as_str)
                    .is_none()
        })
    {
        return Err("WorkIQ calendar result does not satisfy the bounded event contract".into());
    }
    Ok(records.len())
}

async fn calendar_read(
    client: AuthClient<reqwest::Client>,
    expected_account: &str,
) -> Result<(usize, usize), String> {
    let transport = StreamableHttpClientTransport::with_client(client, transport_config());
    let service = client_config()
        .serve(transport)
        .await
        .map_err(initialization_error)?;
    let read = tokio::time::timeout(NETWORK_DEADLINE, async {
        let mut cursor = None;
        let mut cursors = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        let mut fetch = None;
        for page in 0..8 {
            let result = service
                .list_tools(cursor.clone())
                .await
                .map_err(|_| "List hosted WorkIQ tools failed")?;
            for tool in result.tools {
                if !names.insert(tool.name.to_string()) {
                    return Err("Hosted WorkIQ tool discovery returned duplicate names".into());
                }
                if tool.name == "fetch" {
                    validate_fetch_schema(&tool)?;
                    fetch = Some(tool);
                }
            }
            match result.next_cursor {
                Some(next) if page < 7 && cursors.insert(next.clone()) => {
                    cursor = Some(
                        rmcp::model::PaginatedRequestParams::default().with_cursor(Some(next)),
                    );
                }
                Some(_) => {
                    return Err(
                        "Hosted WorkIQ tool discovery exceeded its page bound or repeated a cursor"
                            .into(),
                    );
                }
                None => break,
            }
        }
        let fetch = fetch.ok_or("Hosted WorkIQ does not expose the documented fetch tool")?;
        let identity_arguments = json!({
            "entityUrls": ["/me?$select=userPrincipalName"]
        })
        .as_object()
        .cloned()
        .ok_or("Invalid fixed identity read arguments")?;
        let identity = service
            .call_tool(
                CallToolRequestParams::new(fetch.name.clone()).with_arguments(identity_arguments),
            )
            .await
            .map_err(|_| "Hosted WorkIQ account verification failed; provider content withheld")?;
        verify_account(&identity, expected_account)?;
        let arguments = json!({
            "entityUrls": ["/me/events?$top=1&$select=id,start,end"]
        })
        .as_object()
        .cloned()
        .ok_or("Invalid fixed calendar read arguments")?;
        let result = service
            .call_tool(CallToolRequestParams::new(fetch.name).with_arguments(arguments))
            .await
            .map_err(|_| "Hosted WorkIQ calendar fetch failed; provider content withheld")?;
        Ok((names.len(), verify_calendar(&result)?))
    })
    .await
    .map_err(|_| "Hosted WorkIQ tool discovery/read exceeded its 60-second deadline");
    let closed = tokio::time::timeout(Duration::from_secs(5), service.cancel())
        .await
        .map_err(|_| "Close hosted WorkIQ session timed out")?
        .map_err(|_| "Close hosted WorkIQ session failed");
    closed?;
    read?
}

/// Separate diagnostic: no Copilot runtime, persistent credentials or write tools.
pub async fn probe(account: Option<&str>) -> Result<HostedReport, String> {
    probe_with_progress(account, |phase| eprintln!("{}", phase.message())).await
}

pub async fn probe_with_progress(
    account: Option<&str>,
    progress: impl Fn(Phase),
) -> Result<HostedReport, String> {
    if account.is_some_and(|account| {
        !account.ends_with("@microsoft.com")
            || account.len() > 254
            || account
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    }) {
        return Err("This work-tenant experiment requires an explicit Microsoft work email".into());
    }
    let started = Instant::now();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Create hosted WorkIQ HTTP client failed")?;
    let (manager, scope, issuer) = tokio::time::timeout(NETWORK_DEADLINE, discovery(&client))
        .await
        .map_err(|_| "Hosted WorkIQ discovery exceeded its 60-second deadline")??;
    let mut report = HostedReport {
        endpoint: ENDPOINT,
        tenant: TENANT,
        outcome: "discovery_ready",
        issuer,
        resource_scope: scope.clone(),
        redirect: REDIRECT,
        token_storage: "memory_only",
        account_verified: false,
        authenticated_read_verified: false,
        calendar_records_returned: None,
        tools_discovered: None,
        elapsed_ms: started.elapsed().as_millis(),
        model_calls: 0,
        mutation_calls: 0,
    };
    let Some(account) = account else {
        return Ok(report);
    };
    let client = authenticate(client, manager, &scope, account, progress).await?;
    let (tools, records) =
        tokio::time::timeout(Duration::from_secs(90), calendar_read(client, account))
            .await
            .map_err(|_| "Authenticated WorkIQ experiment exceeded its 90-second deadline")??;
    report.outcome = "calendar_read_verified";
    report.account_verified = true;
    report.authenticated_read_verified = true;
    report.tools_discovered = Some(tools);
    report.calendar_records_returned = Some(records);
    report.elapsed_ms = started.elapsed().as_millis();
    Ok(report)
}

pub(super) async fn authenticated_client(
    account: &str,
    progress: impl Fn(Phase),
) -> Result<AuthClient<reqwest::Client>, String> {
    if account.len() > 254
        || account.split('@').count() != 2
        || account.starts_with('@')
        || !account.ends_with("@microsoft.com")
        || account
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err("WorkIQ requires an explicit Microsoft work email".into());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Create WorkIQ HTTP client failed")?;
    let (manager, scope, _) = tokio::time::timeout(NETWORK_DEADLINE, discovery(&client))
        .await
        .map_err(|_| "WorkIQ discovery exceeded its 60-second deadline")??;
    authenticate(client, manager, &scope, account, progress).await
}

pub(super) async fn saved_client(
    root: &std::path::Path,
    account: &str,
    interactive: bool,
    progress: impl Fn(Phase),
) -> Result<AuthClient<reqwest::Client>, String> {
    if !account.to_ascii_lowercase().ends_with("@microsoft.com")
        || account.split('@').count() != 2
        || account.len() > 254
        || account.starts_with('@')
        || account
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err("Choose your Microsoft work email".into());
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(135))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Create WorkIQ HTTP client failed")?;
    let (mut manager, scope, _) = tokio::time::timeout(NETWORK_DEADLINE, discovery(&client))
        .await
        .map_err(|_| "WorkIQ discovery timed out")??;
    let vault = super::vault::Vault::new(root, account);
    vault.bind().await.map_err(auth_error)?;
    manager.set_credential_store(vault);
    if !interactive && manager.initialize_from_store().await.map_err(auth_error)? {
        progress(Phase::VerifyingRead);
        return Ok(AuthClient::new(client, manager));
    }
    if !interactive {
        return Err("WorkIQ needs sign-in. Click the status bar to connect.".into());
    }
    authenticate(client, manager, &scope, account, progress).await
}
async fn authenticate(
    client: reqwest::Client,
    manager: AuthorizationManager,
    scope: &str,
    account: &str,
    progress: impl Fn(Phase),
) -> Result<AuthClient<reqwest::Client>, String> {
    let listener = TcpListener::bind("127.0.0.1:8080")
        .await
        .map_err(|_| "Could not bind the registered loopback callback on 127.0.0.1:8080; no browser was opened")?;
    let auth_url = manager
        .get_authorization_url(&[scope, "openid", "profile", "offline_access"])
        .await
        .map_err(auth_error)?;
    let mut parsed =
        url::Url::parse(&auth_url).map_err(|_| "Invalid generated authorization URL")?;
    let state = parsed
        .query_pairs()
        .find(|(name, _)| name == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or("Authorization URL has no state")?;
    parsed.query_pairs_mut().append_pair("login_hint", account);
    let auth_url = parsed.to_string();
    tokio::task::spawn_blocking(move || crate::browser::open(&auth_url))
        .await
        .map_err(|_| "Browser opener task failed")??;
    progress(Phase::BrowserSignIn);
    let callback = tokio::time::timeout(LOGIN_DEADLINE, await_callback(&listener, &state))
        .await
        .map_err(|_| "WorkIQ browser sign-in exceeded its 10-minute deadline")??;
    drop(listener);
    if let Some(error) = callback.error {
        return Err(format!(
            "WorkIQ browser authorization returned {error}{}",
            callback
                .provider_code
                .map(|code| format!(" ({code})"))
                .unwrap_or_default()
        ));
    }
    let code = callback
        .code
        .ok_or("Validated callback has no authorization code")?;
    tokio::time::timeout(
        Duration::from_secs(30),
        manager.exchange_code_for_token_with_issuer(
            &code,
            &callback.state,
            callback.issuer.as_deref(),
        ),
    )
    .await
    .map_err(|_| "WorkIQ token exchange exceeded its 30-second deadline")?
    .map_err(auth_error)?;
    progress(Phase::VerifyingRead);
    Ok(AuthClient::new(client, manager))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_structured_and_text_results_trigger_the_same_partition_signal() {
        let structured = result(json!({"data": "x".repeat(1_048_576)}));
        assert_eq!(
            read_json(&structured).unwrap_err(),
            super::super::connection::READ_TOO_LARGE
        );
        let text: CallToolResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "x".repeat(1_048_577)}]
        }))
        .unwrap();
        assert_eq!(
            read_json(&text).unwrap_err(),
            super::super::connection::READ_TOO_LARGE
        );
    }

    #[test]
    fn initialize_transport_supports_optional_server_session_ids() {
        assert!(client_config().protocol_version.has_initialize());
        assert!(transport_config().allow_stateless);
        assert_eq!(transport_config().max_concurrent_requests, 1);
    }

    fn request(query: &str) -> String {
        format!("GET /callback?{query} HTTP/1.1\r\nHost: localhost:8080\r\n\r\n")
    }

    fn result(payload: Value) -> CallToolResult {
        let mut result = CallToolResult::default();
        result.structured_content = Some(payload);
        result
    }

    #[test]
    fn resource_identity_and_scope_are_fixed() {
        let mut metadata = ResourceMetadata {
            resource: ENDPOINT.into(),
            authorization_servers: vec![
                "https://login.microsoftonline.com/organizations/v2.0".into(),
            ],
            scopes_supported: vec!["fdcc1f02-fc51-4226-8753-f668596af7f7/WorkIQAgent.Ask".into()],
        };
        assert!(validate_resource(&metadata).is_ok());
        metadata.scopes_supported.push("unapproved_scope".into());
        assert!(validate_resource(&metadata).is_err());
        metadata.scopes_supported.pop();
        metadata.resource = "https://other.example/mcp".into();
        assert!(validate_resource(&metadata).is_err());
    }

    #[test]
    fn valid_callback_decodes_code_without_logging_it() {
        let callback =
            parse_callback(&request("code=test%2Bcode&state=expected"), "expected").unwrap();
        assert_eq!(callback.code.as_deref(), Some("test+code"));
    }

    #[test]
    fn callback_rejects_state_issuer_and_duplicate_parameters() {
        for query in [
            "code=test&state=wrong",
            "code=test&state=expected&state=expected",
            "code=test&state=expected&code=second",
            "code=test&state=expected&iss=https%3A%2F%2Fevil.example",
            "code=test&error=access_denied&state=expected",
            "state=expected",
        ] {
            assert!(
                parse_callback(&request(query), "expected").is_err(),
                "{query}"
            );
        }
    }

    #[test]
    fn callback_rejects_foreign_host_path_and_method() {
        let valid = request("code=test&state=expected");
        for invalid in [
            valid.replace("Host: localhost:8080", "Host: evil.example"),
            valid.replace(
                "Host: localhost:8080",
                "Host: localhost:8080\r\nHost: localhost:8080",
            ),
            valid.replace("/callback?", "/other?"),
            valid.replace("GET ", "POST "),
        ] {
            assert!(parse_callback(&invalid, "expected").is_err());
        }
    }

    #[test]
    fn provider_errors_expose_only_safe_codes() {
        let callback = parse_callback(
            &request("error=access_denied&state=expected&error_description=AADSTS53003%20private%20details"),
            "expected",
        ).unwrap();
        assert_eq!(callback.provider_code.as_deref(), Some("AADSTS53003"));
        let error = auth_error(AuthError::TokenExchangeFailed(
            "AADSTS65001 secret-token-value".into(),
        ));
        assert!(error.contains("AADSTS65001"));
        assert!(!error.contains("secret-token-value"));
        let initialization =
            initialization_error(rmcp::service::ClientInitializeError::ConnectionClosed(
                "secret-token-value and private provider data".into(),
            ));
        assert!(initialization.contains("connection/lifecycle failure"));
        assert!(!initialization.contains("secret-token-value"));
    }

    #[test]
    fn successful_empty_read_is_distinct_from_error_or_invalid_shape() {
        assert_eq!(
            verify_calendar(&result(
                json!({"results":[{"statusCode":200,"data":{"value":[]}}]})
            ))
            .unwrap(),
            0
        );
        for payload in [
            json!({"results":[{"statusCode":403,"data":{"value":[]}}]}),
            json!({"results":[{"statusCode":200,"data":{}}]}),
            json!({"results":[]}),
            json!({"results":[{"statusCode":200,"data":{"value":[{"subject":"not an event contract"}]}}]}),
        ] {
            assert!(verify_calendar(&result(payload)).is_err());
        }
    }

    #[test]
    fn calendar_read_enforces_one_record_bound_and_shape() {
        let event = json!({"id":"fixture","start":{"dateTime":"2026-10-08T10:00:00"},"end":{"dateTime":"2026-10-08T11:00:00"}});
        assert_eq!(
            verify_calendar(&result(
                json!({"results":[{"statusCode":200,"data":{"value":[event.clone()]}}]})
            ))
            .unwrap(),
            1
        );
        assert!(
            verify_calendar(&result(
                json!({"results":[{"statusCode":200,"data":{"value":[event.clone(),event]}}]})
            ))
            .is_err()
        );
        let mut error = result(json!({"results":[{"statusCode":200,"data":{"value":[]}}]}));
        error.is_error = Some(true);
        assert!(verify_calendar(&error).is_err());
    }

    #[test]
    fn wrong_account_is_rejected_before_calendar_read() {
        let identity = result(
            json!({"results":[{"statusCode":200,"data":{"userPrincipalName":"test@microsoft.com"}}]}),
        );
        assert!(verify_account(&identity, "TEST@microsoft.com").is_ok());
        assert!(verify_account(&identity, "different@microsoft.com").is_err());
        assert!(
            verify_account(
                &result(json!({"results":[{"statusCode":200,"data":{}}]})),
                "test@microsoft.com"
            )
            .is_err()
        );
    }

    #[test]
    fn tool_schema_must_match_documented_read_arguments() {
        let mut tool = Tool::default();
        tool.input_schema = std::sync::Arc::new(
            json!({"properties":{"entityUrls":{"type":"array","items":{"type":"string"}}},"required":["entityUrls"]})
                .as_object().unwrap().clone(),
        );
        assert!(validate_fetch_schema(&tool).is_ok());
        tool.input_schema = std::sync::Arc::new(
            json!({"properties":{"entityUrls":{"type":"array","items":{"type":"string"}}},"required":["entityUrls","unapproved"]})
                .as_object().unwrap().clone(),
        );
        assert!(validate_fetch_schema(&tool).is_err());
    }

    #[test]
    fn context_schema_and_live_answer_envelope_are_required() {
        let mut tool = Tool::default();
        tool.input_schema = std::sync::Arc::new(
            json!({"properties":{"question":{"type":"string"}},"required":["question"]})
                .as_object()
                .unwrap()
                .clone(),
        );
        assert!(validate_ask_schema(&tool).is_ok());
        tool.input_schema = std::sync::Arc::new(
            json!({"properties":{"question":{"type":"string"}},"required":["question","writeApproval"]}).as_object().unwrap().clone()
        );
        assert!(validate_ask_schema(&tool).is_err());
        assert!(
            verify_context(&json!({"answer":"Fixture context","conversationId":"fixture"})).is_ok()
        );
        for value in [
            json!({"response":"Wrong envelope"}),
            json!({"answer":""}),
            json!({"answer":"  "}),
            json!({"answer":null}),
            json!({"answer":["not text"]}),
        ] {
            assert!(verify_context(&value).is_err());
        }
    }

    #[test]
    fn model_context_preserves_answers_and_reports_omitted_large_metadata() {
        let small = json!({"answer":"Fixture context","metadata":{"count":1}});
        assert_eq!(context_data(small.clone()).unwrap(), small);
        let answer = "Original context [1](https://example.com/source)";
        let large = json!({"answer":answer,"metadata":{"content":"x".repeat(100_000)}});
        let bounded = context_data(large).unwrap();
        assert_eq!(bounded["answer"], answer);
        assert_eq!(bounded["supporting_metadata_omitted"], true);
        assert!(bounded.get("metadata").is_none());
        assert!(bounded["original_response_bytes"].as_u64().unwrap() > 100_000);
        assert!(serde_json::to_vec(&bounded).unwrap().len() <= 60_000);
        for answer in [
            "x".repeat(60_000),
            "\n".repeat(30_000),
            "\u{010d}".repeat(30_000),
        ] {
            assert!(context_data(json!({"answer":answer})).is_err());
        }
        assert!(context_data(json!({"answer":null,"metadata":"x".repeat(100_000)})).is_err());
    }
}
