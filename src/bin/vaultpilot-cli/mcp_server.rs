use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;

use vaultpilot_lib::models::*;
use vaultpilot_lib::storage::{
    delete_note_with_context, find_related_notes_with_context, import_markdown_with_context,
    load_chat_state_with_context, load_note_async, load_note_with_context,
    rebuild_index_with_context, save_chat_state_with_context, save_note_with_context,
    search_notes_async, search_notes_with_context, StorageContext,
};
use vaultpilot_lib::{
    ask_with_ai_with_context, finalize_chat_with_ai_answer, prepare_chat_for_ai,
    rollback_last_user_turn, sanitize_error,
};

use super::{chat_session_overview, new_cli_chat_session};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_FALLBACK_PROTOCOL_VERSION: &str = "2024-11-05";
const AI_CALL_TIMEOUT: Duration = Duration::from_secs(120);

// ─── Types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct McpRequest {
    #[serde(default)]
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct McpResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpError>,
}

#[derive(Debug, Serialize)]
struct McpError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Default)]
struct McpServerState {
    initialized: bool,
    protocol_version: String,
}

impl McpResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: String, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(McpError {
                code,
                message,
                data,
            }),
        }
    }
}

// ─── Prompt sanitization ─────────────────────────────────────────

/// Escape XML tags and wrap content in delimiters to mitigate prompt injection
/// in MCP prompt templates. User-controlled content (note titles, bodies, search results)
/// must be sanitized before interpolation into LLM prompts.
///
/// Defense-in-depth: escapes both closing tags (`</` → `<//`) and the specific wrapper
/// tag names (`<user_content>`, `</user_content>`) to prevent nested delimiter breakout.
pub(super) fn sanitize_mcp_prompt_content(content: &str) -> String {
    let escaped = escape_xml_content(content);
    format!("<user_content>\n{escaped}\n</user_content>")
}

pub(super) fn escape_xml_content(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                // Escape all closing tags: </ → <//
                if chars.peek() == Some(&'/') {
                    out.push_str("<//");
                    chars.next();
                }
                // Escape the specific wrapper tag name to prevent nested breakout
                else {
                    let rest: String = chars.clone().take(13).collect();
                    if rest.starts_with("user_content>") || rest.starts_with("user_content ") {
                        out.push_str("< ");
                    } else {
                        out.push('<');
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

// ─── Public entry point ────────────────────────────────────────────

pub(super) fn run_mcp_server(context: &StorageContext, runtime: &Runtime) -> Result<()> {
    runtime.block_on(run_mcp_server_async(context))
}

async fn run_mcp_server_async(context: &StorageContext) -> Result<()> {
    use tokio::io::BufReader;

    const INITIALIZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    const SHUTDOWN_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    /// Maximum bytes allowed for a single JSON-RPC line on stdin.
    /// Prevents OOM from a malicious or buggy MCP client sending an
    /// unbounded payload without a newline delimiter.
    const MAX_MCP_LINE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

    let mut state = McpServerState {
        initialized: false,
        protocol_version: MCP_PROTOCOL_VERSION.to_string(),
    };

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);

    // Spawn a background task that resolves when a termination signal is
    // received so we can incorporate it into the select! loop below.
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        // Wait for either Ctrl-C (SIGINT) or SIGTERM.
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm =
                signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = shutdown_tx.send(true);
    });

    loop {
        // Helper: read one line with a size cap to prevent OOM from
        // unbounded stdin input (#596, #649).
        // Read byte-by-byte, enforcing the limit *during* reading rather
        // than after.  BufReader::read_line() buffers the entire payload
        // before returning, making a post-read size check ineffective
        // against payloads that never contain a newline (#649).
        let read_line_bounded = async {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            let mut exceeded = false;
            // Read in 8 KB chunks instead of byte-by-byte to reduce
            // syscall count from O(n) to O(n/8192) (#907).
            const CHUNK_SIZE: usize = 8 * 1024;
            let mut chunk_buf = [0u8; CHUNK_SIZE];
            'read: loop {
                // Determine how many bytes we can still accept.
                let capacity_left = MAX_MCP_LINE_BYTES.saturating_sub(buf.len());
                let want = capacity_left.min(CHUNK_SIZE);
                if want == 0 {
                    // Already at limit — drain byte-by-byte until newline.
                    match reader.read(&mut chunk_buf[..1]).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    if chunk_buf[0] == b'\n' {
                        break;
                    }
                    exceeded = true;
                    continue;
                }
                let n = match reader.read(&mut chunk_buf[..want]).await {
                    Ok(0) => break 'read, // EOF
                    Ok(n) => n,
                    Err(_) => break 'read,
                };
                // Scan the chunk for newline.
                if let Some(nl_pos) = chunk_buf[..n].iter().position(|&b| b == b'\n') {
                    let end = nl_pos + 1; // include the newline
                    let room = MAX_MCP_LINE_BYTES - buf.len();
                    let take = end.min(room);
                    buf.extend_from_slice(&chunk_buf[..take]);
                    if take < end {
                        exceeded = true; // data was truncated, mark as oversized
                    }
                    break 'read;
                }
                // No newline found — append the whole chunk.
                buf.extend_from_slice(&chunk_buf[..n]);
            }
            if buf.is_empty() && !exceeded {
                return Ok(None); // EOF
            }
            if exceeded {
                return Err(anyhow::anyhow!(
                    "stdin line exceeds {}MB limit",
                    MAX_MCP_LINE_BYTES / (1024 * 1024)
                ));
            }
            let line = std::str::from_utf8(&buf).map_err(|e| {
                anyhow::anyhow!(
                    "{}",
                    vaultpilot_lib::sanitize_error(&format!("invalid UTF-8 in request: {e}"))
                )
            })?;
            // Strip trailing \r\n or \n for consistent handling across platforms.
            let line = line.trim_end_matches('\n').trim_end_matches('\r');
            Ok::<_, anyhow::Error>(Some(line.to_string()))
        };

        // Before initialize, enforce a timeout so we don't block forever
        // waiting for a client that never speaks.
        let line: Option<String> = if !state.initialized {
            let next = tokio::time::timeout(INITIALIZE_TIMEOUT, read_line_bounded);
            tokio::select! {
                result = next => {
                    match result {
                        Ok(Ok(Some(line))) => Some(line),
                        Ok(Ok(None)) => None,
                        Ok(Err(e)) => return Err(anyhow::anyhow!("stdin read error: {e}")),
                        Err(_elapsed) => {
                            eprintln!(
                                "MCP server: no initialize request received within {}s, shutting down",
                                INITIALIZE_TIMEOUT.as_secs()
                            );
                            None
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    eprintln!("MCP server: received shutdown signal before initialize");
                    None
                }
            }
        } else {
            let mut shutdown = false;
            let result = tokio::select! {
                result = read_line_bounded => result?,
                _ = shutdown_rx.changed() => {
                    eprintln!("MCP server: received shutdown signal");
                    shutdown = true;
                    None
                }
            };
            if shutdown {
                break;
            }
            result
        };

        let line = match line {
            Some(l) => l,
            None => break, // EOF or timeout or signal
        };

        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<McpRequest>(&line) {
            Ok(request) => {
                if request.method == "initialize" && request.jsonrpc == "2.0" {
                    let id = request.id.unwrap_or(Value::Null);
                    let requested_version = request
                        .params
                        .get("protocolVersion")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    state.initialized = true;
                    state.protocol_version =
                        negotiate_mcp_protocol_version(requested_version).to_string();
                    Some(McpResponse::ok(
                        id,
                        serde_json::json!({
                            "protocolVersion": state.protocol_version,
                            "capabilities": {
                                "tools": { "listChanged": false },
                                "resources": { "listChanged": false },
                                "prompts": { "listChanged": false }
                            },
                            "serverInfo": {
                                "name": "vaultpilot",
                                "title": "VaultPilot MCP",
                                "version": env!("CARGO_PKG_VERSION")
                            },
                            "instructions": "Use chat.send to talk to VaultPilot through its built-in model. VaultPilot performs local retrieval and model calls internally; clients should treat it as a chat endpoint instead of direct note-search tooling."
                        }),
                    ))
                } else {
                    handle_mcp_request(context, &state, request).await
                }
            }
            Err(error) => Some(McpResponse::error(
                Value::Null,
                -32700,
                format!("failed to parse JSON-RPC request: {error}"),
                None,
            )),
        };

        if let Some(response) = response {
            use tokio::io::AsyncWriteExt;
            let mut out = tokio::io::stdout();
            let payload = serde_json::to_string(&response)?;
            out.write_all(payload.as_bytes()).await?;
            out.write_all(b"\n").await?;
            out.flush().await?;
        }
    }

    // Clean shutdown: log and give in-flight operations a moment to drain.
    eprintln!("MCP server: shutting down cleanly");
    tokio::time::sleep(SHUTDOWN_DRAIN_TIMEOUT).await;

    Ok(())
}

// ─── HTTP MCP server ────────────────────────────────────────────

struct McpHttpState {
    context: StorageContext,
    server_state: tokio::sync::RwLock<McpServerState>,
    token: Option<String>,
}

pub(super) async fn run_mcp_http_server(
    context: StorageContext,
    host: String,
    port: u16,
    token: Option<String>,
) -> Result<()> {
    use std::net::{IpAddr, SocketAddr};

    let ip: IpAddr = host
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid host '{}': {}", host, e))?;

    // Non-loopback binding requires token auth (same policy as HTTP bridge)
    crate::http_bridge::validate_http_bridge_binding(ip, token.as_deref())?;

    let address = SocketAddr::new(ip, port);
    let state = Arc::new(McpHttpState {
        context,
        server_state: tokio::sync::RwLock::new(McpServerState {
            initialized: false,
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
        }),
        token,
    });

    let app = axum::Router::new()
        .route("/mcp", axum::routing::post(mcp_http_handler))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            10 * 1024 * 1024,
        )) // 10 MB limit (#1575)
        .with_state(state);

    eprintln!("MCP HTTP server listening on {address}");
    eprintln!("  POST /mcp  — JSON-RPC endpoint");

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn mcp_http_handler(
    State(state): State<Arc<McpHttpState>>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    // Token auth: require bearer token if configured
    if let Some(ref expected) = state.token {
        let Some(token) = crate::http_bridge::bridge_token_from_headers(&headers) else {
            return Json(McpResponse::error(
                Value::Null,
                -32600,
                "unauthorized".to_string(),
                None,
            ))
            .into_response();
        };
        if !crate::http_bridge::constant_time_eq(token.as_bytes(), expected.as_bytes()) {
            return Json(McpResponse::error(
                Value::Null,
                -32600,
                "unauthorized".to_string(),
                None,
            ))
            .into_response();
        }
    }

    // Parse request manually to return JSON-RPC error on malformed input
    let request: McpRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            return Json(McpResponse::error(
                Value::Null,
                -32700,
                format!("failed to parse JSON-RPC request: {e}"),
                None,
            ))
            .into_response();
        }
    };

    // Handle initialize with write lock; all other requests with read lock
    if request.method == "initialize" && request.jsonrpc == "2.0" {
        let mut server_state = state.server_state.write().await;
        let requested_version = request
            .params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        server_state.initialized = true;
        server_state.protocol_version =
            negotiate_mcp_protocol_version(requested_version).to_string();
        let protocol_version = server_state.protocol_version.clone();
        // Drop the lock before responding
        drop(server_state);

        return Json(McpResponse::ok(
            request.id.unwrap_or(Value::Null),
            serde_json::json!({
                "protocolVersion": protocol_version,
                "capabilities": {
                    "tools": { "listChanged": false },
                    "resources": { "listChanged": false },
                    "prompts": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "vaultpilot",
                    "title": "VaultPilot MCP",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": "Use chat.send to talk to VaultPilot through its built-in model. VaultPilot performs local retrieval and model calls internally; clients should treat it as a chat endpoint instead of direct note-search tooling."
            }),
        )).into_response();
    }

    let state_snapshot = {
        let guard = state.server_state.read().await;
        McpServerState {
            initialized: guard.initialized,
            protocol_version: guard.protocol_version.clone(),
        }
    };

    match handle_mcp_request(&state.context, &state_snapshot, request).await {
        Some(resp) => Json(resp).into_response(),
        None => (axum::http::StatusCode::ACCEPTED,).into_response(),
    }
}

// ─── Request handler ──────────────────────────────────────────────

async fn handle_mcp_request(
    context: &StorageContext,
    state: &McpServerState,
    request: McpRequest,
) -> Option<McpResponse> {
    if request.jsonrpc != "2.0" {
        return Some(McpResponse::error(
            request.id.unwrap_or(Value::Null),
            -32600,
            "jsonrpc must be \"2.0\"".to_string(),
            None,
        ));
    }

    match request.method.as_str() {
        "initialize" => {
            // Should have been handled by the caller (HTTP handler or stdin handler)
            // If we reach here, return an error
            Some(McpResponse::error(
                request.id.unwrap_or(Value::Null),
                -32600,
                "initialize must be handled before handle_mcp_request".to_string(),
                None,
            ))
        }
        "notifications/initialized" => None,
        "ping" => request
            .id
            .map(|id| McpResponse::ok(id, serde_json::json!({}))),
        "tools/list" => {
            let id = match request.id {
                Some(id) => id,
                None => {
                    return Some(McpResponse::error(
                        Value::Null,
                        -32600,
                        "tools/list requires a request id".to_string(),
                        None,
                    ))
                }
            };
            if !state.initialized {
                return Some(McpResponse::error(
                    id,
                    -32002,
                    "server not initialized".to_string(),
                    None,
                ));
            }
            Some(McpResponse::ok(
                id,
                serde_json::json!({
                    "tools": mcp_tools()
                }),
            ))
        }
        "tools/call" => {
            let id = match request.id {
                Some(id) => id,
                None => {
                    return Some(McpResponse::error(
                        Value::Null,
                        -32600,
                        "tools/call requires a request id".to_string(),
                        None,
                    ))
                }
            };
            if !state.initialized {
                return Some(McpResponse::error(
                    id,
                    -32002,
                    "server not initialized".to_string(),
                    None,
                ));
            }

            let tool_name = match request.params.get("name").and_then(Value::as_str) {
                Some(name) => name,
                None => {
                    return Some(McpResponse::error(
                        id,
                        -32602,
                        "tools/call requires a string params.name".to_string(),
                        None,
                    ))
                }
            };
            let arguments = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));

            let result = match tool_name {
                "chat.send" => mcp_call_chat_send(context, arguments).await,
                "chat.list_sessions" => mcp_call_chat_list_sessions(context),
                "chat.get_state" => mcp_call_chat_get_state(context),
                "chat.new" => mcp_call_chat_new(context, arguments).await,
                "chat.delete" => mcp_call_chat_delete(context, arguments).await,
                "notes.list" => mcp_call_notes_list(context, arguments),
                "notes.get" => mcp_call_notes_get(context, arguments),
                "notes.create" => mcp_call_notes_create(context, arguments),
                "notes.delete" => mcp_call_notes_delete(context, arguments),
                "notes.search" => mcp_call_notes_search(context, arguments),
                "notes.related" => mcp_call_notes_related(context, arguments),
                "notes.import" => mcp_call_notes_import(context, arguments),
                "index.rebuild" => mcp_call_index_rebuild(context),
                "ask" => mcp_call_ask(context, arguments).await,
                _ => {
                    return Some(McpResponse::error(
                        id,
                        -32601,
                        format!("unknown tool: {tool_name}"),
                        None,
                    ))
                }
            };

            Some(McpResponse::ok(id, result))
        }
        "resources/list" => {
            let id = match request.id {
                Some(id) => id,
                None => {
                    return Some(McpResponse::error(
                        Value::Null,
                        -32600,
                        "resources/list requires a request id".to_string(),
                        None,
                    ))
                }
            };
            if !state.initialized {
                return Some(McpResponse::error(
                    id,
                    -32002,
                    "server not initialized".to_string(),
                    None,
                ));
            }
            let cursor = request
                .params
                .get("cursor")
                .and_then(Value::as_str)
                .unwrap_or("");
            let limit: usize = 50;
            let offset = cursor.parse::<usize>().unwrap_or(0);
            match search_notes_async(
                context,
                SearchQuery {
                    text: String::new(),
                    tags: Vec::new(),
                    keywords: Vec::new(),
                    limit: Some(limit),
                    offset: Some(offset),
                    ..Default::default()
                },
            )
            .await
            {
                Ok(result) => {
                    let resources: Vec<Value> = result
                        .notes
                        .into_iter()
                        .map(|meta| {
                            serde_json::json!({
                                "uri": format!("vault://notes/{}", meta.id),
                                "name": meta.title,
                                "description": if meta.summary.is_empty() { None } else { Some(&meta.summary) },
                                "mimeType": "text/markdown"
                            })
                        })
                        .collect();
                    let next_offset = offset + resources.len();
                    let has_more = resources.len() == limit;
                    let next_cursor = if has_more {
                        Some(next_offset.to_string())
                    } else {
                        None
                    };
                    let mut payload = serde_json::json!({ "resources": resources });
                    if let Some(cursor) = next_cursor {
                        payload["nextCursor"] = Value::String(cursor);
                    }
                    Some(McpResponse::ok(id, payload))
                }
                Err(e) => Some(McpResponse::error(
                    id,
                    -32603,
                    sanitize_error(&format!("failed to list resources: {e}")),
                    None,
                )),
            }
        }
        "resources/read" => {
            let id = match request.id {
                Some(id) => id,
                None => {
                    return Some(McpResponse::error(
                        Value::Null,
                        -32600,
                        "resources/read requires a request id".to_string(),
                        None,
                    ))
                }
            };
            if !state.initialized {
                return Some(McpResponse::error(
                    id,
                    -32002,
                    "server not initialized".to_string(),
                    None,
                ));
            }
            let uri = match request.params.get("uri").and_then(Value::as_str) {
                Some(u) => u,
                None => {
                    return Some(McpResponse::error(
                        id,
                        -32602,
                        "resources/read requires a string params.uri".to_string(),
                        None,
                    ))
                }
            };
            // Parse vault://notes/{id}
            let note_id = match uri.strip_prefix("vault://notes/") {
                Some(nid) => nid,
                None => {
                    return Some(McpResponse::error(
                        id,
                        -32602,
                        format!("unsupported resource URI scheme: {uri}"),
                        None,
                    ))
                }
            };
            match load_note_async(context, note_id).await {
                Ok(note) => Some(McpResponse::ok(
                    id,
                    serde_json::json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/markdown",
                            "text": note.body
                        }]
                    }),
                )),
                Err(e) => Some(McpResponse::error(
                    id,
                    -32603,
                    sanitize_error(&format!("failed to read resource: {e}")),
                    None,
                )),
            }
        }
        "prompts/list" => {
            let id = match request.id {
                Some(id) => id,
                None => {
                    return Some(McpResponse::error(
                        Value::Null,
                        -32600,
                        "prompts/list requires a request id".to_string(),
                        None,
                    ))
                }
            };
            if !state.initialized {
                return Some(McpResponse::error(
                    id,
                    -32002,
                    "server not initialized".to_string(),
                    None,
                ));
            }
            Some(McpResponse::ok(
                id,
                serde_json::json!({
                    "prompts": [
                        {
                            "name": "summarize-note",
                            "description": "Summarize a vault note by ID",
                            "arguments": [
                                { "name": "noteId", "description": "The ID of the note to summarize", "required": true }
                            ]
                        },
                        {
                            "name": "find-related",
                            "description": "Find notes related to a given topic or note",
                            "arguments": [
                                { "name": "topic", "description": "The topic or keywords to search for", "required": true },
                                { "name": "limit", "description": "Maximum number of related notes to return", "required": false }
                            ]
                        },
                        {
                            "name": "draft-from-keywords",
                            "description": "Draft a note from keywords with optional style guidance",
                            "arguments": [
                                { "name": "keywords", "description": "Comma-separated keywords for the note", "required": true },
                                { "name": "style", "description": "Writing style: concise, detailed, tutorial, reference", "required": false }
                            ]
                        }
                    ]
                }),
            ))
        }
        "prompts/get" => {
            let id = match request.id {
                Some(id) => id,
                None => {
                    return Some(McpResponse::error(
                        Value::Null,
                        -32600,
                        "prompts/get requires a request id".to_string(),
                        None,
                    ))
                }
            };
            if !state.initialized {
                return Some(McpResponse::error(
                    id,
                    -32002,
                    "server not initialized".to_string(),
                    None,
                ));
            }
            let prompt_name = match request.params.get("name").and_then(Value::as_str) {
                Some(n) => n,
                None => {
                    return Some(McpResponse::error(
                        id,
                        -32602,
                        "prompts/get requires a string params.name".to_string(),
                        None,
                    ))
                }
            };
            let args = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));

            let messages = match prompt_name {
                "summarize-note" => {
                    let note_id = match args.get("noteId").and_then(Value::as_str) {
                        Some(nid) => nid,
                        None => {
                            return Some(McpResponse::error(
                                id,
                                -32602,
                                "summarize-note requires 'noteId' argument".to_string(),
                                None,
                            ))
                        }
                    };
                    match load_note_async(context, note_id).await {
                        Ok(note) => vec![serde_json::json!({
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!(
                                    "Please provide a concise summary of the following note:\n\nTitle: {}\n\n{}",
                                    sanitize_mcp_prompt_content(&note.meta.title),
                                    sanitize_mcp_prompt_content(&note.body)
                                )
                            }
                        })],
                        Err(e) => {
                            return Some(McpResponse::error(
                                id,
                                -32603,
                                sanitize_error(&format!("failed to load note: {e}")),
                                None,
                            ))
                        }
                    }
                }
                "find-related" => {
                    let topic = match args.get("topic").and_then(Value::as_str) {
                        Some(t) => t,
                        None => {
                            return Some(McpResponse::error(
                                id,
                                -32602,
                                "find-related requires 'topic' argument".to_string(),
                                None,
                            ))
                        }
                    };
                    let limit = args
                        .get("limit")
                        .and_then(Value::as_u64)
                        .unwrap_or(5)
                        .min(200) as usize;
                    match search_notes_async(
                        context,
                        SearchQuery {
                            text: topic.to_string(),
                            tags: Vec::new(),
                            keywords: Vec::new(),
                            limit: Some(limit),
                            ..Default::default()
                        },
                    )
                    .await
                    {
                        Ok(result) => {
                            let notes_text = result
                                .notes
                                .iter()
                                .map(|m| {
                                    format!(
                                        "- **{}** (id: {}): {}",
                                        sanitize_mcp_prompt_content(&m.title),
                                        escape_xml_content(&m.id),
                                        sanitize_mcp_prompt_content(&m.summary)
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            vec![serde_json::json!({
                                "role": "user",
                                "content": {
                                    "type": "text",
                                    "text": format!(
                                        "Here are notes related to the topic:\n\n{}\n\nPlease analyze their relationships and suggest how they connect.",
                                        notes_text
                                    )
                                }
                            })]
                        }
                        Err(e) => {
                            return Some(McpResponse::error(
                                id,
                                -32603,
                                sanitize_error(&format!("failed to search notes: {e}")),
                                None,
                            ))
                        }
                    }
                }
                "draft-from-keywords" => {
                    let keywords = match args.get("keywords").and_then(Value::as_str) {
                        Some(k) => k,
                        None => {
                            return Some(McpResponse::error(
                                id,
                                -32602,
                                "draft-from-keywords requires 'keywords' argument".to_string(),
                                None,
                            ))
                        }
                    };
                    let style = args
                        .get("style")
                        .and_then(Value::as_str)
                        .unwrap_or("concise");
                    vec![serde_json::json!({
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": format!(
                                "Draft a note about the following keywords:\n{}\nWriting style: {}\n\nPlease write a well-structured note with a title, relevant sections, and key takeaways.",
                                sanitize_mcp_prompt_content(keywords),
                                sanitize_mcp_prompt_content(style)
                            )
                        }
                    })]
                }
                _ => {
                    return Some(McpResponse::error(
                        id,
                        -32601,
                        format!("unknown prompt: {prompt_name}"),
                        None,
                    ))
                }
            };

            Some(McpResponse::ok(
                id,
                serde_json::json!({
                    "description": format!("Prompt: {prompt_name}"),
                    "messages": messages
                }),
            ))
        }
        method if method.starts_with("notifications/") => None,
        _ => request.id.map(|id| {
            McpResponse::error(
                id,
                -32601,
                format!("method not found: {}", request.method),
                None,
            )
        }),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────

fn negotiate_mcp_protocol_version(requested: &str) -> &'static str {
    match requested {
        MCP_PROTOCOL_VERSION => MCP_PROTOCOL_VERSION,
        MCP_FALLBACK_PROTOCOL_VERSION => MCP_FALLBACK_PROTOCOL_VERSION,
        _ => MCP_PROTOCOL_VERSION,
    }
}

fn mcp_tools() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "chat.send",
            "title": "Send Chat Message",
            "description": "Send a message to VaultPilot's built-in model. VaultPilot retrieves local knowledge, calls the configured model provider, and persists the conversation session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "User message to send. May be omitted when sending only images."
                    },
                    "imagePaths": {
                        "type": "array",
                        "description": "Optional local image paths to include with the message.",
                        "items": { "type": "string" }
                    },
                    "sessionId": {
                        "type": "string",
                        "description": "Existing VaultPilot chat session ID. If omitted, the current session is used."
                    },
                    "createNewSession": {
                        "type": "boolean",
                        "description": "If true, create a new session before sending the message.",
                        "default": false
                    }
                },
                "additionalProperties": false
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string" },
                    "sessionTitle": { "type": "string" },
                    "createdSession": { "type": "boolean" },
                    "answer": { "type": "object" },
                    "state": { "type": "object" }
                }
            },
            "annotations": {
                "title": "Send Chat Message",
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "chat.list_sessions",
            "title": "List Chat Sessions",
            "description": "List saved VaultPilot chat sessions without exposing raw note-management tools.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "currentSessionId": { "type": "string" },
                    "sessions": { "type": "array" }
                }
            },
            "annotations": {
                "title": "List Chat Sessions",
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "chat.get_state",
            "title": "Get Chat State",
            "description": "Return the full persisted chat state managed by VaultPilot.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "currentSessionId": { "type": "string" },
                    "sessions": { "type": "array" }
                }
            },
            "annotations": {
                "title": "Get Chat State",
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "chat.new",
            "title": "New Chat Session",
            "description": "Create a new chat session and set it as the current session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Optional title for the new session."
                    }
                },
                "additionalProperties": false
            },
            "annotations": {
                "title": "New Chat Session",
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "chat.delete",
            "title": "Delete Chat Session",
            "description": "Delete a chat session by ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "The session ID to delete."
                    }
                },
                "required": ["sessionId"],
                "additionalProperties": false
            },
            "annotations": {
                "title": "Delete Chat Session",
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "notes.list",
            "title": "List Notes",
            "description": "List notes in the vault, ordered by most recent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of notes to return (max 200).",
                        "default": 20,
                        "maximum": 200
                    }
                },
                "additionalProperties": false
            },
            "annotations": {
                "title": "List Notes",
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "notes.get",
            "title": "Get Note",
            "description": "Retrieve a single note by its ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The note ID to retrieve."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": {
                "title": "Get Note",
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "notes.create",
            "title": "Create Note",
            "description": "Create a new note in the vault. Provide the note document as arguments.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Note title." },
                    "summary": { "type": "string", "description": "Brief summary." },
                    "body": { "type": "string", "description": "Note body content." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for the note." },
                    "keywords": { "type": "array", "items": { "type": "string" }, "description": "Keywords." },
                    "platform": { "type": "string" },
                    "board": { "type": "string" },
                    "kernel": { "type": "string" },
                    "status": { "type": "string" }
                },
                "required": ["title", "body"],
                "additionalProperties": false
            },
            "annotations": {
                "title": "Create Note",
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "notes.delete",
            "title": "Delete Note",
            "description": "Delete a note by its ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The note ID to delete."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": {
                "title": "Delete Note",
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "notes.search",
            "title": "Search Notes",
            "description": "Full-text search across notes in the vault.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text." },
                    "tags": { "type": "string", "description": "Comma-separated tags to filter by." },
                    "keywords": { "type": "string", "description": "Comma-separated keywords to filter by." },
                    "limit": { "type": "integer", "default": 10 }
                },
                "additionalProperties": false
            },
            "annotations": {
                "title": "Search Notes",
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "notes.related",
            "title": "Related Notes",
            "description": "Find notes related to a given note. Returns ranked results with relevance scores.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note_id": { "type": "string", "description": "Note ID to find related notes for." },
                    "limit": { "type": "integer", "default": 5, "description": "Maximum results to return." }
                },
                "required": ["note_id"],
                "additionalProperties": false
            },
            "annotations": {
                "title": "Related Notes",
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "notes.import",
            "title": "Import Notes",
            "description": "Import Markdown files from local paths into the vault.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "File or directory paths to import."
                    }
                },
                "required": ["paths"],
                "additionalProperties": false
            },
            "annotations": {
                "title": "Import Notes",
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "index.rebuild",
            "title": "Rebuild Index",
            "description": "Rebuild the full-text search index from all notes in the vault.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": {
                "title": "Rebuild Index",
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        serde_json::json!({
            "name": "ask",
            "title": "Ask Question",
            "description": "Ask a direct question to the AI with local knowledge retrieval. Unlike chat.send, this is a one-shot Q&A without session persistence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question to ask."
                    }
                },
                "required": ["question"],
                "additionalProperties": false
            },
            "annotations": {
                "title": "Ask Question",
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": false
            }
        }),
    ]
}

// ─── Tool call implementations ───────────────────────────────────

async fn mcp_call_chat_send(context: &StorageContext, arguments: Value) -> Value {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ChatSendArgs {
        #[serde(default)]
        message: String,
        #[serde(default)]
        image_paths: Vec<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        create_new_session: bool,
    }

    let args: ChatSendArgs = match serde_json::from_value(arguments) {
        Ok(args) => args,
        Err(error) => {
            return mcp_tool_error(sanitize_error(&format!(
                "invalid chat.send arguments: {error}"
            )));
        }
    };

    // Phase 1: Load state, add user turn, persist – under lock.
    let prepared = {
        let _guard = context.chat_state_lock.lock().await;
        match prepare_chat_for_ai(
            context,
            args.session_id,
            args.message,
            if args.image_paths.is_empty() {
                None
            } else {
                Some(args.image_paths)
            },
            args.create_new_session,
            |_, _| (),
        )
        .await
        {
            Ok(ctx) => ctx,
            Err(error) => return mcp_tool_error(sanitize_error(&error.to_string())),
        }
    };
    // Lock is released – concurrent chat.new / chat.delete can proceed.

    // Phase 2: AI call (potentially slow) – no lock held.
    let answer = tokio::time::timeout(
        AI_CALL_TIMEOUT,
        ask_with_ai_with_context(
            context,
            prepared.prompt.clone(),
            Some(prepared.history.clone()),
            if prepared.images.is_empty() {
                None
            } else {
                Some(prepared.images.clone())
            },
            None,
            |_, _| (),
        ),
    )
    .await;

    // Phase 3: Persist assistant turn – under lock.
    let _guard = context.chat_state_lock.lock().await;
    match answer {
        Ok(Ok(answer)) => {
            let session_id = prepared.active_session_id.clone();
            match finalize_chat_with_ai_answer(context, prepared, answer).await {
                Ok(result) => {
                    let summary = format!(
                        "Assistant reply from session \"{}\":\n{}",
                        escape_xml_content(&result.session_title),
                        escape_xml_content(&result.answer.answer)
                    );
                    let structured =
                        serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
                    mcp_tool_success(summary, structured)
                }
                Err(error) => {
                    let _ = rollback_last_user_turn(context, &session_id).await;
                    mcp_tool_error(sanitize_error(&error.to_string()))
                }
            }
        },
        Ok(Err(error)) => {
            // Rollback the orphaned user message before returning the error
            let _ = rollback_last_user_turn(context, &prepared.active_session_id).await;
            mcp_tool_error(sanitize_error(&error.to_string()))
        }
        Err(_elapsed) => {
            // Rollback the orphaned user message before returning the timeout error
            let _ = rollback_last_user_turn(context, &prepared.active_session_id).await;
            mcp_tool_error("AI call timed out after 120 seconds".to_string())
        }
    }
}

fn mcp_call_chat_list_sessions(context: &StorageContext) -> Value {
    match load_chat_state_with_context(context) {
        Ok(state) => {
            let sessions = state
                .sessions
                .iter()
                .map(chat_session_overview)
                .collect::<Vec<_>>();
            let structured = serde_json::json!({
                "currentSessionId": state.current_session_id,
                "sessions": sessions
            });
            let count = structured["sessions"]
                .as_array()
                .map(|items| items.len())
                .unwrap_or(0);
            mcp_tool_success(
                format!("Loaded {count} VaultPilot chat session(s)."),
                structured,
            )
        }
        Err(error) => mcp_tool_error(sanitize_error(&error.to_string())),
    }
}

fn mcp_call_chat_get_state(context: &StorageContext) -> Value {
    match load_chat_state_with_context(context) {
        Ok(state) => {
            let structured = serde_json::to_value(state).unwrap_or_else(|_| serde_json::json!({}));
            mcp_tool_success(
                "Loaded persisted VaultPilot chat state.".to_string(),
                structured,
            )
        }
        Err(error) => mcp_tool_error(sanitize_error(&error.to_string())),
    }
}

async fn mcp_call_chat_new(context: &StorageContext, arguments: Value) -> Value {
    #[derive(Deserialize)]
    struct Args {
        #[serde(default)]
        title: Option<String>,
    }
    let args: Args = match serde_json::from_value(arguments) {
        Ok(a) => a,
        Err(e) => {
            return mcp_tool_error(sanitize_error(&format!("invalid chat.new arguments: {e}")))
        }
    };
    let _guard = context.chat_state_lock.lock().await;
    match load_chat_state_with_context(context) {
        Ok(mut state) => {
            let session = new_cli_chat_session(args.title.as_deref());
            state.current_session_id = session.id.clone();
            state.sessions.insert(0, session.clone());
            match save_chat_state_with_context(context, &state) {
                Ok(_) => mcp_tool_success(
                    format!("Created session '{}'", escape_xml_content(&session.title)),
                    serde_json::json!({ "session": session }),
                ),
                Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
            }
        }
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

async fn mcp_call_chat_delete(context: &StorageContext, arguments: Value) -> Value {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        session_id: String,
    }
    let args: Args = match serde_json::from_value(arguments) {
        Ok(a) => a,
        Err(e) => {
            return mcp_tool_error(sanitize_error(&format!(
                "invalid chat.delete arguments: {e}"
            )))
        }
    };
    let _guard = context.chat_state_lock.lock().await;
    match load_chat_state_with_context(context) {
        Ok(mut state) => {
            let original_len = state.sessions.len();
            state.sessions.retain(|s| s.id != args.session_id);
            let deleted = state.sessions.len() != original_len;
            // Reset current_session_id if it points to a deleted or missing session
            if !state
                .sessions
                .iter()
                .any(|s| s.id == state.current_session_id)
            {
                state.current_session_id = state
                    .sessions
                    .first()
                    .map(|s| s.id.clone())
                    .unwrap_or_default();
            }
            match save_chat_state_with_context(context, &state) {
                Ok(_) => mcp_tool_success(
                    format!(
                        "Deleted={deleted}, id={}",
                        escape_xml_content(&args.session_id)
                    ),
                    serde_json::json!({ "deleted": deleted, "id": args.session_id }),
                ),
                Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
            }
        }
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

/// MCP schema sends flat fields (title, tags, keywords, ...) but NoteDocument
/// nests them under meta{}. This struct matches the MCP input shape.
#[derive(Deserialize)]
struct FlatNoteInput {
    #[serde(default)]
    body: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    board: String,
    #[serde(default)]
    kernel: String,
    #[serde(default)]
    status: String,
}

impl FlatNoteInput {
    fn into_note_document(self) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                title: self.title,
                summary: self.summary,
                tags: self.tags,
                keywords: self.keywords,
                platform: self.platform,
                board: self.board,
                kernel: self.kernel,
                status: self.status,
                ..Default::default()
            },
            body: self.body,
            ..Default::default()
        }
    }
}

fn mcp_call_notes_list(context: &StorageContext, arguments: Value) -> Value {
    // Storage layer clamps to 200 (storage.rs:558), so align the MCP cap.
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .min(200) as usize;
    match search_notes_with_context(
        context,
        SearchQuery {
            text: String::new(),
            tags: Vec::new(),
            keywords: Vec::new(),
            limit: Some(limit),
            ..Default::default()
        },
    ) {
        Ok(result) => {
            let count = result.notes.len();
            mcp_tool_success(
                format!("Found {count} note(s)."),
                serde_json::to_value(&result).unwrap_or_default(),
            )
        }
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

fn mcp_call_notes_get(context: &StorageContext, arguments: Value) -> Value {
    let id = match arguments.get("id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return mcp_tool_error("notes.get requires 'id' parameter".to_string()),
    };
    match load_note_with_context(context, &id) {
        Ok(note) => mcp_tool_success(
            format!("Loaded note '{}'", escape_xml_content(&note.meta.title)),
            serde_json::to_value(&note).unwrap_or_default(),
        ),
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

fn mcp_call_notes_create(context: &StorageContext, arguments: Value) -> Value {
    let flat: FlatNoteInput = match serde_json::from_value(arguments) {
        Ok(n) => n,
        Err(e) => {
            return mcp_tool_error(sanitize_error(&format!(
                "invalid notes.create arguments: {e}"
            )))
        }
    };
    if flat.title.trim().is_empty() {
        return mcp_tool_error("notes.create requires a non-empty 'title'".to_string());
    }
    if flat.body.trim().is_empty() {
        return mcp_tool_error("notes.create requires a non-empty 'body'".to_string());
    }
    let note = flat.into_note_document();
    match save_note_with_context(context, note) {
        Ok(saved) => mcp_tool_success(
            format!("Created note '{}'", escape_xml_content(&saved.meta.title)),
            serde_json::to_value(&saved).unwrap_or_default(),
        ),
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

fn mcp_call_notes_delete(context: &StorageContext, arguments: Value) -> Value {
    let id = match arguments.get("id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return mcp_tool_error("notes.delete requires 'id' parameter".to_string()),
    };
    match delete_note_with_context(context, &id) {
        Ok(deleted) => mcp_tool_success(
            format!("Deleted={deleted}, id={}", escape_xml_content(&id)),
            serde_json::json!({ "deleted": deleted, "id": id }),
        ),
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

fn mcp_call_notes_search(context: &StorageContext, arguments: Value) -> Value {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tags_str = arguments.get("tags").and_then(Value::as_str).unwrap_or("");
    let keywords_str = arguments
        .get("keywords")
        .and_then(Value::as_str)
        .unwrap_or("");
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .min(200) as usize;
    let parse_csv = |s: &str| {
        s.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    };
    match search_notes_with_context(
        context,
        SearchQuery {
            text: query,
            tags: parse_csv(tags_str),
            keywords: parse_csv(keywords_str),
            limit: Some(limit),
            ..Default::default()
        },
    ) {
        Ok(result) => {
            let count = result.notes.len();
            mcp_tool_success(
                format!("Found {count} note(s)."),
                serde_json::to_value(&result).unwrap_or_default(),
            )
        }
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

fn mcp_call_notes_related(context: &StorageContext, arguments: Value) -> Value {
    let note_id = match arguments.get("note_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return mcp_tool_error("notes.related requires 'note_id' parameter".to_string()),
    };
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .min(20) as usize;
    match find_related_notes_with_context(context, &note_id, limit) {
        Ok(results) => {
            let count = results.len();
            mcp_tool_success(
                format!("Found {count} related note(s)."),
                serde_json::to_value(&results).unwrap_or_default(),
            )
        }
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

fn mcp_call_notes_import(context: &StorageContext, arguments: Value) -> Value {
    let paths: Vec<String> = match arguments.get("paths") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(p) => p,
            Err(e) => return mcp_tool_error(sanitize_error(&format!("invalid paths: {e}"))),
        },
        None => return mcp_tool_error("notes.import requires 'paths' parameter".to_string()),
    };
    // #1826: Validate that all import paths are confined within the vault
    // directory to prevent path traversal attacks via MCP.
    let vault_dir = context.vault_dir();
    let vault_canonical = match vault_dir.canonicalize() {
        Ok(v) => v,
        Err(e) => {
            return mcp_tool_error(sanitize_error(&format!(
                "cannot resolve vault directory: {e}"
            )))
        }
    };
    for raw_path in &paths {
        let candidate = std::path::Path::new(raw_path);
        let resolved = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            vault_dir.join(candidate)
        };
        if let Ok(canonical) = resolved.canonicalize() {
            if !canonical.starts_with(&vault_canonical) {
                return mcp_tool_error(format!(
                    "import path '{}' is outside the vault directory",
                    raw_path
                ));
            }
        } else {
            // File does not exist — walk up ancestors to find the nearest
            // existing directory and verify it is inside the vault (#1844).
            let mut probe = resolved.as_path();
            let mut confined = false;
            while let Some(parent) = probe.parent() {
                if parent.as_os_str().is_empty() {
                    break;
                }
                if parent.exists() {
                    if let Ok(pc) = parent.canonicalize() {
                        if !pc.starts_with(&vault_canonical) {
                            return mcp_tool_error(format!(
                                "import path '{}' is outside the vault directory",
                                raw_path
                            ));
                        }
                        confined = true;
                    }
                    break;
                }
                probe = parent;
            }
            if !confined {
                return mcp_tool_error(format!(
                    "cannot verify import path '{}' is inside the vault directory",
                    raw_path
                ));
            }
        }
    }
    match import_markdown_with_context(context, &paths) {
        Ok(result) => mcp_tool_success(
            "Import completed.".to_string(),
            serde_json::to_value(&result).unwrap_or_default(),
        ),
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

fn mcp_call_index_rebuild(context: &StorageContext) -> Value {
    match rebuild_index_with_context(context) {
        Ok(stats) => mcp_tool_success(
            "Index rebuilt successfully.".to_string(),
            serde_json::to_value(&stats).unwrap_or_default(),
        ),
        Err(e) => mcp_tool_error(sanitize_error(&e.to_string())),
    }
}

async fn mcp_call_ask(context: &StorageContext, arguments: Value) -> Value {
    let question = match arguments.get("question").and_then(Value::as_str) {
        Some(q) => q.to_string(),
        None => return mcp_tool_error("ask requires 'question' parameter".to_string()),
    };
    match tokio::time::timeout(
        AI_CALL_TIMEOUT,
        ask_with_ai_with_context(context, question, None, None, None, |_, _| ()),
    )
    .await
    {
        Ok(Ok(answer)) => {
            let summary = format!("Answer: {}", escape_xml_content(&answer.answer));
            mcp_tool_success(summary, serde_json::to_value(&answer).unwrap_or_default())
        }
        Ok(Err(e)) => mcp_tool_error(sanitize_error(&e.to_string())),
        Err(_elapsed) => mcp_tool_error("AI call timed out after 120 seconds".to_string()),
    }
}

fn mcp_tool_success(summary: String, structured: Value) -> Value {
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": summary
            }
        ],
        "structuredContent": structured
    })
}

fn mcp_tool_error(message: String) -> Value {
    let structured_message = message.clone();
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": message
            }
        ],
        "structuredContent": {
            "error": structured_message
        },
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── negotiate_mcp_protocol_version ────────────────────────────

    #[test]
    fn negotiate_exact_version() {
        assert_eq!(
            negotiate_mcp_protocol_version(MCP_PROTOCOL_VERSION),
            MCP_PROTOCOL_VERSION
        );
    }

    #[test]
    fn negotiate_fallback_version() {
        assert_eq!(
            negotiate_mcp_protocol_version(MCP_FALLBACK_PROTOCOL_VERSION),
            MCP_FALLBACK_PROTOCOL_VERSION
        );
    }

    #[test]
    fn negotiate_unknown_defaults_to_current() {
        assert_eq!(
            negotiate_mcp_protocol_version("9999-01-01"),
            MCP_PROTOCOL_VERSION
        );
    }

    #[test]
    fn negotiate_empty_defaults_to_current() {
        assert_eq!(negotiate_mcp_protocol_version(""), MCP_PROTOCOL_VERSION);
    }

    // ── escape_xml_content ────────────────────────────────────────

    #[test]
    fn escape_plain_text() {
        assert_eq!(escape_xml_content("hello world"), "hello world");
    }

    #[test]
    fn escape_closing_tag() {
        assert_eq!(escape_xml_content("</secret>"), "<//secret>");
    }

    #[test]
    fn escape_user_content_tag() {
        assert_eq!(escape_xml_content("<user_content>"), "< user_content>");
    }

    #[test]
    fn escape_user_content_with_attr() {
        assert_eq!(
            escape_xml_content("<user_content x=1>"),
            "< user_content x=1>"
        );
    }

    #[test]
    fn escape_regular_opening_tag_unchanged() {
        assert_eq!(escape_xml_content("<bold>"), "<bold>");
    }

    #[test]
    fn escape_multiple_threats() {
        let input = "</a><user_content>safe<b>";
        let escaped = escape_xml_content(input);
        assert!(escaped.contains("<//a>"));
        assert!(escaped.contains("< user_content>"));
        assert!(escaped.contains("<b>")); // opening tags not targeted
    }

    #[test]
    fn escape_empty_string() {
        assert_eq!(escape_xml_content(""), "");
    }

    // ── sanitize_mcp_prompt_content ───────────────────────────────

    #[test]
    fn sanitize_wraps_in_delimiters() {
        let result = sanitize_mcp_prompt_content("hello");
        assert!(result.starts_with("<user_content>\n"));
        assert!(result.ends_with("\n</user_content>"));
        assert!(result.contains("hello"));
    }

    #[test]
    fn sanitize_escapes_injection() {
        let result = sanitize_mcp_prompt_content("</user_content>INJECT");
        assert!(result.contains("<//user_content>"));
        assert!(!result.contains("</user_content>INJECT"));
    }

    // ── mcp_tool_success / mcp_tool_error ─────────────────────────

    #[test]
    fn tool_success_structure() {
        let val = mcp_tool_success("summary".into(), serde_json::json!({"key": "value"}));
        assert_eq!(val["content"][0]["type"], "text");
        assert_eq!(val["content"][0]["text"], "summary");
        assert_eq!(val["structuredContent"]["key"], "value");
        assert!(val.get("isError").is_none());
    }

    #[test]
    fn tool_error_structure() {
        let val = mcp_tool_error("something broke".into());
        assert_eq!(val["content"][0]["type"], "text");
        assert_eq!(val["content"][0]["text"], "something broke");
        assert_eq!(val["structuredContent"]["error"], "something broke");
        assert_eq!(val["isError"], true);
    }

    // ── mcp_tools() ───────────────────────────────────────────────

    #[test]
    fn mcp_tools_count() {
        let tools = mcp_tools();
        assert_eq!(tools.len(), 14);
    }

    #[test]
    fn mcp_tools_each_has_required_fields() {
        for tool in mcp_tools() {
            assert!(tool.get("name").is_some(), "tool missing 'name'");
            assert!(
                tool.get("description").is_some(),
                "tool missing 'description'"
            );
            assert!(
                tool.get("inputSchema").is_some(),
                "tool missing 'inputSchema'"
            );
        }
    }

    #[test]
    fn mcp_tools_names_unique() {
        let tools = mcp_tools();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len(), "duplicate tool names found");
    }

    // ── McpResponse ───────────────────────────────────────────────

    #[test]
    fn mcp_response_ok_structure() {
        let resp = McpResponse::ok(serde_json::json!(1), serde_json::json!({"result": "ok"}));
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn mcp_response_error_structure() {
        let resp = McpResponse::error(serde_json::json!(1), -32600, "bad request".into(), None);
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32600);
    }

    // ── MCP HTTP server binding validation (regression for #1306) ──

    #[test]
    fn mcp_http_rejects_non_loopback_without_token() {
        // Non-loopback binding without token must fail (same as HTTP bridge)
        let result =
            crate::http_bridge::validate_http_bridge_binding("192.168.1.1".parse().unwrap(), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("--token"));
    }

    #[test]
    fn mcp_http_allows_loopback_without_token() {
        let result =
            crate::http_bridge::validate_http_bridge_binding("127.0.0.1".parse().unwrap(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn mcp_http_allows_non_loopback_with_token() {
        let result = crate::http_bridge::validate_http_bridge_binding(
            "192.168.1.1".parse().unwrap(),
            Some("secret"),
        );
        assert!(result.is_ok());
    }

    // ── sanitize_mcp_prompt_content boundary ──────────────────────

    #[test]
    fn sanitize_empty_content() {
        let result = sanitize_mcp_prompt_content("");
        assert!(result.starts_with("<user_content>\n"));
        assert!(result.ends_with("\n</user_content>"));
        // The content between delimiters should be empty
        let inner = &result["<user_content>\n".len()..result.len() - "\n</user_content>".len()];
        assert_eq!(inner, "");
    }

    #[test]
    fn sanitize_nested_injection_attempt() {
        // Try to break out and re-enter: </user_content>...<user_content>
        let input = "</user_content>INJECTED<script>alert(1)</script><user_content>";
        let result = sanitize_mcp_prompt_content(input);
        // Closing tag should be escaped
        assert!(result.contains("<//user_content>"));
        // Opening wrapper tag should be escaped
        assert!(result.contains("< user_content>"));
        // Should not contain unescaped breakout
        assert!(!result.contains("\n</user_content>INJECTED"));
    }

    #[test]
    fn sanitize_unicode_content() {
        let result = sanitize_mcp_prompt_content("你好世界 🌍 café résumé");
        assert!(result.contains("你好世界"));
        assert!(result.contains("🌍"));
        assert!(result.contains("café"));
    }

    // ── escape_xml_content boundary ───────────────────────────────

    #[test]
    fn escape_nested_closing_tags() {
        let input = "</a></b></c>";
        let result = escape_xml_content(input);
        assert_eq!(result, "<//a><//b><//c>");
    }

    #[test]
    fn escape_mixed_threats_and_safe_content() {
        let input = "Hello </inject> world <user_content> safe <b>bold</b>";
        let result = escape_xml_content(input);
        assert!(result.contains("<//inject>"));
        assert!(result.contains("< user_content>"));
        assert!(result.contains("<b>"));
        assert!(result.contains("<//b>")); // ALL closing tags are escaped
    }

    #[test]
    fn escape_long_content_no_panic() {
        let input = "x".repeat(100_000);
        let result = escape_xml_content(&input);
        assert_eq!(result.len(), 100_000);
    }

    // ── mcp_tool_success / mcp_tool_error boundary ────────────────

    #[test]
    fn tool_success_empty_summary() {
        let val = mcp_tool_success(String::new(), serde_json::json!({}));
        assert_eq!(val["content"][0]["text"], "");
        assert_eq!(val["structuredContent"], serde_json::json!({}));
    }

    #[test]
    fn tool_error_empty_message() {
        let val = mcp_tool_error(String::new());
        assert_eq!(val["content"][0]["text"], "");
        assert_eq!(val["isError"], true);
    }

    #[test]
    fn tool_error_special_characters() {
        let val = mcp_tool_error("Error: <script>alert('xss')</script>".into());
        assert_eq!(
            val["content"][0]["text"],
            "Error: <script>alert('xss')</script>"
        );
        assert_eq!(val["isError"], true);
    }

    // ── McpResponse boundary ──────────────────────────────────────

    #[test]
    fn mcp_response_error_with_data() {
        let resp = McpResponse::error(
            serde_json::json!(1),
            -32600,
            "bad request".into(),
            Some(serde_json::json!({"detail": "missing param"})),
        );
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.error.is_some());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, -32600);
        assert!(err.data.is_some());
    }

    #[test]
    fn mcp_response_ok_with_null_id() {
        let resp = McpResponse::ok(serde_json::json!(null), serde_json::json!({"ok": true}));
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_some());
        assert_eq!(resp.id, serde_json::json!(null));
    }

    // ── negotiate_mcp_protocol_version boundary ───────────────────

    #[test]
    fn negotiate_partial_date_format() {
        // A version string that looks like a date but isn't recognized
        let result = negotiate_mcp_protocol_version("2025-01");
        assert_eq!(result, MCP_PROTOCOL_VERSION);
    }

    #[test]
    fn negotiate_whitespace_only() {
        let result = negotiate_mcp_protocol_version("   ");
        assert_eq!(result, MCP_PROTOCOL_VERSION);
    }

    // ── FlatNoteInput → NoteDocument (#1506) ─────────────────────

    #[test]
    fn flat_note_input_preserves_all_meta() {
        let json = serde_json::json!({
            "body": "hello",
            "title": "My Title",
            "summary": "brief",
            "tags": ["rust", "cli"],
            "keywords": ["tool"],
            "platform": "linux",
            "board": "main",
            "kernel": "6.1",
            "status": "active"
        });
        let flat: FlatNoteInput = serde_json::from_value(json).unwrap();
        let doc = flat.into_note_document();
        assert_eq!(doc.body, "hello");
        assert_eq!(doc.meta.title, "My Title");
        assert_eq!(doc.meta.summary, "brief");
        assert_eq!(doc.meta.tags, vec!["rust", "cli"]);
        assert_eq!(doc.meta.keywords, vec!["tool"]);
        assert_eq!(doc.meta.platform, "linux");
        assert_eq!(doc.meta.board, "main");
        assert_eq!(doc.meta.kernel, "6.1");
        assert_eq!(doc.meta.status, "active");
    }

    #[test]
    fn flat_note_input_defaults_missing_fields() {
        let json = serde_json::json!({ "body": "content only" });
        let flat: FlatNoteInput = serde_json::from_value(json).unwrap();
        let doc = flat.into_note_document();
        assert_eq!(doc.body, "content only");
        assert_eq!(doc.meta.title, "");
        assert!(doc.meta.tags.is_empty());
    }

    #[test]
    fn old_note_document_loses_flat_fields() {
        // Regression: direct deserialization of flat JSON into NoteDocument
        // silently drops title/tags/keywords (old buggy behavior).
        let json = serde_json::json!({
            "body": "hello",
            "title": "Should Be Lost",
            "tags": ["a"]
        });
        let doc: NoteDocument = serde_json::from_value(json).unwrap();
        assert_eq!(doc.body, "hello");
        // These are the BUG: title and tags are silently dropped
        assert_eq!(doc.meta.title, ""); // was lost under old code
        assert!(doc.meta.tags.is_empty()); // was lost under old code
    }
}
