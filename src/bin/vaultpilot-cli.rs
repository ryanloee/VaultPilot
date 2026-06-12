use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Runtime;
use uuid::Uuid;

use vaultpilot_lib::models::*;
use vaultpilot_lib::storage::*;
use vaultpilot_lib::{
    ask_with_ai_with_context, chat_with_ai_with_context, compress_chat_history_with_context,
    normalize_tool_path,
};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_FALLBACK_PROTOCOL_VERSION: &str = "2024-11-05";
const MARKDOWN_OPEN_TAG: &str = "<vp-markdown>";
const MARKDOWN_CLOSE_TAG: &str = "</vp-markdown>";

#[derive(Parser)]
#[command(name = "vaultpilot-cli")]
#[command(about = "VaultPilot knowledge base management CLI")]
#[command(version)]
struct Cli {
    /// Override the vault directory
    #[arg(long, global = true)]
    vault_dir: Option<PathBuf>,

    /// Pretty-print JSON output
    #[arg(long, global = true)]
    pretty: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize storage and show resolved settings
    Init,

    /// Start a local chat-completions bridge so external agents talk to VaultPilot as a model endpoint
    Serve {
        /// Bind host, for example 127.0.0.1
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Bind port
        #[arg(long, default_value_t = 8765)]
        port: u16,

        /// Require this bearer token for non-loopback listeners
        #[arg(long)]
        token: Option<String>,
    },

    /// Talk to VaultPilot's built-in model with persisted chat sessions
    Chat {
        #[command(subcommand)]
        action: ChatActions,
    },

    /// View or update settings
    Settings {
        #[command(subcommand)]
        action: SettingsActions,
    },

    /// Manage notes
    Notes {
        #[command(subcommand)]
        action: NotesActions,
    },

    /// Manage the search index
    Index {
        #[command(subcommand)]
        action: IndexActions,
    },

    /// Ask a question against the knowledge base without persisting chat state
    Ask {
        /// The question to ask
        question: String,

        /// Image paths to include
        #[arg(long)]
        image: Vec<String>,

        /// Chat history as JSON (array of {role, text})
        #[arg(long)]
        history: Option<String>,
    },

    /// Compress chat history into a summary
    Compress {
        /// JSON array of conversation turns
        #[arg(long)]
        history: String,

        /// Existing summary JSON (optional)
        #[arg(long)]
        summary: Option<String>,
    },

    /// Start an MCP stdio server for VaultPilot's built-in model chat interface
    Mcp,
}

#[derive(Subcommand)]
enum ChatActions {
    /// Send a message through VaultPilot's built-in model and persisted session state
    Send {
        /// The message to send. Can be omitted when only sending images.
        message: Option<String>,

        /// Image paths to include
        #[arg(long)]
        image: Vec<String>,

        /// Target session ID. Defaults to the current session.
        #[arg(long)]
        session: Option<String>,

        /// Create a new session before sending this message
        #[arg(long)]
        new_session: bool,
    },

    /// List saved chat sessions
    Sessions,

    /// Print full chat state
    State,

    /// Create a new empty session
    New {
        /// Optional session title
        #[arg(long)]
        title: Option<String>,
    },

    /// Delete a session by ID
    Delete {
        /// Session ID
        id: String,
    },
}

#[derive(Subcommand)]
enum SettingsActions {
    /// Print current settings
    Get,

    /// Update settings from JSON on stdin
    Set,
}

#[derive(Subcommand)]
enum NotesActions {
    /// List all notes
    List {
        /// Maximum notes to return
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Get a single note by ID or path
    Get {
        /// Note ID or file path
        id: String,
    },

    /// Create or update a note (JSON on stdin)
    Create,

    /// Delete a note by ID
    Delete {
        /// Note ID
        id: String,
    },

    /// Search notes
    Search {
        /// Search text
        #[arg(long)]
        query: String,

        /// Filter by tags (comma-separated)
        #[arg(long)]
        tags: Option<String>,

        /// Filter by keywords (comma-separated)
        #[arg(long)]
        keywords: Option<String>,

        /// Maximum results
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Import markdown files
    Import {
        /// File or directory paths to import
        paths: Vec<String>,
    },
}

#[derive(Subcommand)]
enum IndexActions {
    /// Rebuild the search index from vault files
    Rebuild,
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

#[derive(Clone)]
struct HttpBridgeState {
    context: StorageContext,
    token: Option<String>,
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
struct OpenAiErrorEnvelope {
    error: OpenAiError,
}

#[derive(Debug, Serialize)]
struct OpenAiError {
    message: String,
    #[serde(rename = "type")]
    kind: &'static str,
}

fn main() {
    let cli = Cli::parse();
    let is_mcp = matches!(cli.command, Commands::Mcp);
    let serve_target = match &cli.command {
        Commands::Serve { host, port, token } => Some((host.clone(), *port, token.clone())),
        _ => None,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to initialize async runtime");

    let context = match StorageContext::for_cli(cli.vault_dir.clone()) {
        Ok(ctx) => ctx,
        Err(err) => exit_error(&cli.pretty, "context_error", err.to_string()),
    };

    if let Some((host, port, token)) = serve_target {
        if let Err(err) = runtime.block_on(run_http_bridge(context, host, port, token)) {
            eprintln!("HTTP bridge failed: {err}");
            process::exit(1);
        }
        return;
    }

    if is_mcp {
        if let Err(err) = run_mcp_server(&context, &runtime) {
            eprintln!("MCP server failed: {err}");
            process::exit(1);
        }
        return;
    }

    let result = runtime.block_on(handle_command(&context, &cli));
    match result {
        Ok(value) => exit_ok(&cli.pretty, value),
        Err(err) => exit_error(&cli.pretty, "command_failed", err.to_string()),
    }
}

async fn handle_command(context: &StorageContext, cli: &Cli) -> Result<Value> {
    match &cli.command {
        Commands::Init => {
            let settings = initialize_storage_with_context(context)?;
            to_json(&settings)
        }
        Commands::Serve { .. } => Ok(serde_json::json!({
            "message": "The HTTP bridge is started by running `vaultpilot-cli serve` directly."
        })),
        Commands::Chat { action } => handle_chat(context, action).await,
        Commands::Settings { action } => handle_settings(context, action),
        Commands::Notes { action } => handle_notes(context, action),
        Commands::Index { action } => handle_index(context, action),
        Commands::Ask {
            question,
            image,
            history,
        } => {
            let parsed_history: Option<Vec<ConversationTurn>> = history
                .as_ref()
                .map(|h| serde_json::from_str(h))
                .transpose()?;
            let images = if image.is_empty() {
                None
            } else {
                Some(image.clone())
            };
            let result = ask_with_ai_with_context(
                context,
                question.clone(),
                parsed_history,
                images,
                None,
                |_, _| (),
            )
            .await?;
            to_json(&strip_cli_markdown_from_grounded_answer(result))
        }
        Commands::Compress { history, summary } => {
            let parsed_history: Vec<ConversationTurn> = serde_json::from_str(history)?;
            let parsed_summary: Option<ConversationSummary> = summary
                .as_ref()
                .map(|s| serde_json::from_str(s))
                .transpose()?;
            let result = compress_chat_history_with_context(
                context,
                parsed_summary,
                parsed_history,
                |_, _| (),
            )
            .await?;
            to_json(&result)
        }
        Commands::Mcp => Ok(serde_json::json!({
            "message": "The MCP server is started by running `vaultpilot-cli mcp` directly."
        })),
    }
}

async fn handle_chat(context: &StorageContext, action: &ChatActions) -> Result<Value> {
    match action {
        ChatActions::Send {
            message,
            image,
            session,
            new_session,
        } => {
            let result = chat_with_ai_with_context(
                context,
                session.clone(),
                message.clone().unwrap_or_default(),
                if image.is_empty() {
                    None
                } else {
                    Some(image.clone())
                },
                *new_session,
                |_, _| (),
            )
            .await?;
            to_json(&strip_cli_markdown_from_chat_result(result))
        }
        ChatActions::Sessions => {
            let state = load_chat_state_with_context(context)?;
            let sessions = state
                .sessions
                .iter()
                .map(chat_session_overview)
                .collect::<Vec<_>>();
            Ok(serde_json::json!({
                "currentSessionId": state.current_session_id,
                "sessions": sessions
            }))
        }
        ChatActions::State => {
            let state = load_chat_state_with_context(context)?;
            to_json(&strip_cli_markdown_from_chat_state(state))
        }
        ChatActions::New { title } => {
            let mut state = load_chat_state_with_context(context)?;
            let session = new_cli_chat_session(title.as_deref());
            state.current_session_id = session.id.clone();
            state.sessions.insert(0, session.clone());
            let saved = save_chat_state_with_context(context, &state)?;
            Ok(serde_json::json!({
                "session": session,
                "state": strip_cli_markdown_from_chat_state(saved)
            }))
        }
        ChatActions::Delete { id } => {
            let mut state = load_chat_state_with_context(context)?;
            let original_len = state.sessions.len();
            state.sessions.retain(|session| session.id != *id);
            let deleted = state.sessions.len() != original_len;
            let saved = save_chat_state_with_context(context, &state)?;
            Ok(serde_json::json!({
                "deleted": deleted,
                "id": id,
                "state": strip_cli_markdown_from_chat_state(saved)
            }))
        }
    }
}

fn strip_cli_markdown_from_chat_result(mut result: ChatExchangeResult) -> ChatExchangeResult {
    result.answer = strip_cli_markdown_from_grounded_answer(result.answer);
    result.state = strip_cli_markdown_from_chat_state(result.state);
    result
}

fn strip_cli_markdown_from_grounded_answer(mut answer: GroundedAnswer) -> GroundedAnswer {
    answer.answer = simplify_cli_text(&answer.answer);
    answer.thinking_trace = None;
    answer
}

fn strip_cli_markdown_from_chat_state(mut state: ChatState) -> ChatState {
    for session in &mut state.sessions {
        for turn in &mut session.turns {
            if turn.role.eq_ignore_ascii_case("assistant") {
                turn.text = simplify_cli_text(&turn.text);
                turn.thinking_trace = None;
            }
        }
    }
    state
}

fn strip_markdown_wrapper_tags(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with(MARKDOWN_OPEN_TAG) && trimmed.ends_with(MARKDOWN_CLOSE_TAG) {
        return trimmed[MARKDOWN_OPEN_TAG.len()..trimmed.len() - MARKDOWN_CLOSE_TAG.len()]
            .trim()
            .to_string();
    }

    text.to_string()
}

fn simplify_cli_text(text: &str) -> String {
    let text = strip_markdown_wrapper_tags(text);
    let mut simplified = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        let line = if in_code_block {
            trimmed.to_string()
        } else {
            simplify_markdown_line(trimmed)
        };

        if line.is_empty() {
            if simplified
                .last()
                .is_some_and(|item: &String| !item.is_empty())
            {
                simplified.push(String::new());
            }
        } else {
            simplified.push(line);
        }
    }

    while simplified.last().is_some_and(|item| item.is_empty()) {
        simplified.pop();
    }

    simplified.join("\n")
}

fn simplify_markdown_line(line: &str) -> String {
    let without_heading = line.trim_start_matches('#').trim();
    let without_bullet = strip_markdown_list_marker(without_heading);
    strip_inline_markdown(without_bullet)
}

fn strip_markdown_list_marker(line: &str) -> &str {
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return rest.trim();
    }

    let bytes = line.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index > 0 && index + 1 < bytes.len() && bytes[index] == b'.' && bytes[index + 1] == b' ' {
        return line[index + 2..].trim();
    }

    line
}

fn strip_inline_markdown(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Inline code span: preserve everything inside backticks verbatim
            '`' => {
                result.push('`');
                for inner in chars.by_ref() {
                    result.push(inner);
                    if inner == '`' {
                        break;
                    }
                }
            }
            // Bold marker **: skip the ** but keep the content
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // consume second *
                              // Copy until closing **
                while let Some(inner) = chars.next() {
                    if inner == '*' && chars.peek() == Some(&'*') {
                        chars.next(); // consume closing *
                        break;
                    }
                    result.push(inner);
                }
            }
            // Strikethrough marker ~~: skip the ~~ but keep the content
            '~' if chars.peek() == Some(&'~') => {
                chars.next(); // consume second ~
                while let Some(inner) = chars.next() {
                    if inner == '~' && chars.peek() == Some(&'~') {
                        chars.next(); // consume closing ~
                        break;
                    }
                    result.push(inner);
                }
            }
            // Italic *: skip the * but keep the content
            '*' => {
                for inner in chars.by_ref() {
                    if inner == '*' {
                        break;
                    }
                    result.push(inner);
                }
            }
            // Italic/bold __: skip the __ but keep the content
            '_' if chars.peek() == Some(&'_') => {
                chars.next(); // consume second _
                while let Some(inner) = chars.next() {
                    if inner == '_' && chars.peek() == Some(&'_') {
                        chars.next(); // consume closing _
                        break;
                    }
                    result.push(inner);
                }
            }
            // Italic _: skip the _ but keep the content
            '_' => {
                for inner in chars.by_ref() {
                    if inner == '_' {
                        break;
                    }
                    result.push(inner);
                }
            }
            // Regular character: pass through
            _ => {
                result.push(c);
            }
        }
    }

    result
}

fn handle_settings(context: &StorageContext, action: &SettingsActions) -> Result<Value> {
    match action {
        SettingsActions::Get => {
            let settings = load_settings_with_context(context)?;
            to_json(&settings)
        }
        SettingsActions::Set => {
            let input = read_stdin_json()?;
            let settings: AppSettings = serde_json::from_value(input)?;
            let saved = save_settings_with_context(context, settings)?;
            to_json(&saved)
        }
    }
}

fn handle_notes(context: &StorageContext, action: &NotesActions) -> Result<Value> {
    match action {
        NotesActions::List { limit } => {
            let result = search_notes_with_context(
                context,
                SearchQuery {
                    text: String::new(),
                    tags: Vec::new(),
                    keywords: Vec::new(),
                    limit: Some(*limit),
                },
            )?;
            to_json(&result)
        }
        NotesActions::Get { id } => {
            let note = load_note_with_context(context, id)?;
            to_json(&note)
        }
        NotesActions::Create => {
            let input = read_stdin_json()?;
            let note: NoteDocument = serde_json::from_value(input)?;
            let saved = save_note_with_context(context, note)?;
            to_json(&saved)
        }
        NotesActions::Delete { id } => {
            let deleted = delete_note_with_context(context, id)?;
            Ok(serde_json::json!({ "deleted": deleted, "id": id }))
        }
        NotesActions::Search {
            query,
            tags,
            keywords,
            limit,
        } => {
            let result = search_notes_with_context(
                context,
                SearchQuery {
                    text: query.clone(),
                    tags: parse_comma_list(tags),
                    keywords: parse_comma_list(keywords),
                    limit: Some(*limit),
                },
            )?;
            to_json(&result)
        }
        NotesActions::Import { paths } => {
            let result = import_markdown_with_context(context, paths)?;
            to_json(&result)
        }
    }
}

fn handle_index(context: &StorageContext, action: &IndexActions) -> Result<Value> {
    match action {
        IndexActions::Rebuild => {
            let stats = rebuild_index_with_context(context)?;
            to_json(&stats)
        }
    }
}

async fn run_http_bridge(
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
    let state = Arc::new(HttpBridgeState { context, token });

    let app = Router::new()
        .route("/health", get(http_health))
        .route("/v1/models", get(http_models))
        .route("/v1/chat/completions", post(http_chat_completions))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024)) // 10 MB
        .with_state(state);

    println!(
        "{}",
        serde_json::json!({
            "status": "listening",
            "baseUrl": format!("http://{}:{}", ip, port),
            "chatCompletions": format!("http://{}:{}/v1/chat/completions", ip, port),
            "models": format!("http://{}:{}/v1/models", ip, port),
            "requiresToken": requires_token
        })
    );

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
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
    let settings = load_settings_with_context(&state.context).unwrap_or_default();
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

    let settings = load_settings_with_context(&state.context)
        .map_err(|error| openai_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()))?;
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
    .map_err(|error| openai_error(StatusCode::BAD_GATEWAY, &error.to_string()))?;

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
        let path = url.trim_start_matches("file://");
        // Validate path is within the vault directory
        let resolved = normalize_tool_path(path, vault_root).map_err(|e| e.to_string())?;
        return Ok(resolved.to_string_lossy().to_string());
    }

    let path_str = url;
    let path = PathBuf::from(path_str);
    if path.exists() {
        // Validate path is within the vault directory
        let resolved = normalize_tool_path(path_str, vault_root).map_err(|e| e.to_string())?;
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

fn normalize_bridge_token(token: Option<String>) -> Option<String> {
    token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_http_bridge_binding(ip: IpAddr, token: Option<&str>) -> Result<()> {
    if ip.is_loopback() || token.is_some() {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "non-loopback host '{}' requires --token",
        ip
    ))
}

/// Constant-time byte-slice comparison to prevent timing side-channel attacks.
/// Returns `true` if `a` and `b` have the same length and contents.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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

fn bridge_token_from_headers(headers: &HeaderMap) -> Option<&str> {
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

fn run_mcp_server(context: &StorageContext, runtime: &Runtime) -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut state = McpServerState {
        initialized: false,
        protocol_version: MCP_PROTOCOL_VERSION.to_string(),
    };

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<McpRequest>(&line) {
            Ok(request) => runtime.block_on(handle_mcp_request(context, &mut state, request)),
            Err(error) => Some(McpResponse::error(
                Value::Null,
                -32700,
                format!("failed to parse JSON-RPC request: {error}"),
                None,
            )),
        };

        if let Some(response) = response {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }
    }

    Ok(())
}

async fn handle_mcp_request(
    context: &StorageContext,
    state: &mut McpServerState,
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
            let id = request.id.unwrap_or(Value::Null);
            let requested_version = request
                .params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            state.initialized = true;
            state.protocol_version = negotiate_mcp_protocol_version(requested_version).to_string();

            Some(McpResponse::ok(
                id,
                serde_json::json!({
                    "protocolVersion": state.protocol_version,
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    },
                    "serverInfo": {
                        "name": "vaultpilot",
                        "title": "VaultPilot MCP",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": "Use chat.send to talk to VaultPilot through its built-in model. VaultPilot performs local retrieval and model calls internally; clients should treat it as a chat endpoint instead of direct note-search tooling."
                }),
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
        "resources/list" => request
            .id
            .map(|id| McpResponse::ok(id, serde_json::json!({ "resources": [] }))),
        "prompts/list" => request
            .id
            .map(|id| McpResponse::ok(id, serde_json::json!({ "prompts": [] }))),
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
                        "items": {
                            "type": "string"
                        }
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
    ]
}

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
            return mcp_tool_error(format!("invalid chat.send arguments: {error}"));
        }
    };

    match chat_with_ai_with_context(
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
        Ok(result) => {
            let summary = format!(
                "Assistant reply from session \"{}\":\n{}",
                result.session_title, result.answer.answer
            );
            let structured = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
            mcp_tool_success(summary, structured)
        }
        Err(error) => mcp_tool_error(error.to_string()),
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
        Err(error) => mcp_tool_error(error.to_string()),
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
        Err(error) => mcp_tool_error(error.to_string()),
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

fn read_stdin_json() -> Result<Value> {
    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer)?;
    let value: Value = serde_json::from_str(&buffer)?;
    Ok(value)
}

fn to_json<T: Serialize>(value: &T) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

fn parse_comma_list(input: &Option<String>) -> Vec<String> {
    input
        .as_ref()
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn new_cli_chat_session(title: Option<&str>) -> ChatSession {
    let now = Utc::now().to_rfc3339();
    ChatSession {
        id: Uuid::new_v4().to_string(),
        title: title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("New Chat")
            .to_string(),
        turns: Vec::new(),
        summary: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn chat_session_overview(session: &ChatSession) -> ChatSessionOverview {
    ChatSessionOverview {
        id: session.id.clone(),
        title: session.title.clone(),
        turn_count: session.turns.len(),
        has_summary: session.summary.is_some(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
    }
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

fn exit_ok(pretty: &bool, value: Value) -> ! {
    let output = if *pretty {
        serde_json::to_string_pretty(&value).unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e))
    } else {
        serde_json::to_string(&value).unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e))
    };
    println!("{output}");
    process::exit(0);
}

fn exit_error(pretty: &bool, code: &str, message: String) -> ! {
    let error = serde_json::json!({ "error": { "code": code, "message": message } });
    let output = if *pretty {
        serde_json::to_string_pretty(&error).unwrap_or_else(|e| e.to_string())
    } else {
        serde_json::to_string(&error).unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e))
    };
    eprintln!("{output}");
    process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::{
        bridge_token_from_headers, constant_time_eq, normalize_bridge_token, simplify_cli_text,
        strip_cli_markdown_from_chat_state, strip_markdown_wrapper_tags,
        validate_http_bridge_binding,
    };
    use axum::http::{HeaderMap, HeaderValue};
    use std::net::{IpAddr, Ipv4Addr};
    use vaultpilot_lib::models::{ChatSession, ChatState, ChatTurn, ThinkingTrace};

    #[test]
    fn normalize_bridge_token_trims_and_drops_empty_values() {
        assert_eq!(
            normalize_bridge_token(Some("  secret  ".to_string())),
            Some("secret".to_string())
        );
        assert_eq!(normalize_bridge_token(Some("   ".to_string())), None);
        assert_eq!(normalize_bridge_token(None), None);
    }

    #[test]
    fn non_loopback_binding_requires_token() {
        let remote = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));
        let local = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(validate_http_bridge_binding(remote, None).is_err());
        assert!(validate_http_bridge_binding(remote, Some("secret")).is_ok());
        assert!(validate_http_bridge_binding(local, None).is_ok());
    }

    #[test]
    fn bridge_token_reads_bearer_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer secret"));
        assert_eq!(bridge_token_from_headers(&headers), Some("secret"));
    }

    #[test]
    fn bridge_token_reads_custom_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-vaultpilot-token", HeaderValue::from_static("secret"));
        assert_eq!(bridge_token_from_headers(&headers), Some("secret"));
    }

    #[test]
    fn strip_markdown_wrapper_tags_removes_wrapper_only() {
        let text = "<vp-markdown>\n## Title\n- item\n</vp-markdown>";
        assert_eq!(strip_markdown_wrapper_tags(text), "## Title\n- item");
    }

    #[test]
    fn strip_markdown_wrapper_tags_keeps_plain_text() {
        let text = "plain text";
        assert_eq!(strip_markdown_wrapper_tags(text), "plain text");
    }

    #[test]
    fn simplify_cli_text_removes_common_markdown_noise() {
        let text = "<vp-markdown>\n### 标题\n1. **第一步**\n- `git fetch`\n```bash\ngit pull\n```\n</vp-markdown>";
        assert_eq!(simplify_cli_text(text), "标题\n第一步\n`git fetch`\ngit pull");
    }

    #[test]
    fn strip_cli_markdown_from_chat_state_updates_assistant_turns_only() {
        let state = ChatState {
            current_session_id: "s1".to_string(),
            sessions: vec![ChatSession {
                id: "s1".to_string(),
                title: "Test".to_string(),
                turns: vec![
                    ChatTurn {
                        role: "user".to_string(),
                        text: "<vp-markdown>keep me</vp-markdown>".to_string(),
                        ..Default::default()
                    },
                    ChatTurn {
                        role: "assistant".to_string(),
                        text: "<vp-markdown>**bold**</vp-markdown>".to_string(),
                        thinking_trace: Some(ThinkingTrace::default()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
        };

        let stripped = strip_cli_markdown_from_chat_state(state);
        assert_eq!(
            stripped.sessions[0].turns[0].text,
            "<vp-markdown>keep me</vp-markdown>"
        );
        assert_eq!(stripped.sessions[0].turns[1].text, "bold");
        assert!(stripped.sessions[0].turns[1].thinking_trace.is_none());
    }

    #[test]
    fn constant_time_eq_matches_and_rejects() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }
}
