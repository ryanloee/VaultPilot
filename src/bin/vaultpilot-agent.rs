use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;

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
use vaultpilot_lib::{ask_with_ai_with_context, compress_chat_history_with_context};

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
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to initialize async runtime");

    for line in stdin.lock().lines() {
        let response = match line {
            Ok(line) => runtime.block_on(handle_line(&line, &mut stdout)),
            Err(error) => AgentResponse::error(
                String::new(),
                "read_error",
                format!("failed to read request: {error}"),
            ),
        };

        if let Ok(serialized) = serde_json::to_string(&response) {
            let _ = writeln!(stdout, "{serialized}");
            let _ = stdout.flush();
        }
    }
}

async fn handle_line(line: &str, stdout: &mut impl Write) -> AgentResponse {
    let request = match serde_json::from_str::<AgentRequest>(line) {
        Ok(request) => request,
        Err(error) => {
            return AgentResponse::error(
                String::new(),
                "invalid_request",
                format!("failed to parse request JSON: {error}"),
            );
        }
    };

    let context = match StorageContext::for_sidecar() {
        Ok(context) => context,
        Err(error) => {
            return AgentResponse::error(
                request.id,
                "context_error",
                format!("failed to initialize backend context: {error}"),
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
        "getSettings" => serialize_result(initialize_storage_with_context(context)),
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
            read_image_preview(&params.path).map(Value::String)
        }
        "openVaultDirectory" => {
            let params: PathParams = parse_params(&request.params)?;
            open_vault_directory(&params.path)?;
            Ok(serde_json::json!({ "ok": true }))
        }
        "askWithAi" => {
            let params: AskWithAiParams = parse_params(&request.params)?;
            let result = ask_with_ai_with_context(
                context,
                params.question,
                params.history,
                params.image_paths,
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
    serde_json::from_value(params.clone()).map_err(|error| error.to_string())
}

fn serialize_result<T>(result: anyhow::Result<T>) -> Result<Value, String>
where
    T: Serialize,
{
    result
        .map_err(|error| error.to_string())
        .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
}

fn serialize_string_result<T>(result: Result<T, String>) -> Result<Value, String>
where
    T: Serialize,
{
    result.and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
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
        let _ = writeln!(stdout, "{serialized}");
        let _ = stdout.flush();
    }
}

fn read_image_preview(path: &str) -> Result<String, String> {
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
