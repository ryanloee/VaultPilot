use std::sync::Mutex;
use std::{collections::HashSet, fs, path::Path, time::Duration};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bytes::BytesMut;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::models::{
    AnswerCitation, AppSettings, ConversationTurn, NoteDocument, NoteMeta, StructuredNoteDraft,
};
use crate::prompting;

const MAX_RESPONSE_SIZE: usize = 50 * 1024 * 1024; // 50MB

/// Cached HTTP client, rebuilt only when provider config changes.
struct CachedClient {
    client: reqwest::Client,
    // Fingerprint of the config used to build this client.
    api_key: String,
    timeout_ms: u64,
}

static CACHED_CLIENT: Mutex<Option<CachedClient>> = Mutex::new(None);

fn get_or_build_client(api_key: &str, timeout_ms: u64) -> Result<reqwest::Client> {
    let mut cache = CACHED_CLIENT
        .lock()
        .map_err(|e| anyhow!("lock poisoned: {e}"))?;
    if let Some(ref cached) = *cache {
        if cached.api_key == api_key && cached.timeout_ms == timeout_ms {
            return Ok(cached.client.clone());
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key).context("invalid API key")?,
    );
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .default_headers(headers)
        .build()?;

    *cache = Some(CachedClient {
        client: client.clone(),
        api_key: api_key.to_string(),
        timeout_ms,
    });
    Ok(client)
}

pub struct ChatAnswerResult {
    pub answer: String,
    pub citations: Vec<AnswerCitation>,
    pub usage: RequestUsage,
}

pub struct RecordInteractionResult {
    pub reply: String,
    pub note_draft: StructuredNoteDraft,
    pub usage: RequestUsage,
}

pub struct ToolSelectionResult {
    pub tool_call: AssistantToolCall,
    pub usage: RequestUsage,
}

#[derive(Debug, Clone, Default)]
pub struct RequestUsage {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
}

struct ModelResponse {
    text: String,
    usage: RequestUsage,
}

#[derive(Debug, Clone)]
pub enum AssistantToolCall {
    None,
    SearchNotes { query: String, limit: usize },
    ListNotes { limit: usize },
    ListDirectory { path: String },
    ReadFile { path: String },
    SaveNote { draft: Box<StructuredNoteDraft> },
}

#[derive(Debug, Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    temperature: f32,
    system: &'a str,
    messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicInputBlock>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicInputBlock {
    Text { text: String },
    Image { source: AnthropicImageSource },
}

#[derive(Debug, Serialize, Clone)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicResponse {
    #[serde(default)]
    content: Vec<AnthropicContentBlock>,
    #[serde(default)]
    usage: AnthropicUsage,
    error: Option<AnthropicApiError>,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicContentBlock {
    #[serde(default, rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicApiError {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: usize,
    #[serde(default)]
    output_tokens: usize,
}

#[derive(Debug, Deserialize, Default)]
struct IngestResponse {
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    board: String,
    #[serde(default)]
    kernel: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AskResponse {
    #[serde(default)]
    answer: String,
    #[serde(default)]
    citations: Vec<AnswerCitation>,
    #[serde(default)]
    note_draft: Option<StructuredNoteDraft>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RecordResponse {
    #[serde(default)]
    reply: String,
    #[serde(default)]
    note_draft: Option<StructuredNoteDraft>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ToolCallResponse {
    #[serde(default)]
    tool: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    path: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    note_draft: Option<StructuredNoteDraft>,
}

#[derive(Debug, Deserialize, Default)]
struct CompressionResponse {
    #[serde(default)]
    summary: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct NoteSelectionResponse {
    #[serde(default)]
    note_ids: Vec<String>,
}

fn default_limit() -> usize {
    6
}

pub async fn organize_note(
    settings: &AppSettings,
    raw_input: &str,
    image_paths: &[String],
) -> Result<StructuredNoteDraft> {
    let system = prompting::ingest_system_prompt();
    let prompt = prompting::ingest_user_prompt(raw_input);
    let response =
        send_request_with_temperature(settings, &system, &prompt, image_paths, 0.1).await?;
    Ok(parse_or_fallback_note(&response.text, raw_input))
}

pub async fn select_tool_call(
    settings: &AppSettings,
    question: &str,
    image_paths: &[String],
    history: &[ConversationTurn],
    prior_tool_results: &[String],
) -> Result<ToolSelectionResult> {
    let system = prompting::tool_call_system_prompt();
    let prompt = prompting::tool_call_user_prompt(
        question,
        !image_paths.is_empty(),
        history,
        prior_tool_results,
    );
    let response =
        send_request_with_temperature(settings, &system, &prompt, image_paths, 0.1).await?;

    let (tool_call, usage) = match parse_tool_call(&response.text, question) {
        Ok(tool_call) => (tool_call, response.usage),
        Err(_) => {
            let retry_prompt = prompting::tool_call_retry_user_prompt(
                question,
                !image_paths.is_empty(),
                history,
                prior_tool_results,
                &response.text,
            );
            let retry_response =
                send_request_with_temperature(settings, &system, &retry_prompt, image_paths, 0.1)
                    .await?;
            let tool_call = parse_tool_call(&retry_response.text, question).with_context(|| {
                format!(
                    "model did not return a valid tool call after retry; last response: {}",
                    retry_response.text.trim()
                )
            })?;
            (tool_call, retry_response.usage)
        }
    };

    Ok(ToolSelectionResult { tool_call, usage })
}

pub async fn answer_question(
    settings: &AppSettings,
    question: &str,
    docs: &[NoteDocument],
    image_paths: &[String],
    history: &[ConversationTurn],
) -> Result<ChatAnswerResult> {
    let (system, prompt) = if docs.is_empty() {
        (
            prompting::general_chat_system_prompt(),
            prompting::general_chat_user_prompt(question, history),
        )
    } else {
        (
            prompting::answer_system_prompt(),
            prompting::answer_user_prompt(question, docs, history),
        )
    };

    let response = send_request(settings, &system, &prompt, image_paths).await?;
    let parsed = parse_or_fallback_answer(&response.text, question, docs.is_empty());

    Ok(ChatAnswerResult {
        answer: parsed.answer,
        citations: if docs.is_empty() {
            Vec::new()
        } else {
            parsed.citations
        },
        usage: response.usage,
    })
}

pub async fn answer_after_tool(
    settings: &AppSettings,
    question: &str,
    tool_name: &str,
    tool_result: &str,
    docs: &[NoteDocument],
    history: &[ConversationTurn],
) -> Result<ChatAnswerResult> {
    let system = prompting::tool_result_system_prompt();
    let prompt =
        prompting::tool_result_user_prompt(question, tool_name, tool_result, docs, history);
    let response = send_request(settings, &system, &prompt, &[]).await?;
    let parsed = parse_or_fallback_answer(&response.text, question, docs.is_empty());

    Ok(ChatAnswerResult {
        answer: parsed.answer,
        citations: if docs.is_empty() {
            Vec::new()
        } else {
            parsed.citations
        },
        usage: response.usage,
    })
}

pub async fn answer_after_tools(
    settings: &AppSettings,
    question: &str,
    tool_results: &[String],
    docs: &[NoteDocument],
    history: &[ConversationTurn],
) -> Result<ChatAnswerResult> {
    let system = prompting::tool_result_system_prompt();
    let prompt = prompting::multi_tool_result_user_prompt(question, tool_results, docs, history);
    let response = send_request(settings, &system, &prompt, &[]).await?;
    let parsed = parse_or_fallback_answer(&response.text, question, docs.is_empty());

    Ok(ChatAnswerResult {
        answer: parsed.answer,
        citations: if docs.is_empty() {
            Vec::new()
        } else {
            parsed.citations
        },
        usage: response.usage,
    })
}

pub async fn record_note_interaction(
    settings: &AppSettings,
    raw_input: &str,
    docs: &[NoteDocument],
    image_paths: &[String],
) -> Result<RecordInteractionResult> {
    let system = prompting::record_system_prompt();
    let prompt = prompting::record_user_prompt(raw_input, docs);
    let response = send_request(settings, &system, &prompt, image_paths).await?;
    parse_record_response(&response.text, raw_input, response.usage)
}

pub async fn compress_conversation(
    settings: &AppSettings,
    existing_summary: &str,
    history: &[ConversationTurn],
) -> Result<String> {
    let system = prompting::compression_system_prompt();
    let prompt = prompting::compression_user_prompt(existing_summary, history);
    let response = send_request(settings, &system, &prompt, &[]).await?;

    let json = extract_json(&response.text)
        .map_err(|_| anyhow!("model did not return valid JSON for conversation compression"))?;
    let parsed: CompressionResponse = serde_json::from_str(&json)
        .map_err(|e| anyhow!("failed to parse conversation compression response: {e}"))?;
    let summary = parsed.summary.trim();
    if summary.is_empty() {
        return Err(anyhow!("model returned an empty conversation summary"));
    }
    Ok(summary.to_string())
}

pub async fn select_relevant_note_ids(
    settings: &AppSettings,
    question: &str,
    candidates: &[NoteMeta],
    history: &[ConversationTurn],
) -> Result<Vec<String>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let system = prompting::note_selection_system_prompt();
    let prompt = prompting::note_selection_user_prompt(question, candidates, history);
    let response = send_request_with_temperature(settings, &system, &prompt, &[], 0.1).await?;

    if let Ok(json) = extract_json(&response.text) {
        if let Ok(parsed) = serde_json::from_str::<NoteSelectionResponse>(&json) {
            let candidate_ids = candidates
                .iter()
                .map(|note| note.id.as_str())
                .collect::<HashSet<_>>();
            let ids = parsed
                .note_ids
                .into_iter()
                .filter(|id| candidate_ids.contains(id.as_str()))
                .take(4)
                .collect::<Vec<_>>();
            if !ids.is_empty() {
                return Ok(ids);
            }
        }
    }

    Ok(candidates
        .iter()
        .take(3)
        .map(|note| note.id.clone())
        .collect())
}

/// Check if a model name refers to an OpenAI reasoning model (o1, o3, o4 series).
/// Uses word-boundary-aware matching to avoid false positives like "phi-1", "co1der".
fn is_openai_reasoning_model(model: &str) -> bool {
    // Known reasoning model prefixes: o1, o3, o4 (with optional suffixes like -mini, -preview)
    for prefix in &["o1", "o3", "o4"] {
        if let Some(rest) = model.strip_prefix(prefix) {
            // Exact match or followed by a separator (not a letter)
            if rest.is_empty() || !rest.as_bytes()[0].is_ascii_alphabetic() {
                return true;
            }
        }
    }
    false
}

pub fn resolve_context_window(settings: &AppSettings) -> (usize, String) {
    if let Some(explicit) = settings
        .provider
        .context_window_tokens
        .filter(|value| *value > 0)
    {
        return (explicit, "manual_override".to_string());
    }

    let model = settings.provider.model.trim().to_ascii_lowercase();
    if model.contains("glm-5.1") {
        return (200_000, "model_registry".to_string());
    }
    if model.contains("claude") {
        if model.contains("1m") {
            return (1_000_000, "model_registry".to_string());
        }
        return (200_000, "model_registry".to_string());
    }
    if model.contains("gpt-4.1") || model.contains("gpt-5") {
        return (1_047_576, "model_registry".to_string());
    }
    if model.contains("gpt-4o") {
        return (128_000, "model_registry".to_string());
    }
    if is_openai_reasoning_model(&model) {
        return (200_000, "model_registry".to_string());
    }
    if model.contains("gemini") {
        return (1_000_000, "model_registry".to_string());
    }

    (128_000, "heuristic_default".to_string())
}

fn parse_or_fallback_note(text: &str, raw_input: &str) -> StructuredNoteDraft {
    let parsed = extract_json(text)
        .ok()
        .and_then(|json| serde_json::from_str::<IngestResponse>(&json).ok());

    if let Some(parsed) = parsed {
        return StructuredNoteDraft {
            title: fallback_title(&parsed.title, raw_input),
            summary: fallback_summary(&parsed.summary, raw_input),
            tags: dedupe_terms(parsed.tags),
            keywords: dedupe_terms(parsed.keywords),
            platform: parsed.platform.trim().to_string(),
            board: parsed.board.trim().to_string(),
            kernel: parsed.kernel.trim().to_string(),
            status: if parsed.status.trim().is_empty() {
                "已记录".to_string()
            } else {
                parsed.status.trim().to_string()
            },
            source: "captured".to_string(),
            body: fallback_body(&parsed.body, raw_input),
        };
    }

    heuristic_note_from_input(raw_input)
}

fn parse_or_fallback_answer(text: &str, question: &str, no_context: bool) -> AskResponse {
    if let Ok(json) = extract_json(text) {
        if let Ok(parsed) = serde_json::from_str::<AskResponse>(&json) {
            let answer = parsed.answer.trim().to_string();
            return AskResponse {
                answer: if answer.is_empty() {
                    fallback_answer(question, no_context)
                } else {
                    answer
                },
                citations: parsed.citations,
                note_draft: parsed.note_draft,
            };
        }
    }

    AskResponse {
        answer: if text.trim().is_empty() {
            fallback_answer(question, no_context)
        } else {
            text.trim().to_string()
        },
        citations: Vec::new(),
        note_draft: None,
    }
}

fn parse_record_response(
    text: &str,
    raw_input: &str,
    usage: RequestUsage,
) -> Result<RecordInteractionResult> {
    if let Ok(json) = extract_json(text) {
        if let Ok(parsed) = serde_json::from_str::<RecordResponse>(&json) {
            if let Some(note_draft) = parsed.note_draft {
                let draft = normalize_draft(note_draft);
                let reply = if parsed.reply.trim().is_empty() {
                    fallback_record_reply(&draft.title)
                } else {
                    parsed.reply.trim().to_string()
                };
                return Ok(RecordInteractionResult {
                    reply,
                    note_draft: draft,
                    usage,
                });
            }
        }
    }

    Err(anyhow!(
        "model did not return a valid note draft for record request: {}",
        truncate(raw_input, 80)
    ))
}

fn parse_tool_call(text: &str, question: &str) -> Result<AssistantToolCall> {
    let parsed = extract_json(text)
        .ok()
        .and_then(|json| parse_tool_call_response(&json))
        .ok_or_else(|| anyhow!("model did not return a valid tool call"))?;

    let limit = parsed.limit.clamp(3, 8);

    match parsed.tool.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(AssistantToolCall::None),
        "search_notes" => Ok(AssistantToolCall::SearchNotes {
            query: if parsed.query.trim().is_empty() {
                question.trim().to_string()
            } else {
                parsed.query.trim().to_string()
            },
            limit,
        }),
        "list_notes" => Ok(AssistantToolCall::ListNotes { limit }),
        "list_directory" => Ok(AssistantToolCall::ListDirectory {
            path: parsed.path.trim().to_string(),
        }),
        "read_file" => Ok(AssistantToolCall::ReadFile {
            path: parsed.path.trim().to_string(),
        }),
        "save_note" => {
            let draft = parsed
                .note_draft
                .map(normalize_draft)
                .ok_or_else(|| anyhow!("save_note was selected but noteDraft is missing"))?;
            Ok(AssistantToolCall::SaveNote {
                draft: Box::new(draft),
            })
        }
        other => Err(anyhow!("unknown tool selected by model: {other}")),
    }
}

fn parse_tool_call_response(json: &str) -> Option<ToolCallResponse> {
    serde_json::from_str::<ToolCallResponse>(json)
        .ok()
        .or_else(|| {
            let repaired = repair_json_string_escapes(json)?;
            serde_json::from_str::<ToolCallResponse>(&repaired).ok()
        })
}

#[allow(clippy::while_let_on_iterator)]
fn repair_json_string_escapes(input: &str) -> Option<String> {
    let mut repaired = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaping = false;

    while let Some(ch) = chars.next() {
        if in_string {
            if escaping {
                if matches!(ch, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u') {
                    repaired.push(ch);
                } else {
                    repaired.push('\\');
                    repaired.push(ch);
                }
                escaping = false;
                continue;
            }

            match ch {
                '\\' => {
                    repaired.push('\\');
                    escaping = true;
                }
                '"' => {
                    repaired.push('"');
                    in_string = false;
                }
                '\n' => repaired.push_str("\\n"),
                '\r' => repaired.push_str("\\r"),
                '\t' => repaired.push_str("\\t"),
                _ => repaired.push(ch),
            }
        } else {
            repaired.push(ch);
            if ch == '"' {
                in_string = true;
            }
        }
    }

    if escaping {
        repaired.push('\\');
    }

    Some(repaired)
}

fn normalize_draft(draft: StructuredNoteDraft) -> StructuredNoteDraft {
    let fallback = heuristic_note_from_input(&draft.body);
    StructuredNoteDraft {
        title: if draft.title.trim().is_empty() {
            fallback.title
        } else {
            draft.title.trim().to_string()
        },
        summary: if draft.summary.trim().is_empty() {
            fallback.summary
        } else {
            draft.summary.trim().to_string()
        },
        tags: dedupe_terms(draft.tags),
        keywords: dedupe_terms(draft.keywords),
        platform: draft.platform.trim().to_string(),
        board: draft.board.trim().to_string(),
        kernel: draft.kernel.trim().to_string(),
        status: if draft.status.trim().is_empty() {
            "已记录".to_string()
        } else {
            draft.status.trim().to_string()
        },
        source: if draft.source.trim().is_empty() {
            "captured".to_string()
        } else {
            draft.source.trim().to_string()
        },
        body: if draft.body.trim().is_empty() {
            fallback.body
        } else {
            draft.body.trim().to_string()
        },
    }
}

fn heuristic_note_from_input(raw_input: &str) -> StructuredNoteDraft {
    let compact = raw_input.trim();
    let title = if compact.contains("刷机") || compact.to_ascii_lowercase().contains("flash") {
        "刷机命令记录".to_string()
    } else if let Some(first_line) = compact.lines().find(|line| !line.trim().is_empty()) {
        truncate(first_line.trim(), 40)
    } else {
        "临时记录".to_string()
    };

    let keywords = extract_command_keywords(compact);
    let mut tags = vec!["record".to_string()];
    if compact.contains("刷机") || compact.to_ascii_lowercase().contains("flash") {
        tags.push("flash".to_string());
    }
    if compact.to_ascii_lowercase().contains("uboot") {
        tags.push("uboot".to_string());
    }

    StructuredNoteDraft {
        title,
        summary: truncate(compact, 120),
        tags: dedupe_terms(tags),
        keywords,
        platform: String::new(),
        board: String::new(),
        kernel: String::new(),
        status: "已记录".to_string(),
        source: "captured".to_string(),
        body: format!(
            "## 摘要\n\n{}\n\n## 背景/上下文\n\n待确认\n\n## 关键信息\n\n{}\n\n## 操作步骤/命令\n\n```\n{}\n```\n\n## 结果/结论\n\n待确认\n\n## 待确认事项\n\n待确认\n\n## 关键词\n\n{}",
            compact,
            compact,
            compact,
            extract_command_keywords(compact).join(", ")
        ),
    }
}

fn extract_command_keywords(raw_input: &str) -> Vec<String> {
    dedupe_terms(
        raw_input
            .split_whitespace()
            .map(|part| {
                part.trim_matches(|ch: char| ",.;:()[]{}'\"".contains(ch))
                    .to_string()
            })
            .filter(|part| !part.is_empty())
            .filter(|part| part.len() > 1)
            .collect(),
    )
}

fn fallback_title(title: &str, raw_input: &str) -> String {
    if title.trim().is_empty() {
        heuristic_note_from_input(raw_input).title
    } else {
        title.trim().to_string()
    }
}

fn fallback_summary(summary: &str, raw_input: &str) -> String {
    if summary.trim().is_empty() {
        truncate(raw_input.trim(), 120)
    } else {
        summary.trim().to_string()
    }
}

fn fallback_body(body: &str, raw_input: &str) -> String {
    if body.trim().is_empty() {
        heuristic_note_from_input(raw_input).body
    } else {
        body.trim().to_string()
    }
}

fn fallback_answer(question: &str, no_context: bool) -> String {
    if no_context {
        format!(
            "我先直接回答这个问题：{}。这次没有检索到可用的本地笔记，所以这是基于通用模型理解给出的回答。",
            question
        )
    } else {
        "我已经拿到了知识库结果，但这次模型没有按 JSON 返回，所以我先把可读文本直接展示给你。"
            .to_string()
    }
}

fn fallback_record_reply(title: &str) -> String {
    format!(
        "我已经理解这条内容，并按“{}”这个主题准备写入知识库。",
        title
    )
}

fn dedupe_terms(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_lowercase()))
        .collect()
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

async fn send_request(
    settings: &AppSettings,
    system: &str,
    prompt: &str,
    image_paths: &[String],
) -> Result<ModelResponse> {
    send_request_with_temperature(settings, system, prompt, image_paths, 0.2).await
}

async fn send_request_with_temperature(
    settings: &AppSettings,
    system: &str,
    prompt: &str,
    image_paths: &[String],
    temperature: f32,
) -> Result<ModelResponse> {
    let provider = &settings.provider;
    if provider.api_key.trim().is_empty() {
        return Err(anyhow!("API key is empty"));
    }

    let client = get_or_build_client(&provider.api_key, provider.request_timeout_ms)?;

    let endpoint = normalize_messages_endpoint(&provider.base_url);
    let content_blocks = build_input_blocks(prompt, image_paths)?;

    for attempt in 0..3 {
        let payload = AnthropicRequest {
            model: &provider.model,
            max_tokens: 8192,
            temperature,
            system,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: content_blocks.clone(),
            }],
        };

        let response = match client.post(&endpoint).json(&payload).send().await {
            Ok(response) => response,
            Err(error) => {
                if should_retry_transport_error(&error) && attempt < 2 {
                    sleep(Duration::from_secs((attempt + 1) as u64 * 2)).await;
                    continue;
                }
                return Err(anyhow!(format_transport_error(&error, &endpoint)));
            }
        };
        let status = response.status();
        let mut buf = BytesMut::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|error| anyhow!(format_transport_error(&error, &endpoint)))?;
            if buf.len() + chunk.len() > MAX_RESPONSE_SIZE {
                return Err(anyhow!(
                    "API response body exceeds {}MB size limit, possible misconfigured endpoint",
                    MAX_RESPONSE_SIZE / (1024 * 1024)
                ));
            }
            buf.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&buf).to_string();

        if !status.is_success() {
            let detail = serde_json::from_str::<AnthropicResponse>(&text)
                .ok()
                .and_then(|value| value.error.map(|error| error.message))
                .filter(|message| !message.trim().is_empty())
                .unwrap_or(text);

            if is_retryable_provider_error(status.as_u16(), &detail) && attempt < 2 {
                sleep(Duration::from_secs((attempt + 1) as u64 * 2)).await;
                continue;
            }

            return Err(anyhow!(
                "API request failed ({}): {}",
                status.as_u16(),
                crate::sanitize_error(&detail)
            ));
        }

        let parsed: AnthropicResponse =
            serde_json::from_str(&text).context("failed to parse API response")?;
        let usage = RequestUsage {
            input_tokens: Some(parsed.usage.input_tokens),
            output_tokens: Some(parsed.usage.output_tokens),
        };
        let joined = parsed
            .content
            .into_iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n");

        if joined.trim().is_empty() {
            return Err(anyhow!("API returned an empty response"));
        }

        return Ok(ModelResponse {
            text: joined,
            usage,
        });
    }

    Err(anyhow!("API request failed after retries"))
}

fn should_retry_transport_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

fn format_transport_error(error: &reqwest::Error, endpoint: &str) -> String {
    if error.is_timeout() {
        return format!("请求超时。模型服务长时间没有响应：{}", endpoint);
    }
    if error.is_connect() {
        return format!("网络连接失败，无法连接到模型服务：{}", endpoint);
    }
    if error.is_request() {
        return format!(
            "请求发送失败，请检查 Base URL、网络或代理配置：{}",
            endpoint
        );
    }
    if error.is_decode() {
        return "模型服务返回的数据格式无法解析。".to_string();
    }
    format!(
        "调用模型服务失败：{}",
        crate::sanitize_error(&error.to_string())
    )
}

fn build_input_blocks(prompt: &str, image_paths: &[String]) -> Result<Vec<AnthropicInputBlock>> {
    let mut blocks = vec![AnthropicInputBlock::Text {
        text: prompt.to_string(),
    }];

    for path in image_paths {
        let media_type = detect_image_media_type(path)?;
        // Guard against OOM from excessively large image files (issue #141)
        const MAX_IMAGE_SIZE: u64 = 20 * 1024 * 1024; // 20 MB
        let metadata =
            fs::metadata(path).with_context(|| format!("failed to stat image: {path}"))?;
        if metadata.len() > MAX_IMAGE_SIZE {
            return Err(anyhow!(
                "image file too large: {} ({} MB > 20 MB limit)",
                path,
                metadata.len() / (1024 * 1024)
            ));
        }
        let data = fs::read(path).with_context(|| format!("failed to read image: {path}"))?;
        blocks.push(AnthropicInputBlock::Image {
            source: AnthropicImageSource {
                kind: "base64".to_string(),
                media_type: media_type.to_string(),
                data: STANDARD.encode(data),
            },
        });
    }

    Ok(blocks)
}

fn detect_image_media_type(path: &str) -> Result<&'static str> {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| anyhow!("unsupported image format: {path}"))?;

    match extension.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        "gif" => Ok("image/gif"),
        _ => Err(anyhow!("unsupported image format: {path}")),
    }
}

fn extract_json(text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Ok(trimmed.to_string());
    }
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return Ok(trimmed.to_string());
    }
    // Try extracting a JSON object
    if let Some(result) = extract_json_block(trimmed, '{', '}') {
        return Ok(result);
    }
    // Try extracting a JSON array
    if let Some(result) = extract_json_block(trimmed, '[', ']') {
        return Ok(result);
    }
    Err(anyhow!("AI response does not contain JSON"))
}

fn extract_json_block(text: &str, open: char, close: char) -> Option<String> {
    let start = text.find(open)?;
    let mut depth = 0;
    let mut in_string = false;
    let mut backslash_count = 0usize;
    for (i, c) in text[start..].char_indices() {
        if in_string {
            if c == '\\' {
                backslash_count += 1;
            } else {
                if c == '"' && backslash_count.is_multiple_of(2) {
                    in_string = false;
                }
                backslash_count = 0;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..=start + i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn normalize_messages_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1/messages") || trimmed.ends_with("/messages") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1/messages")
    }
}

fn is_retryable_provider_error(status: u16, detail: &str) -> bool {
    status == 429
        || status >= 500
        || detail.contains("访问量过大")
        || detail.to_ascii_lowercase().contains("too many requests")
        || detail.to_ascii_lowercase().contains("rate limit")
}

#[cfg(test)]
mod tests {
    use super::{
        dedupe_terms, detect_image_media_type, extract_json, extract_json_block, fallback_answer,
        heuristic_note_from_input, is_openai_reasoning_model, is_retryable_provider_error,
        normalize_draft, normalize_messages_endpoint, parse_or_fallback_answer,
        parse_or_fallback_note, parse_record_response, parse_tool_call, resolve_context_window,
        AssistantToolCall, RequestUsage,
    };
    use crate::models::{AppSettings, ProviderConfig, StructuredNoteDraft};

    #[test]
    fn extracts_json_from_code_fence_like_payload() {
        let raw = "```json\n{\"answer\":\"ok\"}\n```";
        let extracted = extract_json(raw).expect("json extracted");
        assert_eq!(extracted, "{\"answer\":\"ok\"}");
    }

    #[test]
    fn extracts_clean_json_directly() {
        let raw = r#"{"answer":"hello","citations":[]}"#;
        assert_eq!(extract_json(raw).expect("extract"), raw);
    }

    #[test]
    fn extracts_json_from_surrounding_prose() {
        let raw = r#"The result is {"answer":"ok"} great"#;
        let extracted = extract_json(raw).expect("extract");
        assert!(extracted.contains("answer"));
    }

    #[test]
    fn extract_json_returns_err_without_braces() {
        assert!(extract_json("no json here").is_err());
    }

    #[test]
    fn extract_json_handles_nested_objects() {
        let raw = r#"{"a":{"b":1},"c":2}"#;
        let extracted = extract_json(raw).expect("extract nested");
        assert!(extracted.contains("\"a\""));
        assert!(extracted.contains("\"b\""));
    }

    #[test]
    fn extract_json_handles_braces_inside_strings() {
        let raw = r#"Here is the result: {"answer": "use {var} syntax", "ok": true} done"#;
        let extracted = extract_json(raw).expect("extract with braces in string");
        assert!(extracted.contains("{var}"));
        assert!(extracted.contains("\"ok\": true"));
    }

    #[test]
    fn extract_json_block_double_escaped_backslash() {
        // \\" is an escaped backslash followed by a real closing quote
        let text = r#"{"key": "value\\\\"}more"#;
        // This should find the JSON object correctly
        let result = extract_json_block(text, '{', '}');
        assert!(result.is_some(), "Should handle double-escaped backslash");
        let json = result.unwrap();
        assert!(json.ends_with('}'), "JSON should end with closing brace");
    }

    #[test]
    fn appends_messages_path_for_provider_base_url() {
        assert_eq!(
            normalize_messages_endpoint("https://open.bigmodel.cn/api/anthropic"),
            "https://open.bigmodel.cn/api/anthropic/v1/messages"
        );
        assert_eq!(
            normalize_messages_endpoint("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn normalize_endpoint_preserves_trailing_slash_url() {
        let result = normalize_messages_endpoint("https://api.example.com/v1/messages/");
        assert!(result.contains("/v1/messages"));
    }

    #[test]
    fn normalize_endpoint_appends_for_bare_host() {
        let result = normalize_messages_endpoint("https://api.example.com");
        assert_eq!(result, "https://api.example.com/v1/messages");
    }

    #[test]
    fn fallback_mentions_direct_answer_when_no_context() {
        let text = fallback_answer("我之前怎么做的", true);
        assert!(text.contains("直接回答"));
    }

    #[test]
    fn detects_supported_image_types() {
        assert_eq!(detect_image_media_type("a.png").expect("png"), "image/png");
        assert!(detect_image_media_type("a.bmp").is_err());
    }

    #[test]
    fn detects_jpeg_and_webp_and_gif() {
        assert_eq!(
            detect_image_media_type("photo.jpg").expect("jpg"),
            "image/jpeg"
        );
        assert_eq!(
            detect_image_media_type("img.jpeg").expect("jpeg"),
            "image/jpeg"
        );
        assert_eq!(
            detect_image_media_type("pic.webp").expect("webp"),
            "image/webp"
        );
        assert_eq!(
            detect_image_media_type("anim.gif").expect("gif"),
            "image/gif"
        );
    }

    #[test]
    fn heuristically_builds_note_for_command_records() {
        let draft =
            heuristic_note_from_input("我发送刷机命令你记录一下，wboot -w update zboot.img");
        assert!(draft.title.contains("刷机命令"));
        assert!(draft.body.contains("wboot -w update zboot.img"));
    }

    #[test]
    fn uses_plain_text_when_model_does_not_return_json() {
        let parsed = parse_or_fallback_answer("你好，我记住了。", "记录一下", true);
        assert_eq!(parsed.answer, "你好，我记住了。");
    }

    #[test]
    fn record_requires_model_note_draft() {
        assert!(parse_record_response(
            "",
            "记录一下，wboot -w update zboot.img",
            RequestUsage::default()
        )
        .is_err());
    }

    #[test]
    fn parses_list_notes_tool_call() {
        /*
        let tool = parse_tool_call(
            "{\"tool\":\"list_notes\",\"query\":\"\",\"limit\":5,\"noteDraft\":null}",
            "资料库里有什么",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::ListNotes { limit: 5 }));
        */
        let tool = parse_tool_call(
            "{\"tool\":\"list_notes\",\"query\":\"\",\"limit\":5,\"noteDraft\":null}",
            "list notes",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::ListNotes { limit: 5 }));
    }

    #[test]
    fn parses_search_notes_tool_call() {
        let tool = parse_tool_call(
            "{\"tool\":\"search_notes\",\"query\":\"mmc timeout\",\"limit\":6,\"noteDraft\":null}",
            "mmc超时",
        )
        .expect("tool");
        assert!(
            matches!(tool, AssistantToolCall::SearchNotes { query, limit } if query == "mmc timeout" && limit == 6)
        );
    }

    #[test]
    fn parses_read_file_tool_call() {
        let tool = parse_tool_call(
            "{\"tool\":\"read_file\",\"path\":\"C:\\\\Users\\\\test\\\\log.txt\",\"noteDraft\":null}",
            "看下日志",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::ReadFile { path } if path.contains("log.txt")));
    }

    #[test]
    fn parses_read_file_tool_call_with_unescaped_windows_path() {
        let tool = parse_tool_call(
            r#"{"tool":"read_file","query":"","path":"\\?\C:\Users\test\log.txt","limit":6,"noteDraft":null}"#,
            "read the file",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::ReadFile { path } if path.contains("log.txt")));
    }

    #[test]
    fn rejects_run_command_tool_call() {
        assert!(parse_tool_call(
            "{\"tool\":\"run_command\",\"command\":\"dir\",\"cwd\":\"\",\"noteDraft\":null}",
            "列出文件",
        )
        .is_err());
    }

    #[test]
    fn parses_none_tool_call() {
        let tool = parse_tool_call(
            "{\"tool\":\"none\",\"query\":\"\",\"limit\":0,\"noteDraft\":null}",
            "你好",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::None));
    }

    #[test]
    fn parse_tool_call_returns_err_for_unknown_tool() {
        assert!(parse_tool_call(
            "{\"tool\":\"fly_to_moon\",\"query\":\"\",\"limit\":0,\"noteDraft\":null}",
            "去月球",
        )
        .is_err());
    }

    #[test]
    fn parse_or_fallback_note_uses_heuristic_on_plain_text() {
        let draft = parse_or_fallback_note("这不是JSON，只是一段话", "帮我记录一下mmc超时的问题");
        assert!(!draft.body.is_empty());
    }

    #[test]
    fn parse_or_fallback_answer_extracts_citations() {
        let json = r#"{"answer":"参见笔记","citations":[{"noteId":"n1","title":"T","path":"/p.md","snippet":"s"}]}"#;
        let parsed = parse_or_fallback_answer(json, "问题", true);
        assert_eq!(parsed.answer, "参见笔记");
        assert_eq!(parsed.citations.len(), 1);
        assert_eq!(parsed.citations[0].note_id, "n1");
    }

    #[test]
    fn dedupe_terms_removes_duplicates_case_insensitive() {
        let result = dedupe_terms(vec![
            "Kernel".to_string(),
            "kernel".to_string(),
            "KERNEL".to_string(),
        ]);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn dedupe_terms_removes_empty_strings() {
        let result = dedupe_terms(vec!["a".to_string(), "".to_string(), "  ".to_string()]);
        assert_eq!(result, vec!["a"]);
    }

    #[test]
    fn normalize_draft_fills_empty_fields_from_heuristic() {
        let draft = StructuredNoteDraft {
            title: String::new(),
            body: "wboot -w update zboot.img 刷机命令".to_string(),
            ..Default::default()
        };
        let normalized = normalize_draft(draft);
        assert!(!normalized.title.is_empty());
    }

    #[test]
    fn resolve_context_window_uses_manual_override() {
        let settings = AppSettings {
            provider: ProviderConfig {
                context_window_tokens: Some(999_999),
                ..Default::default()
            },
            ..Default::default()
        };
        let (tokens, source) = resolve_context_window(&settings);
        assert_eq!(tokens, 999_999);
        assert_eq!(source, "manual_override");
    }

    #[test]
    fn resolve_context_window_recognizes_claude_models() {
        let settings = AppSettings {
            provider: ProviderConfig {
                model: "claude-3-5-sonnet-latest".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let (tokens, _) = resolve_context_window(&settings);
        assert_eq!(tokens, 200_000);
    }

    #[test]
    fn resolve_context_window_defaults_for_unknown_model() {
        let settings = AppSettings {
            provider: ProviderConfig {
                model: "unknown-model-xyz".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let (tokens, source) = resolve_context_window(&settings);
        assert_eq!(tokens, 128_000);
        assert_eq!(source, "heuristic_default");
    }

    #[test]
    fn is_retryable_detects_429_and_5xx() {
        assert!(is_retryable_provider_error(429, ""));
        assert!(is_retryable_provider_error(500, ""));
        assert!(is_retryable_provider_error(503, ""));
        assert!(!is_retryable_provider_error(400, ""));
        assert!(!is_retryable_provider_error(401, ""));
    }

    #[test]
    fn is_retryable_detects_rate_limit_in_detail() {
        assert!(is_retryable_provider_error(400, "rate limit exceeded"));
        assert!(is_retryable_provider_error(400, "Too Many Requests"));
        assert!(is_retryable_provider_error(400, "访问量过大"));
        assert!(!is_retryable_provider_error(400, "bad request"));
    }

    #[test]
    fn is_openai_reasoning_model_matches_exact_prefix() {
        assert!(is_openai_reasoning_model("o1"));
        assert!(is_openai_reasoning_model("o3"));
        assert!(is_openai_reasoning_model("o4"));
    }

    #[test]
    fn is_openai_reasoning_model_matches_with_suffix() {
        assert!(is_openai_reasoning_model("o1-mini"));
        assert!(is_openai_reasoning_model("o1-preview"));
        assert!(is_openai_reasoning_model("o3-mini"));
        assert!(is_openai_reasoning_model("o4-mini"));
    }

    #[test]
    fn is_openai_reasoning_model_rejects_false_positives() {
        // These should NOT match — "o1"/"o3"/"o4" appear as substrings of longer tokens
        assert!(!is_openai_reasoning_model("phi-1"));
        assert!(!is_openai_reasoning_model("co1der"));
        assert!(!is_openai_reasoning_model("pro1"));
        assert!(!is_openai_reasoning_model("some-o3thing"));
        assert!(!is_openai_reasoning_model("mo4del"));
    }
}
