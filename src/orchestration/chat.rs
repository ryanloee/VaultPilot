use std::path::Path;

use crate::models::{
    AppSettings, ChatAttachment, ChatExchangeResult, ChatSession, ChatState, ChatTurn,
    ConversationTurn, GroundedAnswer,
};
use crate::storage::{
    initialize_storage_async, load_chat_state_async, ocr_image_text_async, save_chat_state_async,
    StorageContext,
};
use chrono::Utc;
use tracing::instrument;
use uuid::Uuid;

use crate::ai;

use super::ask::ask_with_ai_with_context;
use super::compress::compress_chat_history_with_context;

const CONTEXT_COMPRESSION_THRESHOLD: f64 = 0.95;
const RECENT_TURNS_AFTER_COMPRESSION: usize = 8;
const IMAGE_ATTACHMENT_TOKEN_ESTIMATE: u64 = 1_200;
const IMAGE_ONLY_PROMPT: &str = "请结合我发送的图片理解并回复。";
const OCR_SECTION_HEADER: &str = "[图片文字识别结果]:";

#[instrument(skip(context, question, image_paths, emit_status))]
pub async fn chat_with_ai_with_context(
    context: &StorageContext,
    session_id: Option<String>,
    question: String,
    image_paths: Option<Vec<String>>,
    create_new_session: bool,
    mut emit_status: impl FnMut(&str, String),
) -> Result<ChatExchangeResult, anyhow::Error> {
    let settings = initialize_storage_async(context).await?;
    let mut state = load_chat_state_async(context).await?;
    let images = image_paths.unwrap_or_default();
    let trimmed_question = question.trim().to_string();
    if trimmed_question.is_empty() && images.is_empty() {
        return Err(anyhow::anyhow!("question is empty"));
    }

    let prompt = build_effective_question(&trimmed_question, &images).await;
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

    let answer = ask_with_ai_with_context(
        context,
        prompt,
        Some(history),
        if images.is_empty() {
            None
        } else {
            Some(images)
        },
        None,
        |stage, detail| emit_status(stage, detail),
    )
    .await?;

    let assistant_turn = build_chat_turn("assistant", &answer.answer, Some(&answer), &[]);
    append_turn_to_session(&mut state, &active_session_id, assistant_turn)?;
    state = save_chat_state_async(context, &state).await?;

    let session = find_chat_session(&state, &active_session_id).ok_or_else(|| {
        anyhow::anyhow!("chat session not found after save: {}", active_session_id)
    })?;

    Ok(ChatExchangeResult {
        session_id: session.id.clone(),
        session_title: session.title.clone(),
        created_session,
        answer,
        state,
    })
}

pub async fn build_effective_question(question: &str, image_paths: &[String]) -> String {
    let mut prompt = if question.trim().is_empty() {
        IMAGE_ONLY_PROMPT.to_string()
    } else {
        question.trim().to_string()
    };

    if image_paths.is_empty() || prompt.contains(OCR_SECTION_HEADER) {
        return prompt;
    }

    let mut ocr_parts = Vec::new();
    for image_path in image_paths {
        if let Ok(text) = ocr_image_text_async(Path::new(image_path)).await {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                ocr_parts.push(trimmed.to_string());
            }
        }
    }

    if !ocr_parts.is_empty() {
        prompt = append_ocr_text_to_prompt(prompt, &ocr_parts);
    }

    prompt
}

fn append_ocr_text_to_prompt(mut prompt: String, ocr_parts: &[String]) -> String {
    if ocr_parts.is_empty() {
        return prompt;
    }

    prompt.push_str("\n\n");
    prompt.push_str(OCR_SECTION_HEADER);
    prompt.push('\n');
    prompt.push_str(&ocr_parts.join("\n"));
    prompt
}

fn resolve_or_create_chat_session(
    state: &mut ChatState,
    requested_session_id: Option<&str>,
    create_new_session: bool,
) -> Result<(String, bool), anyhow::Error> {
    if create_new_session || state.sessions.is_empty() {
        let session = new_chat_session(None);
        let id = session.id.clone();
        state.current_session_id = id.clone();
        state.sessions.insert(0, session);
        return Ok((id, true));
    }

    if let Some(session_id) = requested_session_id.filter(|value| !value.trim().is_empty()) {
        if state
            .sessions
            .iter()
            .any(|session| session.id == session_id)
        {
            state.current_session_id = session_id.to_string();
            return Ok((session_id.to_string(), false));
        }
        return Err(anyhow::anyhow!("chat session not found: {}", session_id));
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
    paths
        .iter()
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
        citations: answer
            .map(|item| item.citations.clone())
            .unwrap_or_default(),
        saved_note: answer.and_then(|item| item.saved_note.clone()),
        thinking_trace: answer.and_then(|item| item.thinking_trace.clone()),
        attachments: attachments.to_vec(),
        created_at: Utc::now().to_rfc3339(),
        source: String::new(),
    }
}

fn append_turn_to_session(
    state: &mut ChatState,
    session_id: &str,
    turn: ChatTurn,
) -> Result<(), anyhow::Error> {
    let index = state
        .sessions
        .iter()
        .position(|session| session.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("chat session not found: {}", session_id))?;
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

fn replace_chat_session(
    state: &mut ChatState,
    updated_session: ChatSession,
) -> Result<(), anyhow::Error> {
    let index = state
        .sessions
        .iter()
        .position(|session| session.id == updated_session.id)
        .ok_or_else(|| anyhow::anyhow!("chat session not found: {}", updated_session.id))?;
    state.sessions.remove(index);
    state.current_session_id = updated_session.id.clone();
    state.sessions.push(updated_session);
    state
        .sessions
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(())
}

fn find_chat_session<'a>(state: &'a ChatState, session_id: &str) -> Option<&'a ChatSession> {
    state
        .sessions
        .iter()
        .find(|session| session.id == session_id)
}

fn current_session_history(
    state: &ChatState,
    session_id: &str,
) -> Result<Vec<ConversationTurn>, anyhow::Error> {
    let session = find_chat_session(state, session_id)
        .ok_or_else(|| anyhow::anyhow!("chat session not found: {}", session_id))?;
    let mut history = Vec::new();
    if let Some(summary) = session
        .summary
        .as_ref()
        .filter(|summary| !summary.text.trim().is_empty())
    {
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
) -> Result<(), anyhow::Error> {
    let session = find_chat_session(state, session_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("chat session not found: {}", session_id))?;
    let (context_window_tokens, _) = ai::resolve_context_window(settings);
    if context_window_tokens == 0 {
        return Ok(());
    }

    let projected_tokens =
        estimate_session_tokens(&session) + estimate_turn_tokens(pending_text, pending_attachments);
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
            text: enrich_turn_for_compression(turn),
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
    let mut total = estimate_tokens_for_text(
        session
            .summary
            .as_ref()
            .map(|summary| summary.text.as_str()),
    );
    for turn in &session.turns {
        total += estimate_turn_tokens(&turn.text, &turn.attachments);
    }
    total
}

/// Enrich a ChatTurn's text with metadata annotations before compression,
/// so citations, attachments, thinking traces, and saved notes are
/// preserved in the compressed summary rather than silently dropped.
fn enrich_turn_for_compression(turn: &ChatTurn) -> String {
    let mut text = turn.text.clone();

    if !turn.attachments.is_empty() {
        let names: Vec<&str> = turn
            .attachments
            .iter()
            .map(|a| a.name.as_str())
            .filter(|n| !n.is_empty())
            .collect();
        if !names.is_empty() {
            text.push_str(&format!("\n[Attachments: {}]", names.join(", ")));
        }
    }

    if !turn.citations.is_empty() {
        let sources: Vec<String> = turn
            .citations
            .iter()
            .map(|c| {
                if !c.title.is_empty() {
                    c.title.clone()
                } else {
                    c.path.clone()
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
        if !sources.is_empty() {
            text.push_str(&format!("\n[Citations: {}]", sources.join(", ")));
        }
    }

    if let Some(note) = &turn.saved_note {
        if !note.title.is_empty() {
            text.push_str(&format!("\n[Saved note: {}]", note.title));
        }
    }

    if let Some(trace) = &turn.thinking_trace {
        if !trace.summary.is_empty() {
            text.push_str(&format!("\n[Thinking trace summary: {}]", trace.summary));
        }
    }

    text
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
            non_ascii += 2; // CJK characters typically require ~2 tokens
        }
    }

    non_ascii + ascii.div_ceil(4)
}

fn build_chat_session_title(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "新对话".to_string();
    }
    let mut title = String::new();
    for (length, ch) in normalized.chars().enumerate() {
        if length >= 28 {
            break;
        }
        title.push(ch);
    }
    if normalized.chars().count() > 28 {
        format!("{title}...")
    } else {
        normalized
    }
}
