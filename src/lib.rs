pub mod ai;
pub mod models;
pub mod prompting;
pub mod storage;

use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use ai::AssistantToolCall;
use chrono::Utc;
use models::{
    AppSettings, ChatAttachment, ChatExchangeResult, ChatSession, ChatState, ChatTurn,
    ContextStatus, ConversationSummary, ConversationTurn, GroundedAnswer, NoteDocument, NoteMeta,
    StructuredNoteDraft, ThinkingTrace, ThinkingTraceStep,
};
use storage::{
    initialize_storage_with_context, list_notes_with_context, load_chat_state_with_context,
    load_context_notes_with_context, load_note_with_context, save_chat_state_with_context,
    save_note_with_images_with_context, StorageContext,
};
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;
use uuid::Uuid;

pub async fn compress_chat_history_with_context(
    context: &StorageContext,
    summary: Option<ConversationSummary>,
    history: Vec<ConversationTurn>,
    mut emit_status: impl FnMut(&str, String),
) -> Result<ConversationSummary, String> {
    let settings = initialize_storage_with_context(context).map_err(|error| error.to_string())?;
    emit_status(
        "compressing",
        "Compressing earlier conversation context".to_string(),
    );
    let existing_summary = summary
        .as_ref()
        .map(|item| item.text.as_str())
        .unwrap_or_default();
    let text = ai::compress_conversation(&settings, existing_summary, &history)
        .await
        .map_err(|error| error.to_string())?;

    Ok(ConversationSummary {
        text,
        generated_at: Utc::now().to_rfc3339(),
        covered_turn_count: history.len(),
        compression_count: summary.map(|item| item.compression_count + 1).unwrap_or(1),
    })
}

const CONTEXT_COMPRESSION_THRESHOLD: f64 = 0.95;
const RECENT_TURNS_AFTER_COMPRESSION: usize = 8;
const IMAGE_ATTACHMENT_TOKEN_ESTIMATE: u64 = 1_200;

pub async fn chat_with_ai_with_context(
    context: &StorageContext,
    session_id: Option<String>,
    question: String,
    image_paths: Option<Vec<String>>,
    create_new_session: bool,
    mut emit_status: impl FnMut(&str, String),
) -> Result<ChatExchangeResult, String> {
    let settings = initialize_storage_with_context(context).map_err(|error| error.to_string())?;
    let mut state = load_chat_state_with_context(context).map_err(|error| error.to_string())?;
    let images = image_paths.unwrap_or_default();
    let trimmed_question = question.trim().to_string();
    if trimmed_question.is_empty() && images.is_empty() {
        return Err("question is empty".to_string());
    }

    let prompt = if trimmed_question.is_empty() {
        "请结合我发送的图片理解并回复。".to_string()
    } else {
        trimmed_question.clone()
    };
    let user_display = if trimmed_question.is_empty() {
        "（发送了一张图片）".to_string()
    } else {
        trimmed_question
    };
    let attachments = build_chat_attachments(&images);
    let (active_session_id, created_session) =
        resolve_or_create_chat_session(&mut state, session_id.as_deref(), create_new_session)?;

    compress_chat_session_if_needed(
        context,
        &settings,
        &mut state,
        &active_session_id,
        &prompt,
        &attachments,
        &mut emit_status,
    )
    .await?;

    let history = current_session_history(&state, &active_session_id)?;
    let user_turn = build_chat_turn("user", &user_display, None, &attachments);
    append_turn_to_session(&mut state, &active_session_id, user_turn)?;
    state = save_chat_state_with_context(context, &state).map_err(|error| error.to_string())?;

    let answer = ask_with_ai_with_context(
        context,
        prompt,
        Some(history),
        if images.is_empty() { None } else { Some(images) },
        |stage, detail| emit_status(stage, detail),
    )
    .await?;

    let assistant_turn = build_chat_turn("assistant", &answer.answer, Some(&answer), &[]);
    append_turn_to_session(&mut state, &active_session_id, assistant_turn)?;
    state = save_chat_state_with_context(context, &state).map_err(|error| error.to_string())?;

    let session = find_chat_session(&state, &active_session_id)
        .ok_or_else(|| format!("chat session not found after save: {}", active_session_id))?;

    Ok(ChatExchangeResult {
        session_id: session.id.clone(),
        session_title: session.title.clone(),
        created_session,
        answer,
        state,
    })
}

pub async fn ask_with_ai_with_context(
    context: &StorageContext,
    question: String,
    history: Option<Vec<ConversationTurn>>,
    image_paths: Option<Vec<String>>,
    mut emit_status: impl FnMut(&str, String),
) -> Result<GroundedAnswer, String> {
    let settings = initialize_storage_with_context(context).map_err(|error| error.to_string())?;
    let images = image_paths.unwrap_or_default();
    let history = history.unwrap_or_default();
    let session_memory_question = looks_like_session_memory_question(&question);
    let has_local_notes = !list_notes_with_context(context)
        .map_err(|error| error.to_string())?
        .is_empty();
    let mut docs: Vec<NoteDocument> = Vec::new();
    let mut tool_results: Vec<ToolExecution> = Vec::new();
    let mut saved_note: Option<NoteMeta> = None;
    let mut usage = ai::RequestUsage::default();

    emit_status("analyzing", "Analyzing request".to_string());

    for round in 0..4 {
        let tool_history = tool_results
            .iter()
            .map(ToolExecution::render_for_model)
            .collect::<Vec<_>>();

        let selection =
            ai::select_tool_call(&settings, &question, &images, &history, &tool_history)
                .await
                .map_err(|error| error.to_string())?;
        usage = merge_usage(usage, selection.usage);

        let forced_local_path_tool =
            if round == 0 && tool_results.is_empty() && !looks_like_record_request(&question) {
                preferred_explicit_path_tool(&question)
            } else {
                None
            };

        let forced_search = forced_local_path_tool.is_none()
            && round == 0
            && matches!(selection.tool_call, AssistantToolCall::None)
            && has_local_notes
            && !looks_like_small_talk(&question)
            && !session_memory_question;

        let tool_call = if let Some(path_tool) = forced_local_path_tool {
            path_tool
        } else if forced_search {
            AssistantToolCall::SearchNotes {
                query: question.clone(),
                limit: 6,
            }
        } else {
            selection.tool_call
        };

        if has_matching_tool_execution(&tool_results, &tool_call, &settings) {
            emit_status("responding", "Reusing previous tool result".to_string());
            return finalize_grounded_answer(
                &settings,
                &question,
                &history,
                &images,
                &docs,
                &tool_results,
                saved_note,
                usage,
                forced_search,
            )
            .await;
        }

        match tool_call {
            AssistantToolCall::None => {
                emit_status("responding", "Preparing answer".to_string());
                return finalize_grounded_answer(
                    &settings,
                    &question,
                    &history,
                    &images,
                    &docs,
                    &tool_results,
                    saved_note,
                    usage,
                    forced_search,
                )
                .await;
            }
            AssistantToolCall::SearchNotes { query, limit } => {
                if !has_local_notes {
                    emit_status(
                        "retrieving",
                        "Knowledge base is empty; skipping local search".to_string(),
                    );
                    tool_results.push(ToolExecution::new(
                        "search_notes",
                        format!("query={} limit={}", query, limit),
                        "The local knowledge base is currently empty.".to_string(),
                        false,
                    ));
                    continue;
                }

                emit_status("retrieving", format!("Searching notes: {}", query));
                docs = load_context_notes_with_context(
                    context,
                    &query,
                    &images,
                    limit.saturating_mul(3).max(8),
                )
                .map_err(|error| error.to_string())?;
                if docs.is_empty() {
                    emit_status(
                        "retrieving",
                        "No direct match; listing recent notes".to_string(),
                    );
                    docs = load_recent_notes_for_overview(context, limit.min(12))
                        .map_err(|error| error.to_string())?;
                } else {
                    emit_status("ranking", format!("Scored {} candidate notes", docs.len()));
                    docs.truncate(limit.max(1));
                }
                let result = summarize_docs_for_tool_result("search_notes", &docs);
                tool_results.push(ToolExecution::new(
                    "search_notes",
                    format!("query={} limit={}", query, limit),
                    result,
                    false,
                ));
            }
            AssistantToolCall::ListNotes { limit } => {
                emit_status("retrieving", "Loading recent notes".to_string());
                docs = load_recent_notes_for_overview(context, limit)
                    .map_err(|error| error.to_string())?;
                let result = summarize_docs_for_tool_result("list_notes", &docs);
                tool_results.push(ToolExecution::new(
                    "list_notes",
                    format!("limit={}", limit),
                    result,
                    false,
                ));
            }
            AssistantToolCall::ListDirectory { path } => {
                emit_status(
                    "executing",
                    format!("Listing directory: {}", display_path(&path)),
                );
                let result = list_directory_result(&path);
                let is_error = result.is_err();
                let output = match result {
                    Ok(output) => output,
                    Err(error) => format!("tool error: {}", error),
                };
                tool_results.push(ToolExecution::new(
                    "list_directory",
                    format!("path={}", path),
                    output,
                    is_error,
                ));
            }
            AssistantToolCall::ReadFile { path } => {
                emit_status(
                    "executing",
                    format!("Reading file: {}", display_path(&path)),
                );
                let result = read_file_result(&path);
                let is_error = result.is_err();
                let output = match result {
                    Ok(output) => output,
                    Err(error) => format!("tool error: {}", error),
                };
                tool_results.push(ToolExecution::new(
                    "read_file",
                    format!("path={}", path),
                    output,
                    is_error,
                ));
            }
            AssistantToolCall::RunCommand { command, cwd } => {
                let effective_cwd = cwd.or_else(|| {
                    if settings.vault_dir.trim().is_empty() {
                        None
                    } else {
                        Some(settings.vault_dir.clone())
                    }
                });
                emit_status("executing", format!("Running command: {}", command));
                let result = run_command_result(&command, effective_cwd.as_deref()).await;
                let is_error = result.is_err();
                let output = match result {
                    Ok(output) => output,
                    Err(error) => format!("tool error: {}", error),
                };
                tool_results.push(ToolExecution::new(
                    "run_command",
                    match &effective_cwd {
                        Some(dir) => format!(
                            "command={}
cwd={}",
                            command, dir
                        ),
                        None => format!("command={}", command),
                    },
                    output,
                    is_error,
                ));
            }
            AssistantToolCall::SaveNote { draft } => {
                emit_status("saving", "Saving generated note".to_string());
                let saved = save_note_with_images_with_context(
                    context,
                    draft_to_note_document(draft),
                    &images,
                )
                .map_err(|error| error.to_string())?;
                let result = format!(
                    "save_note completed.
Saved title: {}
Saved path: {}
Saved summary: {}",
                    saved.meta.title, saved.meta.path, saved.meta.summary
                );
                saved_note = Some(saved.meta.clone());
                tool_results.push(ToolExecution::new(
                    "save_note",
                    "model_generated_note_draft".to_string(),
                    result,
                    false,
                ));

                emit_status("responding", "Preparing final answer".to_string());
                return finalize_grounded_answer(
                    &settings,
                    &question,
                    &history,
                    &images,
                    &docs,
                    &tool_results,
                    saved_note,
                    usage,
                    false,
                )
                .await;
            }
        }
    }

    emit_status("responding", "Preparing final answer".to_string());
    finalize_grounded_answer(
        &settings,
        &question,
        &history,
        &images,
        &docs,
        &tool_results,
        saved_note,
        usage,
        false,
    )
    .await
}

async fn finalize_grounded_answer(
    settings: &AppSettings,
    question: &str,
    history: &[ConversationTurn],
    images: &[String],
    docs: &[NoteDocument],
    tool_results: &[ToolExecution],
    saved_note: Option<NoteMeta>,
    usage: ai::RequestUsage,
    forced_search: bool,
) -> Result<GroundedAnswer, String> {
    let answer = if tool_results.is_empty() {
        ai::answer_question(settings, question, &[], images, history)
            .await
            .map_err(|error| error.to_string())?
    } else {
        let transcript = tool_results
            .iter()
            .map(ToolExecution::render_for_model)
            .collect::<Vec<_>>();
        ai::answer_after_tools(settings, question, &transcript, docs, history)
            .await
            .map_err(|error| error.to_string())?
    };

    let usage = merge_usage(usage, answer.usage);
    let context_status =
        context_status_from_usage(settings, usage.input_tokens, usage.output_tokens);

    Ok(GroundedAnswer {
        answer: answer.answer,
        citations: answer.citations,
        saved_note,
        thinking_trace: Some(build_agent_trace(tool_results, forced_search)),
        context_status: Some(context_status),
        used_context_count: docs.len(),
    })
}

fn has_matching_tool_execution(
    tool_results: &[ToolExecution],
    tool_call: &AssistantToolCall,
    settings: &AppSettings,
) -> bool {
    let Some((name, input)) = planned_tool_identity(tool_call, settings) else {
        return false;
    };

    tool_results
        .iter()
        .any(|item| item.name == name && item.input == input)
}

fn planned_tool_identity(
    tool_call: &AssistantToolCall,
    settings: &AppSettings,
) -> Option<(&'static str, String)> {
    match tool_call {
        AssistantToolCall::None => None,
        AssistantToolCall::SearchNotes { query, limit } => {
            Some(("search_notes", format!("query={} limit={}", query, limit)))
        }
        AssistantToolCall::ListNotes { limit } => Some(("list_notes", format!("limit={}", limit))),
        AssistantToolCall::ListDirectory { path } => {
            Some(("list_directory", format!("path={}", path.trim())))
        }
        AssistantToolCall::ReadFile { path } => {
            Some(("read_file", format!("path={}", path.trim())))
        }
        AssistantToolCall::RunCommand { command, cwd } => {
            let effective_cwd = cwd.as_deref().or_else(|| {
                if settings.vault_dir.trim().is_empty() {
                    None
                } else {
                    Some(settings.vault_dir.as_str())
                }
            });
            Some((
                "run_command",
                match effective_cwd {
                    Some(dir) => format!("command={}\ncwd={}", command, dir),
                    None => format!("command={}", command),
                },
            ))
        }
        AssistantToolCall::SaveNote { .. } => {
            Some(("save_note", "model_generated_note_draft".to_string()))
        }
    }
}

fn preferred_explicit_path_tool(question: &str) -> Option<AssistantToolCall> {
    let path = extract_explicit_local_path(question)?;
    let target = Path::new(&path);
    if target.is_dir() {
        Some(AssistantToolCall::ListDirectory { path })
    } else if target.is_file() {
        Some(AssistantToolCall::ReadFile { path })
    } else {
        None
    }
}

fn extract_explicit_local_path(input: &str) -> Option<String> {
    for delimiter in ['`', '"', '\''] {
        let parts = input.split(delimiter).collect::<Vec<_>>();
        for candidate in parts.iter().skip(1).step_by(2) {
            let trimmed = trim_path_candidate(candidate);
            if is_existing_local_path(trimmed) {
                return Some(trimmed.to_string());
            }
        }
    }

    input
        .split_whitespace()
        .map(trim_path_candidate)
        .find(|candidate| is_existing_local_path(candidate))
        .map(str::to_string)
}

fn trim_path_candidate(value: &str) -> &str {
    value.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '`' | '"'
                | '\''
                | ','
                | ';'
                | '，'
                | '。'
                | '！'
                | '？'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
        )
    })
}

fn is_existing_local_path(candidate: &str) -> bool {
    if candidate.is_empty() {
        return false;
    }

    let looks_like_path = candidate.contains(":\\")
        || candidate.contains(":/")
        || candidate.starts_with("\\\\")
        || candidate.starts_with('/');

    looks_like_path && Path::new(candidate).exists()
}

#[derive(Debug, Clone)]
struct ToolExecution {
    name: String,
    input: String,
    output: String,
    is_error: bool,
}

impl ToolExecution {
    fn new(
        name: impl Into<String>,
        input: impl Into<String>,
        output: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            name: name.into(),
            input: input.into(),
            output: output.into(),
            is_error,
        }
    }

    fn render_for_model(&self) -> String {
        let status = if self.is_error { "error" } else { "ok" };
        format!(
            "TOOL: {}\nSTATUS: {}\nINPUT:\n{}\nOUTPUT:\n{}",
            self.name, status, self.input, self.output
        )
    }
}

fn build_agent_trace(tool_results: &[ToolExecution], forced_search: bool) -> ThinkingTrace {
    let mut steps = Vec::new();
    if forced_search {
        steps.push(ThinkingTraceStep {
            title: "动作判断".to_string(),
            detail: "模型原本倾向直接回答，但系统按检索优先策略先触发了知识库搜索。".to_string(),
        });
    }

    if tool_results.is_empty() {
        steps.push(ThinkingTraceStep {
            title: "动作判断".to_string(),
            detail: "本轮没有执行额外工具，模型直接生成回答。".to_string(),
        });
    } else {
        for (index, tool) in tool_results.iter().enumerate() {
            steps.push(ThinkingTraceStep {
                title: format!("工具步骤 {}", index + 1),
                detail: format!(
                    "{}\n{}\n{}",
                    tool.name,
                    truncate_for_trace(&tool.input, 200),
                    truncate_for_trace(&tool.output, 600)
                ),
            });
        }
    }

    ThinkingTrace {
        summary: if tool_results.is_empty() {
            "本轮直接回答，没有触发额外工具。".to_string()
        } else {
            format!(
                "本轮执行了 {} 个工具步骤后再组织最终回答。",
                tool_results.len()
            )
        },
        steps,
    }
}

fn truncate_for_trace(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn merge_usage(current: ai::RequestUsage, next: ai::RequestUsage) -> ai::RequestUsage {
    ai::RequestUsage {
        input_tokens: next.input_tokens.or(current.input_tokens),
        output_tokens: next.output_tokens.or(current.output_tokens),
    }
}

fn display_path(path: &str) -> &str {
    if path.trim().is_empty() {
        "(empty path)"
    } else {
        path
    }
}

fn list_directory_result(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path is empty".to_string());
    }
    let directory = Path::new(trimmed);
    if !directory.exists() {
        return Err(format!("path does not exist: {}", trimmed));
    }
    if !directory.is_dir() {
        return Err(format!("path is not a directory: {}", trimmed));
    }

    let mut entries = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let rendered = entries
        .into_iter()
        .take(60)
        .map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok();
            let kind = if path.is_dir() { "dir" } else { "file" };
            let size = metadata.map(|item| item.len()).unwrap_or(0);
            format!("- [{}] {} ({} bytes)", kind, path.display(), size)
        })
        .collect::<Vec<_>>();

    Ok(format!(
        "list_directory returned {} items.\n{}",
        rendered.len(),
        rendered.join("\n")
    ))
}

fn read_file_result(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path is empty".to_string());
    }
    let file_path = Path::new(trimmed);
    if !file_path.exists() {
        return Err(format!("path does not exist: {}", trimmed));
    }
    if !file_path.is_file() {
        return Err(format!("path is not a file: {}", trimmed));
    }

    let content = fs::read_to_string(file_path).map_err(|error| error.to_string())?;
    let clipped = truncate_for_trace(&content, 12_000);
    Ok(format!(
        "read_file returned content for {}:\n{}",
        trimmed, clipped
    ))
}

async fn run_command_result(command: &str, cwd: Option<&str>) -> Result<String, String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("command is empty".to_string());
    }
    if looks_like_dangerous_command(trimmed) {
        return Err(format!("blocked dangerous command: {}", trimmed));
    }

    let mut process = if cfg!(target_os = "windows") {
        let mut cmd = TokioCommand::new("powershell");
        cmd.arg("-NoProfile").arg("-Command").arg(trimmed);
        cmd
    } else {
        let mut cmd = TokioCommand::new("sh");
        cmd.arg("-lc").arg(trimmed);
        cmd
    };

    if let Some(dir) = cwd.filter(|value| !value.trim().is_empty()) {
        process.current_dir(dir);
    }

    process.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = timeout(Duration::from_secs(20), process.output())
        .await
        .map_err(|_| format!("command timed out after 20 seconds: {}", trimmed))?
        .map_err(|error| error.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!(
        "exit_code: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        truncate_for_trace(&stdout, 12_000),
        truncate_for_trace(&stderr, 8_000)
    );

    Ok(combined)
}

fn looks_like_dangerous_command(command: &str) -> bool {
    let normalized = command.to_ascii_lowercase();
    [
        "del /",
        "remove-item",
        "rd /s",
        "rmdir /s",
        "rm -rf",
        "format ",
        "mkfs",
        "shutdown",
        "reboot",
        "halt",
        "poweroff",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn load_recent_notes_for_overview(
    context: &StorageContext,
    limit: usize,
) -> anyhow::Result<Vec<NoteDocument>> {
    let notes = list_notes_with_context(context)?;
    let mut docs = Vec::new();
    for note in notes.into_iter().take(limit) {
        if let Ok(doc) = load_note_with_context(context, &note.id) {
            docs.push(doc);
        }
    }
    Ok(docs)
}

fn summarize_docs_for_tool_result(tool_name: &str, docs: &[NoteDocument]) -> String {
    if docs.is_empty() {
        return format!("{tool_name} returned 0 notes.");
    }

    let items = docs
        .iter()
        .map(|doc| {
            format!(
                "- {} | {} | {}",
                doc.meta.title, doc.meta.path, doc.meta.summary
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("{tool_name} returned {} notes.\n{}", docs.len(), items)
}

fn context_status_from_usage(
    settings: &AppSettings,
    input_tokens: Option<usize>,
    output_tokens: Option<usize>,
) -> ContextStatus {
    let (context_window_tokens, source) = ai::resolve_context_window(settings);
    let live_tokens = input_tokens.unwrap_or_default();
    ContextStatus {
        model: settings.provider.model.clone(),
        context_window_tokens,
        live_tokens,
        threshold_tokens: context_window_tokens * 95 / 100,
        threshold_percent: 95,
        usage_percent: if context_window_tokens == 0 {
            0.0
        } else {
            (live_tokens as f64 / context_window_tokens as f64) * 100.0
        },
        source,
        precise: input_tokens.is_some(),
        last_request_input_tokens: input_tokens,
        last_request_output_tokens: output_tokens,
    }
}

fn looks_like_small_talk(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    [
        "你好",
        "hi",
        "hello",
        "hey",
        "thanks",
        "thank you",
        "谢谢",
        "你是谁",
        "在吗",
    ]
    .iter()
    .any(|needle| normalized == *needle || normalized.starts_with(&format!("{needle} ")))
}

fn looks_like_record_request(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    [
        "记录",
        "记一下",
        "记住",
        "保存",
        "存一下",
        "存到知识库",
        "加入知识库",
        "写入知识库",
        "帮我记",
        "帮我存",
        "record this",
        "save this",
        "remember this",
        "store this",
        "capture this",
        "add to the knowledge base",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn looks_like_session_memory_question(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    [
        "我的名字",
        "我叫什么",
        "你还记得我叫什么",
        "刚才我说",
        "前面我说",
        "之前我说",
        "我刚才说了什么",
        "what is my name",
        "do you remember my name",
        "what did i just say",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn draft_to_note_document(draft: StructuredNoteDraft) -> NoteDocument {
    NoteDocument {
        meta: NoteMeta {
            id: String::new(),
            title: draft.title,
            tags: draft.tags,
            keywords: draft.keywords,
            platform: draft.platform,
            board: draft.board,
            kernel: draft.kernel,
            status: draft.status,
            created_at: String::new(),
            updated_at: String::new(),
            source: if draft.source.trim().is_empty() {
                "captured".to_string()
            } else {
                draft.source
            },
            path: String::new(),
            summary: draft.summary,
        },
        body: draft.body,
    }
}

fn resolve_or_create_chat_session(
    state: &mut ChatState,
    requested_session_id: Option<&str>,
    create_new_session: bool,
) -> Result<(String, bool), String> {
    if create_new_session || state.sessions.is_empty() {
        let session = new_chat_session(None);
        let id = session.id.clone();
        state.current_session_id = id.clone();
        state.sessions.insert(0, session);
        return Ok((id, true));
    }

    if let Some(session_id) = requested_session_id.filter(|value| !value.trim().is_empty()) {
        if state.sessions.iter().any(|session| session.id == session_id) {
            state.current_session_id = session_id.to_string();
            return Ok((session_id.to_string(), false));
        }
        return Err(format!("chat session not found: {}", session_id));
    }

    if state.current_session_id.trim().is_empty()
        || !state
            .sessions
            .iter()
            .any(|session| session.id == state.current_session_id)
    {
        state.current_session_id = state
            .sessions
            .first()
            .map(|session| session.id.clone())
            .unwrap_or_default();
    }

    Ok((state.current_session_id.clone(), false))
}

fn new_chat_session(title: Option<&str>) -> ChatSession {
    let now = Utc::now().to_rfc3339();
    let resolved_title = title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("新对话")
        .to_string();

    ChatSession {
        id: Uuid::new_v4().to_string(),
        title: resolved_title,
        turns: Vec::new(),
        summary: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn build_chat_attachments(paths: &[String]) -> Vec<ChatAttachment> {
    paths.iter()
        .map(|path| ChatAttachment {
            path: path.clone(),
            name: Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("image")
                .to_string(),
        })
        .collect()
}

fn build_chat_turn(
    role: &str,
    text: &str,
    answer: Option<&GroundedAnswer>,
    attachments: &[ChatAttachment],
) -> ChatTurn {
    ChatTurn {
        id: Uuid::new_v4().to_string(),
        role: role.to_string(),
        text: text.to_string(),
        citations: answer.map(|item| item.citations.clone()).unwrap_or_default(),
        saved_note: answer.and_then(|item| item.saved_note.clone()),
        thinking_trace: answer.and_then(|item| item.thinking_trace.clone()),
        attachments: attachments.to_vec(),
        created_at: Utc::now().to_rfc3339(),
    }
}

fn append_turn_to_session(
    state: &mut ChatState,
    session_id: &str,
    turn: ChatTurn,
) -> Result<(), String> {
    let index = state
        .sessions
        .iter()
        .position(|session| session.id == session_id)
        .ok_or_else(|| format!("chat session not found: {}", session_id))?;
    let mut session = state.sessions.remove(index);
    let next_title = if session.title == "新对话" && turn.role == "user" {
        build_chat_session_title(&turn.text)
    } else {
        session.title.clone()
    };
    session.turns.push(turn);
    session.title = next_title;
    session.updated_at = Utc::now().to_rfc3339();
    state.current_session_id = session.id.clone();
    state.sessions.push(session);
    state
        .sessions
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(())
}

fn replace_chat_session(state: &mut ChatState, updated_session: ChatSession) -> Result<(), String> {
    let index = state
        .sessions
        .iter()
        .position(|session| session.id == updated_session.id)
        .ok_or_else(|| format!("chat session not found: {}", updated_session.id))?;
    state.sessions.remove(index);
    state.current_session_id = updated_session.id.clone();
    state.sessions.push(updated_session);
    state
        .sessions
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(())
}

fn find_chat_session<'a>(state: &'a ChatState, session_id: &str) -> Option<&'a ChatSession> {
    state.sessions.iter().find(|session| session.id == session_id)
}

fn current_session_history(state: &ChatState, session_id: &str) -> Result<Vec<ConversationTurn>, String> {
    let session = find_chat_session(state, session_id)
        .ok_or_else(|| format!("chat session not found: {}", session_id))?;
    let mut history = Vec::new();
    if let Some(summary) = session.summary.as_ref().filter(|summary| !summary.text.trim().is_empty()) {
        history.push(ConversationTurn {
            role: "system".to_string(),
            text: format!("此前对话摘要：{}", summary.text),
        });
    }
    history.extend(
        session
            .turns
            .iter()
            .filter(|turn| !turn.text.trim().is_empty())
            .map(|turn| ConversationTurn {
                role: turn.role.clone(),
                text: turn.text.clone(),
            }),
    );
    Ok(history)
}

async fn compress_chat_session_if_needed(
    context: &StorageContext,
    settings: &AppSettings,
    state: &mut ChatState,
    session_id: &str,
    pending_text: &str,
    pending_attachments: &[ChatAttachment],
    emit_status: &mut impl FnMut(&str, String),
) -> Result<(), String> {
    let session = find_chat_session(state, session_id)
        .cloned()
        .ok_or_else(|| format!("chat session not found: {}", session_id))?;
    let (context_window_tokens, _) = ai::resolve_context_window(settings);
    if context_window_tokens == 0 {
        return Ok(());
    }

    let projected_tokens = estimate_session_tokens(&session)
        + estimate_turn_tokens(pending_text, pending_attachments);
    if projected_tokens < ((context_window_tokens as f64) * CONTEXT_COMPRESSION_THRESHOLD) as u64 {
        return Ok(());
    }

    let compressible_count = session
        .turns
        .len()
        .saturating_sub(RECENT_TURNS_AFTER_COMPRESSION);
    if compressible_count < 2 {
        return Ok(());
    }

    let compressible_turns = session
        .turns
        .iter()
        .take(compressible_count)
        .filter(|turn| !turn.text.trim().is_empty())
        .map(|turn| ConversationTurn {
            role: turn.role.clone(),
            text: turn.text.clone(),
        })
        .collect::<Vec<_>>();
    if compressible_turns.len() < 2 {
        return Ok(());
    }

    let summary = compress_chat_history_with_context(
        context,
        session.summary.clone(),
        compressible_turns,
        |stage, detail| emit_status(stage, detail),
    )
    .await?;

    let mut updated_session = session;
    updated_session.summary = Some(summary);
    updated_session.turns = updated_session
        .turns
        .into_iter()
        .skip(compressible_count)
        .collect();
    updated_session.updated_at = Utc::now().to_rfc3339();
    replace_chat_session(state, updated_session)
}

fn estimate_session_tokens(session: &ChatSession) -> u64 {
    let mut total = estimate_tokens_for_text(session.summary.as_ref().map(|summary| summary.text.as_str()));
    for turn in &session.turns {
        total += estimate_turn_tokens(&turn.text, &turn.attachments);
    }
    total
}

fn estimate_turn_tokens(text: &str, attachments: &[ChatAttachment]) -> u64 {
    estimate_tokens_for_text(Some(text))
        + (attachments.len() as u64 * IMAGE_ATTACHMENT_TOKEN_ESTIMATE)
}

fn estimate_tokens_for_text(text: Option<&str>) -> u64 {
    let Some(text) = text else {
        return 0;
    };
    if text.trim().is_empty() {
        return 0;
    }

    let mut ascii = 0u64;
    let mut non_ascii = 0u64;
    for item in text.chars() {
        if item.is_whitespace() {
            continue;
        }
        if item <= '\u{7f}' {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
    }

    non_ascii + ((ascii + 3) / 4)
}

fn build_chat_session_title(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "新对话".to_string();
    }
    let mut title = String::new();
    let mut length = 0usize;
    for ch in normalized.chars() {
        if length >= 28 {
            break;
        }
        title.push(ch);
        length += 1;
    }
    if normalized.chars().count() > 28 {
        format!("{title}...")
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        extract_explicit_local_path, looks_like_record_request, looks_like_session_memory_question,
        looks_like_small_talk,
    };

    #[test]
    fn detects_session_memory_question() {
        assert!(looks_like_session_memory_question("我的名字叫什么？"));
        assert!(looks_like_session_memory_question("what is my name"));
    }

    #[test]
    fn detects_small_talk() {
        assert!(looks_like_small_talk("你好"));
        assert!(looks_like_small_talk("hello"));
    }

    #[test]
    fn detects_record_request() {
        assert!(looks_like_record_request("帮我记录这个命令"));
        assert!(looks_like_record_request("please save this"));
    }

    #[test]
    fn extracts_existing_local_path_from_question() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = env::temp_dir().join(format!("vaultpilot-path-test-{unique}"));
        fs::create_dir_all(&path).expect("create temp dir");
        let question = format!("你看看这个 `{}` 下面的记录符不符合？", path.display());

        let extracted = extract_explicit_local_path(&question).expect("path");
        assert_eq!(Path::new(&extracted), path.as_path());
    }
}
