use crate::copilot::Config;
use rmcp::{
    ServiceExt,
    model::ClientConfig,
    transport::{
        StreamableHttpClientTransport,
        auth::{
            AuthError, AuthorizationManager, AuthorizationMetadata, AuthorizationMetadataSource,
        },
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Serialize;
use std::{path::Path, time::Duration};

pub mod connection;
pub mod hosted;
mod vault;

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    IssuerCompatibilityBlocked,
    ClientIdentityRequired,
    ClientRegistrationApprovalRequired,
    UnexpectedUnauthenticatedConnection,
}

#[derive(Debug, Serialize)]
pub struct ProbeReport {
    pub connector: String,
    pub outcome: Outcome,
    pub metadata_source: String,
    pub issuer_validation_error: Option<String>,
    pub tenant_id: Option<String>,
    pub validated_issuer: Option<String>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub pkce_s256_advertised: bool,
    pub dynamic_registration_advertised: bool,
    pub client_metadata_documents_advertised: bool,
    pub elapsed_ms: u128,
    pub authenticated_read_verified: bool,
    pub model_calls: u32,
    pub message: String,
}

impl ProbeReport {
    pub fn summary(&self) -> String {
        format!(
            "{}: {} ({} ms; no model calls)",
            self.connector, self.message, self.elapsed_ms
        )
    }
}

fn registration_support(metadata: &AuthorizationMetadata) -> (bool, bool) {
    (
        metadata.registration_endpoint.is_some(),
        metadata
            .additional_fields
            .get("client_id_metadata_document_supported")
            .and_then(serde_json::Value::as_bool)
            == Some(true),
    )
}

fn safe_endpoint(value: &str) -> Result<String, String> {
    let url = url::Url::parse(value).map_err(|_| "Invalid OAuth metadata endpoint")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || value.chars().any(char::is_control)
    {
        return Err("OAuth metadata endpoint must be credential-free HTTPS".into());
    }
    Ok(value.into())
}

fn resource_tenant(endpoint: &str) -> Result<String, String> {
    safe_endpoint(endpoint)?;
    let parsed = url::Url::parse(endpoint).map_err(|_| "Invalid WorkIQ resource")?;
    let parts = parsed.path().split('/').collect::<Vec<_>>();
    if parsed.host_str() != Some("agent365.svc.cloud.microsoft")
        || parsed.port_or_known_default() != Some(443)
        || parts.len() != 6
        || parts[1] != "agents"
        || parts[2] != "tenants"
        || parts[4] != "servers"
        || !matches!(
            parts[5],
            "mcp_CalendarTools"
                | "mcp_MailTools"
                | "mcp_TeamsServer"
                | "mcp_SharePointRemoteServer"
        )
    {
        return Err("Tenant discovery requires a configured Agent 365 WorkIQ resource".into());
    }
    let tenant = parts[3];
    let groups = tenant.split('-').collect::<Vec<_>>();
    if groups.len() != 5
        || !groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, length)| {
            group.len() == length && group.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        || tenant.bytes().all(|byte| byte == b'0' || byte == b'-')
    {
        return Err("The WorkIQ resource must contain a nonzero GUID tenant ID".into());
    }
    Ok(tenant.to_ascii_lowercase())
}

fn validate_tenant_metadata(tenant: &str, metadata: &AuthorizationMetadata) -> Result<(), String> {
    let authority = format!("https://login.microsoftonline.com/{tenant}");
    let issuer = format!("{authority}/v2.0");
    if metadata.issuer.as_deref() != Some(issuer.as_str())
        || metadata.authorization_endpoint != format!("{authority}/oauth2/v2.0/authorize")
        || metadata.token_endpoint != format!("{authority}/oauth2/v2.0/token")
    {
        return Err(
            "Tenant-specific Entra issuer or endpoints do not match the configured resource tenant"
                .into(),
        );
    }
    Ok(())
}

fn discovery_error(error: AuthError) -> String {
    let detail = match error {
        AuthError::AuthorizationServerMismatch {
            expected_issuer,
            received_issuer,
        } => match (
            safe_endpoint(&expected_issuer),
            safe_endpoint(&received_issuer),
        ) {
            (Ok(expected), Ok(received)) => {
                format!("issuer mismatch: expected {expected}, received {received}")
            }
            _ => "issuer mismatch (unsafe endpoint details withheld)".into(),
        },
        AuthError::AuthorizationServerMissingIssuer { .. } => {
            "authorization-server metadata is missing its required issuer".into()
        }
        AuthError::HttpError(error) => format!("HTTP failure: {}", error.without_url()),
        AuthError::MetadataError(_) => "published OAuth metadata could not be validated".into(),
        _ => "OAuth discovery could not complete".into(),
    };
    format!("WorkIQ OAuth discovery failed: {detail}. No credentials were acquired")
}

/// No-model preflight. Does not register clients, open browsers, acquire tokens or
/// access personal credential stores. A browser flow needs an approved client identity.
pub async fn probe(root: &Path, connector: &str) -> Result<ProbeReport, String> {
    if !matches!(
        connector,
        "WorkIQ-Calendar" | "WorkIQ-Mail" | "WorkIQ-Teams" | "WorkIQ-Sharepoint"
    ) {
        return Err("Choose a supported WorkIQ connector for this probe".into());
    }
    let started = std::time::Instant::now();
    let connections = Config::load_connections(root)?;
    let connection = connections
        .get(connector)
        .ok_or_else(|| format!("{connector} is not configured in Tomlook connections.json"))?;
    if connection.credential_env.is_some() {
        return Err("This unauthenticated probe does not import configured tokens; select a connector without credential_env".into());
    }
    let mut report = tokio::time::timeout(TIMEOUT, discover(connector, &connection.url))
        .await
        .map_err(|_| "WorkIQ initialization/auth discovery exceeded its 60-second deadline")??;
    report.elapsed_ms = started.elapsed().as_millis();
    Ok(report)
}

async fn discover(connector: &str, endpoint: &str) -> Result<ProbeReport, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Create WorkIQ HTTP client: {}", e.without_url()))?;
    let transport = StreamableHttpClientTransport::with_client(
        client,
        StreamableHttpClientTransportConfig::with_uri(endpoint),
    );
    let challenge = match ClientConfig::default().serve(transport).await {
        Ok(service) => {
            tokio::time::timeout(Duration::from_secs(5), service.cancel())
                .await
                .map_err(|_| "Close unauthenticated WorkIQ session timed out")?
                .map_err(|_| "Close unauthenticated WorkIQ session failed")?;
            return Ok(ProbeReport {
                connector: connector.into(),
                outcome: Outcome::UnexpectedUnauthenticatedConnection,
                metadata_source: "not_checked".into(),
                issuer_validation_error: None,
                tenant_id: None,
                validated_issuer: None,
                authorization_endpoint: None,
                token_endpoint: None,
                pkce_s256_advertised: false,
                dynamic_registration_advertised: false,
                client_metadata_documents_advertised: false,
                elapsed_ms: 0,
                authenticated_read_verified: false,
                model_calls: 0,
                message: "Initialization accepted without authentication; no tools were called and user identity is unverified".into(),
            });
        }
        Err(error) => error
            .auth_challenge()
            .ok_or("WorkIQ initialization failed without a usable OAuth challenge; no sign-in or tool call was attempted")?
            .to_owned(),
    };
    let manager = AuthorizationManager::new(endpoint)
        .await
        .map_err(|_| "Create WorkIQ OAuth discovery manager failed")?;
    let resolution = manager
        .resolve_metadata_from_challenge(Some(&challenge))
        .await;
    let resolution = match resolution {
        Ok(resolution) => resolution,
        Err(error @ AuthError::AuthorizationServerMismatch { .. }) => {
            return inspect_entra_tenant(connector, endpoint, error).await;
        }
        Err(error) => return Err(discovery_error(error)),
    };
    let source = match resolution.source {
        AuthorizationMetadataSource::ProtectedResourceMetadata => "protected_resource_metadata",
        AuthorizationMetadataSource::AuthorizationServerMetadata => "authorization_server_metadata",
        _ => return Err("WorkIQ did not publish supported OAuth metadata; synthesized fallback endpoints are rejected".into()),
    };
    let metadata = resolution.metadata;
    let (dynamic, documents) = registration_support(&metadata);
    let outcome = if dynamic || documents {
        Outcome::ClientRegistrationApprovalRequired
    } else {
        Outcome::ClientIdentityRequired
    };
    Ok(ProbeReport {
        connector: connector.into(),
        outcome,
        metadata_source: source.into(),
        issuer_validation_error: None,
        tenant_id: None,
        validated_issuer: metadata.issuer.clone(),
        authorization_endpoint: Some(safe_endpoint(&metadata.authorization_endpoint)?),
        token_endpoint: Some(safe_endpoint(&metadata.token_endpoint)?),
        pkce_s256_advertised: metadata
            .code_challenge_methods_supported
            .as_ref()
            .is_some_and(|methods| methods.iter().any(|method| method == "S256")),
        dynamic_registration_advertised: dynamic,
        client_metadata_documents_advertised: documents,
        elapsed_ms: 0,
        authenticated_read_verified: false,
        model_calls: 0,
        message: if dynamic || documents {
            "OAuth discovered; choosing/registering a client requires explicit approval before browser sign-in"
        } else {
            "OAuth discovered, but no automatic client-registration mechanism is advertised. Browser sign-in requires an approved client ID and redirect URI"
        }.into(),
    })
}

async fn inspect_entra_tenant(
    connector: &str,
    resource: &str,
    error: AuthError,
) -> Result<ProbeReport, String> {
    let AuthError::AuthorizationServerMismatch {
        expected_issuer,
        received_issuer,
    } = &error
    else {
        return Err(discovery_error(error));
    };
    if expected_issuer != "https://login.microsoftonline.com/organizations/v2.0"
        || received_issuer != "https://login.microsoftonline.com/{tenantid}/v2.0"
    {
        return Err(discovery_error(error));
    }
    let tenant = resource_tenant(resource)?;
    let authority = format!("https://login.microsoftonline.com/{tenant}/v2.0");
    // Diagnose the documented tenant-specific authority independently. Do not
    // rewrite rejected metadata or relax the SDK's exact issuer comparison.
    let manager = AuthorizationManager::new(authority)
        .await
        .map_err(discovery_error)?;
    let resolution = manager.resolve_metadata().await.map_err(discovery_error)?;
    if !matches!(
        resolution.source,
        AuthorizationMetadataSource::AuthorizationServerMetadata
    ) {
        return Err("Tenant-specific Entra discovery did not return published authorization-server metadata".into());
    }
    let metadata = resolution.metadata;
    validate_tenant_metadata(&tenant, &metadata)?;
    let (dynamic, documents) = registration_support(&metadata);
    Ok(ProbeReport {
        connector: connector.into(),
        outcome: Outcome::ClientIdentityRequired,
        metadata_source: "tenant_specific_authorization_server_metadata".into(),
        issuer_validation_error: Some(discovery_error(error)),
        tenant_id: Some(tenant),
        validated_issuer: metadata.issuer.clone(),
        authorization_endpoint: Some(safe_endpoint(&metadata.authorization_endpoint)?),
        token_endpoint: Some(safe_endpoint(&metadata.token_endpoint)?),
        pkce_s256_advertised: metadata
            .code_challenge_methods_supported
            .as_ref()
            .is_some_and(|methods| methods.iter().any(|method| method == "S256")),
        dynamic_registration_advertised: dynamic,
        client_metadata_documents_advertised: documents,
        elapsed_ms: 0,
        authenticated_read_verified: false,
        model_calls: 0,
        message: "Tenant-specific Entra discovery verified an exact issuer and matching endpoints. Browser sign-in still requires an approved public-client ID and redirect URI; no credentials were acquired".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::transport::auth::{
        OAuthClientConfig, OAuthHttpClient, OAuthHttpClientFuture, OAuthHttpRequest,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct NoNetwork {
        attempts: AtomicUsize,
    }

    impl OAuthHttpClient for NoNetwork {
        fn execute(&self, _: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Err(std::io::Error::other("OAuth fixture must not make HTTP requests").into())
            })
        }
    }

    async fn fixture_manager(network: Arc<NoNetwork>) -> AuthorizationManager {
        let mut manager = AuthorizationManager::new_with_oauth_http_client(
            "https://resource.example/mcp",
            network,
        )
        .await
        .unwrap();
        let mut metadata = metadata();
        metadata.issuer = Some("https://login.example".into());
        metadata.additional_fields.insert(
            "authorization_response_iss_parameter_supported".into(),
            serde_json::json!(true),
        );
        manager.set_metadata(metadata);
        manager
            .configure_client(OAuthClientConfig::new(
                "00000000-0000-0000-0000-000000000001",
                "http://localhost:8080/callback",
            ))
            .unwrap();
        manager
    }

    fn query(value: &str) -> std::collections::BTreeMap<String, String> {
        url::Url::parse(value)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    fn metadata() -> AuthorizationMetadata {
        serde_json::from_value(serde_json::json!({
            "authorization_endpoint": "https://login.example/authorize",
            "token_endpoint": "https://login.example/token"
        }))
        .unwrap()
    }

    #[test]
    fn discovery_does_not_invent_registration_support() {
        let mut metadata = metadata();
        assert_eq!(registration_support(&metadata), (false, false));
        metadata.additional_fields.insert(
            "client_id_metadata_document_supported".into(),
            serde_json::json!("true"),
        );
        assert_eq!(registration_support(&metadata), (false, false));
        metadata.additional_fields.insert(
            "client_id_metadata_document_supported".into(),
            serde_json::json!(true),
        );
        metadata.registration_endpoint = Some("https://login.example/register".into());
        assert_eq!(registration_support(&metadata), (true, true));
    }

    #[test]
    fn oauth_endpoints_cannot_expose_credentials() {
        assert!(safe_endpoint("https://login.example/token").is_ok());
        for endpoint in [
            "http://login.example/token",
            "https://user:secret@login.example/token",
            "https://login.example/token?secret=value",
            "https://login.example/token#secret",
            "https://login.example/token\n",
        ] {
            assert!(safe_endpoint(endpoint).is_err());
        }
    }

    #[test]
    fn issuer_validation_failure_is_explicit_and_credential_free() {
        let error = AuthError::AuthorizationServerMismatch {
            expected_issuer: "https://login.example/organizations/v2.0".into(),
            received_issuer: "https://login.example/{tenantid}/v2.0".into(),
        };
        assert!(discovery_error(error).contains("issuer mismatch"));
        let error = AuthError::AuthorizationServerMismatch {
            expected_issuer: "https://login.example/token?secret=value".into(),
            received_issuer: "https://login.example/token".into(),
        };
        assert!(!discovery_error(error).contains("secret=value"));
    }

    #[test]
    fn tenant_authority_is_bound_to_a_configured_workiq_resource() {
        let tenant = "72f988bf-86f1-41af-91ab-2d7cd011db47";
        let resource = format!(
            "https://agent365.svc.cloud.microsoft/agents/tenants/{tenant}/servers/mcp_CalendarTools"
        );
        assert_eq!(resource_tenant(&resource).unwrap(), tenant);
        for endpoint in [
            resource.replace(tenant, "organizations"),
            resource.replace(tenant, "00000000-0000-0000-0000-000000000000"),
            resource.replace("agent365.svc.cloud.microsoft", "attacker.example"),
            resource.replace("/agents/", "/other/"),
            resource.replace("mcp_CalendarTools", "unknown"),
            format!("{resource}/extra"),
            format!("{resource}?tenant=other"),
            resource.replace("https://", "http://"),
        ] {
            assert!(resource_tenant(&endpoint).is_err(), "{endpoint}");
        }
    }

    #[test]
    fn tenant_metadata_requires_exact_issuer_and_endpoints() {
        let tenant = "72f988bf-86f1-41af-91ab-2d7cd011db47";
        let authority = format!("https://login.microsoftonline.com/{tenant}");
        let mut metadata = metadata();
        metadata.issuer = Some(format!("{authority}/v2.0"));
        metadata.authorization_endpoint = format!("{authority}/oauth2/v2.0/authorize");
        metadata.token_endpoint = format!("{authority}/oauth2/v2.0/token");
        assert!(validate_tenant_metadata(tenant, &metadata).is_ok());
        metadata.issuer = Some("https://login.microsoftonline.com/{tenantid}/v2.0".into());
        assert!(validate_tenant_metadata(tenant, &metadata).is_err());
        metadata.issuer = Some(format!("{authority}/v2.0"));
        metadata.token_endpoint = "https://attacker.example/token".into();
        assert!(validate_tenant_metadata(tenant, &metadata).is_err());
    }

    #[tokio::test]
    async fn public_client_request_uses_fresh_s256_state_and_exact_resource_without_network() {
        let network = Arc::new(NoNetwork::default());
        let manager = fixture_manager(network.clone()).await;
        let scopes = ["https://resource.example/mcp/.default", "openid"];
        let first = query(&manager.get_authorization_url(&scopes).await.unwrap());
        let second = query(&manager.get_authorization_url(&scopes).await.unwrap());
        assert_eq!(first["response_type"], "code");
        assert_eq!(first["code_challenge_method"], "S256");
        assert_eq!(first["code_challenge"].len(), 43);
        assert_eq!(first["redirect_uri"], "http://localhost:8080/callback");
        assert_eq!(first["resource"], "https://resource.example/mcp");
        assert_eq!(first["scope"], scopes.join(" "));
        assert_ne!(first["state"], second["state"]);
        assert_ne!(first["code_challenge"], second["code_challenge"]);
        assert!(!first.contains_key("client_secret"));
        assert!(!first.contains_key("code_verifier"));
        assert_eq!(network.attempts.load(Ordering::SeqCst), 0);
        assert!(manager.get_credentials().await.unwrap().1.is_none());
    }

    #[tokio::test]
    async fn invalid_callback_state_and_issuer_are_rejected_before_token_exchange() {
        let network = Arc::new(NoNetwork::default());
        let manager = fixture_manager(network.clone()).await;
        let params = query(&manager.get_authorization_url(&["openid"]).await.unwrap());
        assert!(
            manager
                .exchange_code_for_token_with_issuer(
                    "fixture-code",
                    "wrong-state",
                    Some("https://login.example"),
                )
                .await
                .is_err()
        );
        assert!(matches!(
            manager
                .exchange_code_for_token_with_issuer(
                    "fixture-code",
                    &params["state"],
                    Some("https://attacker.example"),
                )
                .await,
            Err(AuthError::AuthorizationServerMismatch { .. })
        ));
        assert_eq!(network.attempts.load(Ordering::SeqCst), 0);
        assert!(manager.get_credentials().await.unwrap().1.is_none());
    }

    #[tokio::test]
    async fn advertised_plain_only_pkce_never_downgrades_s256() {
        let network = Arc::new(NoNetwork::default());
        let mut manager = fixture_manager(network.clone()).await;
        let mut metadata = metadata();
        metadata.code_challenge_methods_supported = Some(vec!["plain".into()]);
        manager.set_metadata(metadata);
        assert!(matches!(
            manager.get_authorization_url(&["openid"]).await,
            Err(AuthError::PkceUnsupported)
        ));
        assert_eq!(network.attempts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn connector_configuration_does_not_require_a_copilot_runtime() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("connections.json"),
            serde_json::json!({
                "runtime": "C:\\not-installed\\copilot.exe",
                "model": "",
                "servers": {
                    "WorkIQ-Calendar": {"url": "https://workiq.example/calendar"}
                }
            })
            .to_string(),
        )
        .unwrap();
        assert!(Config::load(root.path()).is_err());
        let connections = Config::load_connections(root.path()).unwrap();
        assert_eq!(connections.len(), 1);
        assert!(!root.path().join("copilot").exists());
    }

    #[tokio::test]
    async fn unrelated_connectors_are_rejected_before_reading_configuration() {
        let root = tempfile::tempdir().unwrap();
        assert!(probe(root.path(), "WebIQ-MCP").await.is_err());
        assert!(root.path().read_dir().unwrap().next().is_none());
    }
}
