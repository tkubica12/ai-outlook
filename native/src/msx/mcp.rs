//! Read-only MCP sessions for Dataverse and Power BI with a strict tool allowlist.
use super::Resource;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientConfig, ProtocolVersion},
    service::{Peer, RunningService},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Map, Value};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

/// Read tools Tomlook may call. Everything else, including every create/update/delete tool the
/// servers advertise, is never invoked.
pub const DATAVERSE_READ: &[&str] = &[
    "read_query",
    "describe",
    "list_tables",
    "describe_table",
    "search",
    "search_data",
];
pub const POWERBI_READ: &[&str] = &[
    "ExecuteQuery",
    "GetSemanticModelSchema",
    "GetReportMetadata",
    "GetReportSchema",
    "GetReportSummary",
    "GenerateQuery",
    "DiscoverArtifacts",
    "ResolveFabricItem",
    "ValueSearch",
    "GetVisualsData",
    "GetVisualsDaxQuery",
];
const WRITE_MARKERS: &[&str] = &[
    "create", "update", "delete", "upsert", "insert", "write", "commit", "upload", "patch",
    "remove", "drop", "alter", "merge", "truncate", "grant", "revoke", "assign", "share", "send",
    "publish", "refresh", "import", "set_", "exec_",
];
pub const TOOL_REJECTED: &str = "The service rejected the read; provider content withheld";

#[derive(Clone, Debug, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub schema: Value,
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
}

impl ToolInfo {
    pub fn read(name: &str, schema: Value) -> Self {
        Self {
            name: name.into(),
            schema,
            read_only_hint: None,
            destructive_hint: None,
        }
    }

    pub fn property(&self, name: &str) -> Option<&Value> {
        self.schema.get("properties")?.get(name)
    }

    pub fn required(&self) -> Vec<&str> {
        self.schema
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    /// HTTP 401: the token was rejected.
    Unauthorized,
    /// Network or transport failure that may recover.
    Transport(String),
    /// Permanent protocol, permission or argument failure.
    Rejected(String),
}

#[async_trait::async_trait]
pub trait Session: Send + Sync {
    async fn list_tools(&self) -> Result<Vec<ToolInfo>, CallError>;
    async fn call(
        &self,
        name: &str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CallError>;
    async fn close(&self);
}

#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self, resource: Resource, token: &str)
    -> Result<Arc<dyn Session>, CallError>;
}

fn write_like(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    WRITE_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Whether Tomlook may ever call this discovered tool.
pub fn allowed(resource: Resource, tool: &ToolInfo) -> bool {
    let list = match resource {
        Resource::Dataverse => DATAVERSE_READ,
        Resource::PowerBi => POWERBI_READ,
    };
    list.contains(&tool.name.as_str())
        && !write_like(&tool.name)
        && tool.destructive_hint != Some(true)
        && tool.schema.get("type").and_then(Value::as_str) == Some("object")
}

/// The JSON payload of a successful tool result, bounded before it is parsed.
pub fn payload(result: &CallToolResult, limit: usize) -> Result<Value, String> {
    if result.is_error == Some(true) {
        return Err(TOOL_REJECTED.into());
    }
    if let Some(value) = &result.structured_content {
        if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len()) > limit {
            return Err("The response exceeds its size limit; narrow the query".into());
        }
        return Ok(value.clone());
    }
    let mut text = String::new();
    for content in &result.content {
        if let Some(part) = content.as_text() {
            if text.len() + part.text.len() > limit {
                return Err("The response exceeds its size limit; narrow the query".into());
            }
            text.push_str(&part.text);
        }
    }
    if text.trim().is_empty() {
        return Err("The service returned an empty response; absence is not proven".into());
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// A bounded, single-line excerpt of a rejected tool result, for the model only.
pub fn rejection_detail(result: &CallToolResult) -> Option<String> {
    let text = result
        .content
        .iter()
        .filter_map(|content| content.as_text())
        .map(|part| part.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let excerpt = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(300)
        .collect::<String>();
    (!excerpt.trim().is_empty()).then(|| excerpt.trim().to_owned())
}

/// Rows of a Dataverse or Power BI query result in the shapes the services return.
pub fn rows(value: &Value) -> Option<&Vec<Value>> {
    match value {
        Value::Array(rows) => Some(rows),
        Value::Object(object) => {
            if let Some(rows) = value
                .pointer("/results/0/tables/0/rows")
                .and_then(Value::as_array)
            {
                return Some(rows);
            }
            for key in [
                "value",
                "rows",
                "records",
                "results",
                "data",
                "items",
                "queryResult",
                "queryresult",
            ] {
                if let Some(Value::Array(rows)) = object.get(key) {
                    return Some(rows);
                }
            }
            if let Some(inner) = object.get("result").filter(|inner| inner.is_object()) {
                return rows(inner);
            }
            let mut arrays = object.values().filter_map(Value::as_array);
            match (arrays.next(), arrays.next()) {
                (Some(rows), None) => Some(rows),
                _ => None,
            }
        }
        _ => None,
    }
}

pub struct RmcpConnector;

struct RmcpSession {
    peer: Peer<RoleClient>,
    service: tokio::sync::Mutex<Option<RunningService<RoleClient, ClientConfig>>>,
}

fn classify(reason: String) -> CallError {
    if reason.starts_with("HTTP 401") {
        CallError::Unauthorized
    } else if crate::retry::transient(&reason) {
        CallError::Transport(reason)
    } else {
        CallError::Rejected(reason)
    }
}

fn service_error(error: rmcp::ServiceError) -> CallError {
    match error {
        rmcp::ServiceError::McpError(error) => {
            CallError::Rejected(format!("MCP code {}", error.code.0))
        }
        rmcp::ServiceError::TransportSend(error) => {
            classify(crate::connectors::transport_reason(&error))
        }
        rmcp::ServiceError::TransportClosed => {
            CallError::Transport("MCP transport closed before the response".into())
        }
        rmcp::ServiceError::Timeout { .. } => CallError::Transport("MCP request timed out".into()),
        _ => CallError::Rejected("provider returned unexpected MCP data".into()),
    }
}

fn initialization_error(error: rmcp::service::ClientInitializeError) -> CallError {
    use rmcp::service::ClientInitializeError;
    match error {
        ClientInitializeError::TransportError { error, .. } => {
            classify(crate::connectors::transport_reason(&error))
        }
        ClientInitializeError::JsonRpcError(error) => {
            CallError::Rejected(format!("MCP code {}", error.code.0))
        }
        ClientInitializeError::NoCompatibleProtocolVersion { .. } => {
            CallError::Rejected("no compatible MCP protocol".into())
        }
        _ => CallError::Rejected("invalid MCP initialization response".into()),
    }
}

#[async_trait::async_trait]
impl Connector for RmcpConnector {
    async fn connect(
        &self,
        resource: Resource,
        token: &str,
    ) -> Result<Arc<dyn Session>, CallError> {
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| CallError::Rejected("invalid credential".into()))?;
        value.set_sensitive(true);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, value);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(concat!("Tomlook/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|_| CallError::Rejected("HTTP client could not be created".into()))?;
        let mut config = StreamableHttpClientTransportConfig::with_uri(resource.endpoint());
        config.allow_stateless = true;
        config.max_concurrent_requests = 1;
        config.max_sse_event_size = 4 * 1024 * 1024;
        let service = ClientConfig::default()
            .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
            .serve(StreamableHttpClientTransport::with_client(http, config))
            .await
            .map_err(initialization_error)?;
        Ok(Arc::new(RmcpSession {
            peer: service.peer().clone(),
            service: tokio::sync::Mutex::new(Some(service)),
        }))
    }
}

#[async_trait::async_trait]
impl Session for RmcpSession {
    async fn list_tools(&self) -> Result<Vec<ToolInfo>, CallError> {
        let mut cursor = None;
        let mut seen = BTreeSet::new();
        let mut tools = Vec::new();
        for page in 0..8 {
            let result = self.peer.list_tools(cursor).await.map_err(service_error)?;
            for tool in result.tools {
                if tools.len() >= 256 {
                    return Err(CallError::Rejected(
                        "tool discovery exceeded its bound".into(),
                    ));
                }
                tools.push(ToolInfo {
                    name: tool.name.to_string(),
                    schema: Value::Object((*tool.input_schema).clone()),
                    read_only_hint: tool
                        .annotations
                        .as_ref()
                        .and_then(|hints| hints.read_only_hint),
                    destructive_hint: tool
                        .annotations
                        .as_ref()
                        .and_then(|hints| hints.destructive_hint),
                });
            }
            cursor = match result.next_cursor {
                None => break,
                Some(next) if page < 7 && seen.insert(next.clone()) => {
                    Some(rmcp::model::PaginatedRequestParams::default().with_cursor(Some(next)))
                }
                Some(_) => {
                    return Err(CallError::Rejected(
                        "tool discovery exceeded its pagination bound".into(),
                    ));
                }
            };
        }
        Ok(tools)
    }

    async fn call(
        &self,
        name: &str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CallError> {
        self.peer
            .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments))
            .await
            .map_err(service_error)
    }

    async fn close(&self) {
        if let Some(service) = self.service.lock().await.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), service.cancel()).await;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn text_result(payload: &Value) -> CallToolResult {
        serde_json::from_value(json!({"content":[{"type":"text","text":payload.to_string()}]}))
            .unwrap()
    }

    pub(crate) fn object() -> Value {
        json!({"type":"object","properties":{}})
    }

    #[test]
    fn only_allowlisted_read_tools_pass_and_every_write_tool_is_rejected() {
        for name in ["read_query", "describe", "search", "search_data"] {
            assert!(
                allowed(Resource::Dataverse, &ToolInfo::read(name, object())),
                "{name}"
            );
        }
        for name in [
            "create_record",
            "update_record",
            "delete_record",
            "upsert_skill",
            "init_file_upload",
            "commit_file_upload",
            "create_table",
            "delete_table",
            "update_table",
            "file_download",
            "ExecuteQuery",
        ] {
            assert!(
                !allowed(Resource::Dataverse, &ToolInfo::read(name, object())),
                "{name}"
            );
        }
        for name in [
            "ExecuteQuery",
            "GetSemanticModelSchema",
            "DiscoverArtifacts",
        ] {
            assert!(
                allowed(Resource::PowerBi, &ToolInfo::read(name, object())),
                "{name}"
            );
        }
        for name in [
            "RefreshDataset",
            "UpdateDatasource",
            "read_query",
            "DeleteReport",
        ] {
            assert!(
                !allowed(Resource::PowerBi, &ToolInfo::read(name, object())),
                "{name}"
            );
        }
        let mut destructive = ToolInfo::read("read_query", object());
        destructive.destructive_hint = Some(true);
        assert!(!allowed(Resource::Dataverse, &destructive));
        assert!(!allowed(
            Resource::Dataverse,
            &ToolInfo::read("read_query", json!({"type":"string"}))
        ));
    }

    #[test]
    fn payloads_are_bounded_and_rejections_withhold_provider_content() {
        let result = text_result(&json!({"value":[{"name":"A"}]}));
        assert_eq!(rows(&payload(&result, 1000).unwrap()).unwrap().len(), 1);
        assert!(payload(&result, 5).unwrap_err().contains("size limit"));
        let mut rejected = text_result(&json!("secret provider detail"));
        rejected.is_error = Some(true);
        assert_eq!(payload(&rejected, 1000).unwrap_err(), TOOL_REJECTED);
        assert!(rejection_detail(&rejected).unwrap().len() <= 300);
        let empty: CallToolResult = serde_json::from_value(json!({"content":[]})).unwrap();
        assert!(
            payload(&empty, 1000)
                .unwrap_err()
                .contains("absence is not proven")
        );
        let structured = CallToolResult::structured(json!({"rows":[1,2]}));
        assert_eq!(rows(&payload(&structured, 1000).unwrap()).unwrap().len(), 2);
    }

    #[test]
    fn rows_are_found_in_dataverse_and_power_bi_shapes_but_not_guessed() {
        assert_eq!(rows(&json!([{"a":1}])).unwrap().len(), 1);
        assert_eq!(rows(&json!({"value":[{}, {}]})).unwrap().len(), 2);
        assert_eq!(
            rows(&json!({"results":[{"tables":[{"rows":[{"[ACR]":1.0}]}]}]})).unwrap()[0]["[ACR]"],
            1.0
        );
        assert_eq!(rows(&json!({"result":{"records":[{}]}})).unwrap().len(), 1);
        assert_eq!(
            rows(&json!({"queryresult":[{}], "count": 1}))
                .unwrap()
                .len(),
            1
        );
        assert!(rows(&json!({"a":[1], "b":[2]})).is_none());
        assert!(rows(&json!("| table | text |")).is_none());
    }

    #[test]
    fn http_failures_map_to_sign_in_retry_or_permanent_rejection() {
        assert_eq!(
            classify("HTTP 401: credential rejected".into()),
            CallError::Unauthorized
        );
        assert!(matches!(
            classify("HTTP 503: provider unavailable; retry later".into()),
            CallError::Transport(_)
        ));
        assert!(matches!(
            classify("cannot reach the provider".into()),
            CallError::Transport(_)
        ));
        assert!(matches!(
            classify("HTTP 403: access denied".into()),
            CallError::Rejected(_)
        ));
    }
}
