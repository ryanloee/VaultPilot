//! External Service Connectors — Agent-accessible bridges to Slack, Email,
//! GitHub, and other external services.
//!
//! #1841: The `Connector` trait defines a uniform interface for external
//! service integrations. Connectors are registered in the
//! [`CapabilityRegistry`](crate::capability_registry) as MCP servers, and
//! agents access them through the shared tool layer.
//!
//! ## Architecture
//!
//! - [`Connector`] trait: name, auth_flow, capabilities, execute
//! - [`WebhookConnector`]: Phase 1 reference implementation — receives generic
//!   HTTP callbacks and writes vault notes
//! - Future connectors (GitHub, Email, Slack) implement the same trait
//!
//! ## Usage
//!
//! ```rust,ignore
//! use vaultpilot_lib::connector::{Connector, WebhookConnector};
//! let webhook = WebhookConnector::new("incoming-slack", "/tmp/webhook-secret");
//! webhook.execute(Action {
//!     name: "webhook_receive".into(),
//!     payload: serde_json::json!({"text": "Hello"}),
//! })?;
//! ```

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Core trait ─────────────────────────────────────────────────

/// Authentication flow a connector requires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AuthFlow {
    /// OAuth 2.0 authorization code flow.
    OAuth2 {
        authorize_url: String,
        token_url: String,
        client_id: String,
        /// Scopes (space-separated).
        scopes: String,
    },
    /// Simple API key or bearer token.
    ApiKey {
        /// Environment variable or config key name.
        key_name: String,
        /// Where to get the token (env, config, or prompt).
        source: TokenSource,
    },
    /// Static token passed directly.
    StaticToken { token: String },
    /// No authentication required.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenSource {
    /// Read from an environment variable.
    Env,
    /// Read from AppSettings (provider-style).
    Config,
    /// Prompt the user on first use.
    Prompt,
}

/// Describes a single capability the connector exposes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capability {
    /// Human-readable name (e.g. "Read Channel Messages").
    pub name: String,
    /// Description for AI tool selection.
    pub description: String,
    /// read / write / search / subscribe.
    pub access: AccessLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Read,
    Write,
    Search,
    Subscribe,
    ReadWrite,
    Admin,
}

/// An action dispatched to a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    /// Action name (connector-specific, e.g. "read_messages").
    pub name: String,
    /// Parameters as JSON.
    pub payload: Value,
}

/// Result of executing an action through a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorResponse {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Human-readable result summary.
    pub summary: String,
    /// Raw data returned by the service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// The central trait for all external service connectors.
///
/// # Implementors
///
/// - [`WebhookConnector`] — Phase 1 reference
/// - GitHub / Email / Slack — Phase 2
/// - Third-party MCP — Phase 3
pub trait Connector: Send + Sync {
    /// Unique connector identifier (e.g. "github", "slack-workspace").
    fn name(&self) -> &str;

    /// The authentication method this connector requires.
    fn auth_flow(&self) -> AuthFlow;

    /// List the capabilities this connector provides to agents.
    fn capabilities(&self) -> Vec<Capability>;

    /// Execute an action and return the result.
    fn execute(
        &self,
        action: &Action,
        secrets: &HashMap<String, String>,
    ) -> Result<ConnectorResponse>;

    /// Validate that authentication is configured and working.
    /// Returns `Ok(())` if ready, or an error describing what's missing.
    fn validate(&self, secrets: &HashMap<String, String>) -> Result<()> {
        let flow = self.auth_flow();
        match flow {
            AuthFlow::None => Ok(()),
            AuthFlow::StaticToken { .. } => Ok(()),
            AuthFlow::ApiKey {
                ref key_name,
                ref source,
            } => {
                let has_key = match source {
                    TokenSource::Env => std::env::var(key_name).is_ok(),
                    TokenSource::Config | TokenSource::Prompt => secrets.contains_key(key_name),
                };
                if has_key {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "connector '{}' requires API key '{}' (source: {:?})",
                        self.name(),
                        key_name,
                        source
                    ))
                }
            }
            AuthFlow::OAuth2 { .. } => {
                // OAuth2 readiness depends on stored tokens; check for
                // presence of an access token in secrets.
                let token_key = format!("{}_access_token", self.name());
                if secrets.contains_key(&token_key) {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "connector '{}' requires OAuth2 authorization — visit {} to grant access",
                        self.name(),
                        match flow {
                            AuthFlow::OAuth2 {
                                ref authorize_url, ..
                            } => authorize_url.as_str(),
                            _ => "",
                        }
                    ))
                }
            }
        }
    }
}

// ── Webhook Connector (Phase 1) ───────────────────────────────

/// A generic HTTP webhook connector.
///
/// Receives incoming webhook payloads (from Slack, GitHub, custom services)
/// and writes them as vault notes.  This is the lowest-cost external
/// integration point — users set up a webhook URL on their service and
/// VaultPilot captures the payloads.
///
/// # Security
///
/// Each connector instance has a shared secret (`hmac_secret`) used for
/// HMAC-SHA256 signature verification.  Incoming payloads are rejected if
/// the signature doesn't match (constant-time comparison).
#[derive(Debug, Clone)]
pub struct WebhookConnector {
    /// Connector identifier (e.g. "slack-incoming", "github-events").
    id: String,
    /// Display label.
    label: String,
    /// Path to the shared secret file (HMAC key).
    secret_path: String,
    /// Max payload size in bytes (default: 64 KiB).
    max_payload_bytes: usize,
}

impl WebhookConnector {
    /// Create a new webhook connector.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        secret_path: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            secret_path: secret_path.into(),
            max_payload_bytes: 64 * 1024,
        }
    }

    /// Set a custom max payload size.
    pub fn with_max_payload(mut self, bytes: usize) -> Self {
        self.max_payload_bytes = bytes;
        self
    }

    /// Read the shared secret for HMAC verification.
    fn load_secret(&self) -> Result<String> {
        let content = std::fs::read_to_string(&self.secret_path)
            .map_err(|e| anyhow::anyhow!("webhook '{}': cannot read secret: {e}", self.id))?;
        Ok(content.trim().to_string())
    }

    /// Verify an HMAC-SHA256 signature (constant-time).
    fn verify_signature(&self, payload: &str, signature: &str) -> Result<bool> {
        use sha2::Digest;
        let secret = self.load_secret()?;

        // Build HMAC-SHA256 manually: H(K ⊕ opad ∥ H(K ⊕ ipad ∥ message))
        // Key padding (block size = 64 bytes for SHA256)
        let mut key = [0u8; 64];
        let secret_bytes = secret.as_bytes();
        if secret_bytes.len() > 64 {
            // Hash the key first if it's longer than block size
            let hashed = sha2::Sha256::digest(secret_bytes);
            key[..32].copy_from_slice(&hashed);
        } else {
            let len = secret_bytes.len().min(64);
            key[..len].copy_from_slice(&secret_bytes[..len]);
        }

        // ipad = key ⊕ 0x36, opad = key ⊕ 0x5c
        let mut ipad = [0x36u8; 64];
        let mut opad = [0x5cu8; 64];
        for i in 0..64 {
            ipad[i] ^= key[i];
            opad[i] ^= key[i];
        }

        // Inner hash: SHA256(ipad ∥ message)
        let mut inner = sha2::Sha256::new();
        inner.update(ipad);
        inner.update(payload.as_bytes());
        let inner_hash = inner.finalize();

        // Outer hash: SHA256(opad ∥ inner_hash)
        let mut outer = sha2::Sha256::new();
        outer.update(opad);
        outer.update(inner_hash);
        let tag = outer.finalize();

        // Hex-encode the tag manually
        let expected: String = tag.iter().map(|b| format!("{b:02x}")).collect();

        // Constant-time comparison
        use subtle::ConstantTimeEq;
        Ok(expected.as_bytes().ct_eq(signature.as_bytes()).into())
    }
}

impl Connector for WebhookConnector {
    fn name(&self) -> &str {
        &self.id
    }

    fn auth_flow(&self) -> AuthFlow {
        // Webhook connectors use a shared secret (HMAC), not user-facing
        // OAuth or API keys.  The secret is loaded from a file.
        AuthFlow::ApiKey {
            key_name: format!("{}_webhook_secret", self.id),
            source: TokenSource::Config,
        }
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![
            Capability {
                name: "webhook_receive".into(),
                description: format!(
                    "Receive an incoming webhook payload from {} and save it as a vault note",
                    self.label
                ),
                access: AccessLevel::Write,
            },
            Capability {
                name: "webhook_list_recent".into(),
                description: format!(
                    "List recent webhook deliveries from {} (last 50)",
                    self.label
                ),
                access: AccessLevel::Read,
            },
        ]
    }

    fn execute(
        &self,
        action: &Action,
        _secrets: &HashMap<String, String>,
    ) -> Result<ConnectorResponse> {
        match action.name.as_str() {
            "webhook_receive" => {
                let payload_str = serde_json::to_string(&action.payload)?;

                // Check payload size
                if payload_str.len() > self.max_payload_bytes {
                    return Ok(ConnectorResponse {
                        success: false,
                        summary: format!(
                            "payload too large ({} bytes, max {})",
                            payload_str.len(),
                            self.max_payload_bytes
                        ),
                        data: None,
                    });
                }

                // Verify signature if present
                if let Some(sig) = action.payload.get("_signature").and_then(Value::as_str) {
                    if !self.verify_signature(&payload_str, sig)? {
                        return Ok(ConnectorResponse {
                            success: false,
                            summary: "HMAC signature verification failed".into(),
                            data: None,
                        });
                    }
                }

                // Build a note payload
                let source = action
                    .payload
                    .get("_source")
                    .and_then(Value::as_str)
                    .unwrap_or(&self.label);

                Ok(ConnectorResponse {
                    success: true,
                    summary: format!(
                        "webhook from {} received ({} bytes)",
                        source,
                        payload_str.len()
                    ),
                    data: Some(serde_json::json!({
                        "source": source,
                        "payload": action.payload,
                        "note_ready": true,
                    })),
                })
            }
            "webhook_list_recent" => {
                // Placeholder — in production this queries the vault for
                // recent webhook-derived notes.
                Ok(ConnectorResponse {
                    success: true,
                    summary: "webhook list queried (stub — implement with vault search)".into(),
                    data: Some(serde_json::json!({
                        "deliveries": [],
                        "note": "vault note search for webhook-source notes not yet wired"
                    })),
                })
            }
            other => Err(anyhow::anyhow!(
                "webhook connector '{}': unknown action '{}'",
                self.id,
                other
            )),
        }
    }
}

// ── Registry integration helpers ───────────────────────────────

/// Register a connector as an MCP server capability in the registry.
///
/// This bridges the [`Connector`] trait into the existing
/// [`CapabilityRegistry`](crate::capability_registry::CapabilityRegistry)
/// so agents can discover and use connectors through the shared tool layer.
pub fn register_as_mcp(
    registry: &mut crate::capability_registry::CapabilityRegistry,
    connector_id: &str,
    connector_name: &str,
    description: &str,
    http_url: &str,
) {
    use crate::capability_registry::{AuthConfig, McpTransport};
    registry.add_server(
        connector_id,
        connector_name,
        description,
        McpTransport::Http {
            url: http_url.to_string(),
        },
        None::<AuthConfig>,
    );
    tracing::info!(
        connector_id = connector_id,
        url = http_url,
        "registered connector in capability registry"
    );
}

// ── GitHub Connector (Phase 2) ───────────────────────────────

/// A fully-described outbound HTTP request built by a connector.
///
/// Pure data so it can be inspected in tests (dry-run) and dispatched by an
/// async executor in production without the trait method itself being async.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRequest {
    /// HTTP method (GET/POST/...).
    pub method: String,
    /// Fully-qualified request URL.
    pub url: String,
    /// `Accept` header value.
    pub accept: String,
    /// Optional JSON request body (for writes).
    pub body: Option<String>,
}

/// A summarized GitHub issue record parsed from a REST API response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSummary {
    /// Issue/pull-request number.
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// Issue state (`open` / `closed`).
    pub state: String,
    /// Author login.
    pub author: String,
}

/// GitHub connector — reads/writes issues and PRs via the GitHub REST API.
///
/// Phase 2 of #1841. Implements the [`Connector`] trait on top of the
/// existing foundation. Authentication uses a personal access token supplied
/// through `secrets` under the key `github_token`.
///
/// The connector is built around *pure* request construction
/// ([`GitHubConnector::build_request`]) and parsing
/// ([`GitHubConnector::parse_issue`]) so the logic is fully unit-testable
/// without network access. In `dry_run` mode [`Connector::execute`] returns
/// the constructed request as `data` instead of performing the HTTP call.
#[derive(Debug, Clone)]
pub struct GitHubConnector {
    /// Connector identifier (e.g. "github-vaultpilot").
    id: String,
    /// Repository owner.
    owner: String,
    /// Repository name.
    repo: String,
    /// When `true`, `execute` returns the built request instead of sending it.
    dry_run: bool,
}

impl GitHubConnector {
    /// Create a new GitHub connector for `owner/repo`.
    pub fn new(id: impl Into<String>, owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            owner: owner.into(),
            repo: repo.into(),
            dry_run: false,
        }
    }

    /// Enable dry-run mode (offline request construction, no network).
    pub fn with_dry_run(mut self, dry: bool) -> Self {
        self.dry_run = dry;
        self
    }

    /// Build the REST API URL for a given endpoint path (e.g. `"issues"`).
    pub fn api_url(&self, path: &str) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/{}",
            self.owner,
            self.repo,
            path.trim_start_matches('/')
        )
    }

    /// Serialize an issue-creation payload (pure, testable).
    pub fn build_create_issue_body(
        &self,
        title: &str,
        body: Option<&str>,
        labels: &[String],
    ) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("title".into(), Value::String(title.to_string()));
        if let Some(b) = body {
            m.insert("body".into(), Value::String(b.to_string()));
        }
        if !labels.is_empty() {
            m.insert(
                "labels".into(),
                Value::Array(labels.iter().cloned().map(Value::String).collect()),
            );
        }
        Value::Object(m)
    }

    /// Parse a GitHub issue JSON object into an [`IssueSummary`] (pure, testable).
    pub fn parse_issue(&self, issue_json: &Value) -> Result<IssueSummary> {
        let number = issue_json
            .get("number")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("github issue missing 'number'"))?;
        let title = issue_json
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("github issue missing 'title'"))?
            .to_string();
        let state = issue_json
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("open")
            .to_string();
        let author = issue_json
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        Ok(IssueSummary {
            number,
            title,
            state,
            author,
        })
    }

    /// Build the outbound request for a given action (pure, testable).
    ///
    /// Returns an error if the required `github_token` secret is missing or
    /// the action payload is malformed.
    pub fn build_request(
        &self,
        action: &Action,
        secrets: &HashMap<String, String>,
    ) -> Result<GitHubRequest> {
        // Validate auth up-front so callers get a clear error before building.
        let _token = secrets.get("github_token").ok_or_else(|| {
            anyhow::anyhow!(
                "github connector '{}' requires 'github_token' in secrets",
                self.id
            )
        })?;

        match action.name.as_str() {
            "list_issues" => Ok(GitHubRequest {
                method: "GET".into(),
                url: self.api_url("issues?state=open&per_page=50"),
                accept: "application/vnd.github+json".into(),
                body: None,
            }),
            "get_issue" => {
                let number = action
                    .payload
                    .get("number")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("get_issue requires 'number' (u64)"))?;
                Ok(GitHubRequest {
                    method: "GET".into(),
                    url: self.api_url(&format!("issues/{number}")),
                    accept: "application/vnd.github+json".into(),
                    body: None,
                })
            }
            "create_issue" => {
                let title = action
                    .payload
                    .get("title")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("create_issue requires 'title' (string)"))?;
                let body = action.payload.get("body").and_then(Value::as_str);
                let labels: Vec<String> = action
                    .payload
                    .get("labels")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let payload = self.build_create_issue_body(title, body, &labels);
                Ok(GitHubRequest {
                    method: "POST".into(),
                    url: self.api_url("issues"),
                    accept: "application/vnd.github+json".into(),
                    body: Some(serde_json::to_string(&payload)?),
                })
            }
            other => Err(anyhow::anyhow!(
                "github connector '{}': unknown action '{}'",
                self.id,
                other
            )),
        }
    }

    /// Register this connector as an MCP server capability in the
    /// [`CapabilityRegistry`](crate::capability_registry::CapabilityRegistry),
    /// mirroring [`register_as_mcp`]. The `http_url` is the local endpoint the
    /// connector's bridge listens on.
    pub fn register(
        &self,
        registry: &mut crate::capability_registry::CapabilityRegistry,
        http_url: &str,
    ) {
        let description = format!(
            "Read/write issues in {}/{} via GitHub REST API",
            self.owner, self.repo
        );
        register_as_mcp(
            registry,
            self.name(),
            "GitHub Connector",
            &description,
            http_url,
        );
    }

    /// Perform the actual HTTP request (async). Used by `execute` in live mode.
    async fn send(&self, req: &GitHubRequest, token: &str) -> Result<ConnectorResponse> {
        let client = reqwest::Client::new();
        let method: reqwest::Method = req
            .method
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid HTTP method '{}': {e}", req.method))?;
        let mut builder = client
            .request(method, &req.url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", &req.accept)
            .header("User-Agent", "VaultPilot-Connector")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(b) = &req.body {
            builder = builder
                .header("Content-Type", "application/json")
                .body(b.clone());
        }
        let resp = builder.send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Ok(ConnectorResponse {
            success: status.is_success(),
            summary: format!("github {} {} -> HTTP {}", req.method, req.url, status),
            data: Some(serde_json::json!({ "status": status.as_u16(), "body": text })),
        })
    }
}

impl Connector for GitHubConnector {
    fn name(&self) -> &str {
        &self.id
    }

    fn auth_flow(&self) -> AuthFlow {
        AuthFlow::ApiKey {
            key_name: "github_token".into(),
            source: TokenSource::Config,
        }
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![
            Capability {
                name: "list_issues".into(),
                description: format!("List open issues in {}/{}", self.owner, self.repo),
                access: AccessLevel::Read,
            },
            Capability {
                name: "get_issue".into(),
                description: format!(
                    "Get a single issue by number in {}/{}",
                    self.owner, self.repo
                ),
                access: AccessLevel::Read,
            },
            Capability {
                name: "create_issue".into(),
                description: format!("Create a new issue in {}/{}", self.owner, self.repo),
                access: AccessLevel::Write,
            },
        ]
    }

    fn execute(
        &self,
        action: &Action,
        secrets: &HashMap<String, String>,
    ) -> Result<ConnectorResponse> {
        let req = self.build_request(action, secrets)?;
        if self.dry_run {
            return Ok(ConnectorResponse {
                success: true,
                summary: format!("dry-run {} {}", req.method, req.url),
                data: Some(serde_json::json!({
                    "method": req.method,
                    "url": req.url,
                    "accept": req.accept,
                    "body": req.body,
                })),
            });
        }
        let token = secrets.get("github_token").cloned().unwrap_or_default();
        // The trait method is synchronous; spin a dedicated runtime for the
        // single network call so callers don't need to be inside an async ctx.
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(self.send(&req, &token))
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── WebhookConnector tests ──

    #[test]
    fn webhook_connector_basics() {
        let wh = WebhookConnector::new("test-wh", "Test Webhook", "/tmp/test-secret");
        assert_eq!(wh.name(), "test-wh");
        let caps = wh.capabilities();
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].name, "webhook_receive");
        assert_eq!(caps[0].access, AccessLevel::Write);
        assert_eq!(caps[1].name, "webhook_list_recent");
    }

    #[test]
    fn webhook_receive_ok() {
        let wh = WebhookConnector::new("test-wh", "Test", "/tmp/test-secret");
        let action = Action {
            name: "webhook_receive".into(),
            payload: serde_json::json!({"text": "hello", "_source": "slack"}),
        };
        let secrets = HashMap::new();
        let resp = wh.execute(&action, &secrets).unwrap();
        assert!(resp.success);
        assert!(resp.summary.contains("slack"));
    }

    #[test]
    fn webhook_receive_too_large() {
        let wh = WebhookConnector::new("test-wh", "Test", "/tmp/test-secret").with_max_payload(10);
        let action = Action {
            name: "webhook_receive".into(),
            payload: serde_json::json!({"text": "this is definitely more than 10 bytes long"}),
        };
        let secrets = HashMap::new();
        let resp = wh.execute(&action, &secrets).unwrap();
        assert!(!resp.success);
        assert!(resp.summary.contains("too large"));
    }

    #[test]
    fn webhook_unknown_action() {
        let wh = WebhookConnector::new("test-wh", "Test", "/tmp/test-secret");
        let action = Action {
            name: "nonexistent".into(),
            payload: serde_json::json!({}),
        };
        let secrets = HashMap::new();
        let err = wh.execute(&action, &secrets).unwrap_err();
        assert!(err.to_string().contains("unknown action"));
    }

    #[test]
    fn webhook_list_recent_stub() {
        let wh = WebhookConnector::new("test-wh", "Test", "/tmp/test-secret");
        let action = Action {
            name: "webhook_list_recent".into(),
            payload: serde_json::json!({}),
        };
        let secrets = HashMap::new();
        let resp = wh.execute(&action, &secrets).unwrap();
        assert!(resp.success);
        assert!(resp.summary.contains("stub"));
    }

    // ── Connector trait tests (via WebhookConnector) ──

    #[test]
    fn trait_auth_flow() {
        let wh = WebhookConnector::new("test-wh", "Test", "/tmp/test-secret");
        let flow = wh.auth_flow();
        assert!(matches!(flow, AuthFlow::ApiKey { .. }));
    }

    #[test]
    fn validate_no_auth_connector() {
        // A connector with AuthFlow::None should always validate.
        struct NoAuthConnector;
        impl Connector for NoAuthConnector {
            fn name(&self) -> &str {
                "noauth"
            }
            fn auth_flow(&self) -> AuthFlow {
                AuthFlow::None
            }
            fn capabilities(&self) -> Vec<Capability> {
                vec![]
            }
            fn execute(
                &self,
                _action: &Action,
                _secrets: &HashMap<String, String>,
            ) -> Result<ConnectorResponse> {
                Ok(ConnectorResponse {
                    success: true,
                    summary: "ok".into(),
                    data: None,
                })
            }
        }
        let secrets = HashMap::new();
        assert!(NoAuthConnector.validate(&secrets).is_ok());
    }

    #[test]
    fn validate_missing_api_key() {
        let wh = WebhookConnector::new("test-wh", "Test", "/tmp/test-secret");
        let secrets = HashMap::new();
        let result = wh.validate(&secrets);
        // Should fail because we don't set the env var or secrets
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("requires API key"));
    }

    #[test]
    fn register_as_mcp_adds_server() {
        let mut registry = crate::capability_registry::CapabilityRegistry::default();
        register_as_mcp(
            &mut registry,
            "gh-connector",
            "GitHub Connector",
            "Read issues and PRs",
            "http://localhost:8090",
        );
        assert!(registry.capabilities.contains_key("gh-connector"));
        let cap = &registry.capabilities["gh-connector"];
        match cap {
            crate::capability_registry::Capability::McpServer {
                name, transport, ..
            } => {
                assert_eq!(name, "GitHub Connector");
                assert!(matches!(
                    transport,
                    crate::capability_registry::McpTransport::Http { .. }
                ));
            }
            _ => panic!("expected McpServer capability"),
        }
    }

    // ── GitHubConnector tests (Phase 2, #1841) ──

    #[test]
    fn github_connector_basics() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        assert_eq!(gh.name(), "gh-vp");
        let caps = gh.capabilities();
        assert_eq!(caps.len(), 3);
        assert_eq!(caps[0].name, "list_issues");
        assert_eq!(caps[0].access, AccessLevel::Read);
        assert_eq!(caps[1].name, "get_issue");
        assert_eq!(caps[2].name, "create_issue");
        assert_eq!(caps[2].access, AccessLevel::Write);
    }

    #[test]
    fn github_auth_flow_uses_token() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        match gh.auth_flow() {
            AuthFlow::ApiKey { key_name, source } => {
                assert_eq!(key_name, "github_token");
                assert_eq!(source, TokenSource::Config);
            }
            other => panic!("expected ApiKey auth flow, got {other:?}"),
        }
        // Without the token, validation must fail.
        let secrets = HashMap::new();
        assert!(gh.validate(&secrets).is_err());
        let mut with_token = HashMap::new();
        with_token.insert("github_token".to_string(), "ghp_xxx".to_string());
        assert!(gh.validate(&with_token).is_ok());
    }

    #[test]
    fn github_api_url() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        assert_eq!(
            gh.api_url("issues"),
            "https://api.github.com/repos/ryanloee/VaultPilot/issues"
        );
        // Leading slash is tolerated.
        assert_eq!(
            gh.api_url("/issues/5"),
            "https://api.github.com/repos/ryanloee/VaultPilot/issues/5"
        );
    }

    #[test]
    fn github_build_create_issue_body() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let labels = vec!["bug".to_string(), "priority".to_string()];
        let body = gh.build_create_issue_body("Hello", Some("world"), &labels);
        assert_eq!(body["title"], "Hello");
        assert_eq!(body["body"], "world");
        assert_eq!(body["labels"][0], "bug");
        assert_eq!(body["labels"][1], "priority");
    }

    #[test]
    fn github_build_create_issue_body_no_labels() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let body = gh.build_create_issue_body("Title", None, &[]);
        assert_eq!(body["title"], "Title");
        assert!(body.get("body").is_none());
        assert!(body.get("labels").is_none());
    }

    #[test]
    fn github_parse_issue() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let json = serde_json::json!({
            "number": 42,
            "title": "Fix the thing",
            "state": "open",
            "user": { "login": "octocat" }
        });
        let summary = gh.parse_issue(&json).unwrap();
        assert_eq!(
            summary,
            IssueSummary {
                number: 42,
                title: "Fix the thing".into(),
                state: "open".into(),
                author: "octocat".into()
            }
        );
    }

    #[test]
    fn github_parse_issue_defaults() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        // Missing user -> "unknown"; missing state -> "open".
        let json = serde_json::json!({ "number": 7, "title": "x" });
        let summary = gh.parse_issue(&json).unwrap();
        assert_eq!(summary.state, "open");
        assert_eq!(summary.author, "unknown");
    }

    #[test]
    fn github_parse_issue_missing_number() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let json = serde_json::json!({ "title": "no number" });
        assert!(gh.parse_issue(&json).is_err());
    }

    #[test]
    fn github_build_request_missing_token() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let action = Action {
            name: "list_issues".into(),
            payload: serde_json::json!({}),
        };
        let secrets = HashMap::new();
        assert!(gh.build_request(&action, &secrets).is_err());
    }

    #[test]
    fn github_build_request_list_issues() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let mut secrets = HashMap::new();
        secrets.insert("github_token".to_string(), "ghp_xxx".to_string());
        let action = Action {
            name: "list_issues".into(),
            payload: serde_json::json!({}),
        };
        let req = gh.build_request(&action, &secrets).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(
            req.url,
            "https://api.github.com/repos/ryanloee/VaultPilot/issues?state=open&per_page=50"
        );
        assert!(req.body.is_none());
    }

    #[test]
    fn github_build_request_get_issue() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let mut secrets = HashMap::new();
        secrets.insert("github_token".to_string(), "ghp_xxx".to_string());
        let action = Action {
            name: "get_issue".into(),
            payload: serde_json::json!({ "number": 123 }),
        };
        let req = gh.build_request(&action, &secrets).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(
            req.url,
            "https://api.github.com/repos/ryanloee/VaultPilot/issues/123"
        );
    }

    #[test]
    fn github_build_request_get_issue_missing_number() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let mut secrets = HashMap::new();
        secrets.insert("github_token".to_string(), "ghp_xxx".to_string());
        let action = Action {
            name: "get_issue".into(),
            payload: serde_json::json!({}),
        };
        assert!(gh.build_request(&action, &secrets).is_err());
    }

    #[test]
    fn github_build_request_create_issue() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let mut secrets = HashMap::new();
        secrets.insert("github_token".to_string(), "ghp_xxx".to_string());
        let action = Action {
            name: "create_issue".into(),
            payload: serde_json::json!({
                "title": "New bug",
                "body": "repro steps",
                "labels": ["bug", "p1"]
            }),
        };
        let req = gh.build_request(&action, &secrets).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.url,
            "https://api.github.com/repos/ryanloee/VaultPilot/issues"
        );
        let parsed: Value = serde_json::from_str(req.body.as_deref().unwrap()).unwrap();
        assert_eq!(parsed["title"], "New bug");
        assert_eq!(parsed["body"], "repro steps");
        assert_eq!(parsed["labels"][1], "p1");
    }

    #[test]
    fn github_build_request_unknown_action() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let mut secrets = HashMap::new();
        secrets.insert("github_token".to_string(), "ghp_xxx".to_string());
        let action = Action {
            name: "delete_repo".into(),
            payload: serde_json::json!({}),
        };
        assert!(gh.build_request(&action, &secrets).is_err());
    }

    #[test]
    fn github_dry_run_execute_returns_request() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot").with_dry_run(true);
        let mut secrets = HashMap::new();
        secrets.insert("github_token".to_string(), "ghp_xxx".to_string());
        let action = Action {
            name: "list_issues".into(),
            payload: serde_json::json!({}),
        };
        let resp = gh.execute(&action, &secrets).unwrap();
        assert!(resp.success);
        assert!(resp.summary.contains("dry-run"));
        assert_eq!(resp.data.as_ref().unwrap()["method"], "GET");
        assert_eq!(
            resp.data.as_ref().unwrap()["url"],
            "https://api.github.com/repos/ryanloee/VaultPilot/issues?state=open&per_page=50"
        );
    }

    #[test]
    fn github_execute_missing_token_errors() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let secrets = HashMap::new();
        let action = Action {
            name: "list_issues".into(),
            payload: serde_json::json!({}),
        };
        // No dry-run and no token -> build_request fails before any network.
        assert!(gh.execute(&action, &secrets).is_err());
    }

    #[test]
    fn github_register_creates_mcp_capability() {
        let gh = GitHubConnector::new("gh-vp", "ryanloee", "VaultPilot");
        let mut registry = crate::capability_registry::CapabilityRegistry::default();
        gh.register(&mut registry, "http://localhost:8091");
        assert!(registry.capabilities.contains_key("gh-vp"));
        match &registry.capabilities["gh-vp"] {
            crate::capability_registry::Capability::McpServer {
                name, transport, ..
            } => {
                assert_eq!(name, "GitHub Connector");
                assert!(matches!(
                    transport,
                    crate::capability_registry::McpTransport::Http { .. }
                ));
            }
            _ => panic!("expected McpServer capability"),
        }
    }
}
