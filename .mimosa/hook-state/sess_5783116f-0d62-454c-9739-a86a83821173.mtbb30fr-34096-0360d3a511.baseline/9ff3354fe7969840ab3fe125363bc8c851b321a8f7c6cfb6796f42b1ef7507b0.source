//! MCP Client Runtime — connects to external MCP servers for tool discovery
//! and invocation (#3254 — External AI Agent integration).
//!
//! This module provides the runtime counterpart to [`super::mcp_client`]
//! (which defines config structs). The runtime handles:
//!
//! - **Connection**: spawns a subprocess (stdio transport) or opens an HTTP
//!   client (HTTP transport)
//! - **Handshake**: JSON-RPC `initialize` → `notifications/initialized`
//! - **Tool discovery**: `tools/list`
//! - **Tool invocation**: `tools/call`
//!
//! ## Protocol
//!
//! Follows the [MCP 2025-06-18] spec (with fallback to 2024-11-05).
//! Each transport sends/receives JSON-RPC 2.0 messages:
//!
//! ```jsonc
//! // Request
//! {"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}
//! // Response
//! {"jsonrpc":"2.0","id":1,"result":{"tools":[...]}}
//! ```
//!
//! [MCP 2025-06-18]: https://spec.modelcontextprotocol.io/specification/2025-06-18/

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use super::mcp_client::{McpServerEntry, McpTransport};

/// Default protocol version sent during `initialize`.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
/// Fallback version accepted from servers that don't support the latest.
pub const MCP_FALLBACK_PROTOCOL_VERSION: &str = "2024-11-05";

/// Timeout for a single JSON-RPC request/response round-trip.
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for the `initialize` handshake (servers may take longer to start).
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(60);

// ─── JSON-RPC types ───────────────────────────────────────────────

/// A JSON-RPC 2.0 request.
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    params: Value,
}

/// A JSON-RPC 2.0 response (deserialized loosely; we only need `result` or `error`).
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[allow(dead_code)]
    data: Option<Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

// ─── Tool types ───────────────────────────────────────────────────

/// A tool discovered from an MCP server's `tools/list` response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    /// Tool name (unique within a server).
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema for the tool's input parameters.
    #[serde(default)]
    pub input_schema: Option<Value>,
}

/// Result of a `tools/call` invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolResult {
    /// Whether the tool execution encountered an error.
    pub is_error: bool,
    /// Content blocks returned by the tool (text, image, etc.).
    pub content: Vec<McpContent>,
}

/// A single content block in a tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

/// Capabilities advertised by the server during `initialize`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct McpServerCapabilities {
    #[serde(default)]
    pub tools: Option<Value>,
    #[serde(default)]
    pub resources: Option<Value>,
    #[serde(default)]
    pub prompts: Option<Value>,
}

/// Server info from the `initialize` response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerInfo {
    pub name: String,
    pub version: String,
}

// ─── Client ───────────────────────────────────────────────────────

/// An active MCP client connection.
///
/// Created via [`McpClient::connect`]. After connecting, call [`initialize`]
/// to complete the handshake, then use [`list_tools`] / [`call_tool`].
///
/// The connection is closed when this struct is dropped (stdio) or
/// when the HTTP client is dropped (HTTP — no persistent state to clean up).
pub enum McpClient {
    /// JSON-RPC over a subprocess's stdin/stdout.
    Stdio(Box<StdioClient>),
    /// JSON-RPC over HTTP (stateless — each request is a POST).
    Http(Box<HttpClient>),
}

/// State for a stdio MCP client connection.
pub struct StdioClient {
    child: Child,
    stdin: tokio::process::ChildStdin,
    reader: tokio::io::BufReader<tokio::process::ChildStdout>,
    next_id: AtomicU64,
}

/// State for an HTTP MCP client connection.
pub struct HttpClient {
    client: reqwest::Client,
    url: String,
    auth_header: Option<String>,
    next_id: AtomicU64,
}

/// Error type for MCP client operations.
#[derive(Debug)]
pub enum McpClientError {
    /// I/O error (subprocess crash, broken pipe, network failure).
    Io(String),
    /// JSON-RPC error response from the server.
    Rpc(String),
    /// Protocol violation (malformed response, unexpected message).
    Protocol(String),
    /// Operation timed out.
    Timeout,
    /// Server not initialized (call `initialize` first).
    NotInitialized,
}

impl std::fmt::Display for McpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "MCP I/O error: {msg}"),
            Self::Rpc(err) => write!(f, "MCP RPC error: {err}"),
            Self::Protocol(msg) => write!(f, "MCP protocol error: {msg}"),
            Self::Timeout => write!(f, "MCP operation timed out"),
            Self::NotInitialized => write!(f, "MCP server not initialized"),
        }
    }
}

impl std::error::Error for McpClientError {}

impl From<std::io::Error> for McpClientError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

type McpResult<T> = Result<T, McpClientError>;

impl McpClient {
    /// Connect to an MCP server described by a config entry.
    ///
    /// For stdio transport, this spawns the subprocess.
    /// For HTTP transport, this creates the HTTP client.
    /// Does **not** perform the `initialize` handshake — call [`initialize`] after.
    pub async fn connect(entry: &McpServerEntry) -> McpResult<Self> {
        match &entry.transport {
            McpTransport::Stdio { command, args } => {
                let mut cmd = Command::new(command);
                cmd.args(args);
                cmd.stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true);

                let mut child = cmd.spawn().map_err(|e| {
                    McpClientError::Io(format!(
                        "failed to spawn MCP server '{cmd}': {e}",
                        cmd = command
                    ))
                })?;

                let stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| McpClientError::Io("no stdin from child".into()))?;
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| McpClientError::Io("no stdout from child".into()))?;

                Ok(Self::Stdio(Box::new(StdioClient {
                    child,
                    stdin,
                    reader: BufReader::new(stdout),
                    next_id: AtomicU64::new(1),
                })))
            }
            McpTransport::Http { url, .. } => {
                let client = reqwest::Client::builder()
                    .timeout(RPC_TIMEOUT)
                    .build()
                    .map_err(|e| {
                        McpClientError::Io(format!("failed to create HTTP client: {e}"))
                    })?;

                Ok(Self::Http(Box::new(HttpClient {
                    client,
                    url: url.clone(),
                    auth_header: match &entry.transport {
                        McpTransport::Http { auth_header, .. } => auth_header.clone(),
                        _ => None,
                    },
                    next_id: AtomicU64::new(1),
                })))
            }
        }
    }

    /// Perform the MCP `initialize` handshake.
    ///
    /// Sends `initialize` with our capabilities, then sends
    /// `notifications/initialized`. Returns the server's info and capabilities.
    pub async fn initialize(&mut self) -> McpResult<(McpServerInfo, McpServerCapabilities)> {
        let params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {}
            },
            "clientInfo": {
                "name": "vaultpilot",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let result = self
            .request("initialize", params, INITIALIZE_TIMEOUT)
            .await?;

        // Extract serverInfo and capabilities from the result
        let server_info = result
            .get("serverInfo")
            .and_then(|v| serde_json::from_value::<McpServerInfo>(v.clone()).ok())
            .unwrap_or(McpServerInfo {
                name: "unknown".into(),
                version: "unknown".into(),
            });

        let capabilities = result
            .get("capabilities")
            .and_then(|v| serde_json::from_value::<McpServerCapabilities>(v.clone()).ok())
            .unwrap_or_default();

        // Send initialized notification (no response expected)
        self.notify("notifications/initialized", Value::Null)
            .await?;

        Ok((server_info, capabilities))
    }

    /// List available tools from the server.
    pub async fn list_tools(&mut self) -> McpResult<Vec<McpTool>> {
        let result = self
            .request("tools/list", serde_json::json!({}), RPC_TIMEOUT)
            .await?;

        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| serde_json::from_value::<McpTool>(t.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(tools)
    }

    /// Call a tool by name with the given arguments.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> McpResult<McpToolResult> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let result = self.request("tools/call", params, RPC_TIMEOUT).await?;

        let tool_result = serde_json::from_value::<McpToolResult>(result)
            .map_err(|e| McpClientError::Protocol(format!("malformed tools/call result: {e}")))?;

        Ok(tool_result)
    }

    // ─── Internal request/response ────────────────────────────────

    /// Send a JSON-RPC request and wait for the response.
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> McpResult<Value> {
        match self {
            Self::Stdio(s) => {
                let id = s.next_id.fetch_add(1, Ordering::Relaxed);
                let req = JsonRpcRequest {
                    jsonrpc: "2.0",
                    id,
                    method: method.to_string(),
                    params,
                };
                let line = serde_json::to_string(&req).map_err(|e| {
                    McpClientError::Protocol(format!("failed to serialize request: {e}"))
                })?;

                // Send request
                s.stdin
                    .write_all(line.as_bytes())
                    .await
                    .map_err(|e| McpClientError::Io(format!("write to stdin: {e}")))?;
                s.stdin
                    .write_all(b"\n")
                    .await
                    .map_err(|e| McpClientError::Io(format!("write newline: {e}")))?;
                s.stdin
                    .flush()
                    .await
                    .map_err(|e| McpClientError::Io(format!("flush stdin: {e}")))?;

                // Read response with timeout
                let response =
                    tokio::time::timeout(timeout, Self::read_stdio_response(&mut s.reader))
                        .await
                        .map_err(|_| McpClientError::Timeout)?
                        .map_err(|e| McpClientError::Io(format!("read from stdout: {e}")))?;

                Self::validate_response(&response, id)
            }
            Self::Http(h) => {
                let id = h.next_id.fetch_add(1, Ordering::Relaxed);
                let req = JsonRpcRequest {
                    jsonrpc: "2.0",
                    id,
                    method: method.to_string(),
                    params,
                };

                let mut http_req = h.client.post(h.url.as_str()).json(&req);
                if let Some(ref auth) = h.auth_header {
                    http_req = http_req.header("Authorization", auth.as_str());
                }

                let response = tokio::time::timeout(timeout, http_req.send())
                    .await
                    .map_err(|_| McpClientError::Timeout)?
                    .map_err(|e| McpClientError::Io(format!("HTTP request: {e}")))?;

                let response: JsonRpcResponse = response
                    .json()
                    .await
                    .map_err(|e| McpClientError::Io(format!("parse HTTP response: {e}")))?;

                Self::validate_response(&response, id)
            }
        }
    }

    /// Send a notification (no response expected). Only meaningful for stdio.
    async fn notify(&mut self, method: &str, params: Value) -> McpResult<()> {
        match self {
            Self::Stdio(s) => {
                let notification = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": params
                });
                let line = serde_json::to_string(&notification).map_err(|e| {
                    McpClientError::Protocol(format!("serialize notification: {e}"))
                })?;
                s.stdin
                    .write_all(line.as_bytes())
                    .await
                    .map_err(|e| McpClientError::Io(format!("write notification: {e}")))?;
                s.stdin
                    .write_all(b"\n")
                    .await
                    .map_err(|e| McpClientError::Io(format!("write newline: {e}")))?;
                s.stdin
                    .flush()
                    .await
                    .map_err(|e| McpClientError::Io(format!("flush stdin: {e}")))?;
                Ok(())
            }
            // HTTP is stateless; notifications are just fire-and-forget POSTs
            Self::Http(h) => {
                let notification = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": params
                });
                let mut http_req = h.client.post(h.url.as_str()).json(&notification);
                if let Some(ref auth) = h.auth_header {
                    http_req = http_req.header("Authorization", auth.as_str());
                }
                let _ = tokio::time::timeout(RPC_TIMEOUT, http_req.send())
                    .await
                    .map_err(|_| McpClientError::Timeout)?;
                Ok(())
            }
        }
    }

    /// Read a single JSON-RPC response line from stdout.
    async fn read_stdio_response(
        reader: &mut tokio::io::BufReader<tokio::process::ChildStdout>,
    ) -> std::io::Result<JsonRpcResponse> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "MCP server closed stdout",
                ));
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Try to parse as a JSON-RPC response. Skip notifications
            // (messages without an `id` field) since we're waiting for a response.
            if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(trimmed) {
                if resp.id.is_some() {
                    return Ok(resp);
                }
            }
            // Not a valid response — could be a notification or log output.
            // Skip and read the next line.
        }
    }

    /// Validate a JSON-RPC response and extract the result.
    fn validate_response(response: &JsonRpcResponse, expected_id: u64) -> McpResult<Value> {
        // Check ID match
        let id_matches = response.id.as_ref().is_some_and(|id| {
            id.as_u64() == Some(expected_id) || id.as_i64() == Some(expected_id as i64)
        });
        if !id_matches {
            return Err(McpClientError::Protocol(format!(
                "response ID mismatch: expected {expected_id}, got {:?}",
                response.id
            )));
        }

        if let Some(err) = &response.error {
            return Err(McpClientError::Rpc(format!(
                "[{}] {}",
                err.code, err.message
            )));
        }

        response
            .result
            .clone()
            .ok_or_else(|| McpClientError::Protocol("response has neither result nor error".into()))
    }

    /// Check if the underlying subprocess (stdio) is still alive.
    pub fn is_alive(&mut self) -> bool {
        match self {
            Self::Stdio(s) => match s.child.try_wait() {
                Ok(None) => true,     // still running
                Ok(Some(_)) => false, // exited
                Err(_) => false,
            },
            Self::Http(_) => true, // HTTP is stateless
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Self::Stdio(s) = self {
            // Best-effort kill; kill_on_drop is also set on the Command
            let _ = s.child.start_kill();
        }
    }
}

// ─── Convenience: discover tools from all enabled servers ─────────

/// Connect to all enabled MCP servers, perform the handshake, and collect
/// discovered tools. Returns a map of server name → list of tools.
///
/// Servers that fail to connect or initialize are logged and skipped.
pub async fn discover_all_tools(vault_dir: &str) -> Vec<(String, Vec<McpTool>)> {
    let config = super::mcp_client::load_mcp_servers_config(vault_dir);
    let mut results = Vec::new();

    for entry in super::mcp_client::enabled_servers(&config) {
        match McpClient::connect(entry).await {
            Ok(mut client) => match client.initialize().await {
                Ok(_) => match client.list_tools().await {
                    Ok(tools) => {
                        eprintln!(
                            "[mcp_client] discovered {} tools from '{}'",
                            tools.len(),
                            entry.name
                        );
                        results.push((entry.name.clone(), tools));
                    }
                    Err(e) => {
                        eprintln!("[mcp_client] tools/list failed for '{}': {e}", entry.name);
                    }
                },
                Err(e) => {
                    eprintln!("[mcp_client] initialize failed for '{}': {e}", entry.name);
                }
            },
            Err(e) => {
                eprintln!("[mcp_client] connect failed for '{}': {e}", entry.name);
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Type tests ───────────────────────────────────────────────

    #[test]
    fn parse_tool_from_tools_list_response() {
        let json = r#"{
            "name": "search_notes",
            "description": "Search vault notes by query",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }
        }"#;

        let tool: McpTool = serde_json::from_str(json).unwrap();
        assert_eq!(tool.name, "search_notes");
        assert_eq!(
            tool.description.as_deref(),
            Some("Search vault notes by query")
        );
        assert!(tool.input_schema.is_some());
        assert_eq!(tool.input_schema.as_ref().unwrap()["type"], "object");
    }

    #[test]
    fn parse_tool_result() {
        let json = r#"{
            "isError": false,
            "content": [
                {"type": "text", "text": "Found 3 notes"}
            ]
        }"#;

        let result: McpToolResult = serde_json::from_str(json).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].content_type, "text");
        assert_eq!(result.content[0].text.as_deref(), Some("Found 3 notes"));
    }

    #[test]
    fn parse_tool_result_error() {
        let json = r#"{
            "isError": true,
            "content": [
                {"type": "text", "text": "Tool execution failed: invalid arguments"}
            ]
        }"#;

        let result: McpToolResult = serde_json::from_str(json).unwrap();
        assert!(result.is_error);
        assert!(result.content[0].text.as_ref().unwrap().contains("failed"));
    }

    #[test]
    fn parse_server_capabilities() {
        let json = r#"{
            "tools": {"listChanged": true},
            "resources": {}
        }"#;

        let caps: McpServerCapabilities = serde_json::from_str(json).unwrap();
        assert!(caps.tools.is_some());
        assert!(caps.resources.is_some());
        assert!(caps.prompts.is_none());
    }

    #[test]
    fn parse_server_info() {
        let json = r#"{
            "name": "github-mcp-server",
            "version": "1.2.0"
        }"#;

        let info: McpServerInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name, "github-mcp-server");
        assert_eq!(info.version, "1.2.0");
    }

    // ─── Response validation tests ────────────────────────────────

    #[test]
    fn validate_response_success() {
        let response = JsonRpcResponse {
            jsonrpc: Some("2.0".into()),
            id: Some(Value::from(1u64)),
            result: Some(serde_json::json!({"tools": []})),
            error: None,
        };

        let result = McpClient::validate_response(&response, 1);
        assert!(result.is_ok());
        assert!(result.unwrap()["tools"].is_array());
    }

    #[test]
    fn validate_response_id_mismatch() {
        let response = JsonRpcResponse {
            jsonrpc: Some("2.0".into()),
            id: Some(Value::from(5u64)),
            result: Some(Value::Null),
            error: None,
        };

        let result = McpClient::validate_response(&response, 1);
        assert!(matches!(result, Err(McpClientError::Protocol(_))));
    }

    #[test]
    fn validate_response_error() {
        let response = JsonRpcResponse {
            jsonrpc: Some("2.0".into()),
            id: Some(Value::from(2u64)),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "Method not found".into(),
                data: None,
            }),
        };

        let result = McpClient::validate_response(&response, 2);
        match result {
            Err(McpClientError::Rpc(msg)) => {
                assert!(msg.contains("-32601"));
                assert!(msg.contains("Method not found"));
            }
            _ => panic!("expected Rpc error"),
        }
    }

    #[test]
    fn validate_response_no_result_no_error() {
        let response = JsonRpcResponse {
            jsonrpc: Some("2.0".into()),
            id: Some(Value::from(3u64)),
            result: None,
            error: None,
        };

        let result = McpClient::validate_response(&response, 3);
        assert!(matches!(result, Err(McpClientError::Protocol(_))));
    }

    // ─── Error display tests ──────────────────────────────────────

    #[test]
    fn error_display_io() {
        let err = McpClientError::Io("connection refused".into());
        assert_eq!(err.to_string(), "MCP I/O error: connection refused");
    }

    #[test]
    fn error_display_rpc() {
        let err = McpClientError::Rpc("[-32700] Parse error".into());
        assert_eq!(err.to_string(), "MCP RPC error: [-32700] Parse error");
    }

    #[test]
    fn error_display_protocol() {
        let err = McpClientError::Protocol("unexpected EOF".into());
        assert_eq!(err.to_string(), "MCP protocol error: unexpected EOF");
    }

    #[test]
    fn error_display_timeout() {
        let err = McpClientError::Timeout;
        assert_eq!(err.to_string(), "MCP operation timed out");
    }

    #[test]
    fn error_display_not_initialized() {
        let err = McpClientError::NotInitialized;
        assert_eq!(err.to_string(), "MCP server not initialized");
    }

    // ─── Protocol version constants ───────────────────────────────

    #[test]
    fn protocol_versions_are_valid_strings() {
        assert!(!MCP_PROTOCOL_VERSION.is_empty());
        assert!(!MCP_FALLBACK_PROTOCOL_VERSION.is_empty());
        // Both should look like dates (YYYY-MM-DD)
        assert!(MCP_PROTOCOL_VERSION.contains('-'));
        assert!(MCP_FALLBACK_PROTOCOL_VERSION.contains('-'));
    }

    // ─── JsonRpcRequest serialization ─────────────────────────────

    #[test]
    fn jsonrpc_request_serializes_correctly() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 42,
            method: "tools/list".into(),
            params: serde_json::json!({}),
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 42);
        assert_eq!(json["method"], "tools/list");
    }

    #[test]
    fn jsonrpc_request_with_params() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 7,
            method: "tools/call".into(),
            params: serde_json::json!({
                "name": "search",
                "arguments": {"query": "hello"}
            }),
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["params"]["name"], "search");
        assert_eq!(json["params"]["arguments"]["query"], "hello");
    }
}
