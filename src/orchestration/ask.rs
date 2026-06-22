use std::fs;
use std::path::{Path, PathBuf};

use ai::AssistantToolCall;
use crate::models::{
    AppSettings, ConversationTurn, ContextStatus, GroundedAnswer, NoteDocument,
    NoteMeta, StructuredNoteDraft, ThinkingTrace, ThinkingTraceStep,
};
use crate::storage::{
    has_notes_async, initialize_storage_async, load_context_notes_async,
    load_recent_notes_for_overview_async, save_note_with_images_async, StorageContext,
};
use tracing::instrument;

use crate::ai;

use super::chat::build_effective_question;

#[derive(Debug, Clone)]
pub struct ToolExecution {
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

#[instrument(skip(context, question, history, image_paths, emit_status))]
pub async fn ask_with_ai_with_context(
    context: &StorageContext,
    question: String,
    history: Option<Vec<ConversationTurn>>,
    image_paths: Option<Vec<String>>,
    model_override: Option<String>,
    mut emit_status: impl FnMut(&str, String),
) -> Result<GroundedAnswer, anyhow::Error> {
    let mut settings = initialize_storage_async(context).await?;
    if let Some(model) = model_override.filter(|m| !m.trim().is_empty()) {
        settings.effective_provider_mut().model = model;
    }
    let images = image_paths.unwrap_or_default();
    let raw_question = question.trim().to_string();
    if raw_question.is_empty() && images.is_empty() {
        return Err(anyhow::anyhow!("question is empty"));
    }

    let effective_question = build_effective_question(&raw_question, &images).await;
    let history = history.unwrap_or_default();
    let session_memory_question = looks_like_session_memory_question(&raw_question);
    let has_local_notes = has_notes_async(context).await?;
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

        let selection = ai::select_tool_call(
            &settings,
            &effective_question,
            &images,
            &history,
            &tool_history,
        )
        .await?;
        usage = merge_usage(usage, selection.usage);

        let forced_local_path_tool =
            if round == 0 && tool_results.is_empty() && !looks_like_record_request(&raw_question) {
                preferred_explicit_path_tool(&raw_question)
            } else {
                None
            };

        let forced_search = forced_local_path_tool.is_none()
            && round == 0
            && matches!(selection.tool_call, AssistantToolCall::None)
            && has_local_notes
            && !looks_like_small_talk(&raw_question)
            && !session_memory_question;

        let tool_call = if let Some(path_tool) = forced_local_path_tool {
            path_tool
        } else if forced_search {
            AssistantToolCall::SearchNotes {
                query: effective_question.clone(),
                limit: 6,
            }
        } else {
            selection.tool_call
        };

        if has_matching_tool_execution(&tool_results, &tool_call) {
            emit_status("responding", "Reusing previous tool result".to_string());
            return finalize_checked_grounded_answer(
                &settings,
                &raw_question,
                &effective_question,
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
                return finalize_checked_grounded_answer(
                    &settings,
                    &raw_question,
                    &effective_question,
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
                let mut new_docs = match load_context_notes_async(
                    context,
                    &query,
                    &images,
                    limit.saturating_mul(3).max(8),
                )
                .await
                {
                    Ok(docs) => docs,
                    Err(error) => {
                        tool_results.push(ToolExecution::new(
                            "search_notes",
                            format!("query={} limit={}", query, limit),
                            format!("tool error: {}", error),
                            true,
                        ));
                        continue;
                    }
                };
                if new_docs.is_empty() {
                    emit_status(
                        "retrieving",
                        "No direct match; listing recent notes".to_string(),
                    );
                    match load_recent_notes_for_overview_async(context, limit.min(12)).await {
                        Ok(fallback_docs) => new_docs = fallback_docs,
                        Err(error) => {
                            tool_results.push(ToolExecution::new(
                                "search_notes",
                                format!("query={} limit={}", query, limit),
                                format!("tool error: {}", error),
                                true,
                            ));
                            continue;
                        }
                    }
                } else {
                    emit_status(
                        "ranking",
                        format!("Scored {} candidate notes", new_docs.len()),
                    );
                    new_docs.truncate(limit.max(1));
                }
                // Issue #763: Accumulate docs across tool rounds instead of
                // overwriting, so citations include notes from all searches.
                let existing_ids: std::collections::HashSet<String> =
                    docs.iter().map(|d| d.meta.id.clone()).collect();
                for doc in new_docs {
                    if !existing_ids.contains(&doc.meta.id) {
                        docs.push(doc);
                    }
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
                // Issue #763: Accumulate docs across tool rounds instead of overwriting.
                let new_docs = match load_recent_notes_for_overview_async(context, limit).await {
                    Ok(docs) => docs,
                    Err(error) => {
                        tool_results.push(ToolExecution::new(
                            "list_notes",
                            format!("limit={}", limit),
                            format!("tool error: {}", error),
                            true,
                        ));
                        continue;
                    }
                };
                let existing_ids: std::collections::HashSet<String> =
                    docs.iter().map(|d| d.meta.id.clone()).collect();
                for doc in new_docs {
                    if !existing_ids.contains(&doc.meta.id) {
                        docs.push(doc);
                    }
                }
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
                let path_owned = path.clone();
                let vault_owned = settings.vault_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    list_directory_result(&path_owned, Path::new(&vault_owned))
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("task join error: {}", e)));
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
                let path_owned = path.clone();
                let vault_owned = settings.vault_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    read_file_result(&path_owned, Path::new(&vault_owned))
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("task join error: {}", e)));
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
            AssistantToolCall::SaveNote { draft } => {
                emit_status("saving", "Saving generated note".to_string());
                match save_note_with_images_async(context, draft_to_note_document(*draft), &images)
                    .await
                {
                    Ok(saved) => {
                        let result = format!(
                            "save_note completed.\nSaved title: {}\nSaved path: {}\nSaved summary: {}",
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
                        return finalize_checked_grounded_answer(
                            &settings,
                            &raw_question,
                            &effective_question,
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
                    Err(error) => {
                        // #791: Record save failure as a tool error and continue
                        // to finalize — the user still gets an answer with the
                        // error context instead of an opaque request failure.
                        let error_msg = format!("tool error: save_note failed: {}", error);
                        tool_results.push(ToolExecution::new(
                            "save_note",
                            "model_generated_note_draft".to_string(),
                            error_msg,
                            true,
                        ));
                    }
                }
            }
        }
    }

    emit_status("responding", "Preparing final answer".to_string());
    finalize_checked_grounded_answer(
        &settings,
        &raw_question,
        &effective_question,
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

#[allow(clippy::too_many_arguments)]
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
) -> Result<GroundedAnswer, anyhow::Error> {
    let answer = if tool_results.is_empty() {
        ai::answer_question(settings, question, &[], images, history).await?
    } else {
        let transcript = tool_results
            .iter()
            .map(ToolExecution::render_for_model)
            .collect::<Vec<_>>();
        ai::answer_after_tools(settings, question, &transcript, docs, history).await?
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

#[allow(clippy::too_many_arguments)]
async fn finalize_checked_grounded_answer(
    settings: &AppSettings,
    raw_question: &str,
    effective_question: &str,
    history: &[ConversationTurn],
    images: &[String],
    docs: &[NoteDocument],
    tool_results: &[ToolExecution],
    saved_note: Option<NoteMeta>,
    usage: ai::RequestUsage,
    forced_search: bool,
) -> Result<GroundedAnswer, anyhow::Error> {
    let answer = finalize_grounded_answer(
        settings,
        effective_question,
        history,
        images,
        docs,
        tool_results,
        saved_note,
        usage,
        forced_search,
    )
    .await?;

    require_saved_note_for_record_request(raw_question, answer)
}

fn require_saved_note_for_record_request(
    question: &str,
    answer: GroundedAnswer,
) -> Result<GroundedAnswer, anyhow::Error> {
    if looks_like_record_request(question) && answer.saved_note.is_none() {
        return Err(anyhow::anyhow!(
            "record request did not produce a saved note"
        ));
    }

    Ok(answer)
}

fn has_matching_tool_execution(
    tool_results: &[ToolExecution],
    tool_call: &AssistantToolCall,
) -> bool {
    let Some((name, input)) = planned_tool_identity(tool_call) else {
        return false;
    };

    tool_results
        .iter()
        .any(|item| item.name == name && item.input == input)
}

fn planned_tool_identity(tool_call: &AssistantToolCall) -> Option<(&'static str, String)> {
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
    // Single-pass: collect up to max_chars, then check if we hit the limit
    // by attempting one more character.
    let mut result = String::new();
    let mut chars = value.chars();
    for _ in 0..max_chars {
        match chars.next() {
            Some(ch) => result.push(ch),
            None => return result, // shorter than max_chars, no truncation
        }
    }
    if chars.next().is_some() {
        result.push_str("...");
    }
    result
}

fn merge_usage(current: ai::RequestUsage, next: ai::RequestUsage) -> ai::RequestUsage {
    ai::RequestUsage {
        input_tokens: match (current.input_tokens, next.input_tokens) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        },
        output_tokens: match (current.output_tokens, next.output_tokens) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        },
    }
}

fn display_path(path: &str) -> &str {
    if path.trim().is_empty() {
        "(empty path)"
    } else {
        path
    }
}

fn list_directory_result(path: &str, vault_root: &Path) -> Result<String, anyhow::Error> {
    let directory = normalize_tool_path(path, vault_root)?;
    let display = directory.display().to_string();
    if !directory.exists() {
        return Err(anyhow::anyhow!("path does not exist: {}", display));
    }
    if !directory.is_dir() {
        return Err(anyhow::anyhow!("path is not a directory: {}", display));
    }

    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for entry in fs::read_dir(directory)? {
        match entry {
            Ok(e) => entries.push(e),
            Err(e) => errors.push(e.to_string()),
        }
    }
    entries.sort_by_key(|entry| entry.file_name());

    let total = entries.len();
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

    let mut output = if total > rendered.len() {
        format!(
            "list_directory showing {} of {} total entries.\n{}",
            rendered.len(),
            total,
            rendered.join("\n")
        )
    } else {
        format!(
            "list_directory returned {} items.\n{}",
            rendered.len(),
            rendered.join("\n")
        )
    };

    if !errors.is_empty() {
        output.push_str(&format!(
            "\n\n⚠ {} entries could not be read due to permission or I/O errors:\n{}",
            errors.len(),
            errors
                .iter()
                .take(10)
                .map(|e| format!("- {}", e))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if total > 60 {
        output.push_str(&format!(
            "\n\n(Showing first 60 of {} entries. Use a subdirectory path to see more.)",
            total
        ));
    }

    Ok(output)
}

fn read_file_result(path: &str, vault_root: &Path) -> Result<String, anyhow::Error> {
    let file_path = normalize_tool_path(path, vault_root)?;
    let display = file_path.display().to_string();
    if !file_path.exists() {
        return Err(anyhow::anyhow!("path does not exist: {}", display));
    }
    if !file_path.is_file() {
        return Err(anyhow::anyhow!("path is not a file: {}", display));
    }

    const MAX_FILE_SIZE: u64 = 1024 * 1024; // 1 MB
    let metadata = fs::metadata(&file_path)?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(anyhow::anyhow!(
            "file too large ({} bytes, limit is {} bytes): {}",
            metadata.len(),
            MAX_FILE_SIZE,
            display
        ));
    }

    /// Maximum number of characters to return from read_file (~50 KB).
    const READ_FILE_MAX_CHARS: usize = 50_000;
    /// Maximum number of lines to return from read_file.
    const READ_FILE_MAX_LINES: usize = 200;
    /// Number of lines from the beginning of the file to keep when truncating.
    const READ_FILE_HEAD_LINES: usize = 150;
    /// Number of lines from the end of the file to keep when truncating.
    const READ_FILE_TAIL_LINES: usize = 50;

    let content = fs::read_to_string(&file_path)?;
    let total_bytes = content.len();
    let total_chars = content.chars().count();
    let total_lines = content.lines().count();

    // If content fits within both limits, return as-is (no truncation needed).
    if total_chars <= READ_FILE_MAX_CHARS && total_lines <= READ_FILE_MAX_LINES {
        return Ok(format!(
            "read_file returned content for {}:\n{}",
            display, content
        ));
    }

    // Smart truncation: head + tail with metadata.
    let lines: Vec<&str> = content.lines().collect();
    let head_count = READ_FILE_HEAD_LINES.min(lines.len());
    let tail_count = READ_FILE_TAIL_LINES.min(lines.len());
    let tail_start = lines.len().saturating_sub(tail_count);

    // Avoid overlapping head and tail when file has fewer lines than the
    // combined head+tail budget but exceeds the character limit.
    let (skipped_lines, skipped_content, effective_head, effective_tail) =
        if tail_start >= head_count {
            let skipped = lines.len() - head_count - tail_count;
            let skipped_str = lines[head_count..tail_start].join("\n");
            (
                skipped,
                skipped_str,
                &lines[..head_count],
                &lines[tail_start..],
            )
        } else {
            // File is small enough — show head portion only, no overlapping tail
            (0usize, String::new(), &lines[..head_count], &lines[0..0])
        };

    let shown_chars: usize = effective_head
        .iter()
        .map(|l| l.chars().count())
        .sum::<usize>()
        + effective_tail
            .iter()
            .map(|l| l.chars().count())
            .sum::<usize>();

    let mut output = format!(
        "read_file returned content for {} ({} bytes, {} lines total):\n",
        display, total_bytes, total_lines
    );
    for line in effective_head {
        output.push_str(line);
        output.push('\n');
    }
    if skipped_lines > 0 {
        output.push_str(&format!(
            "\n... [{skipped_lines} lines / {skipped_chars} chars omitted — showing {} of {} total chars; first {head_lines} and last {tail_lines} lines kept] ...\n\n",
            shown_chars,
            total_chars,
            head_lines = READ_FILE_HEAD_LINES,
            tail_lines = READ_FILE_TAIL_LINES,
            skipped_lines = skipped_lines,
            skipped_chars = skipped_content.chars().count(),
        ));
        for line in effective_tail {
            output.push_str(line);
            output.push('\n');
        }
    }

    Ok(output)
}

pub fn normalize_tool_path(path: &str, vault_root: &Path) -> Result<PathBuf, anyhow::Error> {
    let trimmed = path.trim().trim_matches('\"').trim_matches('`');
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("path is empty"));
    }

    let normalized = if let Some(stripped) = trimmed.strip_prefix(r"\\?\") {
        format!(r"\\?\{}", stripped)
    } else if let Some(stripped) = trimmed.strip_prefix("//?/") {
        format!(r"\\?\{}", stripped.replace('/', "\\"))
    } else {
        trimmed.to_string()
    };

    let candidate = PathBuf::from(&normalized);

    // Confinement check: resolved path must stay within the vault directory.
    // Try canonicalize first (requires the path to exist). If it doesn't,
    // walk up to the nearest existing ancestor and verify the prefix.
    // Confinement check is fail-closed: if vault_root cannot be resolved,
    // reject the operation rather than silently skipping the security check.
    let vault_canonical = vault_root.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "cannot resolve vault directory '{}': {error}",
            vault_root.display()
        )
    })?;

    if let Ok(canonical) = candidate.canonicalize() {
        if !canonical.starts_with(&vault_canonical) {
            return Err(anyhow::anyhow!(
                "access denied: path '{}' is outside the vault directory",
                trimmed
            ));
        }
    } else {
        // Path doesn't exist — verify nearest existing ancestor is in-vault.
        let mut probe = candidate.as_path();
        let mut confined = false;
        while let Some(parent) = probe.parent() {
            if parent.as_os_str().is_empty() {
                break;
            }
            if parent.exists() {
                if let Ok(pc) = parent.canonicalize() {
                    if !pc.starts_with(&vault_canonical) {
                        return Err(anyhow::anyhow!(
                            "access denied: path '{}' is outside the vault directory",
                            trimmed
                        ));
                    }
                    confined = true;
                }
                break;
            }
            probe = parent;
        }
        if !confined {
            return Err(anyhow::anyhow!(
                "access denied: cannot verify path '{}' is inside the vault directory",
                trimmed
            ));
        }
    }

    Ok(candidate)
}

fn summarize_docs_for_tool_result(tool_name: &str, docs: &[NoteDocument]) -> String {
    if docs.is_empty() {
        return format!("{} returned 0 notes.", tool_name);
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

    format!("{} returned {} notes.\n{}", tool_name, docs.len(), items)
}

fn context_status_from_usage(
    settings: &AppSettings,
    input_tokens: Option<usize>,
    output_tokens: Option<usize>,
) -> ContextStatus {
    let (context_window_tokens, source) = ai::resolve_context_window(settings);
    let live_tokens = input_tokens.unwrap_or_default() + output_tokens.unwrap_or_default();
    ContextStatus {
        model: settings.effective_provider().model.clone(),
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
        precise: input_tokens.is_some() || output_tokens.is_some(),
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
    .any(|needle| normalized == *needle || normalized.starts_with(&format!("{} ", needle)))
}

fn looks_like_record_request(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    // Specific command phrases — these always indicate a record/save intent.
    let command_phrases = [
        "记录一下",
        "记一下",
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
    ];
    if command_phrases.iter().any(|p| normalized.contains(p)) {
        return true;
    }
    // Short generic verbs (记录, 保存, 记住) are only treated as record
    // requests when the input is NOT a question — otherwise phrases like
    // "目前记录的版本是多少" would false-positive.
    if looks_like_a_question(&normalized) {
        return false;
    }
    let generic_verbs = ["记录", "保存", "记住"];
    generic_verbs.iter().any(|p| normalized.contains(p))
}

/// Lightweight heuristic: does `input` look like a question rather than a
/// command?  Used to avoid false-positive matches on short verbs.
fn looks_like_a_question(input: &str) -> bool {
    // Question punctuation
    if input.contains('?') || input.contains('？') {
        return true;
    }
    // Common Chinese question words
    let question_words = [
        "什么",
        "多少",
        "怎么",
        "如何",
        "哪个",
        "哪些",
        "为什么",
        "是不是",
        "有没有",
    ];
    question_words.iter().any(|w| input.contains(w))
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
        search_snippet: None,
    }
}
