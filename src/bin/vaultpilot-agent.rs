use std::fs;
use std::io::{self, BufRead, Write};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use vaultpilot_lib::models::{AppSettings, ChatState, ConversationSummary, ConversationTurn};
use vaultpilot_lib::storage::{
    import_markdown_with_context, initialize_storage_with_context, list_notes_with_context,
    load_chat_state_with_context, rebuild_index_with_context, save_chat_state_with_context,
    save_settings_with_context, StorageContext,
};
use vaultpilot_lib::{
    ask_with_ai_with_context, compress_chat_history_with_context, normalize_tool_path,
};

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

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to initialize async runtime");

    const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

    for line in stdin.lock().lines() {
        let response = match line {
            Ok(line) => {
                match runtime.block_on(tokio::time::timeout(
                    REQUEST_TIMEOUT,
                    handle_line(&line, &mut stdout),
                )) {
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
                }
            }
            Err(error) => {
                log_agent_event("stdin_error", &format!("{error}"));
                break;
            }
        };

        if let Ok(serialized) = serde_json::to_string(&response) {
            if writeln!(stdout, "{serialized}").is_err() || stdout.flush().is_err() {
                log_agent_event("stdout_error", "stdout write failed, exiting agent loop");
                break;
            }
        }
    }
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
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

        default_hook(info);
    }));
}

fn log_agent_event(event: &str, detail: &str) {
    const MAX_LOG_SIZE: u64 = 512 * 1024; // 512 KB

    let log_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.local.vaultpilot");
    let _ = fs::create_dir_all(&log_dir);
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

async fn handle_line(line: &str, stdout: &mut impl Write) -> AgentResponse {
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

    let context = match StorageContext::for_sidecar() {
        Ok(context) => context,
        Err(error) => {
            return AgentResponse::error(
                request.id,
                "context_error",
                format!(
                    "failed to initialize backend context: {}",
                    vaultpilot_lib::sanitize_error(&error.to_string())
                ),
            );
        }
    };

    match handle_request(&context, &request, stdout).await {
        Ok(result) => AgentResponse::ok(request.id, result),
        Err(error) => AgentResponse::error(request.id, "request_failed", error),
    }
}

async fn handle_request(
    context: &StorageContext,
    request: &AgentRequest,
    stdout: &mut impl Write,
) -> Result<Value, String> {
    match request.method.as_str() {
        "ping" => Ok(serde_json::json!({ "ok": true })),
        "getSettings" => {
            let mut settings =
                initialize_storage_with_context(context).map_err(|e| e.to_string())?;
            settings.provider = settings.provider.masked();
            serde_json::to_value(&settings).map_err(|e| e.to_string())
        }
        "saveSettings" => {
            let params: SaveSettingsParams = parse_params(&request.params)?;
            serialize_result(save_settings_with_context(context, params.settings))
        }
        "loadChatState" => serialize_result(load_chat_state_with_context(context)),
        "saveChatState" => {
            let params: SaveChatStateParams = parse_params(&request.params)?;
            serialize_result(save_chat_state_with_context(context, &params.state))
        }
        "listNotes" => serialize_result(list_notes_with_context(context)),
        "importMarkdown" => {
            let params: ImportMarkdownParams = parse_params(&request.params)?;
            serialize_result(import_markdown_with_context(context, &params.paths))
        }
        "rebuildIndex" => serialize_result(rebuild_index_with_context(context)),
        "readImagePreview" => {
            let params: PathParams = parse_params(&request.params)?;
            let settings = initialize_storage_with_context(context).map_err(|e| e.to_string())?;
            let vault_root = Path::new(&settings.vault_dir);
            let confined =
                normalize_tool_path(&params.path, vault_root).map_err(|e| e.to_string())?;
            read_image_preview(&confined.to_string_lossy()).map(Value::String)
        }
        "openVaultDirectory" => {
            let params: PathParams = parse_params(&request.params)?;
            let settings = initialize_storage_with_context(context).map_err(|e| e.to_string())?;
            let vault_root = Path::new(&settings.vault_dir);
            let confined =
                normalize_tool_path(&params.path, vault_root).map_err(|e| e.to_string())?;
            open_vault_directory(&confined.to_string_lossy())?;
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
                |stage, detail| emit_agent_status(stdout, stage, detail),
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
                |stage, detail| emit_agent_status(stdout, stage, detail),
            )
            .await;
            serialize_string_result(result)
        }
        method => Err(format!("unknown method: {method}")),
    }
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
        .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
}

fn emit_agent_status(stdout: &mut impl Write, stage: &str, detail: String) {
    let event = AgentEvent {
        event: "agentStatus".to_string(),
        payload: AgentStatusPayload {
            stage: stage.to_string(),
            detail,
            timestamp: Utc::now().to_rfc3339(),
        },
    };

    if let Ok(serialized) = serde_json::to_string(&event) {
        if writeln!(stdout, "{serialized}").is_err() {
            return;
        }
        let _ = stdout.flush();
    }
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
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
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
