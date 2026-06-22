use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use uuid::Uuid;

use vaultpilot_lib::models::*;
use vaultpilot_lib::storage::{
    list_notes_async, load_note_async, load_settings_async, search_notes_async, StorageContext,
};
use vaultpilot_lib::{ask_with_ai_with_context, normalize_tool_path};

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
        .route("/api/notes", get(http_list_notes))
        .route("/api/notes/search", get(http_search_notes))
        .route("/api/notes/{note_id}", get(http_get_note))
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
    let notes = list_notes_async(&state.context)
        .await
        .map_err(|e| openai_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let notes: Vec<_> = notes.into_iter().take(limit).collect();
    Ok(Json(serde_json::json!({
        "notes": notes,
        "total": notes.len()
    })))
}

async fn http_get_note(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
    AxumPath(note_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let note = load_note_async(&state.context, &note_id)
        .await
        .map_err(|e| openai_error(StatusCode::NOT_FOUND, &format!("Note not found: {e}")))?;
    Ok(Json(serde_json::to_value(note).unwrap_or_default()))
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
        .map_err(|e| openai_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(Json(serde_json::json!({
        "notes": result.notes,
        "total": result.total
    })))
}

async fn http_health() -> Json<Value> {
    Json(serde_json::json!({
        "status": "ok"
    }))
}

async fn http_models(
    State(state): State<Arc<HttpBridgeState>>,
    headers: HeaderMap,
) -> Result<Json<OpenAiModelsResponse>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    let settings = load_settings_async(&state.context)
        .await
        .unwrap_or_default();
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
) -> Result<Json<OpenAiChatCompletionsResponse>, (StatusCode, Json<OpenAiErrorEnvelope>)> {
    require_bridge_token(&state, &headers)?;
    if request.stream {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "stream=true is not supported by VaultPilot yet",
        ));
    }

    let settings = load_settings_async(&state.context).await.map_err(|error| {
        tracing::warn!("http_chat_completions: failed to load settings: {error}");
        openai_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load settings")
    })?;
    let requested_model = request.model.trim().to_string();
    let vault_root = PathBuf::from(&settings.vault_dir);
    let (question, history, image_paths) = openai_request_to_dialog(request, &vault_root)
        .map_err(|message| openai_error(StatusCode::BAD_REQUEST, &message))?;

    let answer = ask_with_ai_with_context(
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
    .map_err(|error| {
        tracing::warn!("http_chat_completions: upstream AI service error: {error}");
        openai_error(StatusCode::BAD_GATEWAY, "Upstream service error")
    })?;

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

    Ok(Json(OpenAiChatCompletionsResponse {
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
    }))
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
        let parsed = url::Url::parse(url).map_err(|e| format!("invalid file URL: {}", e))?;
        let path = parsed
            .to_file_path()
            .map_err(|_| "invalid file URL path".to_string())?;
        // Validate path is within the vault directory
        let resolved =
            normalize_tool_path(&path.to_string_lossy(), vault_root).map_err(|e| e.to_string())?;
        return Ok(resolved.to_string_lossy().to_string());
    }

    // Validate path confinement BEFORE checking existence to prevent
    // file-existence probing via differing error messages (#768).
    let path_str = url;
    let resolved = normalize_tool_path(path_str, vault_root).map_err(|e| e.to_string())?;
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
    (
        status,
        Json(OpenAiErrorEnvelope {
            error: OpenAiError {
                message: message.to_string(),
                kind: "invalid_request_error",
            },
        }),
    )
}
