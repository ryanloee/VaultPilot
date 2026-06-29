//! VaultPilot MCP Connector -- Standalone Stdio MCP Server

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing_subscriber::EnvFilter;

use vaultpilot_lib::models::SearchQuery;
use vaultpilot_lib::storage::{
    list_collections_with_context, list_notes_in_collection_with_context,
    load_note_with_context, save_note_with_context,
    search_notes_with_context, StorageContext,
};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_MCP_LINE_BYTES: usize = 10 * 1024 * 1024;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let vault_dir = std::env::args()
        .position(|a| a == "--vault-dir")
        .and_then(|i| std::env::args().nth(i + 1))
        .or_else(discover_vault_dir_from_config);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_mcp_stdio(vault_dir))
}

fn discover_vault_dir_from_config() -> Option<String> {
    let candidates = [
        std::env::current_dir().ok()?,
        dirs_or_home().join("Documents").join("VaultPilotVault"),
        dirs_or_home().join(".vaultpilot"),
    ];

    for base in &candidates {
        for dir in &[base.clone(), base.join(".vaultpilot")] {
            let config_path = dir.join("mcp-config.json");
            if config_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&config_path) {
                    if let Ok(config) = serde_json::from_str::<McpConfig>(&content) {
                        if let Some(ref vault) = config.vault_dir {
                            if !vault.is_empty() {
                                tracing::info!("discovered vault from {}: {}", config_path.display(), vault);
                                return Some(vault.clone());
                            }
                        }
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
#[allow(dead_code)]
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
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }

    fn error(id: Value, code: i32, message: String, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(McpError { code, message, data }),
        }
    }
}

// --- Main Loop ---

async fn run_mcp_stdio(vault_dir: Option<String>) -> Result<()> {
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

    eprintln!("VaultPilot MCP Connector started");
    eprintln!("  vault dir: {}", context.vault_dir().display());
    eprintln!("  protocol: MCP {}", MCP_PROTOCOL_VERSION);

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
                    let id = request.id.unwrap_or(Value::Null);
                    let requested_version = request
                        .params
                        .get("protocolVersion")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    state.initialized = true;
                    state.protocol_version =
                        negotiate_protocol(requested_version).to_string();
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
                            "instructions": "VaultPilot MCP Connector provides tools to search, read, write, list, and find related notes in your vault."
                        }),
                    ))
                } else {
                    handle_request(&context, &state, request)
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

// --- Request Handler (sync, called from async context) ---

fn handle_request(
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
        "ping" => {
            request
                .id
                .map(|id| McpResponse::ok(id, serde_json::json!({})))
        }
        "tools/list" => {
            let id = request.id.unwrap_or(Value::Null);
            Some(McpResponse::ok(id, serde_json::json!({ "tools": mcp_tools() })))
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
                "vault_search" => handle_vault_search(context, arguments),
                "vault_read" => handle_vault_read(context, arguments),
                "vault_write" => handle_vault_write(context, arguments),
                "vault_list" => handle_vault_list(context, arguments),
                "vault_related" => handle_vault_related(context, arguments),
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
            let cursor = request
                .params
                .get("cursor")
                .and_then(Value::as_str)
                .unwrap_or("");
            let offset = cursor.parse::<usize>().unwrap_or(0).min(usize::MAX / 2);
            let limit: usize = 50;

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
                    Some(McpResponse::ok(id, payload))
                }
                Err(e) => Some(McpResponse::error(
                    id,
                    -32603,
                    format!("failed to list resources: {e}"),
                    None,
                )),
            }
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
    };

    match save_note_with_context(context, note) {
        Ok(saved) => {
            if let Some(ref coll_name) = collection {
                // Try to find existing collection by name
                let coll_id = list_collections_with_context(context)
                    .ok()
                    .and_then(|colls| {
                        colls.into_iter()
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
    let offset = arguments
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    if let Some(ref coll_name) = arguments.get("collection").and_then(Value::as_str).map(String::from) {
        // Find collection by name
        match list_collections_with_context(context) {
            Ok(collections) => {
                if let Some(coll) = collections.into_iter().find(|c| c.name == *coll_name) {
                    match list_notes_in_collection_with_context(context, &coll.id, limit, offset) {
                        Ok(notes) => {
                            let result: Vec<Value> = notes.into_iter().map(|meta| {
                                serde_json::json!({
                                    "id": meta.id,
                                    "title": meta.title,
                                    "summary": meta.summary,
                                    "tags": meta.tags,
                                })
                            }).collect();
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
