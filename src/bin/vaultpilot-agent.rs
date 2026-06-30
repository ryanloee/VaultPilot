use std::fs;
use std::io::{self, Read, Write};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing_subscriber::EnvFilter;
use vaultpilot_lib::agent::{
    AgentConfig, AgentEvent as LibAgentEvent, AgentPermission, AgentResourceLimits,
};
use vaultpilot_lib::ai::actions::{
    execute_ai_action, list_ai_actions, AiActionRequest, AiActionType,
};
use vaultpilot_lib::models::{AppSettings, ChatState, ConversationSummary, ConversationTurn};
use vaultpilot_lib::storage::{
    import_markdown_async, initialize_storage_async, list_notes_async, load_chat_state_async,
    rebuild_index_async, save_chat_state_async, save_settings_async, StorageContext,
};
use vaultpilot_lib::{
    ask_with_ai_with_context, compress_chat_history_with_context, normalize_tool_path,
};

// ── Agent session state ─────────────────────────────────────────────────
// Allows runAgent (background task) and respondToWriteApproval (main loop)
// to coordinate write-approval decisions via a oneshot channel.

/// Shared state for the active agent session's write-approval channel.
static AGENT_APPROVAL: StdMutex<Option<std::sync::mpsc::Sender<bool>>> = StdMutex::new(None);

/// Shared stdout writer — ensures atomic line writes from both the main loop
/// and the background agent task.
struct SharedWriter {
    inner: StdMutex<Box<dyn Write + Send>>,
}

impl SharedWriter {
    fn write_line(&self, line: &str) {
        if let Ok(mut w) = self.inner.lock() {
            let _ = writeln!(w, "{line}");
            let _ = w.flush();
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRequest {
    id: String,
    method: String,
    #[serde(default, rename = "params")]
    params: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentResponse {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<AgentError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentError {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentEvent<T> {
    event: String,
    payload: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentStatusPayload {
    stage: String,
    detail: String,
    timestamp: String,
}

fn main() {
    install_panic_hook();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("vaultpilot-agent starting");

    // Initialize configurable search rules from user's config directory
    let config_dir = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let rules_path = config_dir.join("search_rules.json");
    vaultpilot_lib::search_rules::SearchRules::init_from_file(&rules_path);

    let stdin = io::stdin();
    let shared_writer: Arc<SharedWriter> = Arc::new(SharedWriter {
        inner: StdMutex::new(Box::new(io::stdout())),
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to initialize async runtime");

    // Create StorageContext once and reuse across all requests.
    // StorageContext already caches the SQLite connection and AppSettings
    // internally via Arc<Mutex<Option<...>>>, so reusing it avoids
    // redundant path resolution and connection setup on every request.
    let context = match StorageContext::for_sidecar() {
        Ok(ctx) => ctx,
        Err(error) => {
            eprintln!("fatal: failed to initialize storage context: {}", error);
            std::process::exit(1);
        }
    };

    const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
    /// Maximum bytes allowed for a single JSON-RPC line on stdin.
    /// Prevents OOM from a malicious or buggy client sending an
    /// unbounded payload without a newline delimiter (#596).
    const MAX_LINE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

    let mut stdin_buf = Vec::new();
    loop {
        stdin_buf.clear();
        // Read in 8 KB chunks until newline, enforcing the size limit during
        // reading rather than after.  `read_line()` buffers the entire line
        // before returning, making the post-read size check ineffective
        // against payloads that never contain a newline (#641).
        // Using chunked reads instead of byte-by-byte read_exact to reduce
        // syscall count from O(n) to O(n/8192) (#907).
        const CHUNK_SIZE: usize = 8 * 1024;
        let mut chunk_buf = [0u8; CHUNK_SIZE];
        let mut exceeded = false;
        let mut lock = stdin.lock();
        'read: loop {
            // Determine how many bytes we can still accept.
            let capacity_left = MAX_LINE_BYTES.saturating_sub(stdin_buf.len());
            let want = capacity_left.min(CHUNK_SIZE);
            if want == 0 {
                // Already at limit — drain byte-by-byte until newline.
                exceeded = true;
                match lock.read(&mut chunk_buf[..1]) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if chunk_buf[0] == b'\n' {
                    break;
                }
                continue;
            }
            let n = match lock.read(&mut chunk_buf[..want]) {
                Ok(0) => break 'read, // EOF
                Ok(n) => n,
                Err(_) => break 'read,
            };
            // Scan the chunk for newline.
            if let Some(nl_pos) = chunk_buf[..n].iter().position(|&b| b == b'\n') {
                let end = nl_pos + 1; // include the newline
                let room = MAX_LINE_BYTES - stdin_buf.len();
                let take = end.min(room);
                stdin_buf.extend_from_slice(&chunk_buf[..take]);
                break 'read;
            }
            // No newline found in this chunk — append the whole thing.
            stdin_buf.extend_from_slice(&chunk_buf[..n]);
        }
        if stdin_buf.is_empty() && !exceeded {
            break; // EOF
        }
        if exceeded {
            log_agent_event(
                "stdin_error",
                &format!(
                    "stdin line exceeds {}MB limit",
                    MAX_LINE_BYTES / (1024 * 1024)
                ),
            );
            let response = AgentResponse::error(
                String::new(),
                "input_too_large",
                format!(
                    "stdin line exceeds {}MB limit",
                    MAX_LINE_BYTES / (1024 * 1024)
                ),
            );
            if let Ok(serialized) = serde_json::to_string(&response) {
                shared_writer.write_line(&serialized);
            }
            continue;
        }
        let line = match std::str::from_utf8(&stdin_buf) {
            Ok(s) => s.to_string(),
            Err(e) => {
                log_agent_event(
                    "stdin_error",
                    &vaultpilot_lib::sanitize_error("invalid UTF-8 in request"),
                );
                let response = AgentResponse::error(
                    String::new(),
                    "invalid_encoding",
                    vaultpilot_lib::sanitize_error(&format!("invalid UTF-8 in request: {e}")),
                );
                if let Ok(serialized) = serde_json::to_string(&response) {
                    shared_writer.write_line(&serialized);
                }
                continue;
            }
        };
        let line = line.trim_end_matches('\n').trim_end_matches('\r');
        let line = line.to_string();
        let response = match runtime.block_on(async {
            tokio::time::timeout(
                REQUEST_TIMEOUT,
                handle_line(&context, &line, &shared_writer),
            )
            .await
        }) {
            Ok(response) => response,
            Err(_elapsed) => {
                log_agent_event(
                    "request_timeout",
                    &format!("request timed out after {}s", REQUEST_TIMEOUT.as_secs()),
                );
                // We don't have the request id here, so use an empty string.
                // The client matches on the sequential id from stdin, so this
                // is acceptable — the client knows which request it sent last.
                AgentResponse::error(
                    String::new(),
                    "timeout",
                    format!("request timed out after {}s", REQUEST_TIMEOUT.as_secs()),
                )
            }
        };

        if let Ok(serialized) = serde_json::to_string(&response) {
            shared_writer.write_line(&serialized);
        }
    }
}

fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };

        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let sanitized_payload = vaultpilot_lib::sanitize_error(&payload);
        let message = format!("panic on thread '{thread_name}': {sanitized_payload} at {location}");
        log_agent_event("panic", &message);

        // Write sanitized output to stderr instead of calling the default hook,
        // which would print the raw (unsanitized) payload and leak secrets. (#924)
        eprintln!("{message}");
    }));
}

fn log_agent_event(event: &str, detail: &str) {
    const MAX_LOG_SIZE: u64 = 512 * 1024; // 512 KB

    let log_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.local.vaultpilot");
    if fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let log_path = log_dir.join("agent-crash.log");

    // Rotate log if it exceeds the size limit — keep the last 256 KB
    if let Ok(metadata) = fs::metadata(&log_path) {
        if metadata.len() > MAX_LOG_SIZE {
            let keep = 256 * 1024usize;
            if let Ok(data) = fs::read(&log_path) {
                let start = data.len().saturating_sub(keep);
                // Find the next newline so we start at a complete line boundary
                let start = data[start..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|p| start + p + 1)
                    .unwrap_or(start);
                let _ = fs::write(&log_path, &data[start..]);
            }
        }
    }

    let timestamp = Utc::now().to_rfc3339();
    let entry = format!("[{timestamp}] {event}: {detail}\n");
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| f.write_all(entry.as_bytes()));
}

async fn handle_line(
    context: &StorageContext,
    line: &str,
    writer: &Arc<SharedWriter>,
) -> AgentResponse {
    let request = match serde_json::from_str::<AgentRequest>(line) {
        Ok(request) => request,
        Err(error) => {
            return AgentResponse::error(
                String::new(),
                "invalid_request",
                format!(
                    "failed to parse request JSON: {}",
                    vaultpilot_lib::sanitize_error(&error.to_string())
                ),
            );
        }
    };

    match handle_request(context, &request, writer).await {
        Ok(result) => AgentResponse::ok(request.id, result),
        Err(error) => AgentResponse::error(request.id, "request_failed", error),
    }
}

async fn handle_request(
    context: &StorageContext,
    request: &AgentRequest,
    writer: &Arc<SharedWriter>,
) -> Result<Value, String> {
    match request.method.as_str() {
        "ping" => Ok(serde_json::json!({ "ok": true })),
        "getSettings" => {
            let mut settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            // Set provider to the active one so legacy consumers get the right config.
            settings.provider = settings.effective_provider().clone();
            settings.provider = settings.provider.masked();
            for p in &mut settings.providers {
                *p = p.masked();
            }
            serde_json::to_value(&settings)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "saveSettings" => {
            let params: SaveSettingsParams = parse_params(&request.params)?;
            let mut result = save_settings_async(context, params.settings)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            result.provider = result.provider.masked();
            for p in &mut result.providers {
                *p = p.masked();
            }
            serde_json::to_value(&result)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "loadChatState" => serialize_result(load_chat_state_async(context).await),
        "saveChatState" => {
            let params: SaveChatStateParams = parse_params(&request.params)?;
            serialize_result(save_chat_state_async(context, &params.state).await)
        }
        "listNotes" => serialize_result(list_notes_async(context).await),
        "importMarkdown" => {
            let params: ImportMarkdownParams = parse_params(&request.params)?;
            serialize_result(import_markdown_async(context, &params.paths).await)
        }
        "rebuildIndex" => serialize_result(rebuild_index_async(context).await),
        "readImagePreview" => {
            let params: PathParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let vault_root = Path::new(&settings.vault_dir);
            let confined = normalize_tool_path(&params.path, vault_root)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            read_image_preview(&confined.to_string_lossy())
                .map(Value::String)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e))
        }
        "openVaultDirectory" => {
            let params: PathParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let vault_root = Path::new(&settings.vault_dir);
            let confined = normalize_tool_path(&params.path, vault_root)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            open_vault_directory(&confined.to_string_lossy())
                .map_err(|e| vaultpilot_lib::sanitize_error(&e))?;
            Ok(serde_json::json!({ "ok": true }))
        }
        "askWithAi" => {
            let params: AskWithAiParams = parse_params(&request.params)?;
            let result = ask_with_ai_with_context(
                context,
                params.question,
                params.history,
                params.image_paths,
                params.model_override,
                |stage, detail| {
                    writer.write_line(
                        &serde_json::to_string(&AgentEvent {
                            event: "agentStatus".to_string(),
                            payload: AgentStatusPayload {
                                stage: stage.to_string(),
                                detail,
                                timestamp: Utc::now().to_rfc3339(),
                            },
                        })
                        .unwrap_or_default(),
                    )
                },
            )
            .await;
            serialize_string_result(result)
        }
        "compressChatHistory" => {
            let params: CompressChatHistoryParams = parse_params(&request.params)?;
            let result = compress_chat_history_with_context(
                context,
                params.summary,
                params.history,
                |stage, detail| {
                    writer.write_line(
                        &serde_json::to_string(&AgentEvent {
                            event: "agentStatus".to_string(),
                            payload: AgentStatusPayload {
                                stage: stage.to_string(),
                                detail,
                                timestamp: Utc::now().to_rfc3339(),
                            },
                        })
                        .unwrap_or_default(),
                    )
                },
            )
            .await;
            serialize_string_result(result)
        }
        "runAgent" => {
            let params: RunAgentParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let ctx = context.clone();
            let writer: Arc<SharedWriter> = Arc::clone(writer);
            let prompt = params.prompt.clone();
            let images = params.images.clone();
            let history = params.history.clone();
            let max_steps = params.max_steps.unwrap_or(20);
            let auto_approve = params.auto_approve.unwrap_or(false);

            // Spawn agent in background so main loop can continue processing
            // respondToWriteApproval requests.
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create agent runtime");
                rt.block_on(async move {
                    run_agent_task(
                        &settings,
                        &ctx,
                        &prompt,
                        &images,
                        &history,
                        max_steps,
                        auto_approve,
                        writer,
                    )
                    .await;
                });
            });

            Ok(serde_json::json!({ "status": "started" }))
        }
        "respondToWriteApproval" => {
            let params: RespondToWriteApprovalParams = parse_params(&request.params)?;
            let tx = AGENT_APPROVAL
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            match tx {
                Some(tx) => {
                    let _ = tx.send(params.approved);
                    Ok(serde_json::json!({ "ok": true }))
                }
                None => Err("no active agent session waiting for approval".to_string()),
            }
        }
        "executeAiAction" => {
            let params: ExecuteAiActionParams = parse_params(&request.params)?;
            let request = AiActionRequest {
                action: params.action,
                text: params.text.unwrap_or_default(),
                target_language: params.target_language,
                tone: params.tone,
                note_id: params.note_id,
                model: params.model_override,
            };
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let result = execute_ai_action(&settings, &request).await;
            serde_json::to_value(&result)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "listAiActions" => {
            let actions = list_ai_actions();
            serde_json::to_value(&actions)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        method => Err(format!("unknown method: {method}")),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteAiActionParams {
    action: AiActionType,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    tone: Option<String>,
    #[serde(default)]
    note_id: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
}

fn parse_params<T>(params: &Value) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(params.clone())
        .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
}

fn serialize_result<T>(result: anyhow::Result<T>) -> Result<Value, String>
where
    T: Serialize,
{
    result
        .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        })
}

fn serialize_string_result<T>(result: Result<T, anyhow::Error>) -> Result<Value, String>
where
    T: Serialize,
{
    result
        .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        })
}

/// Run an agent session in the background. Emits events via `writer` and
/// handles write-approval through the global `AGENT_APPROVAL` channel.
#[allow(clippy::too_many_arguments)]
async fn run_agent_task(
    settings: &AppSettings,
    context: &StorageContext,
    prompt: &str,
    images: &[String],
    history: &[vaultpilot_lib::models::ConversationTurn],
    max_steps: usize,
    auto_approve: bool,
    writer: Arc<SharedWriter>,
) {
    let config = AgentConfig {
        name: "vaultpilot-agent-ui".into(),
        permission: AgentPermission::ReadWrite,
        limits: AgentResourceLimits {
            max_duration: Duration::from_secs(300),
            max_tool_calls: max_steps as u64,
            max_tokens: 0,
        },
        ..AgentConfig::default()
    };

    let result = vaultpilot_lib::agent::run_agent(
        settings,
        context,
        prompt,
        images,
        history,
        config,
        |event| {
            match event {
                LibAgentEvent::Thinking { step } => {
                    emit_event(
                        &writer,
                        "thinking",
                        &format!("Step {step}"),
                        Some(*step),
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::ToolCall { step, tool, args } => {
                    emit_event(
                        &writer,
                        "toolCall",
                        &format!("Calling {tool}"),
                        Some(*step),
                        Some(tool),
                        Some(args),
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::ToolResult {
                    step,
                    tool,
                    result_preview,
                    is_error,
                } => {
                    emit_event(
                        &writer,
                        "toolResult",
                        result_preview,
                        Some(*step),
                        Some(tool),
                        None,
                        Some(result_preview),
                        Some(*is_error),
                    );
                    true
                }
                LibAgentEvent::FinalAnswer { text } => {
                    emit_event(&writer, "finalAnswer", text, None, None, None, None, None);
                    true
                }
                LibAgentEvent::StepLimitReached { steps } => {
                    emit_event(
                        &writer,
                        "stepLimitReached",
                        &format!("{steps} steps reached"),
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::TokenBudgetExceeded {
                    tokens_used,
                    budget,
                } => {
                    emit_event(
                        &writer,
                        "tokenBudgetExceeded",
                        &format!("{tokens_used}/{budget}"),
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::Timeout => {
                    emit_event(
                        &writer,
                        "timeout",
                        "Agent timed out",
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::Error { message } => {
                    emit_event(&writer, "error", message, None, None, None, None, None);
                    true
                }
                LibAgentEvent::WriteApprovalNeeded { tool, args } => {
                    emit_event(
                        &writer,
                        "writeApprovalNeeded",
                        &format!("Write approval needed for {tool}"),
                        None,
                        Some(tool),
                        Some(args),
                        None,
                        None,
                    );

                    if auto_approve {
                        return true;
                    }

                    // Wait for approval from the UI via respondToWriteApproval
                    let (tx, rx) = std::sync::mpsc::channel();
                    *AGENT_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
                    // Block until approval received (this runs in a background thread,
                    // not on the main stdin loop, so blocking is fine).
                    rx.recv().unwrap_or(false)
                }
            }
        },
    )
    .await;

    match result {
        Ok(result) => {
            let final_event = serde_json::json!({
                "event": "agentStatus",
                "payload": {
                    "stage": "agentCompleted",
                    "detail": result.answer,
                    "stepsUsed": result.steps_used,
                    "tokensUsed": result.tokens_used,
                    "timestamp": Utc::now().to_rfc3339()
                }
            });
            writer.write_line(&final_event.to_string());
        }
        Err(e) => {
            let err_event = serde_json::json!({
                "event": "agentStatus",
                "payload": {
                    "stage": "error",
                    "detail": vaultpilot_lib::sanitize_error(&e.to_string()),
                    "timestamp": Utc::now().to_rfc3339()
                }
            });
            writer.write_line(&err_event.to_string());
        }
    }

    // Clear any pending approval channel
    *AGENT_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[allow(clippy::too_many_arguments)]
fn emit_event(
    writer: &SharedWriter,
    stage: &str,
    detail: &str,
    step: Option<usize>,
    tool: Option<&str>,
    args: Option<&str>,
    result_preview: Option<&str>,
    is_error: Option<bool>,
) {
    let mut payload = serde_json::json!({
        "stage": stage,
        "detail": detail,
        "timestamp": Utc::now().to_rfc3339()
    });
    if let Some(s) = step {
        payload["step"] = serde_json::json!(s);
    }
    if let Some(t) = tool {
        payload["tool"] = serde_json::json!(t);
    }
    if let Some(a) = args {
        payload["args"] = serde_json::json!(a);
    }
    if let Some(r) = result_preview {
        payload["resultPreview"] = serde_json::json!(r);
    }
    if let Some(e) = is_error {
        payload["isError"] = serde_json::json!(e);
    }

    let event = serde_json::json!({ "event": "agentStatus", "payload": payload });
    writer.write_line(&event.to_string());
}

fn read_image_preview(path: &str) -> Result<String, String> {
    const MAX_IMAGE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_IMAGE_SIZE {
        return Err(format!(
            "image too large ({} bytes, limit is {} bytes): {}",
            metadata.len(),
            MAX_IMAGE_SIZE,
            path
        ));
    }

    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let media_type = match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => return Err("unsupported image format".to_string()),
    };

    Ok(format!(
        "data:{};base64,{}",
        media_type,
        STANDARD.encode(bytes)
    ))
}

fn open_vault_directory(path: &str) -> Result<(), String> {
    let target = Path::new(path);
    if !target.exists() {
        return Err("vault directory does not exist".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("opening directories is not supported on this platform".to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettingsParams {
    settings: AppSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveChatStateParams {
    state: ChatState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportMarkdownParams {
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathParams {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AskWithAiParams {
    question: String,
    #[serde(default)]
    history: Option<Vec<ConversationTurn>>,
    #[serde(default)]
    image_paths: Option<Vec<String>>,
    #[serde(default)]
    model_override: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompressChatHistoryParams {
    summary: Option<ConversationSummary>,
    history: Vec<ConversationTurn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunAgentParams {
    prompt: String,
    #[serde(default)]
    max_steps: Option<usize>,
    #[serde(default)]
    auto_approve: Option<bool>,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    history: Vec<vaultpilot_lib::models::ConversationTurn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondToWriteApprovalParams {
    approved: bool,
}

impl AgentResponse {
    fn ok(id: String, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: String, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(AgentError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_deserializes_from_json() {
        let json = json!({
            "id": "req-001",
            "method": "ping",
            "params": {}
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.id, "req-001");
        assert_eq!(request.method, "ping");
    }

    #[test]
    fn request_deserializes_with_missing_params() {
        let json = json!({
            "id": "req-002",
            "method": "listNotes"
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.params, Value::Null);
    }

    #[test]
    fn response_ok_serializes_without_error() {
        let response = AgentResponse::ok("req-001".into(), json!({"ok": true}));
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("\"id\":\"req-001\""));
        assert!(serialized.contains("\"result\""));
        assert!(!serialized.contains("\"error\""));
    }

    #[test]
    fn response_error_serializes_without_result() {
        let response = AgentResponse::error("req-002".into(), "invalid_request", "bad input");
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("\"id\":\"req-002\""));
        assert!(serialized.contains("\"error\""));
        assert!(!serialized.contains("\"result\""));
    }

    #[test]
    fn response_error_contains_code_and_message() {
        let response = AgentResponse::error("x".into(), "timeout", "timed out after 120s");
        let value: Value = serde_json::to_value(&response).unwrap();
        let error = &value["error"];
        assert_eq!(error["code"], "timeout");
        assert!(error["message"].as_str().unwrap().contains("120s"));
    }

    #[test]
    fn agent_event_serializes_with_camel_case() {
        let event = AgentEvent {
            event: "agentStatus".to_string(),
            payload: AgentStatusPayload {
                stage: "analyzing".to_string(),
                detail: "Analyzing request".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
            },
        };
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(serialized.contains("\"agentStatus\""));
        assert!(serialized.contains("\"stage\""));
        assert!(serialized.contains("\"analyzing\""));
    }

    #[test]
    fn request_with_complex_params() {
        let json = json!({
            "id": "req-003",
            "method": "askWithAi",
            "params": {
                "question": "What is Rust?",
                "history": [{"role": "user", "content": "hello"}],
                "imagePaths": ["/tmp/img.png"]
            }
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.method, "askWithAi");
        assert!(request.params.is_object());
        assert_eq!(request.params["question"], "What is Rust?");
    }

    #[test]
    fn request_roundtrip_json() {
        let original = json!({
            "id": "test-123",
            "method": "saveChatState",
            "params": {"state": {"sessions": []}}
        });
        let request: AgentRequest = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(request.id, "test-123");
        assert_eq!(request.method, "saveChatState");
        assert!(request.params.is_object());
    }

    #[test]
    fn invalid_json_returns_parse_error() {
        let result = serde_json::from_str::<AgentRequest>("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn empty_method_name_parses() {
        let json = json!({ "id": "req-004", "method": "" });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert!(request.method.is_empty());
    }

    #[test]
    fn non_utf8_bytes_rejected_by_from_utf8() {
        // Simulates the exact validation used in the agent stdin loop.
        // Invalid continuation byte 0xFF is never valid in UTF-8.
        let invalid_bytes: Vec<u8> = vec![0xFF, 0xFE, 0x80, 0x00];
        let result = std::str::from_utf8(&invalid_bytes);
        assert!(result.is_err(), "expected invalid UTF-8 to be rejected");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("invalid"),
            "error message should mention invalid UTF-8: {err_msg}"
        );
    }

    #[test]
    fn sanitize_error_applied_to_utf8_message() {
        // Ensure the error message goes through sanitize_error as required.
        let raw = "invalid UTF-8 in request: invalid utf-8 sequence of 1 bytes from index 0";
        let sanitized = vaultpilot_lib::sanitize_error(raw);
        assert!(
            sanitized.contains("invalid UTF-8 in request"),
            "sanitized message should retain the key information: {sanitized}"
        );
    }
}
