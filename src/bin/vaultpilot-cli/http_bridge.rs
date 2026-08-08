use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::convert::Infallible;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use uuid::Uuid;

use vaultpilot_lib::ai::actions::{
    execute_ai_action, list_ai_actions, AiActionRequest, AiActionType,
};
use vaultpilot_lib::models::*;
use vaultpilot_lib::storage::{
    bulk_delete_notes_async, bulk_move_notes_async, bulk_update_tags_async,
    deep_search_notes_async, delete_note_async, import_markdown_async, load_note_async,
    load_settings_async, save_note_async, search_notes_async, typeahead_search_async, NoteNotFound,
    StorageContext,
};
use vaultpilot_lib::storage::{
    create_subscription_async, delete_subscription_async, get_subscription_async,
    list_subscriptions_async, set_subscription_enabled_async, update_subscription_async,
};
use vaultpilot_lib::{ask_with_ai_with_context, normalize_tool_path, run_single_subscription};

/// Maximum total wall-clock time an upstream AI streaming request may run in
/// the HTTP bridge's `stream: true` path. The `TimeoutLayer(180s)` on the
/// router does NOT cover the SSE body stream (its Response future resolves
/// immediately for SSE), so the streaming task needs its own cap. Without it, a
/// stalled upstream + a non-disconnecting client holds a tokio task, two bounded
/// channels, and an upstream HTTP connection indefinitely. (#2128)
const STREAM_UPSTREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
/// Maximum wall-clock time for the progressive search (keyword + deep semantic).
/// Without this, a hung search (e.g. SQLite WAL stall, DB lock) leaks the tokio
/// task, the bounded channels, and the upstream HTTP connection. The router-level
/// TimeoutLayer(180s) does NOT cover the SSE body stream. (#2547)
const PROGRESSIVE_SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

// ─── Public entry point ────────────────────────────────────────────

pub(super) async fn run_http_bridge(
    context: StorageContext,
    host: String,
    port: u16,
    token: Option<String>,
) -> Result<()> {
    let ip: IpAddr = host
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid host '{}': {}", host, error))?;
    let token = normalize_bridge_token(token);
    validate_http_bridge_binding(ip, token.as_deref())?;
    let address = SocketAddr::new(ip, port);
    let requires_token = token.is_some();
    let rate_limiter = Arc::new(RateLimiter::new(60, std::time::Duration::from_secs(60)));
    let state = Arc::new(HttpBridgeState { context, token });

    let app = Router::new()
        .route("/health", get(http_health))
        .route("/v1/models", get(http_models))
        .route("/v1/chat/completions", post(http_chat_completions))
        .route("/api/notes", get(http_list_notes).post(http_create_note))
        // #3478: Batch folder import — recursively walk a directory and import
        // all .md files as notes. Enables WinUI drag-folder-into-window UX
        // without needing a dedicated batch-import protocol.
        .route("/api/notes/import-folder", post(http_import_folder))
        // #3034 Web Clipper: server-side URL → Markdown note (Plan B)
        .route("/api/clip", post(http_clip_url))
        .route("/api/notes/search", get(http_search_notes))
        .route("/api/notes/typeahead", get(http_typeahead))
        .route(
            "/api/notes/search/progressive",
            get(http_progressive_search),
        )
        .route(
            "/api/notes/{note_id}",
            get(http_get_note).delete(http_delete_note),
        )
        // #3514: Bulk note operations for file-browser multi-select.
        .route("/api/notes/bulk-delete", post(http_bulk_delete_notes))
        .route("/api/notes/bulk-move", post(http_bulk_move_notes))
        .route("/api/notes/bulk-tags", post(http_bulk_update_tags))
        // Vault Health Dashboard (#2014)
        .route("/api/vault/health", get(http_vault_health))
        // Knowledge Graph API (#3460) — expose vault note link graph as JSON
        .route("/api/graph", get(http_graph))
        // Subscriptions API (#2167)
        .route(
            "/api/subscriptions",
            get(http_list_subscriptions).post(http_create_subscription),
        )
        .route(
            "/api/subscriptions/{sub_id}",
            get(http_get_subscription)
                .delete(http_delete_subscription)
                .put(http_update_subscription),
        )
        .route(
            "/api/subscriptions/{sub_id}/run",
            post(http_run_subscription),
        )
        .route(
            "/api/subscriptions/{sub_id}/toggle",
            post(http_toggle_subscription),
        )
        // AI Action Palette (#2188)
        .route("/api/ai/actions", get(http_list_ai_actions))
        .route("/api/ai/action", post(http_ai_action))
        // Declarative settings catalog (#2872) — frontend renders controls
        // dynamically from this schema and evaluates each `visibleWhen`.
        .route("/api/settings/definitions", get(http_settings_definitions))
        // Vault file serving (#1767) — serve PDF/images from vault directory
        .route("/api/vault/files/{*path}", get(http_serve_vault_file))
        // Thumbnail serving (#3371) — auto-generated image previews for Asset Picker
        .route("/api/vault/thumbnails/{*path}", get(http_serve_thumbnail))
        // #790: Rate limiter placed before body limit and timeout so
        // rate-limited requests are rejected immediately without reading
        // the body or consuming timeout budget. In Axum .layer() ordering,
        // first .layer() = innermost, last .layer() = outermost.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(180),
        )) // #605: overall request timeout
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024)) // 10 MB
        .layer(axum::middleware::from_fn_with_state(
            rate_limiter,
            rate_limit_middleware,
        ))
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _parts| {
                    is_loopback_origin(origin)
                }))
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .with_state(state);

    println!(
        "{}",
        serde_json::json!({
            "status": "listening",
            "baseUrl": format!("http://{}:{}", ip, port),
            "chatCompletions": format!("http://{}:{}/v1/chat/completions", ip, port),
            "models": format!("http://{}:{}/v1/models", ip, port),
            "notes": format!("http://{}:{}/api/notes", ip, port),
            "requiresToken": requires_token
        })
    );

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

// ─── State / types ────────────────────────────────────────────────

#[derive(Clone)]
pub(super) struct HttpBridgeState {
    pub(super) context: StorageContext,
    token: Option<String>,
}

/// Simple per-key fixed-window rate limiter with bounded memory.
struct RateLimiter {
    entries: std::sync::Mutex<HashMap<String, (u32, Instant)>>,
    max_requests: u32,
    window: std::time::Duration,
    max_entries: usize,
}

impl RateLimiter {
    fn new(max_requests: u32, window: std::time::Duration) -> Self {
        Self {
            entries: std::sync::Mutex::new(HashMap::new()),
            max_requests,
            window,
            max_entries: 10_000,
        }
    }

    /// Returns `true` if the request is allowed, `false` if rate-limited.
    fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut entries = match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("rate limiter lock was poisoned, recovering");
                poisoned.into_inner()
            }
        };

        // Purge entries older than 2 window durations to prevent unbounded growth.
        let stale_threshold = self.window * 2;
        entries.retain(|_, (_, last)| now.duration_since(*last) < stale_threshold);

        // Evict oldest entries if we exceed max_entries to prevent OOM from spoofed IPs.
        if entries.len() >= self.max_entries {
            // Find and remove the entry with the oldest last-access timestamp.
            if let Some(oldest_key) = entries
                .iter()
                .min_by_key(|(_, (_, last))| *last)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest_key);
            }
        }

        let entry = entries.entry(key.to_string()).or_insert((0, now));

        if now.duration_since(entry.1) > self.window {
            *entry = (0, now);
        }

        if entry.0 >= self.max_requests {
            return false;
        }

        entry.0 += 1;
        true
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChatCompletionsRequest {
    #[serde(default)]
    model: String,
    messages: Vec<OpenAiChatMessage>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatMessage {
    role: String,
    content: OpenAiMessageContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiMessageContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Deserialize)]
struct OpenAiContentPart {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    image_url: Option<OpenAiImageUrl>,
}

#[derive(Debug, Deserialize)]
struct OpenAiImageUrl {
    url: String,
}

#[derive(Debug, Serialize)]
struct OpenAiModelsResponse {
    object: &'static str,
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Serialize)]
struct OpenAiModel {
    id: String,
    object: &'static str,
    created: i64,
    owned_by: &'static str,
}

#[derive(Debug, Serialize)]
struct OpenAiChatCompletionsResponse {
    id: String,
    object: &'static str,
    created: i64,
    model: String,
    choices: Vec<OpenAiChoice>,
    usage: OpenAiUsage,
}

#[derive(Debug, Serialize)]
struct OpenAiChoice {
    index: usize,
    message: OpenAiAssistantMessage,
    finish_reason: &'static str,
}

#[derive(Debug, Serialize)]
struct OpenAiAssistantMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Serialize)]
struct OpenAiUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiErrorEnvelope {
    pub(super) error: OpenAiError,
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiError {
    pub(super) message: String,
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
}

// ─── Middleware ────────────────────────────────────────────────────

async fn rate_limit_middleware(
    State(rate_limiter): State<Arc<RateLimiter>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Exempt /health from rate limiting — monitoring polls should not
    // consume the API rate budget (#774).
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }

    // Use client IP as rate-limit key to prevent token-rotation bypass (#767).
    // Previously the bearer token was used, allowing attackers to send
    // unlimited requests with unique random tokens.
    let key = format!("{}", addr.ip());

    if !rate_limiter.check(&key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(OpenAiErrorEnvelope {
                error: OpenAiError {
                    message: "rate limit exceeded, try again later".to_string(),
                    kind: "rate_limit_error",
                },
            }),
        )
            .into_response();
    }

    next.run(request).await
}

// ─── Request / Response types ────────────────────────────────────

/// Request body for POST /api/notes (browser clipper / external API users).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateNoteRequest {
    /// Note title (required).
    title: String,
    /// Note body in Markdown format (required).
    content: String,
    /// Source URL (e.g. the web page the content was clipped from).
    #[serde(default)]
    source_url: String,
    /// Optional comma-separated tags.
    #[serde(default)]
    tags: String,
    /// Optional target collection name.
    #[serde(default)]
    collection: String,
}

/// Response body for POST /api/notes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateNoteResponse {
    id: String,
    title: String,
}

/// Request body for POST /api/clip (#3034 Web Clipper Plan B).
///
/// The browser Bookmarklet / extension sends only the page URL; the bridge
/// fetches the HTML server-side, extracts a title, strips boilerplate, and
/// converts the body to Markdown before saving as a note. This avoids
/// requiring a heavy browser extension and works with a simple Bookmarklet.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipRequest {
    /// Absolute URL of the page to clip (required).
    url: String,
    /// Optional comma-separated tags (defaults to `clipped`).
    #[serde(default)]
    tags: String,
    /// Optional target collection name.
    #[serde(default)]
    collection: String,
}

/// Response body for POST /api/clip.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipResponse {
    id: String,
    title: String,
    url: String,
}

// ─── Route handlers ───────────────────────────────────────────────

async fn http_list_notes(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .min(200);
    let query = SearchQuery {
        text: String::new(),
        limit: Some(limit),
        ..Default::default()
    };
    let result = search_notes_async(&state.context, query)
        .await
        .map_err(|e| {
            tracing::warn!("http_list_notes: failed to list notes: {e}");
            openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list notes")
        })?;
    Ok(Json(serde_json::json!({
        "notes": result.notes,
        "total": result.total
    })))
}

async fn http_get_note(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(note_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    match load_note_async(&state.context, &note_id).await {
        Ok(note) => {
            let value = serde_json::to_value(note).map_err(|e| {
                tracing::warn!("http_get_note: failed to serialize note: {e}");
                openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to serialize note",
                )
            })?;
            Ok(Json(value))
        }
        Err(e) => {
            // Distinguish "not found" from real load failures. Only the
            // not-found case is a legitimate 404; DB/IO/parse errors are
            // server-side problems that must be 500 and must NOT leak
            // internal details (absolute paths, SQLite text, etc.) to the
            // client. (#2129)
            if classify_note_load_error(&e) == StatusCode::NOT_FOUND {
                Err(openai_error(StatusCode::NOT_FOUND, "Note not found"))
            } else {
                tracing::warn!("http_get_note: failed to load note {note_id}: {e}");
                Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to load note",
                ))
            }
        }
    }
}

/// Classify a note-load error into the appropriate HTTP status code.
///
/// Only the explicit "note not found" condition yields `NOT_FOUND` (404). All
/// other failures (DB errors, file IO errors, parse errors) are server-side
/// problems that must surface as `INTERNAL_SERVER_ERROR` (500), so callers can
/// distinguish "permanently absent" from "temporarily unreadable" and avoid
/// accidentally deleting a local copy on a transient failure. (#2129)
fn classify_note_load_error(e: &anyhow::Error) -> StatusCode {
    if e.downcast_ref::<NoteNotFound>().is_some() {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

// ─── #3514: Single & bulk note operations for file-browser multi-select ───

/// DELETE /api/notes/{note_id} — Delete a single note (#3514).
///
/// Honors the persisted `attachment_cleanup_on_note_delete` setting (#3732):
/// the prior implementation hard-coded `None`, silently purging exclusive
/// attachments even when the user set the mode to `Never`.
async fn http_delete_note(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(note_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #3964 — headless safety gate: note deletion is High risk and cannot be
    // confirmed non-interactively, so it is denied unless explicitly allowed.
    enforce_uri_gate(&state, &headers, "http_delete_note")?;
    // Resolve cleanup mode from the persisted setting (#3732).
    let cleanup = load_settings_async(&state.context)
        .await
        .unwrap_or_default()
        .attachment_cleanup_on_note_delete
        .resolve_delete_attachments();
    let deleted = delete_note_async(&state.context, &note_id, cleanup)
        .await
        .map_err(|e| {
            tracing::warn!("http_delete_note: failed to delete note {note_id}: {e}");
            openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete note")
        })?;
    if deleted {
        Ok(Json(serde_json::json!({ "deleted": true, "id": note_id })))
    } else {
        Err(openai_error(StatusCode::NOT_FOUND, "Note not found"))
    }
}

/// Request body for `POST /api/notes/bulk-delete` (#3514).
#[derive(Deserialize)]
struct BulkDeleteRequest {
    note_ids: Vec<String>,
    #[serde(default)]
    delete_attachments: Option<bool>,
}

/// POST /api/notes/bulk-delete — Delete multiple notes (#3514).
async fn http_bulk_delete_notes(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(req): Json<BulkDeleteRequest>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    enforce_uri_gate(&state, &headers, "http_bulk_delete_notes")?;
    if req.note_ids.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "note_ids must not be empty",
        ));
    }
    // When the caller didn't specify, fall back to the persisted setting (#3732).
    let cleanup = if req.delete_attachments.is_some() {
        req.delete_attachments
    } else {
        Some(
            load_settings_async(&state.context)
                .await
                .unwrap_or_default()
                .attachment_cleanup_on_note_delete
                .resolve_delete_attachments()
                .unwrap_or(false),
        )
    };
    let result = bulk_delete_notes_async(&state.context, req.note_ids, cleanup)
        .await
        .map_err(|e| {
            tracing::warn!("http_bulk_delete_notes: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to bulk delete notes",
            )
        })?;
    Ok(Json(serde_json::to_value(&result).unwrap_or_default()))
}

/// Request body for `POST /api/notes/bulk-move` (#3514).
#[derive(Deserialize)]
struct BulkMoveRequest {
    note_ids: Vec<String>,
    target_dir: String,
}

/// POST /api/notes/bulk-move — Move multiple notes to a target directory (#3514).
async fn http_bulk_move_notes(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(req): Json<BulkMoveRequest>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    enforce_uri_gate(&state, &headers, "http_bulk_move_notes")?;
    if req.note_ids.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "note_ids must not be empty",
        ));
    }
    if req.target_dir.trim().is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "target_dir must not be empty",
        ));
    }
    let result = bulk_move_notes_async(&state.context, req.note_ids, req.target_dir)
        .await
        .map_err(|e| {
            tracing::warn!("http_bulk_move_notes: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to bulk move notes",
            )
        })?;
    Ok(Json(serde_json::to_value(&result).unwrap_or_default()))
}

/// Request body for `POST /api/notes/bulk-tags` (#3514).
#[derive(Deserialize)]
struct BulkTagsRequest {
    note_ids: Vec<String>,
    #[serde(default)]
    add_tags: Vec<String>,
    #[serde(default)]
    remove_tags: Vec<String>,
}

/// POST /api/notes/bulk-tags — Add/remove tags on multiple notes (#3514).
async fn http_bulk_update_tags(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(req): Json<BulkTagsRequest>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    enforce_uri_gate(&state, &headers, "http_bulk_update_tags")?;
    if req.note_ids.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "note_ids must not be empty",
        ));
    }
    if req.add_tags.is_empty() && req.remove_tags.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "At least one of add_tags / remove_tags must be non-empty",
        ));
    }
    let result =
        bulk_update_tags_async(&state.context, req.note_ids, req.add_tags, req.remove_tags)
            .await
            .map_err(|e| {
                tracing::warn!("http_bulk_update_tags: {e}");
                openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to bulk update tags",
                )
            })?;
    Ok(Json(serde_json::to_value(&result).unwrap_or_default()))
}

async fn http_search_notes(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let query_text = params.get("q").cloned().unwrap_or_default();
    if query_text.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "Missing 'q' parameter",
        ));
    }
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20)
        .min(100);
    let query = SearchQuery {
        text: query_text,
        limit: Some(limit),
        ..Default::default()
    };
    let result = search_notes_async(&state.context, query)
        .await
        .map_err(|e| {
            tracing::warn!("http_search_notes: failed to search notes: {e}");
            openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to search notes")
        })?;
    Ok(Json(serde_json::json!({
        "notes": result.notes,
        "total": result.total
    })))
}

/// GET /api/notes/typeahead?q=... — Instant title-only matching for typeahead.
async fn http_typeahead(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "Missing 'q' parameter",
        ));
    }
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
        .min(50);
    let results = typeahead_search_async(&state.context, &query, limit)
        .await
        .map_err(|e| {
            tracing::warn!("http_typeahead: failed: {e}");
            openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Typeahead search failed")
        })?;
    Ok(Json(serde_json::json!({
        "notes": results
    })))
}

/// GET /api/notes/search/progressive?q=... — SSE streaming endpoint
/// that returns keyword results first, then semantic results progressively.
async fn http_progressive_search(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    if let Err(e) = require_bridge_token(&state, &headers) {
        return e.into_response();
    }
    let query_text = params.get("q").cloned().unwrap_or_default();
    if query_text.is_empty() {
        return openai_error(StatusCode::BAD_REQUEST, "Missing 'q' parameter").into_response();
    }
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20)
        .min(100);

    let (sse_tx, sse_rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);
    // Intermediate channel between search task and forwarding task so that
    // client disconnect is detected even while a search future is in flight.
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<String>(8);
    let cancel = CancellationToken::new();

    // --- Search task: runs the actual search stages (keyword → loading → deep → done) ---
    {
        let state_clone = state.clone();
        let q = query_text.clone();
        let cancel_search = cancel.clone();
        let result_tx_search = result_tx.clone();
        let result_tx_done = result_tx.clone();
        tokio::spawn(async move {
            let catch_result = futures_util::future::FutureExt::catch_unwind(
                std::panic::AssertUnwindSafe(async move {
                    tokio::select! {
                        biased;
                        _ = cancel_search.cancelled() => {
                            tracing::info!("progressive search cancelled before starting");
                        }
                        _ = tokio::time::sleep(PROGRESSIVE_SEARCH_TIMEOUT) => {
                            tracing::warn!(
                                "progressive search timed out after {}s",
                                PROGRESSIVE_SEARCH_TIMEOUT.as_secs()
                            );
                            let _ = result_tx_search
                                .send(
                                    serde_json::json!({"stage": "error", "message": "Search timed out"})
                                        .to_string(),
                                )
                                .await;
                            // Send done so the client knows search is complete (#2499)
                            let _ = result_tx_search
                                .send(serde_json::json!({"stage": "done"}).to_string())
                                .await;
                        }
                        _ = async {
                            // Stage 1: Keyword search (FTS5, fast)
                            if cancel_search.is_cancelled() {
                                return;
                            }
                            let kw_query = SearchQuery {
                                text: q.clone(),
                                limit: Some(limit),
                                deep_search: false,
                                ..Default::default()
                            };
                            match search_notes_async(&state_clone.context, kw_query).await {
                                Ok(result) => {
                                    if cancel_search.is_cancelled() {
                                        return;
                                    }
                                    let event = ProgressiveSearchEvent {
                                        stage: "keyword".into(),
                                        results: Some(result),
                                        message: None,
                                    };
                                    let data = serde_json::to_string(&event).unwrap_or_default();
                                    if result_tx_search.send(data).await.is_err() {
                                        cancel_search.cancel();
                                        return;
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("progressive keyword search failed: {e}");
                                    let _ = result_tx_search
                                        .send(
                                            serde_json::json!({"stage": "error", "message": "Internal error"})
                                                .to_string(),
                                        )
                                        .await;
                                    // Send done so the client knows search is complete (#2499)
                                    let _ = result_tx_search
                                        .send(serde_json::json!({"stage": "done"}).to_string())
                                        .await;
                                    return;
                                }
                            }

                            // Stage 2: Loading state
                            if cancel_search.is_cancelled() {
                                return;
                            }
                            let loading = ProgressiveSearchEvent {
                                stage: "loading".into(),
                                results: None,
                                message: Some("正在查找更多相关笔记...".into()),
                            };
                            let data = serde_json::to_string(&loading).unwrap_or_default();
                            if result_tx_search.send(data).await.is_err() {
                                cancel_search.cancel();
                                return;
                            }

                            // Stage 3: Deep semantic search
                            if cancel_search.is_cancelled() {
                                return;
                            }
                            let deep_query = SearchQuery {
                                text: q.clone(),
                                limit: Some(limit),
                                deep_search: true,
                                ..Default::default()
                            };
                            match deep_search_notes_async(&state_clone.context, deep_query).await {
                                Ok(result) => {
                                    if cancel_search.is_cancelled() {
                                        return;
                                    }
                                    let event = ProgressiveSearchEvent {
                                        stage: "semantic".into(),
                                        results: Some(result),
                                        message: None,
                                    };
                                    let data = serde_json::to_string(&event).unwrap_or_default();
                                    if result_tx_search.send(data).await.is_err() {
                                        cancel_search.cancel();
                                        return;
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("progressive semantic search failed: {e}");
                                    // Still send a done event so the client knows search is complete
                                    let _ = result_tx_search
                                        .send(serde_json::json!({"stage": "done"}).to_string())
                                        .await;
                                    return;
                                }
                            }

                            // Done
                            if cancel_search.is_cancelled() {
                                return;
                            }
                            let _ = result_tx_search
                                .send(serde_json::json!({"stage": "done"}).to_string())
                                .await;
                        } => {}
                    }
                }),
            )
            .await;

            if let Err(panic) = catch_result {
                tracing::error!("progressive search background task panicked: {:?}", panic);
                let _ = result_tx_done
                    .send(
                        serde_json::json!({"stage": "error", "message": "Internal error"})
                            .to_string(),
                    )
                    .await;
                // Send done so the client knows search is complete (#2499)
                let _ = result_tx_done
                    .send(serde_json::json!({"stage": "done"}).to_string())
                    .await;
            }
            // result_tx clones dropped here → result_rx returns None → forwarder exits
        });
    }

    // --- Forwarding task: reads from result_rx, sends to sse_tx, detects disconnect ---
    {
        let cancel_forwarder = cancel.clone();
        tokio::spawn(async move {
            let result = futures_util::future::FutureExt::catch_unwind(
                std::panic::AssertUnwindSafe(async move {
                    loop {
                        tokio::select! {
                            biased;
                            _ = cancel_forwarder.cancelled() => break,
                            data = result_rx.recv() => {
                                match data {
                                    Some(data) => {
                                        if sse_tx.send(Ok(Event::default().data(data))).await.is_err() {
                                            // SSE receiver dropped — client disconnected
                                            tracing::info!("progressive search client disconnected, cancelling search");
                                            cancel_forwarder.cancel();
                                            break;
                                        }
                                    }
                                    None => break,
                                }
                            }
                        }
                    }
                }),
            )
            .await;

            if let Err(panic) = result {
                tracing::error!("progressive search forwarder panicked: {:?}", panic);
            }
        });
    }

    let stream = ReceiverStream::new(sse_rx);
    Sse::new(stream).into_response()
}

/// POST /api/notes — Create a new vault note from clipped content (browser clipper MVP).
///
/// Accepts markdown content with metadata (title, source URL, tags, collection).
/// Returns the new note's ID and title.
/// Build the tag list for a clipped note.
///
/// Web Clipper (#3189) callers always send `tags`, but we guarantee every
/// clipped note carries the `clipped` tag so the Reader Mode (#3150) view and
/// any `tag:clipped` queries can find it. Empty/missing input collapses to a
/// single `clipped` tag; an explicit tag list keeps `clipped` appended if it
/// isn't already present.
pub(crate) fn build_clip_tags(raw_tags: &str) -> Vec<String> {
    if raw_tags.trim().is_empty() {
        return vec!["clipped".to_string()];
    }
    let mut t: Vec<String> = raw_tags
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if !t.contains(&"clipped".to_string()) {
        t.push("clipped".to_string());
    }
    t
}

/// Resolve the `source` metadata field for a clipped note.
///
/// When the clipper supplies a `sourceUrl` we record it verbatim; otherwise we
/// fall back to the literal `web` sentinel so the note still sorts into the
/// Web Clipper bucket in the UI.
pub(crate) fn build_clip_source(raw_source_url: &str) -> String {
    if raw_source_url.trim().is_empty() {
        "web".to_string()
    } else {
        raw_source_url.to_string()
    }
}

async fn http_create_note(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(request): Json<CreateNoteRequest>,
) -> Result<Json<CreateNoteResponse>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #3964 — headless safety gate: creating notes is Medium risk; allowed
    // only from a trusted source (x-vaultpilot-source header).
    enforce_uri_gate(&state, &headers, "http_create_note")?;

    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err(openai_error(StatusCode::BAD_REQUEST, "title is required"));
    }
    let content = request.content.trim().to_string();
    if content.is_empty() {
        return Err(openai_error(StatusCode::BAD_REQUEST, "content is required"));
    }

    // Build frontmatter metadata
    let now = Utc::now().to_rfc3339();
    let tags = build_clip_tags(&request.tags);
    let source = build_clip_source(&request.source_url);

    let mut body = content;
    // Prepend source URL as a reference line if provided
    if !request.source_url.trim().is_empty() {
        body = format!("> Source: {}\n\n{}", request.source_url.trim(), body);
    }

    let note = NoteDocument {
        meta: NoteMeta {
            title,
            tags,
            source,
            created_at: now.clone(),
            updated_at: now,
            collections: if request.collection.trim().is_empty() {
                vec![]
            } else {
                vec![request.collection.trim().to_string()]
            },
            ..Default::default()
        },
        body,
        search_snippet: None,
        search_score: None,
    };

    let saved = save_note_async(&state.context, note).await.map_err(|e| {
        tracing::warn!("http_create_note: failed to save note: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to save note")
    })?;

    Ok(Json(CreateNoteResponse {
        id: saved.meta.id,
        title: saved.meta.title,
    }))
}

// ─── Folder import (#3478) — recursive directory → batch note import ──

/// Request body for POST /api/notes/import-folder.
///
/// WinUI (or any client) sends a local filesystem path to a directory; the
/// bridge recursively walks it and imports every `.md` file as a note,
/// preserving the existing import semantics (frontmatter parsing, tag inference).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportFolderRequest {
    /// Absolute (or vault-relative) path to the directory to import.
    folder_path: String,
}

/// Response body for POST /api/notes/import-folder.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportFolderResponse {
    /// Number of `.md` files successfully imported.
    imported: usize,
    /// Number of `.md` files skipped (e.g. duplicates).
    skipped: usize,
    /// Per-file error messages (path: reason).
    errors: Vec<String>,
}

/// `POST /api/notes/import-folder` — recursively imports all Markdown files
/// under the given directory as vault notes.
///
/// This backs the WinUI "drag folder into window" UX (#3478): the front-end
/// collects the dropped folder path and POSTs it here; the existing
/// `import_markdown_async` service handles the recursive walk + per-file
/// ingestion (frontmatter parsing, title detection, FTS indexing).
async fn http_import_folder(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(request): Json<ImportFolderRequest>,
) -> Result<Json<ImportFolderResponse>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    enforce_uri_gate(&state, &headers, "http_import_folder")?;

    let folder_path = request.folder_path.trim().to_string();
    if folder_path.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "folderPath is required",
        ));
    }

    let path = PathBuf::from(&folder_path);
    if !path.is_dir() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "folderPath is not a directory or does not exist: {}",
                folder_path
            ),
        ));
    }

    let result = import_markdown_async(&state.context, &[folder_path])
        .await
        .map_err(|e| {
            tracing::warn!("http_import_folder: import failed: {e}");
            openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Folder import failed")
        })?;

    Ok(Json(ImportFolderResponse {
        imported: result.imported,
        skipped: result.skipped,
        errors: result.errors,
    }))
}

// ─── Web Clipper — server-side URL → Markdown note (#3034 Plan B) ──

/// Maximum size of HTML we are willing to fetch and parse for clipping.
/// Pages larger than this are rejected to avoid memory exhaustion.
const CLIP_MAX_HTML_BYTES: usize = 5 * 1024 * 1024; // 5 MB

/// Error-message prefix used by `stream_body_with_cap` to signal "body
/// exceeded the cap" so the caller can map it to HTTP 413 instead of 502.
/// Kept as a `const` (not an enum) so the helper stays self-contained and
/// trivially callable from tests.
const CLIP_TOO_LARGE_MARKER: &str = "[clip-too-large]";

/// Push `chunk` into `buf`, returning `Err(too-large marker)` if the
/// running total would exceed `max_bytes`.
///
/// Extracted from [`stream_body_with_cap`] as a pure helper so the cap
/// policy is unit-testable without spinning up a mock HTTP server. The
/// check runs *before* the `extend_from_slice`, so an oversized chunk
/// never lands in `buf` — important because a malicious upstream could
/// send a single 1 GB chunk and we want to reject it without first
/// allocating space for it (#3060).
fn push_chunk_with_cap(buf: &mut Vec<u8>, chunk: &[u8], max_bytes: usize) -> Result<(), String> {
    if buf.len().saturating_add(chunk.len()) > max_bytes {
        return Err(format!(
            "{CLIP_TOO_LARGE_MARKER}Page too large (exceeded {max_bytes} bytes mid-stream)"
        ));
    }
    buf.extend_from_slice(chunk);
    Ok(())
}

/// Stream a response body into a `Vec<u8>` with a hard byte cap.
///
/// Used by `/api/clip` (#3060) so a hijacked upstream cannot stream
/// unbounded data into memory before the size check fires. The cap is
/// enforced on the running `buf.len() + chunk.len()`, so it triggers as
/// soon as a single oversized chunk arrives rather than after it has been
/// fully buffered. The cap policy itself lives in [`push_chunk_with_cap`]
/// so it can be unit-tested deterministically.
///
/// Returns:
/// - `Ok(buf)` when the body completes within `max_bytes`.
/// - `Err(CLIP_TOO_LARGE_MARKER + reason)` when the cap is exceeded; the
///   caller maps this to HTTP 413.
/// - `Err(other)` for transport-level read failures (mapped to 502).
async fn stream_body_with_cap(
    resp: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;
    // Pre-size to either the declared Content-Length or a modest 64 KiB
    // headroom, whichever is smaller — keeps small-page clips alloc-free
    // while bounding speculative allocation for chunked / unknown-length
    // streams.
    let initial_cap = resp
        .content_length()
        .map(|cl| (cl as usize).min(max_bytes).min(64 * 1024))
        .unwrap_or(64 * 1024);
    let mut buf = Vec::with_capacity(initial_cap);
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("upstream stream read failed: {e}"))?;
        push_chunk_with_cap(&mut buf, &chunk, max_bytes)?;
    }
    Ok(buf)
}

/// POST /api/clip — fetch a URL server-side, extract title + readable content,
/// convert to Markdown, and save as a note. This implements #3034 Plan B
/// (Bookmarklet-friendly: the browser only needs to send the URL).
///
/// The conversion is intentionally lightweight (regex + tag stripping) rather
/// than pulling a full Readability crate, to keep the dependency footprint
/// small. It handles the common article patterns: `<title>`, `<h1>`-`<h6>`,
/// `<p>`, `<ul>`/`<ol>`/`<li>`, `<a>`, `<blockquote>`, `<code>`/`<pre>`, and
/// `<img>` tags.
async fn http_clip_url(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(request): Json<ClipRequest>,
) -> Result<Json<ClipResponse>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    enforce_uri_gate(&state, &headers, "http_clip_url")?;

    let url = request.url.trim().to_string();
    if url.is_empty() {
        return Err(openai_error(StatusCode::BAD_REQUEST, "url is required"));
    }
    // Basic scheme validation — only http/https to prevent file:/// SSRF.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "url must start with http:// or https://",
        ));
    }

    // SSRF protection (#3040) + DNS pinning (#3059): resolve the URL host
    // and reject if any resolved IP falls in a forbidden range (loopback /
    // private / link-local / multicast / unspecified / broadcast / IPv6
    // ULA). The verified `(host, SocketAddr)` pairs are returned so the
    // client builder can pin DNS, preventing a rebinding TOCTOU between
    // this check and the actual fetch.
    let initial_pins = validate_clip_url_host(&url)
        .await
        .map_err(|msg| openai_error(StatusCode::BAD_REQUEST, &msg))?;

    // Build the client with NO automatic redirects. We follow them manually
    // below so we can re-validate AND re-pin each hop's resolved IPs before
    // fetching — otherwise a public URL that 302's to http://169.254.169.254/
    // would sail past the initial check, and a rebinding DNS server could
    // flip its answer between validation and fetch.
    let mut client = build_clip_client(&initial_pins)?;

    // Fetch with manual redirect handling (cap at 5 hops, matching the previous
    // `Policy::limited(5)` semantics, but with per-hop SSRF re-validation).
    let mut current_url = url.clone();
    let mut redirects_followed: usize = 0;
    const MAX_CLIP_REDIRECTS: usize = 5;
    let resp = loop {
        let r = client
            .get(&current_url)
            .header(
                "User-Agent",
                "VaultPilot-WebClipper/1.0 (+https://vaultpilot.app)",
            )
            .header("Accept", "text/html,application/xhtml+xml,*/*;q=0.8")
            .send()
            .await
            .map_err(|e| {
                tracing::warn!("http_clip_url: fetch failed for {current_url}: {e}");
                openai_error(
                    StatusCode::BAD_GATEWAY,
                    &format!("Failed to fetch URL: {e}"),
                )
            })?;

        if !r.status().is_redirection() {
            break r;
        }

        // Cap redirect chain at MAX_CLIP_REDIRECTS total hops.
        if redirects_followed >= MAX_CLIP_REDIRECTS {
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "Too many redirects (max 5)",
            ));
        }
        redirects_followed += 1;

        // Resolve the Location header (may be relative).
        let location = r
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                openai_error(
                    StatusCode::BAD_GATEWAY,
                    "Redirect response missing Location header",
                )
            })?;
        let base = match url::Url::parse(&current_url) {
            Ok(u) => u,
            Err(e) => {
                return Err(openai_error(
                    StatusCode::BAD_GATEWAY,
                    &format!("Invalid current URL after redirect: {e}"),
                ));
            }
        };
        let next = match base.join(location) {
            Ok(u) => u,
            Err(e) => {
                return Err(openai_error(
                    StatusCode::BAD_GATEWAY,
                    &format!("Invalid redirect Location '{location}': {e}"),
                ));
            }
        };
        // Refuse non-http(s) schemes (e.g. file://, ftp://, gopher://) on hop.
        if !matches!(next.scheme(), "http" | "https") {
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                &format!(
                    "Refusing redirect to non-http(s) scheme '{}'",
                    next.scheme()
                ),
            ));
        }
        let next_str = next.to_string();
        // Re-validate SSRF on the new host AND rebuild the client with the
        // newly-verified DNS pins. Both steps are required: validation alone
        // still leaves the re-resolution window open (#3059), and pinning
        // alone would let a redirect to a forbidden host through.
        let next_pins = validate_clip_url_host(&next_str)
            .await
            .map_err(|msg| openai_error(StatusCode::BAD_REQUEST, &msg))?;
        client = build_clip_client(&next_pins)?;
        current_url = next_str;
    };

    let status = resp.status();
    if !status.is_success() {
        return Err(openai_error(
            StatusCode::BAD_GATEWAY,
            &format!("Upstream returned HTTP {status}"),
        ));
    }

    // Bound the body size to avoid pathological pages.
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if !content_type.contains("html") && !content_type.contains("xml") {
        return Err(openai_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            &format!("Expected HTML content, got '{content_type}'"),
        ));
    }

    // Stream the body with a hard byte cap so a malicious / hijacked
    // upstream cannot exhaust memory by streaming GBs of data while we
    // wait for EOF. The previous implementation called `resp.bytes().await`
    // first and checked the size only after the full body was already in
    // RAM, so the 5 MB cap was effectively advisory (#3060).
    //
    // 1. Pre-check Content-Length when present: reject immediately without
    //    reading a single body byte.
    if let Some(content_length) = resp.content_length() {
        if content_length > CLIP_MAX_HTML_BYTES as u64 {
            return Err(openai_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!(
                    "Page too large (Content-Length {content_length} bytes); max is {CLIP_MAX_HTML_BYTES} bytes"
                ),
            ));
        }
    }
    // 2. Stream chunks, accumulating until we either exhaust the body or
    //    exceed the cap. Cap is enforced on `buf.len() + chunk.len()` so a
    //    10 MB chunk can't sneak past by being one large allocation.
    let body_bytes = stream_body_with_cap(resp, CLIP_MAX_HTML_BYTES)
        .await
        .map_err(|e| {
            // Distinguish "too large" from generic I/O failure so the client
            // sees 413 (and can choose a different URL) vs 502.
            if let Some(reason) = e.strip_prefix(CLIP_TOO_LARGE_MARKER) {
                openai_error(StatusCode::PAYLOAD_TOO_LARGE, reason)
            } else {
                tracing::warn!("http_clip_url: failed to read body: {e}");
                openai_error(StatusCode::BAD_GATEWAY, "Failed to read response body")
            }
        })?;

    let html = String::from_utf8_lossy(&body_bytes).into_owned();
    let title = extract_html_title(&html).unwrap_or_else(|| url.clone());
    let markdown = html_to_markdown(&html);

    if markdown.trim().is_empty() {
        return Err(openai_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "No readable content could be extracted from the page",
        ));
    }

    // Build tags (defaults to ["clipped"], otherwise user-provided + clipped).
    let tags: Vec<String> = if request.tags.trim().is_empty() {
        vec!["clipped".to_string()]
    } else {
        let mut t: Vec<String> = request
            .tags
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !t.contains(&"clipped".to_string()) {
            t.push("clipped".to_string());
        }
        t
    };

    let now = Utc::now().to_rfc3339();
    // Prepend a source reference line so the clipped note is traceable.
    let body = format!("> Source: {}\n\n{}", url.trim(), markdown.trim());

    let note = NoteDocument {
        meta: NoteMeta {
            title,
            tags,
            source: url.clone(),
            created_at: now.clone(),
            updated_at: now,
            collections: if request.collection.trim().is_empty() {
                vec![]
            } else {
                vec![request.collection.trim().to_string()]
            },
            ..Default::default()
        },
        body,
        search_snippet: None,
        search_score: None,
    };

    let saved = save_note_async(&state.context, note).await.map_err(|e| {
        tracing::warn!("http_clip_url: failed to save note: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to save note")
    })?;

    Ok(Json(ClipResponse {
        id: saved.meta.id,
        title: saved.meta.title,
        url,
    }))
}

/// Case-insensitive substring search that returns a byte offset into the
/// ORIGINAL haystack (not a lowercased copy).
///
/// This is necessary because Rust's `str::to_lowercase()` is Unicode-aware and
/// can change the byte length of certain characters (e.g. `İ` U+0130 is 2 bytes
/// but lowercases to `i` + U+0307 combining dot, 3 bytes; Kelvin sign U+212A is
/// 3 bytes but lowercases to ASCII `k`, 1 byte). Naively calling
/// `haystack.to_lowercase().find(needle)` and then slicing the original with
/// that offset panics on non-char-boundaries or silently mis-extracts content
/// (#3044).
///
/// Since HTML tag and attribute names are ASCII, an ASCII-only case-fold is
/// correct here and avoids the byte-length-mismatch pitfall entirely. ASCII
/// letters never appear inside multi-byte UTF-8 continuation bytes (those are
/// all >= 0x80), so there is no risk of false matches mid-codepoint.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let needle_bytes = needle.as_bytes();
    let needle_lower: Vec<u8> = needle_bytes
        .iter()
        .map(|b| b.to_ascii_lowercase())
        .collect();
    let hb = haystack.as_bytes();
    let n = needle_lower.len();
    if n > hb.len() {
        return None;
    }
    let last_start = hb.len() - n;
    let mut i = 0;
    while i <= last_start {
        if hb[i..i + n]
            .iter()
            .zip(needle_lower.iter())
            .all(|(h, n_l)| h.to_ascii_lowercase() == *n_l)
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Classify an IP as forbidden for outbound Web Clipper fetches (#3040).
///
/// Forbidden ranges cover the classic SSRF targets:
/// - IPv4: loopback (127/8), private (10/8, 172.16/12, 192.168/16), link-local
///   (169.254/16, includes the AWS/GCE/Azure metadata endpoint
///   169.254.169.254), multicast (224/4), broadcast (255.255.255.255),
///   unspecified (0/0), documentation (RFC 5737).
/// - IPv6: loopback (::1), multicast (ff00::/8), unspecified (::),
///   unique-local (fc00::/7), link-local (fe80::/10).
fn ip_is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
        }
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            // Unique-local addresses (fc00::/7): fc00:: through fdff:....
            let is_ula = (segs[0] & 0xfe00) == 0xfc00;
            // Link-local (fe80::/10): fe80:: through febf:....
            let is_link_local = (segs[0] & 0xffc0) == 0xfe80;
            v6.is_loopback() || v6.is_multicast() || v6.is_unspecified() || is_ula || is_link_local
        }
    }
}

/// Validate that the host of `url_str` does not resolve to a forbidden IP
/// (#3040 SSRF mitigation). Used by `/api/clip` before every outbound fetch,
/// including each redirect hop. Also reused by the feed poller (#3041).
///
/// Returns `Ok(())` if every resolved IP is acceptable, or `Err(message)` with
/// a human-readable reason. The caller is responsible for mapping the message
/// to an HTTP error response.
///
/// Failure mode: DNS resolution failure is reported as an error (fail-closed)
/// rather than silently letting the request through.
///
/// Resolve and validate the host of `url_str` for SSRF safety (#3040),
/// returning the verified `(hostname, SocketAddr)` pairs so the caller
/// can **pin DNS** via `reqwest::ClientBuilder::resolve()`.
///
/// Returning the verified addresses (instead of `()`) closes the DNS
/// rebinding TOCTOU window that would otherwise exist between this check
/// and the subsequent HTTP fetch: without pinning, `reqwest` re-resolves
/// the hostname independently, and an attacker-controlled authoritative
/// DNS server can return a public IP here (passing validation) and a
/// private / link-local / metadata IP for the actual fetch (#3059 — same
/// class of bug as #503 on the AI base_url path; this is the same fix
/// applied to the new Web Clipper code path introduced in #3034/#3045).
///
/// Returns:
/// - `Ok(vec![])` for literal-IP URLs (no DNS pinning needed — reqwest
///   does not resolve literal IPs) that pass the forbidden-range check.
/// - `Ok(pins)` for hostname URLs where every resolved IP is acceptable;
///   `pins` is suitable for `reqwest::ClientBuilder::resolve()` calls.
/// - `Err(message)` for invalid URLs, non-http(s) schemes, missing host,
///   DNS resolution failure, or any resolved IP in a forbidden range.
///
/// Failure mode: DNS resolution failure is reported as an error (fail-closed)
/// rather than silently letting the request through.
pub(crate) async fn validate_clip_url_host(
    url_str: &str,
) -> Result<Vec<(String, SocketAddr)>, String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("invalid URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("refusing non-http(s) scheme '{}'", parsed.scheme()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    // Literal IP — validate directly without DNS. No pinning needed
    // because reqwest does not perform DNS resolution for IP-literal
    // URLs, so there is no TOCTOU window to close.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_is_forbidden(ip) {
            return Err(format!(
                "url host {host} is a forbidden IP (loopback/private/link-local/multicast/unspecified/broadcast)"
            ));
        }
        return Ok(Vec::new());
    }

    // Hostname — resolve via DNS and reject if ANY returned IP is forbidden.
    // (Refusing on any-forbidden rather than all-forbidden defends against DNS
    // rebinding setups that mix public and private IPs in a single response.)
    //
    // The resolved `SocketAddr`s are returned to the caller so they can be
    // passed to `reqwest::ClientBuilder::resolve()`, pinning DNS for the
    // subsequent fetch and eliminating the re-resolution window (#3059).
    let port = parsed
        .port_or_known_default()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let lookup_target = format!("{host}:{port}");
    let resolved = tokio::net::lookup_host(lookup_target.as_str())
        .await
        .map_err(|e| format!("DNS resolution failed for '{host}': {e}"))?;
    let mut pinned: Vec<(String, SocketAddr)> = Vec::new();
    for addr in resolved {
        if ip_is_forbidden(addr.ip()) {
            return Err(format!(
                "url host '{host}' resolves to forbidden IP {} (loopback/private/link-local/multicast/unspecified/broadcast)",
                addr.ip()
            ));
        }
        // Normalize the port to the URL's port: `lookup_host` already uses
        // the URL's port (via `lookup_target`), so `addr` carries the
        // correct port for pinning.
        pinned.push((host.to_string(), addr));
    }
    Ok(pinned)
}

/// Build a `reqwest::Client` for `/api/clip` with DNS pinning applied.
///
/// `pins` is the verified `(hostname, SocketAddr)` list returned by
/// [`validate_clip_url_host`]. Each entry is passed to
/// `ClientBuilder::resolve()` so the client connects to the verified IP
/// instead of re-resolving the hostname — closing the DNS rebinding
/// TOCTOU window (#3059).
///
/// The client is configured with no-redirect policy (redirects are
/// followed manually by [`http_clip_url`] so each hop can be re-validated
/// and re-pinned) and a 20 s timeout matching the previous behavior.
fn build_clip_client(
    pins: &[(String, SocketAddr)],
) -> Result<reqwest::Client, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none());
    for (host, addr) in pins {
        builder = builder.resolve(host, *addr);
    }
    builder.build().map_err(|e| {
        tracing::warn!("http_clip_url: failed to build HTTP client: {e}");
        openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to build HTTP client",
        )
    })
}

/// Extract the page title from the `<title>...</title>` tag (case-insensitive).
/// Returns the trimmed title, or `None` if no `<title>` tag is present.
///
/// Uses [`find_ci`] instead of `to_lowercase().find()` so that non-ASCII
/// characters whose lowercase form has a different byte length (e.g. `İ`,
/// Kelvin sign) do not corrupt the offsets used to slice the original string
/// (#3044).
/// Extract the page title from the `<title>...</title>` tag (case-insensitive).
/// Returns the trimmed title, or `None` if no `<title>` tag is present.
///
/// Used by both the Web Clipper (`/api/clip`) and the Feed poller (#3041) to
/// derive a note title from fetched HTML.
pub(crate) fn extract_html_title(html: &str) -> Option<String> {
    let start = find_ci(html, "<title")?;
    let after_open = &html[start..];
    let tag_end = after_open.find('>')?;
    let content_start = start + tag_end + 1;
    let rest = &html[content_start..];
    let close = find_ci(rest, "</title>")?;
    let title = rest[..close].trim();
    if title.is_empty() {
        None
    } else {
        Some(decode_html_entities(title))
    }
}

/// Strip the outermost wrapper tags (`<html>`, `<head>`, `<body>`,
/// `<script>`, `<style>`, `<nav>`, `<header>`, `<footer>`, `<aside>`,
/// `<noscript>`, `<iframe>`) from the HTML to isolate article content.
/// This is a deliberately crude Readability stand-in: it does not score
/// nodes, but removing nav/header/footer/script/style is enough to get a
/// useful Markdown dump for most article pages.
fn strip_boilerplate(html: &str) -> String {
    // Remove <script>...</script>, <style>...</style>, <noscript>...</noscript>,
    // <nav>...</nav>, <header>...</header>, <footer>...</footer>,
    // <aside>...</aside>, <iframe>...</iframe> blocks (case-insensitive, DOTALL).
    let patterns = [
        "script", "style", "noscript", "nav", "header", "footer", "aside", "iframe",
    ];
    let mut out = html.to_string();
    for tag in patterns {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        // Use find_ci (ASCII case-fold on the original string) so byte
        // offsets are valid char boundaries even when the page contains
        // characters like `İ` whose lowercase form has a different byte
        // length. Mixing offsets from `to_lowercase()` with `replace_range`
        // on the original panics (#3044).
        while let Some(s) = find_ci(&out, &open) {
            // Find the end of the open tag (the next '>').
            let Some(gt) = out[s..].find('>') else {
                break;
            };
            let open_end = s + gt + 1;
            let Some(close_pos) = find_ci(&out[open_end..], &close) else {
                break;
            };
            let close_end = open_end + close_pos + close.len();
            // Drop the matched region.
            out.replace_range(s..close_end, "");
        }
    }
    out
}

/// Decode the most common HTML entities to their literal characters.
fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

/// Wrap the trailing slice `out[anchor_start..]` as a Markdown link.
///
/// Used by [`html_to_markdown`] to convert `<a href="...">anchor text</a>`
/// into `[anchor text](href)`. On entry `anchor_start` is the offset recorded
/// when the `<a>` open tag was processed; we drain everything after that
/// offset to obtain the anchor text, then push the formatted link back.
///
/// Graceful no-ops (the anchor text is left as plain text):
/// - `anchor_start` is out of bounds (defensive; should not happen),
/// - `href` is empty (e.g. `<a name="x">` anchor targets, `<a href="#">`),
/// - the anchor text is empty / whitespace-only.
///
/// Both `href` and the anchor text are trimmed of surrounding whitespace so
/// `<a href=" x "> text </a>` still produces `[text](x)` rather than
/// `[ text ]( x )` — the latter is valid Markdown but visually noisy.
fn wrap_link_in_place(out: &mut String, anchor_start: usize, href: &str) {
    if anchor_start >= out.len() {
        return;
    }
    let href_trimmed = href.trim();
    if href_trimmed.is_empty() {
        // No usable destination — leave the anchor text verbatim.
        return;
    }
    let anchor_text: String = out.drain(anchor_start..).collect();
    let anchor_trimmed = anchor_text.trim();
    if anchor_trimmed.is_empty() {
        // Nothing to wrap — restore the original whitespace and bail.
        out.push_str(&anchor_text);
        return;
    }
    // `anchor_trimmed` is a sub-slice of `anchor_text`; using it directly
    // would borrow the local we just drained into, so copy via to_string.
    out.push('[');
    out.push_str(anchor_trimmed);
    out.push_str("](");
    out.push_str(href_trimmed);
    out.push(')');
}

/// Convert block-level and inline HTML tags to Markdown. This is a
/// lightweight regex-free converter: it handles headings, paragraphs,
/// lists, links, images, code blocks, blockquotes, emphasis, and `<br>`.
/// Anything unrecognised is stripped of its tags but its text is kept.
///
/// Shared by the Web Clipper (`/api/clip`) and the Feed poller (#3041).
pub(crate) fn html_to_markdown(html: &str) -> String {
    let cleaned = strip_boilerplate(html);
    let mut out = String::with_capacity(cleaned.len());

    // Tokenise into tags and text by scanning character by character.
    let bytes = cleaned.as_bytes();
    let mut i = 0;
    // Track list nesting for indentation.
    let mut list_stack: Vec<&'static str> = Vec::new();
    // Track open <a> anchors so the closing tag can wrap the inner text as
    // `[text](href)`. Each entry is `(anchor_start_offset_in_out, href)`.
    // HTML disallows nested <a>; if we encounter an open while one is already
    // active we defensively close the outer first (see the open-tag branch).
    // Fixes #3061: previously the href was silently dropped on the floor and
    // every link degraded to bare anchor text.
    let mut link_stack: Vec<(usize, String)> = Vec::new();
    // Track whether we're inside <pre> (suppress newline insertion).
    let mut in_pre = false;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Find the matching '>'.
            if let Some(gt) = cleaned[i..].find('>') {
                let tag_raw = &cleaned[i + 1..i + gt];
                let tag_lower = tag_raw.trim_start_matches('/').to_lowercase();
                let tag_name: String = tag_lower
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let is_closing = tag_raw.trim_start().starts_with('/');

                // Action per tag name.
                match tag_name.as_str() {
                    "br" => out.push('\n'),
                    "p" if !is_closing => out.push_str("\n\n"),
                    "p" if is_closing => out.push('\n'),
                    "h1" if !is_closing => out.push_str("\n\n# "),
                    "h2" if !is_closing => out.push_str("\n\n## "),
                    "h3" if !is_closing => out.push_str("\n\n### "),
                    "h4" if !is_closing => out.push_str("\n\n#### "),
                    "h5" if !is_closing => out.push_str("\n\n##### "),
                    "h6" if !is_closing => out.push_str("\n\n###### "),
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if is_closing => out.push('\n'),
                    "strong" | "b" if !is_closing => out.push_str("**"),
                    "strong" | "b" if is_closing => out.push_str("**"),
                    "em" | "i" if !is_closing => out.push('_'),
                    "em" | "i" if is_closing => out.push('_'),
                    "code" if !is_closing && !in_pre => out.push('`'),
                    "code" if is_closing && !in_pre => out.push('`'),
                    "pre" if !is_closing => {
                        out.push_str("\n```\n");
                        in_pre = true;
                    }
                    "pre" if is_closing => {
                        out.push_str("\n```\n");
                        in_pre = false;
                    }
                    "blockquote" if !is_closing => out.push_str("\n> "),
                    "blockquote" if is_closing => out.push('\n'),
                    "hr" => out.push_str("\n\n---\n\n"),
                    "ul" if !is_closing => list_stack.push("ul"),
                    "ul" if is_closing => {
                        list_stack.pop();
                        if list_stack.is_empty() {
                            out.push('\n');
                        }
                    }
                    "ol" if !is_closing => list_stack.push("ol"),
                    "ol" if is_closing => {
                        list_stack.pop();
                        if list_stack.is_empty() {
                            out.push('\n');
                        }
                    }
                    "li" if !is_closing => {
                        let depth = list_stack.len().saturating_sub(1);
                        let indent = "  ".repeat(depth);
                        // Both <ul> and <ol> use "-" as the marker (ordered-list
                        // numbering is simplified for the lightweight converter).
                        out.push('\n');
                        out.push_str(&indent);
                        out.push_str("- ");
                    }
                    "img" => {
                        // Extract src and alt attributes from the raw tag.
                        let src = extract_attr(tag_raw, "src");
                        let alt = extract_attr(tag_raw, "alt");
                        if let Some(s) = src {
                            out.push_str(&format!("![{}]({})", alt.unwrap_or_default(), s));
                        }
                    }
                    "a" if !is_closing => {
                        // #3061: remember where the anchor text starts and
                        // the href, so the matching </a> can wrap the
                        // in-between text as `[text](href)`. HTML disallows
                        // nested <a>; defensively pop+format any outer link
                        // first so the inner's offset is correct.
                        while let Some((anchor_start, outer_href)) = link_stack.pop() {
                            wrap_link_in_place(&mut out, anchor_start, &outer_href);
                        }
                        let href = extract_attr(tag_raw, "href").unwrap_or_default();
                        link_stack.push((out.len(), href));
                    }
                    "a" if is_closing => {
                        // #3061: wrap everything emitted since the matching
                        // <a> as a Markdown link. If the open tag never
                        // happened (or href was missing), this is a no-op
                        // and the text stays as-is.
                        if let Some((anchor_start, href)) = link_stack.pop() {
                            wrap_link_in_place(&mut out, anchor_start, &href);
                        }
                    }
                    _ => {} // ignore unknown tags (script/style already stripped)
                }
                i += gt + 1;
                continue;
            }
        }
        // Plain text character — decode entities and append.
        if bytes[i] == b'&' {
            if let Some(semi) = cleaned[i..].find(';') {
                let entity = &cleaned[i..=i + semi];
                out.push_str(&decode_html_entities(entity));
                i += semi + 1;
                continue;
            }
        }
        // Normalise whitespace inside <pre> we keep verbatim; outside, collapse.
        let ch = cleaned.as_bytes()[i] as char;
        if in_pre {
            out.push(ch);
        } else if ch.is_whitespace() {
            // Collapse runs of whitespace into a single space, but preserve
            // existing newlines so paragraph breaks survive.
            if ch == '\n' {
                out.push('\n');
            } else if !out.ends_with(' ') && !out.ends_with('\n') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
        i += 1;
    }

    // Post-process: collapse 3+ consecutive newlines into exactly 2.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_string()
}

/// Extract the value of a named attribute from an HTML tag's raw inner text
/// (e.g. `a href="..."`, `img src="..."`).
///
/// Uses [`find_ci`] instead of `to_lowercase().find()` so that characters whose
/// lowercase form changes byte length (e.g. `İ`) do not corrupt offsets used to
/// slice the original tag text (#3044).
fn extract_attr(tag_inner: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let idx = find_ci(tag_inner, &needle)?;
    let after = &tag_inner[idx + needle.len()..];
    let after_trimmed = after.trim_start();
    if let Some(rest) = after_trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    } else if let Some(rest) = after_trimmed.strip_prefix('\'') {
        let end = rest.find('\'')?;
        Some(rest[..end].to_string())
    } else {
        // Unquoted attribute value — read until whitespace or '/'.
        let end = after_trimmed
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(after_trimmed.len());
        let val = after_trimmed[..end].trim().to_string();
        if val.is_empty() {
            None
        } else {
            Some(val)
        }
    }
}

// ─── Subscription API handlers (#2167) ──────────────────────────

/// GET /api/subscriptions — List all subscriptions.
async fn http_list_subscriptions(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let subs = list_subscriptions_async(&state.context)
        .await
        .map_err(|e| {
            tracing::warn!("http_list_subscriptions: failed to list subscriptions: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to list subscriptions",
            )
        })?;
    let count = subs.len();
    Ok(Json(serde_json::json!({
        "subscriptions": subs,
        "count": count
    })))
}

/// POST /api/subscriptions — Create a new subscription.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSubscriptionRequest {
    name: String,
    #[serde(default = "default_schedule")]
    schedule: String,
    prompt: String,
    #[serde(default = "default_tools")]
    tools: String,
    #[serde(default = "default_target_collection")]
    target_collection: String,
}

fn default_schedule() -> String {
    "0 0 * * *".to_string()
}
fn default_tools() -> String {
    "web_search".to_string()
}
fn default_target_collection() -> String {
    "Scheduled Research".to_string()
}

async fn http_create_subscription(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(req): Json<CreateSubscriptionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #3964 gate: creating a subscription is a Medium mutation (reversible),
    // so token holders are no longer exempt from the trusted-source check
    // (#3993).
    enforce_uri_gate(&state, &headers, "http_create_subscription")?;
    let sub = create_subscription_async(
        &state.context,
        req.name,
        req.schedule,
        req.prompt,
        req.tools,
        req.target_collection,
    )
    .await
    .map_err(|e| {
        tracing::warn!("http_create_subscription: failed to create subscription: {e}");
        openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create subscription",
        )
    })?;
    Ok(Json(serde_json::json!({
        "created": true,
        "subscription": sub
    })))
}

/// GET /api/subscriptions/{sub_id} — Get a subscription by ID.
async fn http_get_subscription(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(sub_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let sub = get_subscription_async(&state.context, sub_id.clone())
        .await
        .map_err(|e| {
            tracing::warn!("http_get_subscription: failed to load subscription: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load subscription",
            )
        })?;
    match sub {
        Some(s) => Ok(Json(serde_json::json!({ "subscription": s }))),
        None => Err(openai_error(
            StatusCode::NOT_FOUND,
            "Subscription not found",
        )),
    }
}

/// DELETE /api/subscriptions/{sub_id} — Delete a subscription.
async fn http_delete_subscription(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(sub_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #3964/#3993: Medium mutation.
    enforce_uri_gate(&state, &headers, "http_delete_subscription")?;
    let deleted = delete_subscription_async(&state.context, sub_id.clone())
        .await
        .map_err(|e| {
            tracing::warn!("http_delete_subscription: failed to delete subscription: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to delete subscription",
            )
        })?;
    if deleted {
        Ok(Json(serde_json::json!({
            "deleted": true,
            "id": sub_id
        })))
    } else {
        Err(openai_error(
            StatusCode::NOT_FOUND,
            "Subscription not found",
        ))
    }
}

/// POST /api/subscriptions/{sub_id}/run — Run a specific subscription.
async fn http_run_subscription(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(sub_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #3964/#3993: running a subscription writes research results into the
    // vault as notes — assert trusted-source before running.
    enforce_uri_gate(&state, &headers, "http_run_subscription")?;
    let sub = get_subscription_async(&state.context, sub_id.clone())
        .await
        .map_err(|e| {
            tracing::warn!("http_run_subscription: failed to load subscription: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load subscription",
            )
        })?
        .ok_or_else(|| openai_error(StatusCode::NOT_FOUND, "Subscription not found"))?;

    let result = run_single_subscription(&state.context, &sub).await;
    Ok(Json(serde_json::json!({
        "ran": true,
        "result": result
    })))
}

/// POST /api/subscriptions/{sub_id}/toggle — Enable or disable a subscription.
#[derive(Debug, Deserialize)]
struct ToggleSubscriptionRequest {
    enabled: bool,
}

async fn http_toggle_subscription(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(sub_id): AxumPath<String>,
    Json(req): Json<ToggleSubscriptionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #3964/#3993: Medium mutation.
    enforce_uri_gate(&state, &headers, "http_toggle_subscription")?;
    let updated = set_subscription_enabled_async(&state.context, sub_id.clone(), req.enabled)
        .await
        .map_err(|e| {
            tracing::warn!("http_toggle_subscription: failed to toggle subscription: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to toggle subscription",
            )
        })?;
    if updated {
        Ok(Json(serde_json::json!({
            "updated": true,
            "id": sub_id,
            "enabled": req.enabled
        })))
    } else {
        Err(openai_error(
            StatusCode::NOT_FOUND,
            "Subscription not found",
        ))
    }
}

/// Request body for PUT /api/subscriptions/{sub_id} — Update a subscription.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSubscriptionRequest {
    name: String,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    tools: Option<String>,
    #[serde(default)]
    target_collection: Option<String>,
}

/// PUT /api/subscriptions/{sub_id} — Update an existing subscription.
async fn http_update_subscription(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(sub_id): AxumPath<String>,
    Json(req): Json<UpdateSubscriptionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #3964/#3993: Medium mutation.
    enforce_uri_gate(&state, &headers, "http_update_subscription")?;

    // Load existing subscription for partial merge
    let existing = get_subscription_async(&state.context, sub_id.clone())
        .await
        .map_err(|e| {
            tracing::warn!("http_update_subscription: failed to load subscription: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load subscription",
            )
        })?
        .ok_or_else(|| openai_error(StatusCode::NOT_FOUND, "Subscription not found"))?;

    let new_schedule = req.schedule.unwrap_or(existing.schedule.clone());
    let new_prompt = req.prompt.unwrap_or(existing.prompt.clone());
    let new_tools = req.tools.unwrap_or(existing.tools.clone());
    let new_target = req
        .target_collection
        .unwrap_or(existing.target_collection.clone());

    let updated = update_subscription_async(
        &state.context,
        sub_id.clone(),
        req.name,
        new_schedule.clone(),
        new_prompt,
        new_tools,
        new_target,
    )
    .await
    .map_err(|e| {
        tracing::warn!("http_update_subscription: failed to update subscription: {e}");
        openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to update subscription",
        )
    })?;

    if !updated {
        return Err(openai_error(
            StatusCode::NOT_FOUND,
            "Subscription not found",
        ));
    }

    // Reload and return the updated subscription
    let sub = get_subscription_async(&state.context, sub_id.clone())
        .await
        .map_err(|e| {
            tracing::warn!("http_update_subscription: failed to reload subscription: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to reload subscription",
            )
        })?
        .ok_or_else(|| openai_error(StatusCode::NOT_FOUND, "Subscription not found"))?;

    Ok(Json(serde_json::json!({
        "updated": true,
        "subscription": sub
    })))
}

// ─── AI Action Handlers (#2188) ─────────────────────────────────────

/// Deserialize incoming request for the /api/ai/action endpoint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiActionHttpRequest {
    action: AiActionType,
    #[serde(default)]
    text: String,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    tone: Option<String>,
    #[serde(default)]
    note_id: Option<String>,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// POST /api/ai/action — Execute an AI quick action (non-streaming).
async fn http_ai_action(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(req): Json<AiActionHttpRequest>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #3964/#3993: AI actions accept a note_id and can rewrite the note —
    // require a trusted source, matching http_create_note's bar.
    enforce_uri_gate(&state, &headers, "http_ai_action")?;
    let ai_request = AiActionRequest {
        action: req.action,
        text: req.text,
        target_language: req.target_language,
        tone: req.tone,
        note_id: req.note_id,
        instruction: req.instruction,
        model: req.model,
        export_format: None,
    };
    let settings = load_settings_async(&state.context).await.map_err(|e| {
        tracing::warn!("http_ai_action: failed to load settings: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load settings")
    })?;
    let result = execute_ai_action(&settings, &ai_request).await;
    let value = serde_json::to_value(&result).map_err(|e| {
        tracing::warn!("http_ai_action: failed to serialize result: {e}");
        openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to serialize result",
        )
    })?;
    Ok(Json(value))
}

/// GET /api/ai/actions — List all available AI action types.
async fn http_list_ai_actions(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let actions = list_ai_actions();
    let value = serde_json::to_value(&actions).map_err(|e| {
        tracing::warn!("http_list_ai_actions: failed to list actions: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list actions")
    })?;
    Ok(Json(value))
}

/// GET /api/settings/definitions — Return the declarative settings catalog
/// (#2872). The WinUI frontend renders controls dynamically from this schema
/// and evaluates each definition's `visibleWhen` predicate client-side, so
/// new settings appear without editing the UI.
async fn http_settings_definitions(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let defs = vaultpilot_lib::settings_schema::collect_setting_definitions();
    let value = serde_json::to_value(&defs).map_err(|e| {
        tracing::warn!("http_settings_definitions: serialization failed: {e}");
        openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to serialize settings definitions",
        )
    })?;
    Ok(Json(serde_json::json!({ "definitions": value })))
}

async fn http_health() -> Json<Value> {
    Json(serde_json::json!({
        "status": "ok"
    }))
}

/// GET /api/vault/health — Return vault health report as JSON (#2014).
async fn http_vault_health(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // Run health check synchronously via spawn_blocking
    let report = tokio::task::spawn_blocking({
        let ctx = state.context.clone();
        move || vaultpilot_lib::health::health_check(&ctx)
    })
    .await
    .map_err(|e| {
        tracing::warn!("http_vault_health: spawn_blocking failed: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Health check failed")
    })?
    .map_err(|e| {
        tracing::warn!("http_vault_health: health check failed: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Health check failed")
    })?;

    let value = serde_json::to_value(&report).map_err(|e| {
        tracing::warn!("http_vault_health: serialization failed: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Serialization failed")
    })?;
    Ok(Json(value))
}

/// GET /api/graph: knowledge graph as JSON (headless server #3460).
///
/// Returns the full vault knowledge graph (nodes + edges) in JSON format,
/// enabling scripts, CI, and headless agents to query note link relationships
/// without invoking the CLI.
///
/// Query parameters:
/// - mentions (bool, optional): include plain-text mention edges (#2832)
/// - inferred (bool, optional): include AI-inferred relationships (#3370)
//   Note: inferred is accepted but not yet implemented; needs NoteMeta.
async fn http_graph(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;

    let include_mentions = params
        .get("mentions")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    let graph = tokio::task::spawn_blocking({
        let ctx = state.context.clone();
        move || {
            if include_mentions {
                vaultpilot_lib::knowledge_graph::build_knowledge_graph_with_mentions(&ctx)
            } else {
                vaultpilot_lib::knowledge_graph::build_knowledge_graph(&ctx)
            }
        }
    })
    .await
    .map_err(|e| {
        tracing::warn!("graph: spawn_blocking failed: {e}");
        openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Knowledge graph generation failed",
        )
    })?
    .map_err(|e| {
        tracing::warn!("graph: build failed: {e}");
        openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Knowledge graph generation failed",
        )
    })?;

    let value = serde_json::to_value(&graph).map_err(|e| {
        tracing::warn!("graph: serialization failed: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Serialization failed")
    })?;

    Ok(Json(value))
}

/// GET /api/vault/files/{*path} — Serve a file (PDF / image) from inside the
/// vault so the desktop/mobile UI can embed a preview via `![[file.pdf]]` and
/// similar wikilink syntax (#1767).
///
/// Security: the requested path is resolved against the configured vault root
/// and is **only** served when the canonicalized path stays within the vault
/// root. Path-traversal attempts (`../`, symlinks pointing outside the vault,
/// absolute paths) are rejected with 404 and never leak a file outside the
/// vault. The file must also physically exist (we never enumerate or confirm
/// the presence of anything outside the vault).
async fn http_serve_vault_file(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(rel): AxumPath<String>,
) -> Result<Response, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;

    let settings = load_settings_async(&state.context).await.map_err(|e| {
        tracing::warn!("http_serve_vault_file: failed to load settings: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load settings")
    })?;

    let vault_root = PathBuf::from(&settings.vault_dir);
    let Some(real) = resolve_vault_file_path(&vault_root, &rel) else {
        return Err(openai_error(StatusCode::NOT_FOUND, "File not found"));
    };

    let bytes = tokio::fs::read(&real).await.map_err(|e| {
        // A race between resolve and read (file deleted) or a permission
        // problem: report a generic 404 so we never leak the reason.
        tracing::warn!("http_serve_vault_file: failed to read {real:?}: {e}");
        openai_error(StatusCode::NOT_FOUND, "File not found")
    })?;

    let content_type = vault_file_content_type(&real);
    let body = Body::from(bytes);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        // `inline` lets the browser/WebView render the PDF/image in-place
        // instead of forcing a download.
        .header(header::CONTENT_DISPOSITION, "inline")
        .body(body)
        .map_err(|e| {
            tracing::warn!("http_serve_vault_file: failed to build response: {e}");
            openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to serve file")
        })
}

/// GET /api/vault/thumbnails/{*path} — serve or generate a cached thumbnail
/// for a vault image (#3371).
async fn http_serve_thumbnail(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(rel): AxumPath<String>,
) -> Result<Response, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;

    let settings = load_settings_async(&state.context).await.map_err(|e| {
        tracing::warn!("http_serve_thumbnail: failed to load settings: {e}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load settings")
    })?;

    let vault_root = PathBuf::from(&settings.vault_dir);
    let Some(real) = resolve_vault_file_path(&vault_root, &rel) else {
        return Err(openai_error(StatusCode::NOT_FOUND, "File not found"));
    };

    let thumb_path =
        match vaultpilot_lib::thumbnail::get_or_create_thumbnail(&vault_root, &real, None) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    "http_serve_thumbnail: failed to generate thumbnail for {real:?}: {e}"
                );
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to generate thumbnail",
                ));
            }
        };

    let bytes = tokio::fs::read(&thumb_path).await.map_err(|e| {
        tracing::warn!("http_serve_thumbnail: failed to read thumbnail {thumb_path:?}: {e}");
        openai_error(StatusCode::NOT_FOUND, "Thumbnail not found")
    })?;

    let body = Body::from(bytes);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "public, max-age=86400") // 24h cache
        .body(body)
        .map_err(|e| {
            tracing::warn!("http_serve_thumbnail: failed to build response: {e}");
            openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to serve thumbnail",
            )
        })
}

/// Resolve `rel` against `vault_root`, guaranteeing the result stays inside the
/// vault. Returns `None` when the file does not exist or the resolved path
/// escapes the vault root (traversal / out-of-vault symlink).
fn resolve_vault_file_path(vault_root: &Path, rel: &str) -> Option<PathBuf> {
    let candidate = vault_root.join(rel);
    let canon_root = vault_root.canonicalize().ok()?;
    let canon = candidate.canonicalize().ok()?;
    // `Path::starts_with` is component-based, so `/vault` will NOT match
    // `/vault-stolen` — a naive string prefix check would be wrong here.
    canon.starts_with(&canon_root).then_some(canon)
}

/// Map a file extension to a safe `Content-Type` for in-vault previews.
/// Unknown types fall back to `application/octet-stream` so we never claim a
/// specific format we cannot honor.
fn vault_file_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("tiff") | Some("tif") => "image/tiff",
        _ => "application/octet-stream",
    }
}

async fn http_models(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
) -> Result<Json<OpenAiModelsResponse>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    // #2105: Surface settings load failures instead of silently degrading to the
    // default config. This keeps /v1/models consistent with /v1/chat/completions,
    // which returns 500 on the same failure — otherwise callers would receive a
    // seemingly valid model id and then hit a 500 when they actually use it.
    let settings = match load_settings_async(&state.context).await {
        Ok(s) => s,
        Err(error) => {
            tracing::warn!("http_models: failed to load settings: {error}");
            return Err(openai_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load settings",
            ));
        }
    };
    let now = Utc::now().timestamp();
    Ok(Json(OpenAiModelsResponse {
        object: "list",
        data: vec![OpenAiModel {
            id: bridge_model_id(&settings),
            object: "model",
            created: now,
            owned_by: "vaultpilot",
        }],
    }))
}

async fn http_chat_completions(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(request): Json<OpenAiChatCompletionsRequest>,
) -> axum::response::Response {
    if let Err(e) = require_bridge_token(&state, &headers) {
        return e.into_response();
    }

    let settings = match load_settings_async(&state.context).await {
        Ok(s) => s,
        Err(error) => {
            tracing::warn!("http_chat_completions: failed to load settings: {error}");
            return openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load settings")
                .into_response();
        }
    };
    let requested_model = request.model.trim().to_string();
    let vault_root = PathBuf::from(&settings.vault_dir);
    let is_stream = request.stream;
    let model_id = if requested_model.is_empty() {
        bridge_model_id(&settings)
    } else {
        requested_model.clone()
    };
    let (question, history, image_paths) = match openai_request_to_dialog(request, &vault_root) {
        Ok(v) => v,
        Err(message) => return openai_error(StatusCode::BAD_REQUEST, &message).into_response(),
    };

    if is_stream {
        // Streaming mode: use channel-based SSE approach with cancellation support.
        // CancellationToken stops the upstream request when the client disconnects.
        let (sse_tx, sse_rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);
        // #2104: Bounded buffer between the upstream AI task and the SSE forwarder.
        // Previously this was an unbounded channel, which broke backpressure: a slow
        // client filled the downstream sse_tx(16) and stalled the forwarder, while the
        // upstream task kept pushing at full speed and the buffer grew without bound.
        // A bounded channel caps per-connection memory; the sync on_chunk callback uses
        // try_send (never blocking the executor thread) and drops the incoming chunk
        // when full instead of growing unboundedly.
        let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<String>(64);
        let settings_arc = Arc::new(settings);
        let system_owned = format!(
            "{}{}",
            vaultpilot_lib::prompting::general_chat_system_prompt(&settings_arc.system_directive),
            vaultpilot_lib::prompting::response_style_suffix(settings_arc.response_style),
        );
        let user_prompt_owned =
            vaultpilot_lib::prompting::general_chat_user_prompt(&question, &history);
        let model_for_task = model_id;
        let cancel = CancellationToken::new();

        // Upstream request task: runs the AI streaming request, sends chunks via the
        // bounded channel. The async on_chunk callback uses send().await so it
        // never blocks the executor thread, preserving cooperative scheduling.
        let cancel_on_chunk = cancel.clone();
        let cancel_upstream = cancel.clone();
        let upstream_tx = chunk_tx.clone();
        tokio::spawn(async move {
            let model_ref = &model_for_task;
            let model_for_inner = model_for_task.clone();
            let chunk_tx_ref = &chunk_tx;
            // #2128: Cap the total upstream streaming time. The HTTP
            // TimeoutLayer(180s) does NOT cover the SSE body stream (it only
            // times the Response future, which for SSE resolves immediately),
            // so without this an upstream that stalls (accepts connection,
            // sends partial data, then hangs) with a non-disconnecting client
            // would hold a tokio task + channels + upstream HTTP connection
            // indefinitely. Consistent with the non-streaming 180s cap.
            let catch_result = futures_util::future::FutureExt::catch_unwind(
                std::panic::AssertUnwindSafe(async move {
                    let result = tokio::select! {
                        biased;
                        _ = cancel_upstream.cancelled() => {
                            tracing::info!("client disconnected, cancelling upstream AI request");
                            return;
                        }
                        _ = tokio::time::sleep(STREAM_UPSTREAM_TIMEOUT) => {
                            tracing::warn!(
                                "streaming upstream timed out after {}s, aborting stream",
                                STREAM_UPSTREAM_TIMEOUT.as_secs()
                            );
                            let error_data = serde_json::json!({
                                "error": {
                                    "message": "Upstream service timed out",
                                    "type": "upstream_timeout",
                                    "code": "upstream_timeout"
                                }
                            });
                            let _ = chunk_tx_ref.send(error_data.to_string()).await;
                            let _ = chunk_tx_ref.send("[DONE]".to_string()).await;
                            return;
                        }
                        result = vaultpilot_lib::ai::send_request_streaming(
                            &settings_arc,
                            &system_owned,
                            &user_prompt_owned,
                            &image_paths,
                            0.2,
                            |chunk| {
                                let chunk_data = serde_json::json!({
                                    "id": format!("chatcmpl-{}", Uuid::new_v4().simple()),
                                    "object": "chat.completion.chunk",
                                    "created": Utc::now().timestamp(),
                                    "model": model_ref,
                                    "choices": [{
                                        "index": 0,
                                        "delta": { "content": chunk },
                                        "finish_reason": null
                                    }]
                                });
                                let tx = upstream_tx.clone();
                                let cancel_inner = cancel_on_chunk.clone();
                                Box::pin(async move {
                                    tokio::select! {
                                        biased;
                                        _ = cancel_inner.cancelled() => {
                                            tracing::debug!("on_chunk: upstream cancelled, skipping chunk");
                                        }
                                        result = tx.send(chunk_data.to_string()) => {
                                            if let Err(e) = result {
                                                tracing::warn!("streaming chunk send failed (client disconnected?): {e}");
                                            }
                                        }
                                    }
                                })
                            },
                        ) => result,
                    };

                    match result {
                        Ok((_text, _usage)) => {
                            let finish_data = serde_json::json!({
                                "id": format!("chatcmpl-{}", Uuid::new_v4().simple()),
                                "object": "chat.completion.chunk",
                                "created": Utc::now().timestamp(),
                                "model": model_for_inner,
                                "choices": [{
                                    "index": 0,
                                    "delta": {},
                                    "finish_reason": "stop"
                                }]
                            });
                            let _ = chunk_tx_ref.send(finish_data.to_string()).await;
                            let _ = chunk_tx_ref.send("[DONE]".to_string()).await;
                        }
                        Err(error) => {
                            tracing::warn!("http_chat_completions streaming error: {error}");
                            let error_data = serde_json::json!({
                                "error": {
                                    "message": "Upstream service error",
                                    "type": "upstream_error",
                                    "code": "upstream_error"
                                }
                            });
                            let _ = chunk_tx_ref.send(error_data.to_string()).await;
                            let _ = chunk_tx_ref.send("[DONE]".to_string()).await;
                        }
                    }
                }),
            )
            .await;

            if let Err(panic) = catch_result {
                tracing::error!("upstream streaming task panicked: {:?}", panic);
                // Send error + [DONE] to the client so the SSE stream doesn't hang
                let error_data = serde_json::json!({
                    "error": {
                        "message": "Internal service error",
                        "type": "internal_error",
                        "code": "internal_error"
                    }
                });
                let _ = chunk_tx.send(error_data.to_string()).await;
                let _ = chunk_tx.send("[DONE]".to_string()).await;
            }
            // chunk_tx is dropped here, causing chunk_rx to return None
        });

        // Forwarding task: reads from the bounded chunk channel and sends to the
        // bounded SSE channel. Detects client disconnect via send failure and
        // triggers cancellation of the upstream task.
        let cancel_forwarder = cancel.clone();
        tokio::spawn(async move {
            let cancel_on_panic = cancel_forwarder.clone();
            let result = futures_util::future::FutureExt::catch_unwind(
                std::panic::AssertUnwindSafe(async move {
                    loop {
                        tokio::select! {
                            biased;
                            _ = cancel_forwarder.cancelled() => break,
                            data = chunk_rx.recv() => {
                                match data {
                                    Some(data) => {
                                        if sse_tx.send(Ok(Event::default().data(data))).await.is_err() {
                                            // SSE receiver dropped — client disconnected
                                            cancel_forwarder.cancel();
                                            break;
                                        }
                                    }
                                    None => break,
                                }
                            }
                        }
                    }
                }),
            )
            .await;

            if let Err(panic) = result {
                tracing::error!("forwarding task panicked: {:?}", panic);
                // Ensure upstream cancellation is triggered even if forwarding task panics
                cancel_on_panic.cancel();
            }
            // sse_tx dropped here, closing SSE channel
        });

        let stream = ReceiverStream::new(sse_rx);
        Sse::new(stream).into_response()
    } else {
        // Non-streaming mode: original behavior
        let answer = match ask_with_ai_with_context(
            &state.context,
            question,
            Some(history),
            if image_paths.is_empty() {
                None
            } else {
                Some(image_paths)
            },
            None,
            |_, _| (),
        )
        .await
        {
            Ok(a) => a,
            Err(error) => {
                tracing::warn!("http_chat_completions: upstream AI service error: {error}");
                return openai_error(StatusCode::BAD_GATEWAY, "Upstream service error")
                    .into_response();
            }
        };

        let prompt_tokens = answer
            .context_status
            .as_ref()
            .and_then(|status| status.last_request_input_tokens)
            .unwrap_or_default();
        let completion_tokens = answer
            .context_status
            .as_ref()
            .and_then(|status| status.last_request_output_tokens)
            .unwrap_or_default();

        Json(OpenAiChatCompletionsResponse {
            id: format!("chatcmpl-{}", Uuid::new_v4().simple()),
            object: "chat.completion",
            created: Utc::now().timestamp(),
            model: if requested_model.is_empty() {
                bridge_model_id(&settings)
            } else {
                requested_model
            },
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiAssistantMessage {
                    role: "assistant",
                    content: answer.answer,
                },
                finish_reason: "stop",
            }],
            usage: OpenAiUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            },
        })
        .into_response()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────

fn openai_request_to_dialog(
    request: OpenAiChatCompletionsRequest,
    vault_root: &Path,
) -> Result<(String, Vec<ConversationTurn>, Vec<String>), String> {
    let total_messages = request.messages.len();
    if total_messages == 0 {
        return Err("messages must not be empty".to_string());
    }

    let mut history = Vec::new();
    let mut question = None;
    let mut image_paths = Vec::new();

    for (index, message) in request.messages.into_iter().enumerate() {
        let is_last = index + 1 == total_messages;
        let (text, images) = render_openai_message_content(message.content, vault_root)?;
        if is_last {
            if message.role != "user" {
                return Err("the final message must have role=user".to_string());
            }
            question = Some(text);
            image_paths = images;
        } else if !text.trim().is_empty() {
            history.push(ConversationTurn {
                role: message.role,
                text,
            });
        }
    }

    let question = question
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() || !image_paths.is_empty())
        .ok_or_else(|| {
            "the final user message must include text or supported local image paths".to_string()
        })?;
    let question = if question.is_empty() && !image_paths.is_empty() {
        "请结合我发送的图片理解并回复。".to_string()
    } else {
        question
    };

    Ok((question, history, image_paths))
}

fn render_openai_message_content(
    content: OpenAiMessageContent,
    vault_root: &Path,
) -> Result<(String, Vec<String>), String> {
    match content {
        OpenAiMessageContent::Text(text) => Ok((text, Vec::new())),
        OpenAiMessageContent::Parts(parts) => {
            let mut segments = Vec::new();
            let mut image_paths = Vec::new();
            for part in parts {
                match part.kind.as_str() {
                    "text" => {
                        if let Some(text) = part.text.filter(|value| !value.trim().is_empty()) {
                            segments.push(text);
                        }
                    }
                    "image_url" => {
                        let url = part
                            .image_url
                            .map(|item| item.url)
                            .ok_or_else(|| "image_url part is missing url".to_string())?;
                        image_paths.push(resolve_local_image_url(&url, vault_root)?);
                    }
                    _ => {}
                }
            }
            Ok((segments.join("\n"), image_paths))
        }
    }
}

fn resolve_local_image_url(url: &str, vault_root: &Path) -> Result<String, String> {
    if url.starts_with("file://") {
        // Parse as URL to properly decode percent-encoded characters (#773).
        // RFC 8089 file:// URLs encode spaces as %20, Unicode as %XX sequences, etc.
        let parsed = url::Url::parse(url).map_err(|e| {
            tracing::warn!("resolve_local_image_url: invalid file URL: {e}");
            "invalid file URL".to_string()
        })?;
        let path = parsed
            .to_file_path()
            .map_err(|_| "invalid file URL path".to_string())?;
        // Validate path is within the vault directory
        let resolved = normalize_tool_path(&path.to_string_lossy(), vault_root).map_err(|e| {
            tracing::warn!("resolve_local_image_url: path resolution failed: {e}");
            "image path is invalid".to_string()
        })?;
        return Ok(resolved.to_string_lossy().to_string());
    }

    // Validate path confinement BEFORE checking existence to prevent
    // file-existence probing via differing error messages (#768).
    let path_str = url;
    let resolved = normalize_tool_path(path_str, vault_root).map_err(|e| {
        tracing::warn!("resolve_local_image_url: path resolution failed: {e}");
        "image path is invalid".to_string()
    })?;
    if resolved.exists() {
        return Ok(resolved.to_string_lossy().to_string());
    }

    Err("only local file image URLs are supported".to_string())
}

fn bridge_model_id(settings: &AppSettings) -> String {
    let underlying = settings.effective_provider().model.trim();
    if underlying.is_empty() {
        "vaultpilot-chat".to_string()
    } else {
        format!("vaultpilot-chat:{}", underlying)
    }
}

pub(super) fn normalize_bridge_token(token: Option<String>) -> Option<String> {
    token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn validate_http_bridge_binding(ip: IpAddr, token: Option<&str>) -> Result<()> {
    if ip.is_loopback() || token.is_some() {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "non-loopback host '{}' requires --token",
        ip
    ))
}

/// Constant-time byte-slice comparison to prevent timing side-channel attacks.
/// Length comparison is not constant-time (length is not secret), but the
/// byte-level comparison uses `subtle::ConstantTimeEq` to prevent leaking
/// the token content via timing.  The previous 256-byte fixed-buffer approach
/// had a correctness bug: tokens longer than 256 bytes that differed only
/// after byte 256 were incorrectly reported as equal (#660).
pub(super) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    // When lengths match, compare every byte in constant time.
    bool::from(a.ct_eq(b))
}

/// #3964 — non-interactive URI safety gate for the HTTP bridge.
///
/// The bridge cannot present a confirmation dialog, so mutating/destructive
/// endpoints are routed through the shared headless policy
/// (`deep_link::should_allow_tool_non_interactive`): Low always allowed,
/// Medium only from a trusted source, High always denied. `endpoint` is the
/// handler name (e.g. `"http_create_note"`); ungated (read-only) endpoints
/// return `None` from `automation_tool_gate` and stay ungated.
///
/// The caller app name is taken from the `x-vaultpilot-source` header
/// (mirrors the CLI's x-callback `x-source`); an absent header is treated
/// as an untrusted source.
fn enforce_uri_gate(
    state: &HttpBridgeState,
    headers: &HeaderMap,
    endpoint: &str,
) -> Result<(), (StatusCode, Json<OpenAiErrorEnvelope>)> {
    use vaultpilot_lib::deep_link::{should_allow_tool_non_interactive, TrustedAppRegistry};
    let source = headers
        .get("x-vaultpilot-source")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let trusted = TrustedAppRegistry::load(state.context.vault_dir());
    should_allow_tool_non_interactive(endpoint, &source, &trusted)
        .map_err(|message| openai_error(StatusCode::FORBIDDEN, &message))
}

fn require_bridge_token(
    state: &HttpBridgeState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<OpenAiErrorEnvelope>)> {
    let Some(expected) = state.token.as_deref() else {
        return Ok(());
    };

    let Some(actual) = bridge_token_from_headers(headers) else {
        return Err(openai_error(
            StatusCode::UNAUTHORIZED,
            "missing authorization token",
        ));
    };

    if !constant_time_eq(actual.as_bytes(), expected.as_bytes()) {
        return Err(openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid authorization token",
        ));
    }

    Ok(())
}

pub(super) fn bridge_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
    {
        if let Some((scheme, token)) = value.split_once(' ') {
            if scheme.eq_ignore_ascii_case("bearer") {
                let token = token.trim();
                if !token.is_empty() {
                    return Some(token);
                }
            }
        }
    }

    headers
        .get("x-vaultpilot-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn openai_error(status: StatusCode, message: &str) -> (StatusCode, Json<OpenAiErrorEnvelope>) {
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "authentication_error",
        StatusCode::NOT_FOUND => "not_found_error",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        s if s.is_client_error() => "invalid_request_error",
        _ => "api_error",
    };
    (
        status,
        Json(OpenAiErrorEnvelope {
            error: OpenAiError {
                message: message.to_string(),
                kind,
            },
        }),
    )
}

/// Returns true iff `origin` is a loopback origin (localhost, 127.0.0.1,
/// or IPv6 [::1]) over either http or https, on any port. Used by the CORS
/// `allow_origin` predicate so that HTTPS-fronted local clients (TLS-terminating
/// dev proxy, Electron/Obsidian frontends served over https) are not rejected.
///
/// The bridge always binds to loopback and is never exposed, so allowing
/// `https://localhost` / `https://127.0.0.1` / `https://[::1]` introduces
/// no security risk.
pub(crate) fn is_loopback_origin(origin: &axum::http::HeaderValue) -> bool {
    let o = origin.to_str().unwrap_or("");
    let is_loopback_http = o.starts_with("http://localhost:")
        || o.starts_with("http://127.0.0.1:")
        || o.starts_with("http://[::1]:")
        || o == "http://localhost"
        || o == "http://127.0.0.1"
        || o == "http://[::1]";
    let is_loopback_https = o.starts_with("https://localhost:")
        || o.starts_with("https://127.0.0.1:")
        || o.starts_with("https://[::1]:")
        || o == "https://localhost"
        || o == "https://127.0.0.1"
        || o == "https://[::1]";
    is_loopback_http || is_loopback_https
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_loopback_origin (CORS predicate, #3023) ────────────────

    fn hv(s: &str) -> axum::http::HeaderValue {
        axum::http::HeaderValue::from_str(s).unwrap()
    }

    #[test]
    fn loopback_allows_http_localhost_with_port() {
        assert!(is_loopback_origin(&hv("http://localhost:3000")));
    }

    #[test]
    fn loopback_allows_http_127_with_port() {
        assert!(is_loopback_origin(&hv("http://127.0.0.1:8080")));
    }

    #[test]
    fn loopback_allows_https_localhost_with_port() {
        // #3023: HTTPS-fronted local clients must not be rejected.
        assert!(is_loopback_origin(&hv("https://localhost:3000")));
    }

    #[test]
    fn loopback_allows_https_127_with_port() {
        assert!(is_loopback_origin(&hv("https://127.0.0.1:8443")));
    }

    #[test]
    fn loopback_allows_exact_http_localhost() {
        assert!(is_loopback_origin(&hv("http://localhost")));
        assert!(is_loopback_origin(&hv("http://127.0.0.1")));
    }

    #[test]
    fn loopback_allows_exact_https_localhost() {
        assert!(is_loopback_origin(&hv("https://localhost")));
        assert!(is_loopback_origin(&hv("https://127.0.0.1")));
    }

    #[test]
    fn loopback_rejects_non_loopback() {
        assert!(!is_loopback_origin(&hv("http://example.com")));
        assert!(!is_loopback_origin(&hv("https://example.com")));
        assert!(!is_loopback_origin(&hv("http://192.168.1.5:3000")));
    }

    #[test]
    fn loopback_rejects_garbage_origin() {
        assert!(!is_loopback_origin(&hv("not a valid origin")));
        assert!(!is_loopback_origin(&hv("ftp://localhost:21")));
    }

    // ── IPv6 loopback [::1] (#3495) ──────────────────────────────

    #[test]
    fn loopback_allows_http_ipv6_with_port() {
        assert!(is_loopback_origin(&hv("http://[::1]:3000")));
        assert!(is_loopback_origin(&hv("http://[::1]:8080")));
    }

    #[test]
    fn loopback_allows_https_ipv6_with_port() {
        assert!(is_loopback_origin(&hv("https://[::1]:8443")));
        assert!(is_loopback_origin(&hv("https://[::1]:443")));
    }

    #[test]
    fn loopback_allows_exact_http_ipv6() {
        assert!(is_loopback_origin(&hv("http://[::1]")));
    }

    #[test]
    fn loopback_allows_exact_https_ipv6() {
        assert!(is_loopback_origin(&hv("https://[::1]")));
    }

    // ── constant_time_eq ──────────────────────────────────────────

    #[test]
    fn constant_time_eq_identical() {
        assert!(constant_time_eq(b"secret-token", b"secret-token"));
    }

    #[test]
    fn constant_time_eq_different() {
        assert!(!constant_time_eq(b"secret-token", b"secret-tokez"));
    }

    #[test]
    fn constant_time_eq_different_length() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_long_identical() {
        let a = vec![0xABu8; 256];
        assert!(constant_time_eq(&a, &a));
    }

    // ── validate_http_bridge_binding ──────────────────────────────

    #[test]
    fn binding_loopback_without_token_ok() {
        assert!(validate_http_bridge_binding("127.0.0.1".parse().unwrap(), None).is_ok());
    }

    #[test]
    fn binding_loopback_with_token_ok() {
        assert!(validate_http_bridge_binding("127.0.0.1".parse().unwrap(), Some("t")).is_ok());
    }

    #[test]
    fn binding_non_loopback_without_token_fails() {
        let err = validate_http_bridge_binding("192.168.1.1".parse().unwrap(), None);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("--token"));
    }

    #[test]
    fn binding_non_loopback_with_token_ok() {
        assert!(
            validate_http_bridge_binding("192.168.1.1".parse().unwrap(), Some("mytoken")).is_ok()
        );
    }

    // ── normalize_bridge_token ────────────────────────────────────

    #[test]
    fn normalize_token_trims() {
        assert_eq!(
            normalize_bridge_token(Some("  tok  ".into())),
            Some("tok".into())
        );
    }

    #[test]
    fn normalize_token_none_is_none() {
        assert_eq!(normalize_bridge_token(None), None);
    }

    #[test]
    fn normalize_token_empty_is_none() {
        assert_eq!(normalize_bridge_token(Some("".into())), None);
    }

    #[test]
    fn normalize_token_whitespace_only_is_none() {
        assert_eq!(normalize_bridge_token(Some("   ".into())), None);
    }

    // ── bridge_token_from_headers ─────────────────────────────────

    #[test]
    fn token_from_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer my-token".parse().unwrap());
        assert_eq!(bridge_token_from_headers(&headers), Some("my-token"));
    }

    #[test]
    fn token_from_custom_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-vaultpilot-token", "custom-tok".parse().unwrap());
        assert_eq!(bridge_token_from_headers(&headers), Some("custom-tok"));
    }

    #[test]
    fn token_missing_returns_none() {
        let headers = HeaderMap::new();
        assert_eq!(bridge_token_from_headers(&headers), None);
    }

    #[test]
    fn token_bearer_empty_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer  ".parse().unwrap());
        assert_eq!(bridge_token_from_headers(&headers), None);
    }

    #[test]
    fn token_non_bearer_scheme_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Basic abc".parse().unwrap());
        assert_eq!(bridge_token_from_headers(&headers), None);
    }

    #[test]
    fn token_bearer_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "bearer my-token".parse().unwrap());
        assert_eq!(bridge_token_from_headers(&headers), Some("my-token"));
    }

    // ── bridge_model_id ───────────────────────────────────────────

    #[test]
    fn model_id_with_model() {
        let mut settings = AppSettings::default();
        settings.provider.model = "deepseek-v3".into();
        assert_eq!(bridge_model_id(&settings), "vaultpilot-chat:deepseek-v3");
    }

    #[test]
    fn model_id_empty_model() {
        let mut settings = AppSettings::default();
        settings.provider.model = "".into();
        assert_eq!(bridge_model_id(&settings), "vaultpilot-chat");
    }

    #[test]
    fn model_id_whitespace_model() {
        let mut settings = AppSettings::default();
        settings.provider.model = "  ".into();
        assert_eq!(bridge_model_id(&settings), "vaultpilot-chat");
    }

    // ── openai_request_to_dialog ──────────────────────────────────

    #[test]
    fn dialog_empty_messages_errors() {
        let req = OpenAiChatCompletionsRequest {
            model: "test".into(),
            messages: vec![],
            stream: false,
        };
        assert!(openai_request_to_dialog(req, Path::new("/vault")).is_err());
    }

    #[test]
    fn dialog_last_not_user_errors() {
        let req = OpenAiChatCompletionsRequest {
            model: "test".into(),
            messages: vec![OpenAiChatMessage {
                role: "assistant".into(),
                content: OpenAiMessageContent::Text("hi".into()),
            }],
            stream: false,
        };
        assert!(openai_request_to_dialog(req, Path::new("/vault")).is_err());
    }

    #[test]
    fn dialog_single_user_message() {
        let req = OpenAiChatCompletionsRequest {
            model: "test".into(),
            messages: vec![OpenAiChatMessage {
                role: "user".into(),
                content: OpenAiMessageContent::Text("hello".into()),
            }],
            stream: false,
        };
        let (question, history, images) =
            openai_request_to_dialog(req, Path::new("/vault")).unwrap();
        assert_eq!(question, "hello");
        assert!(history.is_empty());
        assert!(images.is_empty());
    }

    #[test]
    fn dialog_multi_turn_preserves_history() {
        let req = OpenAiChatCompletionsRequest {
            model: "test".into(),
            messages: vec![
                OpenAiChatMessage {
                    role: "user".into(),
                    content: OpenAiMessageContent::Text("first".into()),
                },
                OpenAiChatMessage {
                    role: "assistant".into(),
                    content: OpenAiMessageContent::Text("reply".into()),
                },
                OpenAiChatMessage {
                    role: "user".into(),
                    content: OpenAiMessageContent::Text("second".into()),
                },
            ],
            stream: false,
        };
        let (question, history, _) = openai_request_to_dialog(req, Path::new("/vault")).unwrap();
        assert_eq!(question, "second");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].text, "first");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].text, "reply");
    }

    #[test]
    fn dialog_empty_last_user_with_images_ok() {
        let req = OpenAiChatCompletionsRequest {
            model: "test".into(),
            messages: vec![OpenAiChatMessage {
                role: "user".into(),
                content: OpenAiMessageContent::Text("".into()),
            }],
            stream: false,
        };
        // Empty text with no images should fail
        assert!(openai_request_to_dialog(req, Path::new("/vault")).is_err());
    }

    // ── render_openai_message_content ─────────────────────────────

    #[test]
    fn render_text_content() {
        let (text, images) = render_openai_message_content(
            OpenAiMessageContent::Text("hello".into()),
            Path::new("/vault"),
        )
        .unwrap();
        assert_eq!(text, "hello");
        assert!(images.is_empty());
    }

    #[test]
    fn render_parts_text_only() {
        let (text, images) = render_openai_message_content(
            OpenAiMessageContent::Parts(vec![OpenAiContentPart {
                kind: "text".into(),
                text: Some("hi".into()),
                image_url: None,
            }]),
            Path::new("/vault"),
        )
        .unwrap();
        assert_eq!(text, "hi");
        assert!(images.is_empty());
    }

    #[test]
    fn render_parts_multiple_text_joined() {
        let (text, _) = render_openai_message_content(
            OpenAiMessageContent::Parts(vec![
                OpenAiContentPart {
                    kind: "text".into(),
                    text: Some("a".into()),
                    image_url: None,
                },
                OpenAiContentPart {
                    kind: "text".into(),
                    text: Some("b".into()),
                    image_url: None,
                },
            ]),
            Path::new("/vault"),
        )
        .unwrap();
        assert_eq!(text, "a\nb");
    }

    #[test]
    fn render_parts_unknown_kind_ignored() {
        let (text, images) = render_openai_message_content(
            OpenAiMessageContent::Parts(vec![
                OpenAiContentPart {
                    kind: "text".into(),
                    text: Some("hi".into()),
                    image_url: None,
                },
                OpenAiContentPart {
                    kind: "unknown_type".into(),
                    text: None,
                    image_url: None,
                },
            ]),
            Path::new("/vault"),
        )
        .unwrap();
        assert_eq!(text, "hi");
        assert!(images.is_empty());
    }

    // ── RateLimiter ───────────────────────────────────────────────

    #[test]
    fn rate_limiter_allows_within_limit() {
        let rl = RateLimiter::new(3, std::time::Duration::from_secs(1));
        assert!(rl.check("key1"));
        assert!(rl.check("key1"));
        assert!(rl.check("key1"));
    }

    #[test]
    fn rate_limiter_blocks_over_limit() {
        let rl = RateLimiter::new(2, std::time::Duration::from_secs(60));
        assert!(rl.check("key1"));
        assert!(rl.check("key1"));
        assert!(!rl.check("key1"));
    }

    #[test]
    fn rate_limiter_independent_keys() {
        let rl = RateLimiter::new(1, std::time::Duration::from_secs(60));
        assert!(rl.check("a"));
        assert!(!rl.check("a"));
        assert!(rl.check("b"));
        assert!(!rl.check("b"));
    }

    // ── render_openai_message_content boundary ────────────────────

    #[test]
    fn render_empty_parts_array() {
        let (text, images) =
            render_openai_message_content(OpenAiMessageContent::Parts(vec![]), Path::new("/vault"))
                .unwrap();
        assert_eq!(text, "");
        assert!(images.is_empty());
    }

    #[test]
    fn render_whitespace_only_text_part_filtered() {
        let (text, images) = render_openai_message_content(
            OpenAiMessageContent::Parts(vec![OpenAiContentPart {
                kind: "text".into(),
                text: Some("   ".into()),
                image_url: None,
            }]),
            Path::new("/vault"),
        )
        .unwrap();
        // Whitespace-only text parts should be filtered out
        assert_eq!(text, "");
        assert!(images.is_empty());
    }

    #[test]
    fn render_text_part_with_none_text() {
        let (text, images) = render_openai_message_content(
            OpenAiMessageContent::Parts(vec![OpenAiContentPart {
                kind: "text".into(),
                text: None,
                image_url: None,
            }]),
            Path::new("/vault"),
        )
        .unwrap();
        assert_eq!(text, "");
        assert!(images.is_empty());
    }

    // ── bridge_model_id boundary ──────────────────────────────────

    #[test]
    fn model_id_with_colon_in_name() {
        let mut settings = AppSettings::default();
        settings.provider.model = "provider/model:v1".into();
        assert_eq!(
            bridge_model_id(&settings),
            "vaultpilot-chat:provider/model:v1"
        );
    }

    #[test]
    fn model_id_with_unicode() {
        let mut settings = AppSettings::default();
        settings.provider.model = "模型v1".into();
        assert_eq!(bridge_model_id(&settings), "vaultpilot-chat:模型v1");
    }

    // ── RateLimiter boundary ──────────────────────────────────────

    #[test]
    fn rate_limiter_zero_limit_blocks_immediately() {
        let rl = RateLimiter::new(0, std::time::Duration::from_secs(60));
        assert!(!rl.check("key"));
    }

    // ── openai_request_to_dialog boundary ─────────────────────────

    #[test]
    fn dialog_system_message_in_history() {
        let req = OpenAiChatCompletionsRequest {
            model: "test".into(),
            messages: vec![
                OpenAiChatMessage {
                    role: "system".into(),
                    content: OpenAiMessageContent::Text("You are helpful".into()),
                },
                OpenAiChatMessage {
                    role: "user".into(),
                    content: OpenAiMessageContent::Text("hello".into()),
                },
            ],
            stream: false,
        };
        let (question, history, _) = openai_request_to_dialog(req, Path::new("/vault")).unwrap();
        assert_eq!(question, "hello");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, "system");
        assert_eq!(history[0].text, "You are helpful");
    }

    #[test]
    fn dialog_whitespace_only_history_skipped() {
        let req = OpenAiChatCompletionsRequest {
            model: "test".into(),
            messages: vec![
                OpenAiChatMessage {
                    role: "user".into(),
                    content: OpenAiMessageContent::Text("   ".into()),
                },
                OpenAiChatMessage {
                    role: "assistant".into(),
                    content: OpenAiMessageContent::Text("   ".into()),
                },
                OpenAiChatMessage {
                    role: "user".into(),
                    content: OpenAiMessageContent::Text("real question".into()),
                },
            ],
            stream: false,
        };
        let (question, history, _) = openai_request_to_dialog(req, Path::new("/vault")).unwrap();
        assert_eq!(question, "real question");
        // Whitespace-only messages should be filtered from history
        assert!(
            history.is_empty(),
            "whitespace-only history should be empty, got {}",
            history.len()
        );
    }

    // ── #2128: streaming upstream timeout safety net ──────────────

    #[tokio::test]
    async fn streaming_upstream_timeout_aborts_on_stall() {
        // Mirrors the production select! in http_chat_completions' streaming
        // task: a non-cancelled CancellationToken, a bounded timeout, and a
        // stalled upstream (never resolves). The timeout branch must win and
        // emit an upstream_timeout error chunk followed by [DONE]. A short
        // test timeout is used instead of the 180s STREAM_UPSTREAM_TIMEOUT so
        // the test runs fast, but the structure is identical.
        use tokio::sync::mpsc;

        let (chunk_tx, mut chunk_rx) = mpsc::channel::<String>(64);
        let cancel = CancellationToken::new();
        let timeout = std::time::Duration::from_millis(50);

        tokio::spawn(async move {
            let chunk_tx = chunk_tx;
            let _result: () = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    return;
                }
                _ = tokio::time::sleep(timeout) => {
                    let error_data = serde_json::json!({
                        "error": {
                            "message": "Upstream service timed out",
                            "type": "upstream_timeout",
                            "code": "upstream_timeout"
                        }
                    });
                    let _ = chunk_tx.send(error_data.to_string()).await;
                    let _ = chunk_tx.send("[DONE]".to_string()).await;
                }
                // Stalled upstream: never resolves (simulates a provider that
                // accepted the connection but never sends a chunk).
                _ = std::future::pending::<()>() => {}
            };
        });

        let error_chunk = tokio::time::timeout(std::time::Duration::from_secs(2), chunk_rx.recv())
            .await
            .expect("timed out waiting for error chunk")
            .expect("channel closed before error chunk");
        assert!(
            error_chunk.contains("upstream_timeout"),
            "expected upstream_timeout error chunk, got: {error_chunk}"
        );

        let done = tokio::time::timeout(std::time::Duration::from_secs(2), chunk_rx.recv())
            .await
            .expect("timed out waiting for [DONE]")
            .expect("channel closed before [DONE]");
        assert_eq!(done, "[DONE]");
    }

    #[test]
    fn stream_upstream_timeout_is_finite_and_bounded() {
        // Guard: the safety-net timeout must be a finite, sane value. If someone
        // accidentally sets it to an effectively-infinite duration, the #2128
        // stall protection would be defeated.
        assert!(STREAM_UPSTREAM_TIMEOUT.as_secs() > 0);
        assert!(
            STREAM_UPSTREAM_TIMEOUT.as_secs() <= 600,
            "streaming upstream timeout should stay reasonable (<=600s), got {}s",
            STREAM_UPSTREAM_TIMEOUT.as_secs()
        );
    }

    // ── #2129: note-load error status classification ──────────────

    #[test]
    fn classify_note_load_error_not_found_is_404() {
        // The storage layer emits `NoteNotFound` wrapped in an `anyhow::Error`
        // for a genuinely absent note. This must classify as 404.
        let e = anyhow::Error::from(NoteNotFound("some-note-id".to_string()));
        assert_eq!(classify_note_load_error(&e), StatusCode::NOT_FOUND);
    }

    #[test]
    fn classify_note_load_error_db_failure_is_500() {
        // A DB / IO failure is NOT a 404 — it must be 500 so callers don't
        // mistake a transient storage failure for a permanently absent note.
        let e = anyhow::Error::msg("database is locked");
        assert_eq!(
            classify_note_load_error(&e),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn classify_note_load_error_io_failure_is_500() {
        // File IO errors (permission denied, missing file on disk) must be 500
        // and must not be misclassified as 404.
        let e = anyhow::Error::msg("Permission denied (os error 13): /vault/note.md");
        assert_eq!(
            classify_note_load_error(&e),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn classify_note_load_error_chained_not_found_is_404() {
        // When `NoteNotFound` appears in the error CHAIN (wrapped by another
        // error using `.context()`), it should still be recognized as 404.
        let inner = anyhow::Error::from(NoteNotFound("wrapped-id".to_string()));
        let outer = inner.context("failed during sync");
        assert_eq!(classify_note_load_error(&outer), StatusCode::NOT_FOUND);
    }
    #[test]
    fn vault_file_resolve_accepts_inside_vault() {
        // Unique per-test dir so parallel tests don't clobber each other.
        let tmp = std::env::temp_dir().join(format!("vp_test_vault_in_{}", std::process::id()));
        let _ = std::fs::create_dir_all(tmp.join("docs"));
        let _ = std::fs::write(tmp.join("docs/report.pdf"), b"%PDF-1.4");
        let _ = std::fs::write(tmp.join("note.md"), b"hello");

        // Normal nested path resolves to a canonical path inside the vault.
        let got = resolve_vault_file_path(&tmp, "docs/report.pdf");
        assert!(got.is_some());
        let p = got.unwrap();
        assert!(p.starts_with(tmp.canonicalize().unwrap()));
        assert!(p.ends_with("report.pdf"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn vault_file_resolve_blocks_dotdot_traversal() {
        let tmp = std::env::temp_dir().join(format!("vp_test_vault_dot_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        // A sibling directory OUTSIDE the vault that must never be served.
        let outside = std::env::temp_dir().join(format!("vp_test_outside_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&outside);
        let _ = std::fs::write(outside.join("secret.pdf"), b"%PDF-1.4 secret");

        // `../<outside>/secret.pdf` is a real file but lives outside the vault.
        let rel = format!(
            "../{}/secret.pdf",
            outside.file_name().unwrap().to_str().unwrap()
        );
        assert!(
            resolve_vault_file_path(&tmp, &rel).is_none(),
            "path traversal must be rejected even when the target file exists"
        );

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn vault_file_resolve_rejects_absolute_path() {
        let tmp = std::env::temp_dir().join(format!("vp_test_vault_abs_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);

        // An absolute path pointing at a system file must never resolve.
        assert!(resolve_vault_file_path(&tmp, "/etc/passwd").is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn vault_file_resolve_rejects_missing_file() {
        let tmp = std::env::temp_dir().join(format!("vp_test_vault_miss_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);

        // Non-existent file inside the vault must not resolve (no leak of
        // presence/absence).
        assert!(resolve_vault_file_path(&tmp, "does-not-exist.pdf").is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn vault_file_content_type_mapping() {
        let cases = [
            ("a.pdf", "application/pdf"),
            ("a.PDF", "application/pdf"),
            ("a.png", "image/png"),
            ("a.jpeg", "image/jpeg"),
            ("a.jpg", "image/jpeg"),
            ("a.gif", "image/gif"),
            ("a.webp", "image/webp"),
            ("a.svg", "image/svg+xml"),
            ("a.bmp", "image/bmp"),
            ("a.tiff", "image/tiff"),
            ("a.unknown", "application/octet-stream"),
            ("a", "application/octet-stream"),
        ];
        for (name, expected) in cases {
            assert_eq!(
                vault_file_content_type(Path::new(name)),
                expected,
                "content-type for {name}"
            );
        }
    }

    // ── #3034 Web Clipper: HTML → Markdown helpers ──────────────────

    #[test]
    fn clip_extract_html_title_basic() {
        let html = "<html><head><title>My Page Title</title></head><body>hi</body></html>";
        assert_eq!(extract_html_title(html).as_deref(), Some("My Page Title"));
    }

    #[test]
    fn clip_extract_html_title_case_insensitive_and_whitespace() {
        let html = r#"<HTML><HEAD><TITLE>  Spaced Title  </TITLE></HEAD></HTML>"#;
        assert_eq!(extract_html_title(html).as_deref(), Some("Spaced Title"));
    }

    #[test]
    fn clip_extract_html_title_with_attributes() {
        let html = r#"<html><head><title id="t">Title With Attr</title></head></html>"#;
        assert_eq!(extract_html_title(html).as_deref(), Some("Title With Attr"));
    }

    #[test]
    fn clip_extract_html_title_missing_returns_none() {
        assert!(extract_html_title("<html><body>no title</body></html>").is_none());
        assert!(extract_html_title("").is_none());
    }

    #[test]
    fn clip_extract_html_title_decodes_entities() {
        let html = "<title>Tom &amp; Jerry &lt;cartoon&gt;</title>";
        assert_eq!(
            extract_html_title(html).as_deref(),
            Some("Tom & Jerry <cartoon>")
        );
    }

    #[test]
    fn clip_decode_html_entities_all_common() {
        assert_eq!(
            decode_html_entities("&amp;&lt;&gt;&quot;&#39;&apos;&nbsp;"),
            "&<>\"'' "
        );
    }

    #[test]
    fn clip_strip_boilerplate_removes_script_style_nav() {
        let html = concat!(
            r#"<nav><a href="/">Home</a></nav>"#,
            r#"<header><h1>Site Header</h1></header>"#,
            r#"<article><p>Real content</p></article>"#,
            r#"<footer>Copyright</footer>"#,
            r#"<script>alert("xss")</script>"#,
            r#"<style>.x { color: red; }</style>"#,
        );
        let stripped = strip_boilerplate(html);
        assert!(!stripped.to_lowercase().contains("<script"));
        assert!(!stripped.to_lowercase().contains("<style"));
        assert!(!stripped.to_lowercase().contains("<nav"));
        assert!(!stripped.to_lowercase().contains("<header"));
        assert!(!stripped.to_lowercase().contains("<footer"));
        assert!(!stripped.to_lowercase().contains("alert"));
        // Article content survives
        assert!(stripped.contains("Real content"));
    }

    #[test]
    fn clip_html_to_markdown_headings() {
        let html = "<h1>Title</h1><h2>Subtitle</h2><p>Body text</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("## Subtitle"));
        assert!(md.contains("Body text"));
    }

    #[test]
    fn clip_html_to_markdown_unordered_list() {
        let html = "<ul><li>Apple</li><li>Banana</li></ul>";
        let md = html_to_markdown(html);
        assert!(md.contains("- Apple"));
        assert!(md.contains("- Banana"));
    }

    #[test]
    fn clip_html_to_markdown_code_block() {
        let html = "<pre><code>fn main() {}</code></pre>";
        let md = html_to_markdown(html);
        assert!(md.contains("```"));
        assert!(md.contains("fn main() {}"));
    }

    #[test]
    fn clip_html_to_markdown_blockquote_and_emphasis() {
        let html =
            "<blockquote>A quote</blockquote><p><strong>bold</strong> and <em>italic</em></p>";
        let md = html_to_markdown(html);
        assert!(md.contains("> A quote"));
        assert!(md.contains("**bold**"));
        assert!(md.contains("_italic_"));
    }

    #[test]
    fn clip_html_to_markdown_image_and_link() {
        // #3061: this test previously only asserted that the anchor text
        // ("link text") survived conversion. That check passed even though
        // the href was silently dropped — the bug the report was filed
        // against. The assertion below now requires the full Markdown
        // link syntax `[text](url)` to be present.
        let html =
            r#"<img src="https://x.com/a.png" alt="pic"><a href="https://x.com">link text</a>"#;
        let md = html_to_markdown(html);
        // Image Markdown syntax
        assert!(md.contains("![pic](https://x.com/a.png)"));
        // #3061: href MUST be preserved as a Markdown link, not just the
        // bare anchor text.
        assert!(
            md.contains("[link text](https://x.com)"),
            "#3061: link must render as '[link text](https://x.com)', got: {md:?}"
        );
    }

    // ────────────────────────────────────────────────────────────────────
    // #3061 regression: <a href> preservation across various edge cases.
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn clip_html_to_markdown_link_in_paragraph_context() {
        // #3061: link surrounded by prose must produce a valid inline
        // Markdown link, not strip the href.
        let html = r#"<p>Read <a href="https://example.com/article">more</a> here.</p>"#;
        let md = html_to_markdown(html);
        assert!(
            md.contains("[more](https://example.com/article)"),
            "#3061: inline link must keep href, got: {md:?}"
        );
        assert!(
            md.contains("Read") && md.contains("here."),
            "surrounding text must survive, got: {md:?}"
        );
    }

    #[test]
    fn clip_html_to_markdown_link_with_inline_formatting() {
        // #3061: inline formatting inside the anchor must end up inside
        // the link text — `<a href="x"><strong>bold</strong></a>` should
        // produce `[**bold**](x)`, not `**[bold](x)**` or worse.
        let html = r#"<a href="https://x.com"><strong>bold</strong></a>"#;
        let md = html_to_markdown(html);
        assert!(
            md.contains("[**bold**](https://x.com)"),
            "#3061: inline-formatted anchor text must render inside the link, got: {md:?}"
        );
    }

    #[test]
    fn clip_html_to_markdown_link_empty_href_falls_back_to_text() {
        // #3061: empty href degrades gracefully — the anchor text survives
        // as plain text rather than producing broken `[](text)` or similar.
        let html = r#"<a href="">anchor target</a>"#;
        let md = html_to_markdown(html);
        assert!(
            md.contains("anchor target"),
            "anchor text must survive even with empty href, got: {md:?}"
        );
        assert!(
            !md.contains("]("),
            "empty href must NOT produce Markdown link syntax, got: {md:?}"
        );
    }

    #[test]
    fn clip_html_to_markdown_link_missing_href_falls_back_to_text() {
        // #3061: `<a name="x">` (anchor target without href) must not
        // produce `[](x)` or any spurious Markdown link.
        let html = r#"<a name="section-1">Section heading</a>"#;
        let md = html_to_markdown(html);
        assert!(
            md.contains("Section heading"),
            "anchor text must survive, got: {md:?}"
        );
        assert!(
            !md.contains("]("),
            "missing href must NOT produce Markdown link syntax, got: {md:?}"
        );
    }

    #[test]
    fn clip_html_to_markdown_link_unclosed_does_not_panic() {
        // #3061: unclosed `<a>` (no matching `</a>`) must leave the text
        // as-is rather than panicking or producing malformed output.
        let html = r#"<a href="https://x.com">text without close"#;
        let md = html_to_markdown(html);
        // We don't assert exact format (implementation may choose to
        // flush pending links at EOF or leave them as plain text); we
        // only assert the function returns without panicking and the
        // anchor text survives.
        assert!(md.contains("text without close"));
    }

    #[test]
    fn clip_html_to_markdown_multiple_links() {
        // #3061: multiple consecutive links must each get their own href.
        let html = r#"<p><a href="https://a.com">first</a> <a href="https://b.com">second</a></p>"#;
        let md = html_to_markdown(html);
        assert!(
            md.contains("[first](https://a.com)"),
            "first link must keep href, got: {md:?}"
        );
        assert!(
            md.contains("[second](https://b.com)"),
            "second link must keep href, got: {md:?}"
        );
    }

    // ────────────────────────────────────────────────────────────────────
    // #3059 regression: validate_clip_url_host returns DNS pins, and
    // build_clip_client accepts them.
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn clip_validate_host_returns_empty_pins_for_public_literal_ip() {
        // #3059: the new contract returns Vec<(host, SocketAddr)>. For a
        // literal IP, no DNS pinning is needed (reqwest does not resolve
        // literal IPs), so the returned Vec is empty.
        let pins = validate_clip_url_host("http://8.8.8.8/")
            .await
            .expect("public literal IP should be allowed");
        assert!(
            pins.is_empty(),
            "#3059: literal-IP URL must return empty pins (no DNS to pin), got {pins:?}"
        );
    }

    #[test]
    fn clip_build_clip_client_succeeds_without_pins() {
        // #3059: building a client with no DNS pins (literal-IP case)
        // must succeed and return a usable reqwest::Client.
        let client = build_clip_client(&[]).expect("build with no pins must succeed");
        // Smoke-test the returned client by checking it has no-redirect
        // policy (we can't easily assert policy from outside reqwest, but
        // the existence of the client is enough).
        let _ = client;
    }

    #[test]
    fn clip_build_clip_client_succeeds_with_pins() {
        // #3059: building a client with DNS pins must succeed. The pin
        // itself doesn't have to be reachable — we just verify the
        // builder accepts the (host, SocketAddr) pairs without error.
        let pins = vec![(
            "example.test".to_string(),
            "203.0.113.1:80".parse::<std::net::SocketAddr>().unwrap(),
        )];
        let client = build_clip_client(&pins).expect("build with valid pins must succeed");
        let _ = client;
    }

    // ────────────────────────────────────────────────────────────────────
    // #3060 regression: push_chunk_with_cap enforces the size limit
    // without first buffering the oversized chunk.
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn clip_push_chunk_with_cap_accepts_within_limit() {
        // #3060: a chunk that keeps the running total under the cap is
        // appended normally.
        let mut buf = Vec::new();
        push_chunk_with_cap(&mut buf, b"hello", 100).expect("5 bytes < 100 cap");
        assert_eq!(buf, b"hello");

        // Second chunk still within cap.
        push_chunk_with_cap(&mut buf, b" world", 100).expect("11 bytes < 100 cap");
        assert_eq!(buf, b"hello world");
    }

    #[test]
    fn clip_push_chunk_with_cap_rejects_chunk_that_would_exceed_limit() {
        // #3060: a chunk that would push the total past the cap is
        // rejected AND must not be partially appended to `buf` — that's
        // the whole point of the streaming fix (don't materialise the
        // oversized body in memory).
        let mut buf = b"abc".to_vec();
        let err = push_chunk_with_cap(&mut buf, b"oversized chunk", 5)
            .expect_err("3 + 15 > 5 must be rejected");
        assert!(
            err.starts_with(CLIP_TOO_LARGE_MARKER),
            "#3060: rejection must carry the too-large marker for 413 mapping, got: {err}"
        );
        // Critical: buf must be unchanged — the oversized chunk was not
        // partially appended.
        assert_eq!(buf, b"abc", "#3060: oversized chunk must not be buffered");
    }

    #[test]
    fn clip_push_chunk_with_cap_boundary_exact_equal() {
        // #3060: a chunk that exactly reaches the cap (==) is accepted;
        // the cap is enforced as `>`, not `>=`.
        let mut buf = Vec::new();
        push_chunk_with_cap(&mut buf, b"abcd", 4).expect("4 == 4 must be accepted");
        assert_eq!(buf, b"abcd");
    }

    #[test]
    fn clip_push_chunk_with_cap_rejects_single_oversized_chunk() {
        // #3060: the report specifically calls out a malicious upstream
        // sending one large chunk (e.g. 10 MB) that would briefly allocate
        // 10 MB even though we'd reject it. The helper must reject this
        // without growing `buf`.
        let mut buf = Vec::new();
        let huge_chunk = vec![b'x'; 10 * 1024 * 1024]; // 10 MB
        let err = push_chunk_with_cap(&mut buf, &huge_chunk, 5 * 1024 * 1024)
            .expect_err("10 MB chunk must be rejected against 5 MB cap");
        assert!(err.starts_with(CLIP_TOO_LARGE_MARKER));
        assert!(
            buf.is_empty(),
            "#3060: 10 MB chunk must not be buffered before rejection"
        );
    }

    #[test]
    fn clip_html_to_markdown_collapses_excess_newlines() {
        let html = "<p>A</p><p>B</p><p>C</p>";
        let md = html_to_markdown(html);
        assert!(
            !md.contains("\n\n\n"),
            "triple newlines should be collapsed"
        );
    }

    #[test]
    fn clip_html_to_markdown_empty_yields_empty() {
        assert_eq!(html_to_markdown(""), "");
        assert_eq!(html_to_markdown("   \n  \n  "), "");
    }

    #[test]
    fn clip_extract_attr_quoted_double() {
        let tag = r#"a href="https://example.com" class="link""#;
        assert_eq!(
            extract_attr(tag, "href").as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn clip_extract_attr_quoted_single() {
        let tag = r#"img src='photo.jpg'"#;
        assert_eq!(extract_attr(tag, "src").as_deref(), Some("photo.jpg"));
    }

    #[test]
    fn clip_extract_attr_unquoted() {
        // Unquoted values are read until whitespace, '/', or '>'.
        let tag = r#"a href=page.html class="x""#;
        assert_eq!(extract_attr(tag, "href").as_deref(), Some("page.html"));
    }

    #[test]
    fn clip_extract_attr_missing_returns_none() {
        let tag = r#"a class="link""#;
        assert!(extract_attr(tag, "href").is_none());
    }

    // ── #3044: byte-offset corruption from `to_lowercase()` ────────
    //
    // These tests guard against the regression where offsets produced by
    // `to_lowercase().find(...)` were used to slice the original string. The
    // Turkish İ (U+0130) lowercases from 2 bytes to 3 bytes (`i` + combining
    // dot above), which previously caused panics in `strip_boilerplate` and
    // mis-extraction in `extract_html_title` / `extract_attr`.

    #[test]
    fn clip_find_ci_basic_ascii() {
        assert_eq!(find_ci("Hello World", "world"), Some(6));
        assert_eq!(find_ci("Hello World", "WORLD"), Some(6));
        assert_eq!(find_ci("Hello World", "xyz"), None);
        assert_eq!(find_ci("", "x"), None);
        // Empty needle matches at position 0.
        assert_eq!(find_ci("anything", ""), Some(0));
    }

    #[test]
    fn clip_find_ci_returns_original_string_offset() {
        // İ (U+0130) is 2 bytes; its lowercase form is 3 bytes. A naive
        // `s.to_lowercase().find("world")` would return a byte offset that
        // points into the wrong position when sliced against `s`. `find_ci`
        // must return an offset that is a valid char boundary of the original.
        let s = "İİİworld";
        let off = find_ci(s, "world").expect("needle must be found");
        assert_eq!(&s[off..], "world");
    }

    #[test]
    fn clip_extract_html_title_ascii_baseline() {
        let html = "<html><head><title>My Page</title></head></html>";
        assert_eq!(extract_html_title(html).as_deref(), Some("My Page"));
    }

    #[test]
    fn clip_extract_html_title_uppercase_tag() {
        let html = "<HEAD><TITLE>My Page</TITLE></HEAD>";
        assert_eq!(extract_html_title(html).as_deref(), Some("My Page"));
    }

    #[test]
    fn clip_extract_html_title_with_turkish_i_before_tag() {
        // #3044 regression: İ before `<title>` used to produce `"My Page<"`
        // (the trailing `<` came from the offset shift).
        let html = "İ<HEAD><TITLE>My Page</TITLE></HEAD>";
        assert_eq!(extract_html_title(html).as_deref(), Some("My Page"));
    }

    #[test]
    fn clip_extract_html_title_with_many_i_with_dot_above() {
        // 20 İ's followed by a title: emphasises the offset drift.
        let html = format!("{}<title>T</title>", "İ".repeat(20));
        assert_eq!(extract_html_title(&html).as_deref(), Some("T"));
    }

    #[test]
    fn clip_strip_boilerplate_ascii_baseline() {
        let html = "<nav>menu</nav><p>main</p>";
        let out = strip_boilerplate(html);
        assert!(!out.to_lowercase().contains("<nav"));
        assert!(out.contains("main"));
    }

    #[test]
    fn clip_strip_boilerplate_no_panic_with_turkish_i_before_nav() {
        // #3044 regression: previously panicked at
        // `replace_range(s..close_end, "")` because `s` was a `to_lowercase()`
        // offset (3 bytes/İ) applied to the original string (2 bytes/İ),
        // landing on a non-char-boundary.
        let html = format!("{}<nav>stuff</nav>after", "İ".repeat(20));
        let out = strip_boilerplate(&html); // must not panic
        assert!(!out.to_lowercase().contains("<nav"));
        assert!(out.contains("after"));
        // Turkish İ's before the tag must be preserved.
        assert_eq!(out.matches('İ').count(), 20);
    }

    #[test]
    fn clip_strip_boilerplate_no_panic_with_kelvin_sign() {
        // Kelvin sign U+212A is 3 bytes in UTF-8 but lowercases to ASCII 'k'
        // (1 byte) — the inverse byte-shrink case.
        let html = format!("{}<script>alert(1)</script>ok", "\u{212A}".repeat(15));
        let out = strip_boilerplate(&html); // must not panic
        assert!(!out.to_lowercase().contains("<script"));
        assert!(out.contains("ok"));
    }

    #[test]
    fn clip_extract_attr_ascii_baseline() {
        let tag = r#"img src="photo.jpg" alt="pic""#;
        assert_eq!(extract_attr(tag, "src").as_deref(), Some("photo.jpg"));
        assert_eq!(extract_attr(tag, "alt").as_deref(), Some("pic"));
    }

    #[test]
    fn clip_extract_attr_with_turkish_i_before_attr() {
        // #3044 regression: an İ before the attribute name shifted the offset
        // so the quote could not be located and the value was silently dropped.
        let tag = "İmg src=\"photo.jpg\"";
        assert_eq!(extract_attr(tag, "src").as_deref(), Some("photo.jpg"));
    }

    #[test]
    fn clip_extract_attr_uppercase_attr_name() {
        let tag = r#"IMG SRC="photo.jpg""#;
        assert_eq!(extract_attr(tag, "src").as_deref(), Some("photo.jpg"));
    }

    // ── #3040: SSRF IP classification ─────────────────────────────

    #[test]
    fn clip_ip_forbidden_ipv4_loopback() {
        assert!(ip_is_forbidden("127.0.0.1".parse().unwrap()));
        assert!(ip_is_forbidden("127.255.255.255".parse().unwrap()));
    }

    #[test]
    fn clip_ip_forbidden_ipv4_private() {
        assert!(ip_is_forbidden("10.0.0.1".parse().unwrap()));
        assert!(ip_is_forbidden("172.16.0.1".parse().unwrap()));
        assert!(ip_is_forbidden("172.31.255.255".parse().unwrap()));
        assert!(ip_is_forbidden("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn clip_ip_forbidden_ipv4_link_local_metadata_endpoint() {
        // The cloud metadata endpoint (AWS/GCE/Azure) is the headline SSRF
        // target from the issue.
        assert!(ip_is_forbidden("169.254.169.254".parse().unwrap()));
        assert!(ip_is_forbidden("169.254.0.1".parse().unwrap()));
    }

    #[test]
    fn clip_ip_forbidden_ipv4_multicast_unspecified_broadcast() {
        assert!(ip_is_forbidden("224.0.0.1".parse().unwrap()));
        assert!(ip_is_forbidden("0.0.0.0".parse().unwrap()));
        assert!(ip_is_forbidden("255.255.255.255".parse().unwrap()));
    }

    #[test]
    fn clip_ip_forbidden_ipv4_documentation() {
        // RFC 5737 documentation ranges.
        assert!(ip_is_forbidden("192.0.2.1".parse().unwrap()));
        assert!(ip_is_forbidden("198.51.100.1".parse().unwrap()));
        assert!(ip_is_forbidden("203.0.113.1".parse().unwrap()));
    }

    #[test]
    fn clip_ip_allowed_ipv4_public() {
        // 192.0.2.x is documentation; 8.8.8.8 is a real public DNS server.
        assert!(!ip_is_forbidden("8.8.8.8".parse().unwrap()));
        assert!(!ip_is_forbidden("1.1.1.1".parse().unwrap()));
        assert!(!ip_is_forbidden("203.0.114.1".parse().unwrap()));
    }

    #[test]
    fn clip_ip_forbidden_ipv6_loopback_unspecified_multicast() {
        assert!(ip_is_forbidden("::1".parse().unwrap()));
        assert!(ip_is_forbidden("::".parse().unwrap()));
        assert!(ip_is_forbidden("ff02::1".parse().unwrap()));
    }

    #[test]
    fn clip_ip_forbidden_ipv6_ula_and_link_local() {
        // Unique-local fc00::/7.
        assert!(ip_is_forbidden("fc00::1".parse().unwrap()));
        assert!(ip_is_forbidden("fd00::1".parse().unwrap()));
        // Link-local fe80::/10.
        assert!(ip_is_forbidden("fe80::1".parse().unwrap()));
        assert!(ip_is_forbidden("febf::1".parse().unwrap()));
    }

    #[test]
    fn clip_ip_allowed_ipv6_public() {
        // Real public IPv6 (Google's public DNS).
        assert!(!ip_is_forbidden("2001:4860:4860::8888".parse().unwrap()));
    }

    // We can't unit-test `validate_clip_url_host`'s DNS path deterministically
    // (it depends on the live resolver), but we can test the literal-IP branch
    // and the URL/scheme validation paths, which are the parts most likely to
    // regress.

    #[tokio::test]
    async fn clip_validate_host_rejects_loopback_literal_ip() {
        let err = validate_clip_url_host("http://127.0.0.1:8080/secret")
            .await
            .unwrap_err();
        assert!(err.contains("forbidden IP"), "got: {err}");
    }

    #[tokio::test]
    async fn clip_validate_host_rejects_metadata_endpoint_literal_ip() {
        let err = validate_clip_url_host("http://169.254.169.254/latest/meta-data/")
            .await
            .unwrap_err();
        assert!(err.contains("forbidden IP"), "got: {err}");
    }

    #[tokio::test]
    async fn clip_validate_host_rejects_ipv6_loopback_literal_ip() {
        let err = validate_clip_url_host("http://[::1]/").await.unwrap_err();
        assert!(err.contains("forbidden IP"), "got: {err}");
    }

    #[tokio::test]
    async fn clip_validate_host_rejects_non_http_scheme() {
        let err = validate_clip_url_host("file:///etc/passwd")
            .await
            .unwrap_err();
        assert!(err.contains("non-http(s) scheme"), "got: {err}");
    }

    #[tokio::test]
    async fn clip_validate_host_rejects_garbage_url() {
        let err = validate_clip_url_host("not a url at all")
            .await
            .unwrap_err();
        assert!(err.contains("invalid URL"), "got: {err}");
    }

    // ── Web Clipper frontmatter helpers (#3189) ──────────────────
    // These two pure functions are the contract between the browser
    // extension (extensions/clipper/*) and the vault: every clipped note
    // must be tagged `clipped` and carry a meaningful `source` so the
    // Reader Mode view (#3150) and `tag:clipped` queries can find it.

    #[test]
    fn clip_tags_empty_input_defaults_to_clipped() {
        assert_eq!(build_clip_tags(""), vec!["clipped".to_string()]);
        assert_eq!(build_clip_tags("   "), vec!["clipped".to_string()]);
    }

    #[test]
    fn clip_tags_explicit_list_keeps_clipped_appended() {
        let tags = build_clip_tags("article,rust");
        assert_eq!(
            tags,
            vec![
                "article".to_string(),
                "rust".to_string(),
                "clipped".to_string()
            ]
        );
    }

    #[test]
    fn clip_tags_does_not_duplicate_existing_clipped() {
        let tags = build_clip_tags("clipped,news");
        assert_eq!(tags, vec!["clipped".to_string(), "news".to_string()]);
    }

    #[test]
    fn clip_tags_trims_whitespace_and_drops_empty() {
        let tags = build_clip_tags("  a , , b ");
        assert_eq!(
            tags,
            vec!["a".to_string(), "b".to_string(), "clipped".to_string()]
        );
    }

    #[test]
    fn clip_source_records_url_when_present() {
        assert_eq!(
            build_clip_source("https://example.com/post"),
            "https://example.com/post"
        );
    }

    #[test]
    fn clip_source_falls_back_to_web_sentinel() {
        assert_eq!(build_clip_source(""), "web");
        assert_eq!(build_clip_source("   "), "web");
    }

    // ── #3478: folder import endpoint validation ──────────────────

    /// `ImportFolderRequest` should deserialize from camelCase JSON, matching
    /// the wire format sent by WinUI / mobile clients.
    #[test]
    fn import_folder_request_parses_camel_case() {
        let json = serde_json::json!({ "folderPath": "/tmp/notes" });
        let req: ImportFolderRequest = serde_json::from_value(json).expect("parse");
        assert_eq!(req.folder_path, "/tmp/notes");
    }

    /// Empty `folderPath` must still deserialize (the HTTP handler rejects it,
    /// not the deserializer) — ensures clients can send the field and get a
    /// proper 400 rather than a 422 parse error.
    #[test]
    fn import_folder_request_parses_empty_path() {
        let json = serde_json::json!({ "folderPath": "" });
        let req: ImportFolderRequest = serde_json::from_value(json).expect("parse empty");
        assert_eq!(req.folder_path, "");
    }

    /// Missing `folderPath` field must fail to deserialize (the handler relies
    /// on serde rejecting the request, not a manual check).
    #[test]
    fn import_folder_request_rejects_missing_field() {
        let json = serde_json::json!({ "path": "/tmp/notes" });
        let res: Result<ImportFolderRequest, _> = serde_json::from_value(json);
        assert!(res.is_err());
    }
}
