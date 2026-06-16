use std::fs;
use std::io::{self, Read, Write};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing_subscriber::EnvFilter;
use vaultpilot_lib::models::{AppSettings, ChatState, ConversationSummary, ConversationTurn};
use vaultpilot_lib::storage::{
    import_markdown_async, initialize_storage_async, list_notes_async, load_chat_state_async,
    rebuild_index_async, save_chat_state_async, save_settings_async, StorageContext,
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
    let mut stdout = io::stdout();
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
        // Read byte-by-byte until newline, enforcing the size limit during
        // reading rather than after.  `read_line()` buffers the entire line
        // before returning, making the post-read size check ineffective
        // against payloads that never contain a newline (#641).
        let mut byte = [0u8; 1];
        let mut exceeded = false;
        loop {
            match stdin.lock().read_exact(&mut byte) {
                Ok(()) => {}
                Err(_) => {
                    // EOF or read error
                    break;
                }
            }
            if stdin_buf.len() >= MAX_LINE_BYTES {
                exceeded = true;
                // Keep draining until newline so the stream stays in sync
                // for the next request, but don't buffer the excess.
                if byte[0] == b'\n' {
                    break;
                }
                continue;
            }
            stdin_buf.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
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
                let _ = writeln!(stdout, "{serialized}");
                let _ = stdout.flush();
            }
            continue;
        }
        let line = String::from_utf8_lossy(&stdin_buf);
        let line = line.trim_end_matches('\n').trim_end_matches('\r');
        let line = line.to_string();
        let response = match runtime.block_on(async {
            tokio::time::timeout(REQUEST_TIMEOUT, handle_line(&context, &line, &mut stdout)).await
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
    stdout: &mut impl Write,
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

    match handle_request(context, &request, stdout).await {
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
            let mut settings = initialize_storage_async(context)
                .await
                .map_err(|e| e.to_string())?;
            settings.provider = settings.provider.masked();
            serde_json::to_value(&settings).map_err(|e| e.to_string())
        }
        "saveSettings" => {
            let params: SaveSettingsParams = parse_params(&request.params)?;
            serialize_result(save_settings_async(context, params.settings).await)
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
                .map_err(|e| e.to_string())?;
            let vault_root = Path::new(&settings.vault_dir);
            let confined =
                normalize_tool_path(&params.path, vault_root).map_err(|e| e.to_string())?;
            read_image_preview(&confined.to_string_lossy()).map(Value::String)
        }
        "openVaultDirectory" => {
            let params: PathParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| e.to_string())?;
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
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        })
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
        if let Err(e) = stdout.flush() {
            eprintln!("[emit_agent_status] Failed to flush stdout: {e}");
        }
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
        let json = json!({
            "id": "req-004",
            "method": ""
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert!(request.method.is_empty());
    }
}
