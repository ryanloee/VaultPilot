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

// ── GitHub Connector (Phase 2) ─────────────────────────────────
//
// See <https://docs.github.com/en/rest> for the API reference.

/// A connector for GitHub repository operations — issue/PR reading,
/// notification fetching, and issue search.
///
/// Uses a GitHub personal access token (PAT) stored in
/// `GITHUB_TOKEN` env var or passed via secrets.  Calls the GitHub
/// REST API through `reqwest`.
#[derive(Debug, Clone)]
pub struct GitHubConnector {
    /// Owner/repo slug (e.g. "ryanloee/VaultPilot").
    repo: String,
    /// Default GitHub API base (https://api.github.com).
    api_base: String,
    /// Maximum items per page for list/search endpoints.
    per_page: usize,
}

impl GitHubConnector {
    /// Create a new GitHub connector for a repository.
    pub fn new(repo: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            api_base: "https://api.github.com".into(),
            per_page: 30,
        }
    }

    /// Set a custom API base (e.g. for GitHub Enterprise Server).
    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into();
        self
    }

    /// Set the max items per page for list / search endpoints.
    pub fn with_per_page(mut self, n: usize) -> Self {
        self.per_page = n;
        self
    }

    /// Extract a GitHub token from secrets, falling back to the
    /// `GITHUB_TOKEN` environment variable.
    fn get_token(&self, secrets: &HashMap<String, String>) -> Option<String> {
        secrets
            .get("GITHUB_TOKEN")
            .cloned()
            .or_else(|| std::env::var("GITHUB_TOKEN").ok())
    }

    /// Make an authenticated GET request to the GitHub REST API.
    fn gh_get(&self, path: &str, token: &str) -> Result<Value> {
        let url = format!("{}{}", self.api_base, path);
        let token = token.to_string();
        let path = path.to_string();

        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            anyhow::anyhow!("no tokio runtime — GitHubConnector requires async context")
        })?;

        handle.block_on(async { gh_get_async(url, token, path).await })
    }

    /// Fetch issues from the configured repository.
    fn list_issues(
        &self,
        state: &str,
        labels: Option<&str>,
        token: &str,
        page: usize,
    ) -> Result<Value> {
        let path = format!(
            "/repos/{}/issues?state={}&per_page={}&page={}",
            self.repo, state, self.per_page, page
        );
        let final_path = if let Some(labels_str) = labels {
            format!("{path}&labels={labels_str}")
        } else {
            path
        };
        self.gh_get(&final_path, token)
    }

    /// Fetch a single issue (or PR) by number.
    fn get_issue(&self, number: u64, token: &str) -> Result<Value> {
        let path = format!("/repos/{}/issues/{number}", self.repo);
        self.gh_get(&path, token)
    }

    /// Fetch current user notifications.
    fn list_notifications(&self, token: &str, page: usize) -> Result<Value> {
        let path = format!("/notifications?per_page={}&page={}", self.per_page, page);
        self.gh_get(&path, token)
    }

    /// Search GitHub issues globally by keywords.
    fn search_issues(&self, query: &str, token: &str) -> Result<Value> {
        let encoded = urlencoding_hack(query);
        let path = format!("/search/issues?q={encoded}&per_page={}", self.per_page);
        self.gh_get(&path, token)
    }
}

impl Connector for GitHubConnector {
    fn name(&self) -> &str {
        &self.repo
    }

    fn auth_flow(&self) -> AuthFlow {
        AuthFlow::ApiKey {
            key_name: "GITHUB_TOKEN".into(),
            source: TokenSource::Env,
        }
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![
            Capability {
                name: "list_issues".into(),
                description: format!(
                    "List open/closed issues in repository {} with optional label filter",
                    self.repo
                ),
                access: AccessLevel::Read,
            },
            Capability {
                name: "get_issue".into(),
                description: format!(
                    "Fetch a single issue or pull request by number from {}",
                    self.repo
                ),
                access: AccessLevel::Read,
            },
            Capability {
                name: "list_notifications".into(),
                description: "Read current GitHub notifications for the authenticated user".into(),
                access: AccessLevel::Read,
            },
            Capability {
                name: "search_issues".into(),
                description: "Search GitHub issues globally by keywords".into(),
                access: AccessLevel::Search,
            },
        ]
    }

    fn execute(
        &self,
        action: &Action,
        secrets: &HashMap<String, String>,
    ) -> Result<ConnectorResponse> {
        let token = self
            .get_token(secrets)
            .ok_or_else(|| anyhow::anyhow!("GITHUB_TOKEN not set"))?;

        match action.name.as_str() {
            "list_issues" => {
                let state = action
                    .payload
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("open");
                let labels = action.payload.get("labels").and_then(Value::as_str);
                let page = action
                    .payload
                    .get("page")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;

                let data = self.list_issues(state, labels, &token, page)?;
                let count = data.as_array().map(|a| a.len()).unwrap_or(0);
                Ok(ConnectorResponse {
                    success: true,
                    summary: format!(
                        "listed {count} {state} issues in {} (page {page})",
                        self.repo
                    ),
                    data: Some(data),
                })
            }
            "get_issue" => {
                let number = action
                    .payload
                    .get("number")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("'get_issue' requires 'number' parameter"))?;

                let data = self.get_issue(number, &token)?;
                let title = data
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("(no title)");
                let gh_state = data
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");

                Ok(ConnectorResponse {
                    success: true,
                    summary: format!("issue #{number} in {}: {title} [{gh_state}]", self.repo),
                    data: Some(data),
                })
            }
            "list_notifications" => {
                let page = action
                    .payload
                    .get("page")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;

                let data = self.list_notifications(&token, page)?;
                let count = data.as_array().map(|a| a.len()).unwrap_or(0);
                Ok(ConnectorResponse {
                    success: true,
                    summary: format!("listed {count} notifications (page {page})"),
                    data: Some(data),
                })
            }
            "search_issues" => {
                let query = action
                    .payload
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("'search_issues' requires 'query' parameter"))?;

                let data = self.search_issues(query, &token)?;
                let total = data
                    .get("total_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                Ok(ConnectorResponse {
                    success: true,
                    summary: format!("found {total} issues matching \"{query}\""),
                    data: Some(data),
                })
            }
            other => Err(anyhow::anyhow!(
                "GitHub connector '{}': unknown action '{}'",
                self.repo,
                other
            )),
        }
    }
}

/// Perform a single GitHub REST API GET call on the tokio runtime.
async fn gh_get_async(url: String, token: String, path: String) -> Result<Value> {
    use anyhow::Context;

    let response = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "VaultPilot-GitHubConnector/1.0")
        .send()
        .await
        .with_context(|| format!("GitHub API request failed: GET {url}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "GitHub API returned {status} for {path}: {body}"
        ));
    }

    response
        .json::<Value>()
        .await
        .with_context(|| format!("failed to parse GitHub API response for {path}"))
}

/// Minimal percent-encoding for GitHub search query parameters.
fn urlencoding_hack(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b' ' => out.push_str("%20"),
            b'"' => out.push_str("%22"),
            b'#' => out.push_str("%23"),
            b'%' => out.push_str("%25"),
            b'&' => out.push_str("%26"),
            _ => {
                if b.is_ascii_graphic() && *b != b'\\' {
                    out.push(*b as char);
                } else {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
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

    // ── GitHubConnector tests (unit, no network) ──

    #[test]
    fn github_connector_basics() {
        let gh = GitHubConnector::new("test-user/test-repo");
        assert_eq!(gh.name(), "test-user/test-repo");
        let flow = gh.auth_flow();
        match flow {
            AuthFlow::ApiKey {
                ref key_name,
                ref source,
            } => {
                assert_eq!(key_name, "GITHUB_TOKEN");
                assert_eq!(*source, TokenSource::Env);
            }
            _ => panic!("expected ApiKey, got {flow:?}"),
        }
        let caps = gh.capabilities();
        assert_eq!(caps.len(), 4);
        assert!(caps.iter().any(|c| c.name == "list_issues"));
        assert!(caps.iter().any(|c| c.name == "get_issue"));
        assert!(caps.iter().any(|c| c.name == "list_notifications"));
        assert!(caps.iter().any(|c| c.name == "search_issues"));
    }

    #[test]
    fn github_connector_missing_token() {
        let gh = GitHubConnector::new("test-user/test-repo");
        let action = Action {
            name: "list_issues".into(),
            payload: serde_json::json!({}),
        };
        let secrets = HashMap::new();
        std::env::remove_var("GITHUB_TOKEN");
        let err = gh.execute(&action, &secrets).unwrap_err();
        assert!(err.to_string().contains("GITHUB_TOKEN"));
    }

    #[test]
    fn github_get_issue_missing_number() {
        let mut secrets = HashMap::new();
        secrets.insert("GITHUB_TOKEN".into(), "fake-token".into());
        let gh = GitHubConnector::new("test-user/test-repo");
        let action = Action {
            name: "get_issue".into(),
            payload: serde_json::json!({}),
        };
        let err = gh.execute(&action, &secrets).unwrap_err();
        assert!(err.to_string().contains("'number'"));
    }

    #[test]
    fn github_search_issues_missing_query() {
        let mut secrets = HashMap::new();
        secrets.insert("GITHUB_TOKEN".into(), "fake-token".into());
        let gh = GitHubConnector::new("test-user/test-repo");
        let action = Action {
            name: "search_issues".into(),
            payload: serde_json::json!({}),
        };
        let err = gh.execute(&action, &secrets).unwrap_err();
        assert!(err.to_string().contains("'query'"));
    }

    // ── urlencoding_hack tests ──

    #[test]
    fn urlencoding_hack_basic() {
        assert_eq!(urlencoding_hack("hello world"), "hello%20world");
        assert_eq!(urlencoding_hack("foo&bar"), "foo%26bar");
        assert_eq!(urlencoding_hack("is:issue"), "is:issue");
        assert_eq!(urlencoding_hack(""), "");
    }

    #[test]
    fn urlencoding_hack_preserves_ascii() {
        let input = "abcXYZ123-._~:+/@!$'()*,;=?";
        assert_eq!(urlencoding_hack(input), input);
    }
}
