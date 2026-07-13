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

// ── Shared execution helpers ──────────────────────────────────

/// Drive an async connector I/O future to completion from a *synchronous*
/// [`Connector::execute`] body without ever panicking.
///
/// # Background (issue #2791)
/// The original code did `tokio::runtime::Runtime::new()?.block_on(fut)` on the
/// caller's thread. That panics with *"Cannot block the current thread from
/// within a runtime"* whenever `execute` is invoked from a task already running
/// on a Tokio runtime (the MCP bridge / agent event loop).
///
/// # Fix
/// We always run the future on a **brand-new OS thread** that owns a **fresh,
/// private Tokio runtime**. A thread that is not part of any existing runtime
/// can freely `block_on` its own runtime, so the panic can never occur — no
/// matter whether the caller is sync or async.
/// Maximum wall-clock time a single connector I/O dispatch may take before it
/// is aborted and reported as an error (issue #2793). Without this bound an
/// unresponsive external service would block the private I/O thread forever,
/// permanently hanging the calling agent.
const CONNECTOR_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Drive an async connector I/O future to completion on a fresh private thread
/// + runtime, aborting if it exceeds [`CONNECTOR_IO_TIMEOUT`] (issue #2793).
fn run_connector_io_owned<F, Fut, T>(make_future: F) -> Result<T>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    run_connector_io_owned_timeout(make_future, CONNECTOR_IO_TIMEOUT)
}

/// Like [`run_connector_io_owned`] but with an explicit timeout — used by
/// tests to exercise the hang guard without waiting the full 30s.
///
/// The I/O future runs on a dedicated OS thread owning a private runtime
/// (issue #2791). Its result is sent over a channel and the caller waits on
/// that channel with [`std::sync::mpsc::Receiver::recv_timeout`] — so a
/// hung external service can never block the caller forever (issue #2793).
/// The per-request reqwest client timeout normally makes the future complete
/// well within `timeout`; this channel backstop covers any residual hang.
fn run_connector_io_owned_timeout<F, Fut, T>(
    make_future: F,
    timeout: std::time::Duration,
) -> Result<T>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<Result<T>>();
    let worker = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| anyhow::anyhow!("connector: failed to build runtime: {e}"))?;
        let result = rt.block_on(make_future());
        let _ = tx.send(result);
        Ok::<(), anyhow::Error>(())
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(anyhow::anyhow!(
            "connector: I/O timed out after {}s",
            timeout.as_secs()
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let _ = worker.join();
            Err(anyhow::anyhow!("connector: I/O thread panicked"))
        }
    }
}

/// Canonical body used for HMAC signing: the payload with the `_signature`
/// field removed (a signature must never cover itself). Returns a stable JSON
/// serialization so sender and receiver compute identical signatures.
fn webhook_signing_body(payload: &Value) -> Result<String> {
    let mut body = payload.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.remove("_signature");
    }
    serde_json::to_string(&body).map_err(Into::into)
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



    /// Compute the HMAC-SHA256 hex signature for `payload` using the on-disk
    /// shared secret. Inverse of [`WebhookConnector::verify_signature`].
    fn compute_signature(&self, payload: &str) -> Result<String> {
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
        Ok(tag.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Verify an HMAC-SHA256 signature (constant-time).
    fn verify_signature(&self, payload: &str, signature: &str) -> Result<bool> {
        let expected = self.compute_signature(payload)?;
        // Constant-time comparison
        use subtle::ConstantTimeEq;
        Ok(expected.as_bytes().ct_eq(signature.as_bytes()).into())
    }

    /// Build the canonical signing string for an incoming `webhook_receive`
    /// action (issue #2794).
    ///
    /// When the caller supplies the original raw request body under the
    /// `_raw_body` field, we verify against those **exact bytes** — because
    /// that is what the external sender (Slack, GitHub, …) signed. Re-serializing
    /// a parsed JSON `Value` reorders keys / alters whitespace / re-escapes
    /// unicode, so an HMAC computed over the raw bytes would never match the
    /// re-serialized form, rejecting every legitimately-signed webhook. The
    /// `_raw_body` path fixes that. When `_raw_body` is absent we fall back to
    /// the legacy canonical re-serialization for backward compatibility.
    fn signing_body(&self, payload: &Value) -> Result<String> {
        if let Some(raw) = payload.get("_raw_body").and_then(Value::as_str) {
            return Ok(raw.to_string());
        }
        webhook_signing_body(payload)
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

    fn validate(&self, _secrets: &HashMap<String, String>) -> Result<()> {
        // Webhook auth is file-based (HMAC shared secret on disk), not the
        // `ApiKey`-in-`secrets` model the trait default assumes. The default
        // implementation would require a `{id}_webhook_secret` key that is
        // never populated for file-based auth, so validate could never pass
        // (issue #2790). Here we validate against the on-disk secret instead.
        match self.load_secret() {
            Ok(s) if !s.is_empty() => Ok(()),
            Ok(_) => Err(anyhow::anyhow!(
                "webhook '{}': secret file '{}' is empty",
                self.id,
                self.secret_path
            )),
            Err(e) => Err(anyhow::anyhow!("webhook '{}' not ready: {e}", self.id)),
        }
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

                // When a shared secret is configured, the HMAC signature is
                // MANDATORY and must match (see #2789). Without a configured
                // secret the webhook is open (unsigned payloads accepted).
                // Verify against the raw body when available (issue #2794);
                // otherwise fall back to the canonical re-serialization.
                let signed_str = self.signing_body(&action.payload)?;
                if self.secret_configured() {
                    match action.payload.get("_signature").and_then(Value::as_str) {
                        Some(sig) => {
                            if !self.verify_signature(&signed_str, sig)? {
                                return Ok(ConnectorResponse {
                                    success: false,
                                    summary: "HMAC signature verification failed".into(),
                                    data: None,
                                });
                            }
                        }
                        None => {
                            return Ok(ConnectorResponse {
                                success: false,
                                summary: "HMAC signature required but missing".into(),
                                data: None,
                            });
                        }
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
        // The trait method is synchronous. Drive the async HTTP call on a
        // private thread+runtime so it never panics when called from an async
        // context (issue #2791).
        run_connector_io_owned(move || github_dispatch(req, token))
    }
}

/// Perform the actual GitHub REST API request (async). Owned params so it can
/// be driven on a private runtime without borrowing `&self` (issue #2791).
async fn github_dispatch(req: GitHubRequest, token: String) -> Result<ConnectorResponse> {
    let client = reqwest::Client::builder()
        .timeout(CONNECTOR_IO_TIMEOUT)
        .build()
        .map_err(|e| anyhow::anyhow!("connector: failed to build http client: {e}"))?;
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

// ── Slack Connector (Phase 2) ──────────────────────────────

/// A summarized Slack message parsed from a `conversations.history` response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackMessage {
    /// Message timestamp (unique ID within a channel), e.g. "1699000000.000100".
    pub ts: String,
    /// Author user ID (e.g. "U123") or bot id when posted by a bot.
    pub user: String,
    /// Message text (may be empty for e.g. file uploads).
    pub text: String,
    /// Parent thread timestamp if this message belongs to a thread.
    pub thread_ts: Option<String>,
}

/// A fully-described outbound HTTP request built by the Slack connector.
///
/// Pure data so it can be inspected in tests (dry-run) and dispatched by an
/// async executor in production without the trait method itself being async.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackRequest {
    /// HTTP method (GET/POST/...).
    pub method: String,
    /// Fully-qualified request URL.
    pub url: String,
    /// `Accept` header value.
    pub accept: String,
    /// Optional JSON request body (for writes).
    pub body: Option<String>,
}

/// Slack connector — reads channel history and posts messages via the
/// Slack Web API.
///
/// Phase 2 of #1841. Implements the [`Connector`] trait on top of the
/// existing foundation, mirroring [`GitHubConnector`]. Authentication uses a
/// bot/user token (xoxb-/xoxp-) supplied through `secrets` under the key
/// `slack_token`.
///
/// All request construction ([`SlackConnector::build_request`]) and parsing
/// ([`SlackConnector::parse_message`]) is pure and unit-testable without
/// network access. In `dry_run` mode [`Connector::execute`] returns the
/// constructed request as `data` instead of performing the HTTP call.
#[derive(Debug, Clone)]
pub struct SlackConnector {
    /// Connector identifier (e.g. "slack-vaultpilot").
    id: String,
    /// Target channel ID (e.g. "C0123ABCD").
    channel: String,
    /// When `true`, `execute` returns the built request instead of sending it.
    dry_run: bool,
}

impl SlackConnector {
    /// Create a new Slack connector for a given channel.
    pub fn new(id: impl Into<String>, channel: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            channel: channel.into(),
            dry_run: false,
        }
    }

    /// Enable dry-run mode (offline request construction, no network).
    pub fn with_dry_run(mut self, dry: bool) -> Self {
        self.dry_run = dry;
        self
    }

    /// Build the Web API URL for a given method (e.g. `chat.postMessage`).
    pub fn api_url(&self, method: &str) -> String {
        format!("https://slack.com/api/{}", method.trim_start_matches('/'))
    }

    /// Parse a single Slack message object into a [`SlackMessage`] (pure, testable).
    ///
    /// Falls back to `bot_id` when `user` is absent (bot-posted messages), and
    /// to `"unknown"` if neither is present, so history parsing never panics.
    pub fn parse_message(&self, msg: &Value) -> Result<SlackMessage> {
        let ts = msg
            .get("ts")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("slack message missing 'ts'"))?
            .to_string();
        let user = msg
            .get("user")
            .or_else(|| msg.get("bot_id"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let text = msg
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let thread_ts = msg
            .get("thread_ts")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(SlackMessage {
            ts,
            user,
            text,
            thread_ts,
        })
    }

    /// Build the outbound request for a given action (pure, testable).
    ///
    /// Returns an error if the required `slack_token` secret is missing or the
    /// action payload is malformed.
    pub fn build_request(
        &self,
        action: &Action,
        secrets: &HashMap<String, String>,
    ) -> Result<SlackRequest> {
        let _token = secrets.get("slack_token").ok_or_else(|| {
            anyhow::anyhow!(
                "slack connector '{}' requires 'slack_token' in secrets",
                self.id
            )
        })?;

        match action.name.as_str() {
            "list_messages" => Ok(SlackRequest {
                method: "GET".into(),
                url: self.api_url(&format!(
                    "conversations.history?channel={}&limit=50",
                    self.channel
                )),
                accept: "application/json".into(),
                body: None,
            }),
            "post_message" => {
                let text = action
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("post_message requires 'text' (string)"))?
                    .to_string();
                let payload = serde_json::json!({
                    "channel": self.channel,
                    "text": text,
                });
                Ok(SlackRequest {
                    method: "POST".into(),
                    url: self.api_url("chat.postMessage"),
                    accept: "application/json".into(),
                    body: Some(serde_json::to_string(&payload)?),
                })
            }
            other => Err(anyhow::anyhow!(
                "slack connector '{}': unknown action '{}'",
                self.id,
                other
            )),
        }
    }

    /// Register this connector as an MCP server capability in the
    /// [`CapabilityRegistry`], mirroring [`register_as_mcp`]. The `http_url`
    /// is the local endpoint the connector's bridge listens on.
    pub fn register(
        &self,
        registry: &mut crate::capability_registry::CapabilityRegistry,
        http_url: &str,
    ) {
        let description = format!(
            "Read/post messages in Slack channel {} via Slack Web API",
            self.channel
        );
        register_as_mcp(
            registry,
            self.name(),
            "Slack Connector",
            &description,
            http_url,
        );
    }
}

impl Connector for SlackConnector {
    fn name(&self) -> &str {
        &self.id
    }

    fn auth_flow(&self) -> AuthFlow {
        AuthFlow::ApiKey {
            key_name: "slack_token".into(),
            source: TokenSource::Config,
        }
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![
            Capability {
                name: "list_messages".into(),
                description: format!("List recent messages in Slack channel {}", self.channel),
                access: AccessLevel::Read,
            },
            Capability {
                name: "post_message".into(),
                description: format!("Post a message to Slack channel {}", self.channel),
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
        let token = secrets.get("slack_token").cloned().unwrap_or_default();
        // The trait method is synchronous. Drive the async HTTP call on a
        // private thread+runtime so it never panics when called from an async
        // context (issue #2791).
        run_connector_io_owned(move || slack_dispatch(req, token))
    }
}

/// Map a Slack Web API HTTP response to a [`ConnectorResponse`].
///
/// Slack returns **HTTP 200** even for API-level failures via the
/// `{"ok": false, "error": "..."}` envelope, so a 2xx status alone is NOT
/// success. We inspect the `ok` field and surface `error` (issue #2788).
/// Pure + unit-testable (no network).
fn slack_response_to_connector_response(status: u16, body: &str) -> ConnectorResponse {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let ok = parsed
        .as_ref()
        .and_then(|v| v.get("ok").and_then(Value::as_bool))
        .unwrap_or_else(|| (200..300).contains(&status));
    let error = if ok {
        None
    } else {
        parsed
            .as_ref()
            .and_then(|v| v.get("error").and_then(Value::as_str))
            .map(str::to_string)
    };
    let success = ok && (200..300).contains(&status);
    ConnectorResponse {
        success,
        summary: format!("slack -> HTTP {} (ok={}, error={:?})", status, ok, error),
        data: Some(serde_json::json!({
            "status": status,
            "ok": ok,
            "error": error,
            "body": body
        })),
    }
}

/// Perform the actual Slack Web API request (async). Owned params so it can
/// be driven on a private runtime without borrowing `&self` (issue #2791).
async fn slack_dispatch(req: SlackRequest, token: String) -> Result<ConnectorResponse> {
    let client = reqwest::Client::builder()
        .timeout(CONNECTOR_IO_TIMEOUT)
        .build()
        .map_err(|e| anyhow::anyhow!("connector: failed to build http client: {e}"))?;
    let method: reqwest::Method = req
        .method
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid HTTP method '{}': {e}", req.method))?;
    let mut builder = client
        .request(method, &req.url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", &req.accept)
        .header("User-Agent", "VaultPilot-Connector");
    if let Some(b) = &req.body {
        builder = builder
            .header("Content-Type", "application/json")
            .body(b.clone());
    }
    let resp = builder.send().await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let mut response = slack_response_to_connector_response(status.as_u16(), &text);
    response.summary = format!("slack {} {} -> HTTP {}", req.method, req.url, status);
    Ok(response)
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
        // Webhook auth is file-based: validate checks the secret file on disk,
        // not `secrets`. A missing secret file must fail validation (#2790).
        let wh = WebhookConnector::new("test-wh", "Test", "/tmp/vp_wh_missing_secret_2790");
        let secrets = HashMap::new();
        let result = wh.validate(&secrets);
        assert!(result.is_err());
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

    // ── SlackConnector tests (Phase 2, #1841) ──

    #[test]
    fn slack_connector_basics() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        assert_eq!(sl.name(), "slack-vp");
        let caps = sl.capabilities();
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].name, "list_messages");
        assert_eq!(caps[0].access, AccessLevel::Read);
        assert_eq!(caps[1].name, "post_message");
        assert_eq!(caps[1].access, AccessLevel::Write);
    }

    #[test]
    fn slack_auth_flow_uses_token() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        match sl.auth_flow() {
            AuthFlow::ApiKey { key_name, source } => {
                assert_eq!(key_name, "slack_token");
                assert_eq!(source, TokenSource::Config);
            }
            other => panic!("expected ApiKey auth flow, got {other:?}"),
        }
        // Without the token, validation must fail.
        let secrets = HashMap::new();
        assert!(sl.validate(&secrets).is_err());
        let mut with_token = HashMap::new();
        with_token.insert("slack_token".to_string(), "xoxb-xxx".to_string());
        assert!(sl.validate(&with_token).is_ok());
    }

    #[test]
    fn slack_api_url() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        assert_eq!(
            sl.api_url("chat.postMessage"),
            "https://slack.com/api/chat.postMessage"
        );
        // Leading slash is tolerated.
        assert_eq!(
            sl.api_url("/conversations.history"),
            "https://slack.com/api/conversations.history"
        );
    }

    #[test]
    fn slack_parse_message() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let json = serde_json::json!({
            "ts": "1699000000.000100",
            "user": "U123",
            "text": "hello world",
            "thread_ts": "1699000000.000000"
        });
        let msg = sl.parse_message(&json).unwrap();
        assert_eq!(
            msg,
            SlackMessage {
                ts: "1699000000.000100".into(),
                user: "U123".into(),
                text: "hello world".into(),
                thread_ts: Some("1699000000.000000".into()),
            }
        );
    }

    #[test]
    fn slack_parse_message_bot_fallback() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        // Bot-posted messages lack `user` but carry `bot_id`.
        let json = serde_json::json!({
            "ts": "1699000000.000200",
            "bot_id": "B456",
            "text": "bot says hi"
        });
        let msg = sl.parse_message(&json).unwrap();
        assert_eq!(msg.user, "B456");
        assert_eq!(msg.thread_ts, None);
    }

    #[test]
    fn slack_parse_message_missing_ts() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let json = serde_json::json!({ "user": "U123", "text": "no ts" });
        assert!(sl.parse_message(&json).is_err());
    }

    #[test]
    fn slack_build_request_missing_token() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let action = Action {
            name: "list_messages".into(),
            payload: serde_json::json!({}),
        };
        let secrets = HashMap::new();
        assert!(sl.build_request(&action, &secrets).is_err());
    }

    #[test]
    fn slack_build_request_list_messages() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let mut secrets = HashMap::new();
        secrets.insert("slack_token".to_string(), "xoxb-xxx".to_string());
        let action = Action {
            name: "list_messages".into(),
            payload: serde_json::json!({}),
        };
        let req = sl.build_request(&action, &secrets).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(
            req.url,
            "https://slack.com/api/conversations.history?channel=C0123ABCD&limit=50"
        );
        assert!(req.body.is_none());
    }

    #[test]
    fn slack_build_request_post_message() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let mut secrets = HashMap::new();
        secrets.insert("slack_token".to_string(), "xoxb-xxx".to_string());
        let action = Action {
            name: "post_message".into(),
            payload: serde_json::json!({ "text": "ping the channel" }),
        };
        let req = sl.build_request(&action, &secrets).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "https://slack.com/api/chat.postMessage");
        let parsed: Value = serde_json::from_str(req.body.as_deref().unwrap()).unwrap();
        assert_eq!(parsed["channel"], "C0123ABCD");
        assert_eq!(parsed["text"], "ping the channel");
    }

    #[test]
    fn slack_build_request_post_missing_text() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let mut secrets = HashMap::new();
        secrets.insert("slack_token".to_string(), "xoxb-xxx".to_string());
        let action = Action {
            name: "post_message".into(),
            payload: serde_json::json!({}),
        };
        assert!(sl.build_request(&action, &secrets).is_err());
    }

    #[test]
    fn slack_build_request_unknown_action() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let mut secrets = HashMap::new();
        secrets.insert("slack_token".to_string(), "xoxb-xxx".to_string());
        let action = Action {
            name: "delete_everything".into(),
            payload: serde_json::json!({}),
        };
        assert!(sl.build_request(&action, &secrets).is_err());
    }

    #[test]
    fn slack_dry_run_execute_returns_request() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD").with_dry_run(true);
        let mut secrets = HashMap::new();
        secrets.insert("slack_token".to_string(), "xoxb-xxx".to_string());
        let action = Action {
            name: "list_messages".into(),
            payload: serde_json::json!({}),
        };
        let resp = sl.execute(&action, &secrets).unwrap();
        assert!(resp.success);
        assert!(resp.summary.contains("dry-run"));
        assert_eq!(resp.data.as_ref().unwrap()["method"], "GET");
        assert_eq!(
            resp.data.as_ref().unwrap()["url"],
            "https://slack.com/api/conversations.history?channel=C0123ABCD&limit=50"
        );
    }

    #[test]
    fn slack_execute_missing_token_errors() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let secrets = HashMap::new();
        let action = Action {
            name: "list_messages".into(),
            payload: serde_json::json!({}),
        };
        // No dry-run and no token -> build_request fails before any network.
        assert!(sl.execute(&action, &secrets).is_err());
    }

    #[test]
    fn slack_register_creates_mcp_capability() {
        let sl = SlackConnector::new("slack-vp", "C0123ABCD");
        let mut registry = crate::capability_registry::CapabilityRegistry::default();
        sl.register(&mut registry, "http://localhost:8092");
        assert!(registry.capabilities.contains_key("slack-vp"));
        match &registry.capabilities["slack-vp"] {
            crate::capability_registry::Capability::McpServer {
                name, transport, ..
            } => {
                assert_eq!(name, "Slack Connector");
                assert!(matches!(
                    transport,
                    crate::capability_registry::McpTransport::Http { .. }
                ));
            }
            _ => panic!("expected McpServer capability"),
        }
    }

    // ── Regression: #2788 — Slack reports success on API errors ──

    #[test]
    fn slack_response_ok_false_is_failure() {
        // Slack returns HTTP 200 with {"ok":false} for API failures; that must
        // be surfaced as a failure, not success.
        let resp =
            slack_response_to_connector_response(200, r#"{"ok":false,"error":"invalid_auth"}"#);
        assert!(!resp.success, "ok:false must not be reported as success");
        assert_eq!(resp.data.as_ref().unwrap()["ok"], false);
        assert_eq!(resp.data.as_ref().unwrap()["error"], "invalid_auth");
    }

    #[test]
    fn slack_response_ok_true_is_success() {
        let resp = slack_response_to_connector_response(200, r#"{"ok":true,"ts":"123.45"}"#);
        assert!(resp.success);
        assert_eq!(resp.data.as_ref().unwrap()["ok"], true);
    }

    #[test]
    fn slack_response_http_error_is_failure() {
        let resp =
            slack_response_to_connector_response(429, r#"{"ok":false,"error":"rate_limited"}"#);
        assert!(!resp.success);
        assert_eq!(resp.data.as_ref().unwrap()["error"], "rate_limited");
    }

    #[test]
    fn slack_response_non_json_falls_back_to_http_status() {
        // Unparseable body: fall back to HTTP status semantics.
        let ok = slack_response_to_connector_response(200, "not json");
        assert!(ok.success);
        let bad = slack_response_to_connector_response(500, "not json");
        assert!(!bad.success);
    }

    // ── Regression: #2789 — webhook must reject unsigned payloads ──

    #[test]
    fn webhook_rejects_unsigned_when_secret_configured() {
        let path = std::env::temp_dir().join("vp_wh_secret_test_2789a").to_string_lossy().into_owned();
        std::fs::write(&path, "super-secret-hmac-key").unwrap();
        let wh = WebhookConnector::new("wh-sec", "Sec", path.as_str());
        let action = Action {
            name: "webhook_receive".into(),
            payload: serde_json::json!({"text": "hi"}),
        };
        let resp = wh.execute(&action, &HashMap::new()).unwrap();
        assert!(
            !resp.success,
            "unsigned payload must be rejected when secret configured"
        );
        assert!(
            resp.summary.contains("required") || resp.summary.contains("signature"),
            "unexpected summary: {}",
            resp.summary
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn webhook_accepts_valid_signature() {
        let path = std::env::temp_dir().join("vp_wh_secret_test_2789b").to_string_lossy().into_owned();
        std::fs::write(&path, "super-secret-hmac-key").unwrap();
        let wh = WebhookConnector::new("wh-sec", "Sec", path.as_str());
        let payload = serde_json::json!({"text": "hi"});
        let signed_str = webhook_signing_body(&payload).unwrap();
        let sig = wh.compute_signature(&signed_str).unwrap();
        let mut full = payload.clone();
        full["_signature"] = serde_json::json!(sig);
        let action = Action {
            name: "webhook_receive".into(),
            payload: full,
        };
        let resp = wh.execute(&action, &HashMap::new()).unwrap();
        assert!(resp.success, "valid signature must be accepted");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn webhook_rejects_bad_signature() {
        let path = std::env::temp_dir().join("vp_wh_secret_test_2789c").to_string_lossy().into_owned();
        std::fs::write(&path, "super-secret-hmac-key").unwrap();
        let wh = WebhookConnector::new("wh-sec", "Sec", path.as_str());
        let action = Action {
            name: "webhook_receive".into(),
            payload: serde_json::json!({"text": "hi", "_signature": "deadbeef"}),
        };
        let resp = wh.execute(&action, &HashMap::new()).unwrap();
        assert!(!resp.success, "tampered signature must be rejected");
        let _ = std::fs::remove_file(&path);
    }

    // ── Regression: #2794 — HMAC must verify the RAW body, not re-serialized JSON ──

    #[test]
    fn webhook_verifies_raw_body_not_reserialized() {
        // An external sender (Slack/GitHub) signs the EXACT raw request bytes
        // it sent — which may have a non-canonical key order. Re-serializing a
        // parsed Value reorders keys, so the signature would never match and
        // every legitimately-signed webhook would be rejected (#2794).
        let path = std::env::temp_dir().join("vp_wh_secret_test_2794").to_string_lossy().into_owned();
        std::fs::write(&path, "raw-body-secret").unwrap();
        let wh = WebhookConnector::new("wh-raw", "Raw", path.as_str());

        // Raw body with non-canonical ordering (b before a).
        let raw = r#"{"b":2,"a":1,"text":"hi"}"#;
        let sig = wh.compute_signature(raw).unwrap();

        // Caller passes the exact raw bytes under `_raw_body` + the signature.
        let mut payload = serde_json::json!({});
        payload["_raw_body"] = serde_json::Value::String(raw.to_string());
        payload["_signature"] = serde_json::Value::String(sig.clone());
        let action = Action {
            name: "webhook_receive".into(),
            payload,
        };
        let resp = wh.execute(&action, &HashMap::new()).unwrap();
        assert!(
            resp.success,
            "raw-body signature must verify (got: {})",
            resp.summary
        );

        // Sanity: without `_raw_body` we fall back to canonical re-serialization,
        // which must NOT match the raw signature (this is the old bug).
        let mut canonical = serde_json::json!({"b":2,"a":1,"text":"hi"});
        canonical["_signature"] = serde_json::Value::String(sig.clone());
        let action2 = Action {
            name: "webhook_receive".into(),
            payload: canonical,
        };
        let resp2 = wh.execute(&action2, &HashMap::new()).unwrap();
        assert!(
            !resp2.success,
            "canonical re-serialization must NOT match the raw signature"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn webhook_open_when_no_secret() {
        // No secret file configured -> unsigned payloads are accepted (open).
        let wh = WebhookConnector::new("wh-open", "Open", "/tmp/vp_wh_missing_2789");
        let action = Action {
            name: "webhook_receive".into(),
            payload: serde_json::json!({"text": "hi"}),
        };
        let resp = wh.execute(&action, &HashMap::new()).unwrap();
        assert!(
            resp.success,
            "open webhook (no secret) should accept unsigned"
        );
    }

    // ── Regression: #2790 — WebhookConnector::validate file-based auth ──

    #[test]
    fn webhook_validate_requires_secret_file() {
        // Missing secret file -> validate fails (file-based auth).
        let wh = WebhookConnector::new("wh-v", "V", "/tmp/vp_wh_nonexistent_2790");
        assert!(wh.validate(&HashMap::new()).is_err());
        // Present secret file -> validate passes.
        let path = std::env::temp_dir().join("vp_wh_secret_2790").to_string_lossy().into_owned();
        std::fs::write(&path, "hunter2").unwrap();
        let wh2 = WebhookConnector::new("wh-v", "V", path.as_str());
        assert!(wh2.validate(&HashMap::new()).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn webhook_validate_empty_secret_fails() {
        let path = std::env::temp_dir().join("vp_wh_empty_2790").to_string_lossy().into_owned();
        std::fs::write(&path, "").unwrap();
        let wh = WebhookConnector::new("wh-v", "V", path.as_str());
        assert!(wh.validate(&HashMap::new()).is_err());
        let _ = std::fs::remove_file(&path);
    }

    // ── Regression: #2791 — execute must not panic inside a runtime ──

    #[test]
    fn run_connector_io_owned_works_sync() {
        let v: u32 = run_connector_io_owned(|| async { Ok(7u32) }).unwrap();
        assert_eq!(v, 7);
    }

    #[test]
    fn run_connector_io_owned_times_out_instead_of_hanging() {
        // Regression for #2793: a future that would hang forever (blocks far
        // longer than the timeout) must return an error quickly rather than
        // block the caller indefinitely.
        let start = std::time::Instant::now();
        let res: Result<()> = run_connector_io_owned_timeout(
            || async {
                // Block the worker thread (not a timer — avoids needing a
                // Tokio time driver inside the private runtime).
                std::thread::sleep(std::time::Duration::from_secs(10));
                Ok(())
            },
            std::time::Duration::from_millis(50),
        );
        assert!(res.is_err(), "hung I/O must be aborted with an error");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "I/O timeout must not block the caller for long"
        );
    }

    #[test]
    fn run_connector_io_owned_works_in_async_context() {
        // The live caller (MCP bridge / agent loop) runs on a Tokio runtime.
        // The old Runtime::new().block_on() panicked here; the new helper must
        // not — simulate a current runtime with a manual one.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .expect("build runtime");
        let v: u32 = rt.block_on(async { run_connector_io_owned(|| async { Ok(42u32) }).unwrap() });
        assert_eq!(v, 42);
    }

    #[test]
    fn slack_execute_dry_run_in_async_context() {
        // execute() now funnels through run_connector_io_owned even in async.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .expect("build runtime");
        let sl = SlackConnector::new("slack-vp", "C0123ABCD").with_dry_run(true);
        let mut secrets = HashMap::new();
        secrets.insert("slack_token".to_string(), "xoxb-test".to_string());
        let action = Action {
            name: "post_message".into(),
            payload: serde_json::json!({"text": "hi"}),
        };
        let resp = rt.block_on(async { sl.execute(&action, &secrets).unwrap() });
        assert!(resp.success);
    }
}
