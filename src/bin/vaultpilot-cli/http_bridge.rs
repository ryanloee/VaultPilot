use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
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
    create_subscription_async, delete_subscription_async, get_subscription_async,
    list_subscriptions_async, set_subscription_enabled_async, update_subscription_async,
};
use vaultpilot_lib::storage::{
    deep_search_notes_async, load_note_async, load_settings_async, save_note_async,
    search_notes_async, typeahead_search_async, NoteNotFound, StorageContext,
};
use vaultpilot_lib::{ask_with_ai_with_context, normalize_tool_path, run_single_subscription};

/// Maximum total wall-clock time an upstream AI streaming request may run in
/// the HTTP bridge's `stream: true` path. The `TimeoutLayer(180s)` on the
/// router does NOT cover the SSE body stream (its Response future resolves
/// immediately for SSE), so the streaming task needs its own cap. Without it, a
/// stalled upstream + a non-disconnecting client holds a tokio task, two bounded
/// channels, and an upstream HTTP connection indefinitely. (#2128)
const STREAM_UPSTREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

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
        .route("/api/notes/search", get(http_search_notes))
        .route("/api/notes/typeahead", get(http_typeahead))
        .route(
            "/api/notes/search/progressive",
            get(http_progressive_search),
        )
        .route("/api/notes/{note_id}", get(http_get_note))
        // Vault Health Dashboard (#2014)
        .route("/api/vault/health", get(http_vault_health))
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
                    // Only allow localhost/127.0.0.1 origins (any port).
                    let o = origin.to_str().unwrap_or("");
                    o.starts_with("http://localhost:")
                        || o.starts_with("http://127.0.0.1:")
                        || o == "http://localhost"
                        || o == "http://127.0.0.1"
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
    let state_clone = state.clone();
    let q = query_text.clone();
    let cancel = CancellationToken::new();

    tokio::spawn(async move {
        let sse_tx_done = sse_tx.clone();
        let cancel = cancel;
        let result = futures_util::future::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
            async move {
                // Stage 1: Keyword search (FTS5, fast)
                if cancel.is_cancelled() {
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
                        if cancel.is_cancelled() {
                            return;
                        }
                        let event = ProgressiveSearchEvent {
                            stage: "keyword".into(),
                            results: Some(result),
                            message: None,
                        };
                        let data = serde_json::to_string(&event).unwrap_or_default();
                        if sse_tx.send(Ok(Event::default().data(data))).await.is_err() {
                            cancel.cancel();
                            return;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("progressive keyword search failed: {e}");
                        let _ = sse_tx
                            .send(Ok(Event::default().data(
                                serde_json::json!({"stage": "error", "message": "Internal error"})
                                    .to_string(),
                            )))
                            .await;
                        return;
                    }
                }

                // Stage 2: Loading state
                if cancel.is_cancelled() {
                    return;
                }
                let loading = ProgressiveSearchEvent {
                    stage: "loading".into(),
                    results: None,
                    message: Some("正在查找更多相关笔记...".into()),
                };
                let data = serde_json::to_string(&loading).unwrap_or_default();
                if sse_tx.send(Ok(Event::default().data(data))).await.is_err() {
                    cancel.cancel();
                    return;
                }

                // Stage 3: Deep semantic search
                if cancel.is_cancelled() {
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
                        if cancel.is_cancelled() {
                            return;
                        }
                        let event = ProgressiveSearchEvent {
                            stage: "semantic".into(),
                            results: Some(result),
                            message: None,
                        };
                        let data = serde_json::to_string(&event).unwrap_or_default();
                        if sse_tx.send(Ok(Event::default().data(data))).await.is_err() {
                            cancel.cancel();
                            return;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("progressive semantic search failed: {e}");
                        // Still send a done event so the client knows search is complete
                        let _ = sse_tx
                            .send(Ok(Event::default()
                                .data(serde_json::json!({"stage": "done"}).to_string())))
                            .await;
                        return;
                    }
                }

                // Done
                if cancel.is_cancelled() {
                    return;
                }
                let _ = sse_tx
                    .send(Ok(
                        Event::default().data(serde_json::json!({"stage": "done"}).to_string())
                    ))
                    .await;
            },
        ))
        .await;

        if let Err(panic) = result {
            tracing::error!("progressive search background task panicked: {:?}", panic);
            let _ = sse_tx_done
                .send(Ok(Event::default().data(
                    serde_json::json!({"stage": "error", "message": "Internal error"}).to_string(),
                )))
                .await;
        }
    });

    let stream = ReceiverStream::new(sse_rx);
    Sse::new(stream).into_response()
}

/// POST /api/notes — Create a new vault note from clipped content (browser clipper MVP).
///
/// Accepts markdown content with metadata (title, source URL, tags, collection).
/// Returns the new note's ID and title.
async fn http_create_note(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(request): Json<CreateNoteRequest>,
) -> Result<Json<CreateNoteResponse>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;

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

    let source = if request.source_url.trim().is_empty() {
        "web".to_string()
    } else {
        request.source_url.clone()
    };

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
    model: Option<String>,
}

/// POST /api/ai/action — Execute an AI quick action (non-streaming).
async fn http_ai_action(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    Json(req): Json<AiActionHttpRequest>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let ai_request = AiActionRequest {
        action: req.action,
        text: req.text,
        target_language: req.target_language,
        tone: req.tone,
        note_id: req.note_id,
        model: req.model,
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
            vaultpilot_lib::prompting::general_chat_system_prompt(),
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
                        Ok(_) => {
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
                            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                                tracing::warn!("forwarding task: no progress for 5s, breaking to prevent permanent hang");
                                cancel_forwarder.cancel();
                                break;
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
    let underlying = settings.provider.model.trim();
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
