//! VaultPilot MCP Connector -- Standalone Stdio MCP Server

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing_subscriber::EnvFilter;

use vaultpilot_lib::models::SearchQuery;
use vaultpilot_lib::storage::{
    list_collections_with_context, list_notes_in_collection_with_context, load_note_with_context,
    save_note_with_context, search_notes_with_context, StorageContext,
};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_MCP_LINE_BYTES: usize = 10 * 1024 * 1024;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let arg_vault_dir = std::env::args()
        .position(|a| a == "--vault-dir")
        .and_then(|i| std::env::args().nth(i + 1));
    let arg_token = std::env::args()
        .position(|a| a == "--token")
        .and_then(|i| std::env::args().nth(i + 1));

    let discovered = discover_mcp_config();
    let vault_dir = arg_vault_dir.or_else(|| {
        discovered
            .as_ref()
            .and_then(|c| c.vault_dir.clone())
            .map(|v| {
                tracing::info!("discovered vault from mcp-config.json: {}", v);
                v
            })
    });
    // Expected token precedence: CLI arg > mcp-config.json. The proof side is
    // the VAULTPILOT_MCP_TOKEN env var injected by the MCP client (or the
    // initialize params `_meta` field, checked per-request).
    let expected_token = arg_token.or_else(|| {
        discovered
            .as_ref()
            .and_then(|c| c.token.clone())
            .inspect(|_| tracing::info!("token requirement loaded from mcp-config.json"))
    });

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_mcp_stdio(vault_dir, expected_token))
}

/// Constant-time byte-slice equality (avoids leaking token prefix matches
/// through timing). Length mismatch returns early — acceptable here, matching
/// the http_bridge implementation.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Decide whether this launch / initialize is authorized.
///
/// - `expected: None`  — auth not configured; allow (legacy behavior, caller
///   warns).
/// - `expected: Some`  — allow only if `provided` (env var at launch, or the
///   initialize `_meta` field) matches in constant time.
fn authorize(expected: Option<&str>, provided: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    match provided {
        Some(provided) if constant_time_eq(provided.as_bytes(), expected.as_bytes()) => Ok(()),
        Some(_) => Err(
            "unauthorized: VAULTPILOT_MCP_TOKEN does not match the configured token".to_string(),
        ),
        None => Err(
            "unauthorized: token required but not provided (set VAULTPILOT_MCP_TOKEN in the client's env)"
                .to_string(),
        ),
    }
}

/// Extract the token proof an MCP client may attach to `initialize` params
/// under the reserved `_meta` extension object.
fn init_meta_token(params: &Value) -> Option<&str> {
    params
        .get("_meta")
        .and_then(|m| m.get("vaultpilotToken"))
        .and_then(Value::as_str)
}

fn discover_mcp_config() -> Option<McpConfig> {
    let candidates = [
        std::env::current_dir().unwrap_or_default(),
        dirs_or_home().join("Documents").join("VaultPilotVault"),
        dirs_or_home().join(".vaultpilot"),
    ];

    for base in &candidates {
        for dir in &[base.clone(), base.join(".vaultpilot")] {
            let config_path = dir.join("mcp-config.json");
            if config_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&config_path) {
                    if let Ok(config) = serde_json::from_str::<McpConfig>(&content) {
                        return Some(config);
                    }
                }
            }
        }
    }
    None
}

fn dirs_or_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| {
            tracing::warn!("HOME/USERPROFILE unset, using current dir");
            std::env::current_dir().unwrap_or_default()
        })
}

#[derive(Debug, Deserialize)]
struct McpConfig {
    #[serde(default)]
    vault_dir: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

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

// --- Main Loop ---

async fn run_mcp_stdio(vault_dir: Option<String>, expected_token: Option<String>) -> Result<()> {
    let context = match vault_dir {
        Some(ref dir) => {
            let path = PathBuf::from(dir);
            if !path.exists() {
                anyhow::bail!("vault directory does not exist: {}", path.display());
            }
            StorageContext::for_cli(Some(path))
                .context("failed to initialize storage for vault directory")?
        }
        None => StorageContext::for_sidecar().context("failed to initialize storage")?,
    };

    vaultpilot_lib::storage::initialize_storage_with_context(&context)
        .context("failed to initialize storage schema")?;

    // Launch-time fast-fail: a *present but wrong* env proof kills the server
    // immediately. A missing env proof is fine at this point — the client may
    // instead present the token per-request in initialize `_meta`.
    if let Ok(env_proof) = std::env::var("VAULTPILOT_MCP_TOKEN") {
        authorize(expected_token.as_deref(), Some(env_proof.as_str()))
            .map_err(|reason| anyhow::anyhow!(reason))?;
    }

    eprintln!("VaultPilot MCP Connector started");
    eprintln!("  vault dir: {}", context.vault_dir().display());
    eprintln!("  protocol: MCP {}", MCP_PROTOCOL_VERSION);
    match expected_token.as_deref() {
        Some(_) => eprintln!("  auth: token required (VAULTPILOT_MCP_TOKEN / initialize _meta)"),
        None => eprintln!(
            "  auth: WARNING no token configured — any local process can read this vault. \
             Set \"token\" in mcp-config.json or pass --token to enable."
        ),
    }

    let mut state = McpServerState::default();
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
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
        let line: Option<String> = tokio::select! {
            result = read_next_line(&mut lines) => {
                match result {
                    Ok(Some(line)) => Some(line),
                    Ok(None) => None,
                    Err(e) => {
                        eprintln!("MCP stdin read error: {e}");
                        None
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                eprintln!("MCP server: received shutdown signal");
                break;
            }
        };

        let line = match line {
            Some(l) => l,
            None => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<McpRequest>(&line) {
            Ok(request) => {
                if request.method == "initialize" && request.jsonrpc == "2.0" {
                    let id = request.id.clone().unwrap_or(Value::Null);
                    // Per-request auth: accept either proof — the env var
                    // (already fast-fail-checked at launch, so present = valid)
                    // or the initialize params `_meta` token. On failure
                    // `initialized` stays false, so every later tools/call is
                    // rejected by the -32002 gate below.
                    let env_proof = std::env::var("VAULTPILOT_MCP_TOKEN").ok();
                    let provided = env_proof
                        .as_deref()
                        .or_else(|| init_meta_token(&request.params));
                    if let Err(reason) = authorize(expected_token.as_deref(), provided) {
                        Some(McpResponse::error(id, -32600, reason, None))
                    } else {
                        let requested_version = request
                            .params
                            .get("protocolVersion")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        state.initialized = true;
                        state.protocol_version = negotiate_protocol(requested_version).to_string();
                        Some(McpResponse::ok(
                            id,
                            serde_json::json!({
                                "protocolVersion": state.protocol_version,
                                "capabilities": {
                                    "tools": { "listChanged": false },
                                    "resources": { "listChanged": false }
                                },
                                "serverInfo": {
                                    "name": "vaultpilot-mcp",
                                    "title": "VaultPilot MCP Connector",
                                    "version": env!("CARGO_PKG_VERSION")
                                },
                                "instructions": "VaultPilot MCP Connector provides tools to search, read, write, list, and find related notes in your vault. Also includes GitHub connector tools (list/get issues) when GITHUB_TOKEN is configured."
                            }),
                        ))
                    }
                } else {
                    handle_request(&context, &state, request).await
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
            let mut stdout = tokio::io::stdout();
            let payload = serde_json::to_string(&response)?;
            stdout.write_all(payload.as_bytes()).await?;
            stdout.write_all(b"\x0a").await?;
            stdout.flush().await?;
        }
    }

    eprintln!("VaultPilot MCP Connector shut down cleanly");
    Ok(())
}

async fn read_next_line(
    lines: &mut tokio::io::Lines<BufReader<tokio::io::Stdin>>,
) -> Result<Option<String>> {
    match lines.next_line().await {
        Ok(Some(line)) => {
            if line.len() > MAX_MCP_LINE_BYTES {
                return Err(anyhow::anyhow!(
                    "stdin line exceeds {}MB limit",
                    MAX_MCP_LINE_BYTES / (1024 * 1024)
                ));
            }
            Ok(Some(line))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("stdin read error: {e}")),
    }
}

fn negotiate_protocol(requested: &str) -> &str {
    if requested.is_empty() {
        MCP_PROTOCOL_VERSION
    } else if requested == MCP_PROTOCOL_VERSION || requested == MCP_DEFAULT_PROTOCOL_VERSION {
        requested
    } else {
        MCP_PROTOCOL_VERSION
    }
}

// --- Request Handler (async, wrapped spawn_blocking for sync I/O) ---

async fn handle_request(
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

    if !state.initialized && request.method != "initialize" {
        return Some(McpResponse::error(
            request.id.unwrap_or(Value::Null),
            -32002,
            "server not initialized".to_string(),
            None,
        ));
    }

    match request.method.as_str() {
        "notifications/initialized" => None,
        "ping" => request
            .id
            .map(|id| McpResponse::ok(id, serde_json::json!({}))),
        "tools/list" => {
            let id = request.id.unwrap_or(Value::Null);
            Some(McpResponse::ok(
                id,
                serde_json::json!({ "tools": mcp_tools() }),
            ))
        }
        "tools/call" => {
            let id = request.id.unwrap_or(Value::Null);
            let tool_name = match request.params.get("name").and_then(Value::as_str) {
                Some(name) => name,
                None => {
                    return Some(McpResponse::error(
                        id,
                        -32602,
                        "tools/call requires a string params.name".to_string(),
                        None,
                    ));
                }
            };
            let arguments = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));

            let result = match tool_name {
                "vault_search" => {
                    let ctx = context.clone();
                    tokio::task::spawn_blocking(move || handle_vault_search(&ctx, arguments))
                        .await
                        .unwrap_or_else(|e| {
                            serde_json::json!({
                                "isError": true,
                                "content": [{"type": "text", "text": format!("task join failed: {e}")}],
                            })
                        })
                }
                "vault_read" => {
                    let ctx = context.clone();
                    tokio::task::spawn_blocking(move || handle_vault_read(&ctx, arguments))
                        .await
                        .unwrap_or_else(|e| {
                            serde_json::json!({
                                "isError": true,
                                "content": [{"type": "text", "text": format!("task join failed: {e}")}],
                            })
                        })
                }
                "vault_write" => {
                    let ctx = context.clone();
                    tokio::task::spawn_blocking(move || handle_vault_write(&ctx, arguments))
                        .await
                        .unwrap_or_else(|e| {
                            serde_json::json!({
                                "isError": true,
                                "content": [{"type": "text", "text": format!("task join failed: {e}")}],
                            })
                        })
                }
                "vault_list" => {
                    let ctx = context.clone();
                    tokio::task::spawn_blocking(move || handle_vault_list(&ctx, arguments))
                        .await
                        .unwrap_or_else(|e| {
                            serde_json::json!({
                                "isError": true,
                                "content": [{"type": "text", "text": format!("task join failed: {e}")}],
                            })
                        })
                }
                "vault_related" => {
                    let ctx = context.clone();
                    tokio::task::spawn_blocking(move || handle_vault_related(&ctx, arguments))
                        .await
                        .unwrap_or_else(|e| {
                            serde_json::json!({
                                "isError": true,
                                "content": [{"type": "text", "text": format!("task join failed: {e}")}],
                            })
                        })
                }
                "github_list_issues" => tokio::task::spawn_blocking(move || {
                    handle_github_list_issues(arguments)
                })
                .await
                .unwrap_or_else(|e| {
                    serde_json::json!({
                        "isError": true,
                        "content": [{"type": "text", "text": format!("task join failed: {e}")}],
                    })
                }),
                "github_get_issue" => tokio::task::spawn_blocking(move || {
                    handle_github_get_issue(arguments)
                })
                .await
                .unwrap_or_else(|e| {
                    serde_json::json!({
                        "isError": true,
                        "content": [{"type": "text", "text": format!("task join failed: {e}")}],
                    })
                }),
                _ => {
                    return Some(McpResponse::error(
                        id,
                        -32601,
                        format!("unknown tool: {tool_name}"),
                        None,
                    ));
                }
            };

            Some(McpResponse::ok(id, result))
        }
        "resources/list" => {
            let id = request.id.unwrap_or(Value::Null);
            let ctx = context.clone();
            let cursor = request
                .params
                .get("cursor")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let offset = cursor.parse::<usize>().unwrap_or(0).min(usize::MAX / 2);
            let limit: usize = 50;
            let id_for_blocking = id.clone();

            Some(
                tokio::task::spawn_blocking(move || {
                    match search_notes_with_context(
                        &ctx,
                        SearchQuery {
                            text: String::new(),
                            tags: Vec::new(),
                            keywords: Vec::new(),
                            limit: Some(limit),
                            offset: Some(offset),
                            ..Default::default()
                        },
                    ) {
                        Ok(result) => {
                            let resources: Vec<Value> = result
                                .notes
                                .into_iter()
                                .map(|meta| {
                                    serde_json::json!({
                                        "uri": format!("vault://notes/{}", meta.id),
                                        "name": meta.title,
                                        "description": if meta.summary.is_empty() { Value::Null } else { Value::String(meta.summary) },
                                        "mimeType": "text/markdown"
                                    })
                                })
                                .collect();
                            let next_offset = offset.saturating_add(resources.len());
                            let has_more = next_offset < result.total;
                            let mut payload = serde_json::json!({ "resources": resources });
                            if has_more {
                                payload["nextCursor"] = Value::String(next_offset.to_string());
                            }
                            McpResponse::ok(id_for_blocking, payload)
                        }
                        Err(e) => McpResponse::error(
                            id_for_blocking,
                            -32603,
                            format!("failed to list resources: {e}"),
                            None,
                        ),
                    }
                })
                .await
                .unwrap_or_else(|e| {
                    McpResponse::error(
                        id,
                        -32603,
                        format!("task join failed: {e}"),
                        None,
                    )
                }),
            )
        }
        _ => Some(McpResponse::error(
            request.id.unwrap_or(Value::Null),
            -32601,
            format!("unknown method: {}", request.method),
            None,
        )),
    }
}

// --- Tool Definitions ---

fn mcp_tools() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "vault_search",
            "description": "Search notes in the vault by text query. Returns matching note metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search text (keywords, phrases, or full-text search)"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of results (default: 10, max: 50)",
                        "default": 10
                    }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "vault_read",
            "description": "Read the full content of a note by its ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note_id": {
                        "type": "string",
                        "description": "The note ID (UUID) to read"
                    }
                },
                "required": ["note_id"]
            }
        }),
        serde_json::json!({
            "name": "vault_write",
            "description": "Write a new note to the vault. Returns the created note ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Note title"
                    },
                    "content": {
                        "type": "string",
                        "description": "Note body content (Markdown)"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional tags for the note"
                    },
                    "collection": {
                        "type": "string",
                        "description": "Optional collection name to add the note to"
                    }
                },
                "required": ["title", "content"]
            }
        }),
        serde_json::json!({
            "name": "vault_list",
            "description": "List notes in the vault, optionally filtered by collection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Optional collection name to filter by"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of results (default: 20, max: 100)",
                        "default": 20
                    },
                    "offset": {
                        "type": "number",
                        "description": "Number of results to skip (for pagination)",
                        "default": 0
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "vault_related",
            "description": "Find notes related to a given note ID based on content similarity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note_id": {
                        "type": "string",
                        "description": "The note ID to find related notes for"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of related notes (default: 5, max: 20)",
                        "default": 5
                    }
                },
                "required": ["note_id"]
            }
        }),
        serde_json::json!({
            "name": "github_list_issues",
            "description": "List GitHub issues for a repository. Requires GITHUB_TOKEN env var or --github-token CLI arg.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "owner": {
                        "type": "string",
                        "description": "Repository owner (username or org)"
                    },
                    "repo": {
                        "type": "string",
                        "description": "Repository name"
                    },
                    "state": {
                        "type": "string",
                        "description": "Filter by state: open, closed, or all (default: open)",
                        "default": "open"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of issues to return (default: 10, max: 30)",
                        "default": 10
                    }
                },
                "required": ["owner", "repo"]
            }
        }),
        serde_json::json!({
            "name": "github_get_issue",
            "description": "Get details of a specific GitHub issue by number. Requires GITHUB_TOKEN env var or --github-token CLI arg.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "owner": {
                        "type": "string",
                        "description": "Repository owner (username or org)"
                    },
                    "repo": {
                        "type": "string",
                        "description": "Repository name"
                    },
                    "issue_number": {
                        "type": "number",
                        "description": "The issue number to fetch"
                    }
                },
                "required": ["owner", "repo", "issue_number"]
            }
        }),
    ]
}

// --- Tool Handlers (all sync, called from sync handle_request) ---

fn handle_vault_search(context: &StorageContext, arguments: Value) -> Value {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .min(50) as usize;

    match search_notes_with_context(
        context,
        SearchQuery {
            text: query,
            tags: Vec::new(),
            keywords: Vec::new(),
            limit: Some(limit),
            offset: Some(0),
            ..Default::default()
        },
    ) {
        Ok(result) => {
            let notes: Vec<Value> = result
                .notes
                .into_iter()
                .map(|meta| {
                    serde_json::json!({
                        "id": meta.id,
                        "title": meta.title,
                        "summary": meta.summary,
                        "tags": meta.tags,
                        "source": meta.source,
                        "createdAt": meta.created_at,
                        "updatedAt": meta.updated_at,
                    })
                })
                .collect();
            serde_json::json!({
                "content": notes,
                "total": result.total,
            })
        }
        Err(e) => {
            serde_json::json!({
                "isError": true,
                "content": [{"type": "text", "text": format!("search failed: {e}")}],
            })
        }
    }
}

fn handle_vault_read(context: &StorageContext, arguments: Value) -> Value {
    let note_id = arguments
        .get("note_id")
        .and_then(Value::as_str)
        .unwrap_or("");

    if note_id.is_empty() {
        return serde_json::json!({
            "isError": true,
            "content": [{"type": "text", "text": "note_id is required"}],
        });
    }

    match load_note_with_context(context, note_id) {
        Ok(note) => {
            serde_json::json!({
                "content": [{"type": "text", "text": note.body}],
                "meta": {
                    "id": note.meta.id,
                    "title": note.meta.title,
                    "summary": note.meta.summary,
                    "tags": note.meta.tags,
                    "source": note.meta.source,
                    "createdAt": note.meta.created_at,
                    "updatedAt": note.meta.updated_at,
                }
            })
        }
        Err(e) => {
            serde_json::json!({
                "isError": true,
                "content": [{"type": "text", "text": format!("failed to read note: {e}")}],
            })
        }
    }
}

fn handle_vault_write(context: &StorageContext, arguments: Value) -> Value {
    let title = arguments
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled")
        .to_string();
    let body = arguments
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tags: Vec<String> = arguments
        .get("tags")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let collection = arguments
        .get("collection")
        .and_then(Value::as_str)
        .map(String::from);

    let note = vaultpilot_lib::models::NoteDocument {
        meta: vaultpilot_lib::models::NoteMeta {
            title,
            tags,
            source: "mcp_connector".to_string(),
            ..Default::default()
        },
        body,
        search_snippet: None,
        search_score: None,
    };

    match save_note_with_context(context, note) {
        Ok(saved) => {
            if let Some(ref coll_name) = collection {
                // Try to find existing collection by name
                let coll_id = list_collections_with_context(context)
                    .ok()
                    .and_then(|colls| {
                        colls
                            .into_iter()
                            .find(|c| c.name == *coll_name)
                            .map(|c| c.id)
                    })
                    .or_else(|| {
                        // Create new collection
                        vaultpilot_lib::storage::create_collection_with_context(
                            context,
                            coll_name,
                            "Created by MCP connector",
                        )
                        .ok()
                        .map(|c| c.id)
                    });

                if let Some(id) = coll_id {
                    let _ = vaultpilot_lib::storage::add_note_to_collection_with_context(
                        context,
                        &saved.meta.id,
                        &id,
                    );
                }
            }
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Created note '{}' with ID: {}", saved.meta.title, saved.meta.id)
                }],
                "meta": {
                    "id": saved.meta.id,
                    "title": saved.meta.title,
                    "createdAt": saved.meta.created_at,
                }
            })
        }
        Err(e) => {
            serde_json::json!({
                "isError": true,
                "content": [{"type": "text", "text": format!("failed to save note: {e}")}],
            })
        }
    }
}

fn handle_vault_list(context: &StorageContext, arguments: Value) -> Value {
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .min(100) as usize;
    let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;

    if let Some(ref coll_name) = arguments
        .get("collection")
        .and_then(Value::as_str)
        .map(String::from)
    {
        // Find collection by name
        match list_collections_with_context(context) {
            Ok(collections) => {
                if let Some(coll) = collections.into_iter().find(|c| c.name == *coll_name) {
                    match list_notes_in_collection_with_context(context, &coll.id, limit, offset) {
                        Ok(notes) => {
                            let result: Vec<Value> = notes
                                .into_iter()
                                .map(|meta| {
                                    serde_json::json!({
                                        "id": meta.id,
                                        "title": meta.title,
                                        "summary": meta.summary,
                                        "tags": meta.tags,
                                    })
                                })
                                .collect();
                            return serde_json::json!({
                                "content": result,
                            });
                        }
                        Err(e) => {
                            return serde_json::json!({
                                "isError": true,
                                "content": [{"type": "text", "text": format!("failed to list collection: {e}")}],
                            });
                        }
                    }
                } else {
                    return serde_json::json!({
                        "content": [],
                        "total": 0,
                        "note": format!("collection '{}' not found", coll_name),
                    });
                }
            }
            Err(e) => {
                return serde_json::json!({
                    "isError": true,
                    "content": [{"type": "text", "text": format!("failed to list collections: {e}")}],
                });
            }
        }
    }

    // List all notes (no collection filter)
    match search_notes_with_context(
        context,
        SearchQuery {
            text: String::new(),
            tags: Vec::new(),
            keywords: Vec::new(),
            limit: Some(limit),
            offset: Some(offset),
            ..Default::default()
        },
    ) {
        Ok(result) => {
            let notes: Vec<Value> = result
                .notes
                .into_iter()
                .map(|meta| {
                    serde_json::json!({
                        "id": meta.id,
                        "title": meta.title,
                        "summary": meta.summary,
                        "tags": meta.tags,
                    })
                })
                .collect();
            serde_json::json!({
                "content": notes,
                "total": result.total,
            })
        }
        Err(e) => {
            serde_json::json!({
                "isError": true,
                "content": [{"type": "text", "text": format!("failed to list notes: {e}")}],
            })
        }
    }
}

fn handle_vault_related(context: &StorageContext, arguments: Value) -> Value {
    let note_id = arguments
        .get("note_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .min(20) as usize;

    if note_id.is_empty() {
        return serde_json::json!({
            "isError": true,
            "content": [{"type": "text", "text": "note_id is required"}],
        });
    }

    match vaultpilot_lib::storage::find_related_notes_with_context(context, note_id, limit) {
        Ok(notes) => {
            let related: Vec<Value> = notes
                .into_iter()
                .map(|related_note| {
                    serde_json::json!({
                        "id": related_note.meta.id,
                        "title": related_note.meta.title,
                        "summary": related_note.meta.summary,
                        "tags": related_note.meta.tags,
                        "relevance": "related",
                    })
                })
                .collect();
            serde_json::json!({ "content": related })
        }
        Err(e) => {
            serde_json::json!({
                "isError": true,
                "content": [{"type": "text", "text": format!("failed to find related notes: {e}")}],
            })
        }
    }
}

// --- GitHub Connector Handlers ---

/// Resolve GitHub token: CLI arg --github-token first, then GITHUB_TOKEN env var.
fn github_token_from_args() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--github-token") {
        if let Some(token) = args.get(pos + 1) {
            if !token.is_empty() {
                return Some(token.clone());
            }
        }
    }
    std::env::var("GITHUB_TOKEN").ok()
}

/// Call GitHub REST API with ureq and return MCP-style result Value.
fn github_api(path: &str) -> Value {
    let token = match github_token_from_args() {
        Some(t) => t,
        None => {
            return serde_json::json!({
                "isError": true,
                "content": [{"type": "text", "text": "GitHub token not configured. Set GITHUB_TOKEN env var or pass --github-token."}],
            })
        }
    };

    let url = format!("https://api.github.com{}", path);
    let mut resp = match ureq::get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "vaultpilot-mcp")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) => {
            return serde_json::json!({
                "isError": true,
                "content": [{"type": "text", "text": format!("GitHub API HTTP {} for {}", code, path)}],
            })
        }
        Err(e) => {
            return serde_json::json!({
                "isError": true,
                "content": [{"type": "text", "text": format!("GitHub API request failed: {e}")}],
            })
        }
    };

    match resp.body_mut().read_json::<Value>() {
        Ok(v) => serde_json::json!({
            "content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}],
        }),
        Err(e) => serde_json::json!({
            "isError": true,
            "content": [{"type": "text", "text": format!("Failed to parse GitHub response: {e}")}],
        }),
    }
}

fn handle_github_list_issues(arguments: Value) -> Value {
    let owner = arguments.get("owner").and_then(Value::as_str).unwrap_or("");
    let repo = arguments.get("repo").and_then(Value::as_str).unwrap_or("");
    let state = arguments
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("open");
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .min(30);

    if owner.is_empty() || repo.is_empty() {
        return serde_json::json!({
            "isError": true,
            "content": [{"type": "text", "text": "owner and repo are required"}],
        });
    }

    github_api(&format!(
        "/repos/{}/{}/issues?state={}&per_page={}&sort=updated&direction=desc",
        owner, repo, state, limit
    ))
}

fn handle_github_get_issue(arguments: Value) -> Value {
    let owner = arguments.get("owner").and_then(Value::as_str).unwrap_or("");
    let repo = arguments.get("repo").and_then(Value::as_str).unwrap_or("");
    let issue_number = arguments
        .get("issue_number")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    if owner.is_empty() || repo.is_empty() || issue_number == 0 {
        return serde_json::json!({
            "isError": true,
            "content": [{"type": "text", "text": "owner, repo, and issue_number are required"}],
        });
    }

    github_api(&format!(
        "/repos/{}/{}/issues/{}",
        owner, repo, issue_number
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Test-only stand-in secret — never a real credential.
    const TEST_TOKEN: &str = "test-token-standin";

    #[test]
    fn authorize_no_expected_allows_anything() {
        // Legacy behavior: without a configured token, launch is allowed.
        assert!(authorize(None, None).is_ok());
        assert!(authorize(None, Some("anything")).is_ok());
    }

    #[test]
    fn authorize_matching_token_passes() {
        assert!(authorize(Some(TEST_TOKEN), Some(TEST_TOKEN)).is_ok());
    }

    #[test]
    fn authorize_wrong_or_missing_token_rejected() {
        let wrong = authorize(Some(TEST_TOKEN), Some("wrong"));
        assert!(wrong.is_err());
        assert!(wrong.unwrap_err().contains("unauthorized"));

        let missing = authorize(Some(TEST_TOKEN), None);
        assert!(missing.is_err());
        assert!(missing.unwrap_err().contains("not provided"));
    }

    #[test]
    fn authorize_rejects_empty_string_proof() {
        // A client sending an empty token is "provided" but wrong.
        assert!(authorize(Some(TEST_TOKEN), Some("")).is_err());
    }

    #[test]
    fn init_meta_token_extracts_from_params() {
        let params = json!({
            "protocolVersion": "2025-06-18",
            "_meta": { "vaultpilotToken": TEST_TOKEN }
        });
        assert_eq!(init_meta_token(&params), Some(TEST_TOKEN));
    }

    #[test]
    fn init_meta_token_absent_meta_or_field() {
        assert_eq!(init_meta_token(&json!({ "protocolVersion": "x" })), None);
        assert_eq!(init_meta_token(&json!({ "_meta": { "other": "y" } })), None);
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_github_token_not_configured() {
        // Ensure GITHUB_TOKEN is not in env for this test (and restore it after).
        // Without this, the test makes a real network call and only passes by
        // coincidence because "foo/bar" is a 404.
        let saved = std::env::var_os("GITHUB_TOKEN");
        std::env::remove_var("GITHUB_TOKEN");
        let result = github_api("/repos/foo/bar/issues");
        if let Some(v) = saved {
            std::env::set_var("GITHUB_TOKEN", v);
        }
        assert!(
            result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "should return isError=true when no token configured"
        );
    }

    #[test]
    fn test_github_list_issues_missing_owner() {
        let result = handle_github_list_issues(json!({"repo": "bar"}));
        assert!(
            result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "should return error for missing owner"
        );
    }

    #[test]
    fn test_github_list_issues_missing_repo() {
        let result = handle_github_list_issues(json!({"owner": "foo"}));
        assert!(
            result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "should return error for missing repo"
        );
    }

    #[test]
    fn test_github_get_issue_missing_number() {
        let result = handle_github_get_issue(json!({"owner": "foo", "repo": "bar"}));
        assert!(
            result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "should return error when issue_number=0"
        );
    }

    #[test]
    fn test_github_tool_definitions_present() {
        let tools = mcp_tools();
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert!(
            names.contains(&"github_list_issues"),
            "github_list_issues tool should be present"
        );
        assert!(
            names.contains(&"github_get_issue"),
            "github_get_issue tool should be present"
        );
        assert!(
            names.contains(&"vault_search"),
            "vault tools should still be present"
        );
    }
}
