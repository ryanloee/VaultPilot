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
use super::mention::inject_mention_context;
use super::tweet_import::{detect_tweet_url, fetch_tweet_context};

/// Default compression threshold (80% of the model context window) used when
/// automatic compression is enabled but the configured value is unusable (#1928).
const DEFAULT_COMPRESSION_THRESHOLD: f64 = 0.8;
/// Clamp bounds for the user-configurable threshold. Values below 10% would
/// compress almost every turn; values above 100% are meaningless.
const MIN_COMPRESSION_THRESHOLD: f64 = 0.1;
const MAX_COMPRESSION_THRESHOLD: f64 = 1.0;
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
    let _guard = context.chat_state_lock.lock().await;
    let mut state = load_chat_state_async(context).await?;
    let images = image_paths.unwrap_or_default();
    let trimmed_question = question.trim().to_string();
    if trimmed_question.is_empty() && images.is_empty() {
        return Err(anyhow::anyhow!("question is empty"));
    }

    let prompt = build_effective_question(&trimmed_question, &images).await;
    // Resolve @-mention references to notes and inject their content (#3548).
    // Parse mentions from the *original* user text only, not the fully-assembled
    // prompt — tweet/OCR content routinely contains `@handle` patterns that must
    // not be treated as note mentions (#3552).
    let prompt = inject_mention_context(context, prompt, &trimmed_question).await;
    let user_display = if trimmed_question.is_empty() {
        "（发送了一张图片）".to_string()
    } else {
        trimmed_question.clone()
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

/// Intermediate context produced by [`prepare_chat_for_ai`] and consumed by
/// [`finalize_chat_with_ai_answer`].  Splitting the chat flow this way lets
/// callers release any coarse-grained lock between the prepare phase (state
/// I/O) and the expensive AI call.
pub struct PreparedChatContext {
    pub active_session_id: String,
    pub created_session: bool,
    pub prompt: String,
    pub history: Vec<ConversationTurn>,
    pub images: Vec<String>,
    /// The unique ID of the user turn added during prepare.
    /// Used by rollback to delete the exact turn, avoiding race conditions
    /// when concurrent requests target the same session.
    pub user_turn_id: String,
}

/// Phase 1 of a chat exchange: load state, resolve/create session, append the
/// user turn, and persist.  Returns a [`PreparedChatContext`] that carries all
/// the data needed for the subsequent AI call and finalisation.
pub async fn prepare_chat_for_ai(
    context: &StorageContext,
    session_id: Option<String>,
    question: String,
    image_paths: Option<Vec<String>>,
    create_new_session: bool,
    mut emit_status: impl FnMut(&str, String),
) -> Result<PreparedChatContext, anyhow::Error> {
    let settings = initialize_storage_async(context).await?;
    let mut state = load_chat_state_async(context).await?;
    let images = image_paths.unwrap_or_default();
    let trimmed_question = question.trim().to_string();
    if trimmed_question.is_empty() && images.is_empty() {
        return Err(anyhow::anyhow!("question is empty"));
    }

    let prompt = build_effective_question(&trimmed_question, &images).await;
    // Resolve @-mention references to notes and inject their content (#3548).
    // Parse mentions from the *original* user text only, not the fully-assembled
    // prompt — tweet/OCR content routinely contains `@handle` patterns that must
    // not be treated as note mentions (#3552).
    let prompt = inject_mention_context(context, prompt, &trimmed_question).await;
    let user_display = if trimmed_question.is_empty() {
        "（发送了一张图片）".to_string()
    } else {
        trimmed_question.clone()
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
    let user_turn_id = user_turn.id.clone();
    append_turn_to_session(&mut state, &active_session_id, user_turn)?;
    save_chat_state_async(context, &state).await?;

    Ok(PreparedChatContext {
        active_session_id,
        created_session,
        prompt,
        history,
        images,
        user_turn_id,
    })
}

/// Phase 3 of a chat exchange: reload state, append the assistant turn, and
/// persist.  Must be called *after* the AI call that produced `answer`.
pub async fn finalize_chat_with_ai_answer(
    context: &StorageContext,
    prepared: PreparedChatContext,
    answer: GroundedAnswer,
) -> Result<ChatExchangeResult, anyhow::Error> {
    let mut state = load_chat_state_async(context).await?;
    let assistant_turn = build_chat_turn("assistant", &answer.answer, Some(&answer), &[]);
    append_turn_to_session(&mut state, &prepared.active_session_id, assistant_turn)?;
    state = save_chat_state_async(context, &state).await?;

    let session = find_chat_session(&state, &prepared.active_session_id).ok_or_else(|| {
        anyhow::anyhow!(
            "chat session not found after save: {}",
            prepared.active_session_id
        )
    })?;

    Ok(ChatExchangeResult {
        session_id: session.id.clone(),
        session_title: session.title.clone(),
        created_session: prepared.created_session,
        answer,
        state,
    })
}

/// Rollback the last user turn from a chat session when the AI call fails.
/// This prevents orphaned user messages that confuse subsequent AI responses.
/// Must be called under `chat_state_lock`.
pub async fn rollback_last_user_turn(
    context: &StorageContext,
    session_id: &str,
    turn_id: &str,
) -> Result<(), anyhow::Error> {
    let mut state = load_chat_state_async(context).await?;
    let session = state
        .sessions
        .iter_mut()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("chat session not found: {}", session_id))?;

    // Remove the specific user turn by ID (safe under concurrency)
    let before = session.turns.len();
    session.turns.retain(|t| t.id != turn_id);
    let removed = before != session.turns.len();

    // If session was newly created and now has no turns, delete it
    if session.turns.is_empty() {
        state.sessions.retain(|s| s.id != session_id);
    }

    if removed {
        save_chat_state_async(context, &state).await?;
    }
    Ok(())
}

pub async fn build_effective_question(question: &str, image_paths: &[String]) -> String {
    let mut prompt = if question.trim().is_empty() {
        IMAGE_ONLY_PROMPT.to_string()
    } else {
        question.trim().to_string()
    };

    // Detect tweet/X URLs and fetch content via oEmbed API (#1864)
    if let Some(tweet_url) = detect_tweet_url(&prompt) {
        let tweet_context = fetch_tweet_context(&tweet_url).await;
        if !tweet_context.is_empty() {
            prompt.push_str(&tweet_context);
        }
    }

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

/// Resolve the effective compression threshold from user settings (#1928).
///
/// Returns `None` when automatic compression is disabled (the default), which
/// causes [`compress_chat_session_if_needed`] to skip compression entirely.
/// Otherwise returns the user's threshold clamped to
/// `[MIN_COMPRESSION_THRESHOLD, MAX_COMPRESSION_THRESHOLD]`, falling back to
/// [`DEFAULT_COMPRESSION_THRESHOLD`] for non-finite or otherwise unusable
/// configured values.
///
/// Extracted as a pure function so the decision logic is unit-testable without
/// a storage context or live AI call.
fn effective_compression_threshold(settings: &AppSettings) -> Option<f64> {
    if !settings.context_compression {
        return None;
    }
    let raw = f64::from(settings.compression_threshold);
    if !raw.is_finite() {
        return Some(DEFAULT_COMPRESSION_THRESHOLD);
    }
    Some(raw.clamp(MIN_COMPRESSION_THRESHOLD, MAX_COMPRESSION_THRESHOLD))
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
    // #1928: automatic context compression is opt-in. When disabled (the
    // default), long conversations are passed through unchanged.
    let Some(threshold) = effective_compression_threshold(settings) else {
        return Ok(());
    };
    let session = find_chat_session(state, session_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("chat session not found: {}", session_id))?;
    let (context_window_tokens, _) = ai::resolve_context_window(settings);
    if context_window_tokens == 0 {
        return Ok(());
    }

    let projected_tokens =
        estimate_session_tokens(&session) + estimate_turn_tokens(pending_text, pending_attachments);
    if projected_tokens < ((context_window_tokens as f64) * threshold) as u64 {
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

    let summary = match compress_chat_history_with_context(
        context,
        session.summary.clone(),
        compressible_turns,
        |stage, detail| emit_status(stage, detail),
    )
    .await
    {
        Ok(s) if !s.text.trim().is_empty() => s,
        Ok(_) => {
            tracing::warn!("compression returned empty summary; skipping compression");
            return Ok(());
        }
        Err(e) => {
            tracing::warn!("compression failed ({}); skipping compression", e);
            return Ok(());
        }
    };

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
                let label = if !c.title.is_empty() {
                    c.title.clone()
                } else {
                    c.path.clone()
                };
                if label.is_empty() {
                    return String::new();
                }
                if let Some(score) = c.score {
                    format!("{} ({:.0}%)", label, (score * 100.0).round())
                } else {
                    label
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AnswerCitation, NoteMeta, ThinkingTrace};

    // ── estimate_tokens_for_text ────────────────────────────────────

    #[test]
    fn estimate_tokens_none_returns_zero() {
        assert_eq!(estimate_tokens_for_text(None), 0);
    }

    #[test]
    fn estimate_tokens_empty_returns_zero() {
        assert_eq!(estimate_tokens_for_text(Some("")), 0);
        assert_eq!(estimate_tokens_for_text(Some("   ")), 0);
    }

    #[test]
    fn estimate_tokens_ascii_text() {
        // 4 ASCII chars → ceil(4/4) = 1 token
        assert_eq!(estimate_tokens_for_text(Some("abcd")), 1);
        // 8 ASCII chars → ceil(8/4) = 2 tokens
        assert_eq!(estimate_tokens_for_text(Some("abcdefgh")), 2);
    }

    #[test]
    fn estimate_tokens_cjk_text() {
        // Each CJK char counts as 2 tokens.
        assert_eq!(estimate_tokens_for_text(Some("你好")), 4);
    }

    #[test]
    fn estimate_tokens_mixed_ascii_cjk() {
        // "hello你好" → ASCII: 5 chars → ceil(5/4)=2, CJK: 2 chars → 4 → total 6
        assert_eq!(estimate_tokens_for_text(Some("hello你好")), 6);
    }

    #[test]
    fn estimate_tokens_ignores_whitespace() {
        // "a b c" → 3 ASCII non-whitespace → ceil(3/4) = 1
        assert_eq!(estimate_tokens_for_text(Some("a b c")), 1);
    }

    // ── estimate_turn_tokens ────────────────────────────────────────

    #[test]
    fn estimate_turn_tokens_with_attachments() {
        let attachments = vec![
            ChatAttachment {
                path: "a.png".into(),
                name: "a.png".into(),
            },
            ChatAttachment {
                path: "b.png".into(),
                name: "b.png".into(),
            },
        ];
        let text_tokens = estimate_tokens_for_text(Some("hello"));
        let expected = text_tokens + 2 * IMAGE_ATTACHMENT_TOKEN_ESTIMATE;
        assert_eq!(estimate_turn_tokens("hello", &attachments), expected);
    }

    // ── build_chat_session_title ────────────────────────────────────

    #[test]
    fn title_empty_returns_default() {
        assert_eq!(build_chat_session_title(""), "新对话");
        assert_eq!(build_chat_session_title("   "), "新对话");
    }

    #[test]
    fn title_short_text_returned_as_is() {
        assert_eq!(build_chat_session_title("hello world"), "hello world");
    }

    #[test]
    fn title_long_text_truncated_with_ellipsis() {
        let long = "a".repeat(30);
        let title = build_chat_session_title(&long);
        assert!(title.ends_with("..."));
        // 28 chars + "..." = 31
        assert_eq!(title.chars().count(), 31);
    }

    #[test]
    fn title_collapses_whitespace() {
        assert_eq!(build_chat_session_title("hello   world"), "hello world");
    }

    // ── enrich_turn_for_compression ─────────────────────────────────

    #[test]
    fn enrich_plain_text_unchanged() {
        let turn = ChatTurn {
            text: "hello".into(),
            ..Default::default()
        };
        assert_eq!(enrich_turn_for_compression(&turn), "hello");
    }

    #[test]
    fn enrich_appends_attachment_names() {
        let turn = ChatTurn {
            text: "see this".into(),
            attachments: vec![ChatAttachment {
                path: "a.png".into(),
                name: "photo.png".into(),
            }],
            ..Default::default()
        };
        let result = enrich_turn_for_compression(&turn);
        assert!(result.contains("[Attachments: photo.png]"));
    }

    #[test]
    fn enrich_appends_citations() {
        let turn = ChatTurn {
            text: "according to".into(),
            citations: vec![AnswerCitation {
                title: "My Note".into(),
                path: "notes/my.md".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let result = enrich_turn_for_compression(&turn);
        assert!(result.contains("[Citations: My Note]"));
    }

    #[test]
    fn enrich_empty_label_citation_with_score_filtered() {
        // #2671: citation with empty title + path but score present
        // should be filtered out, not produce " (75%)"
        let turn = ChatTurn {
            text: "hello".into(),
            citations: vec![AnswerCitation {
                title: String::new(),
                path: String::new(),
                score: Some(0.75),
                ..Default::default()
            }],
            ..Default::default()
        };
        let result = enrich_turn_for_compression(&turn);
        assert!(
            !result.contains("(75%)"),
            "empty-label citation with score should be filtered, got: {result}"
        );
        assert!(
            !result.contains("[Citations:"),
            "should have no citations section when all are empty, got: {result}"
        );
    }

    #[test]
    fn enrich_appends_saved_note_title() {
        let turn = ChatTurn {
            text: "saved".into(),
            saved_note: Some(NoteMeta {
                title: "Important Note".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = enrich_turn_for_compression(&turn);
        assert!(result.contains("[Saved note: Important Note]"));
    }

    #[test]
    fn enrich_appends_thinking_trace_summary() {
        let turn = ChatTurn {
            text: "thinking".into(),
            thinking_trace: Some(ThinkingTrace {
                summary: "Analyzed 3 sources".into(),
                steps: vec![],
            }),
            ..Default::default()
        };
        let result = enrich_turn_for_compression(&turn);
        assert!(result.contains("[Thinking trace summary: Analyzed 3 sources]"));
    }

    // ── append_ocr_text_to_prompt (#1488) ─────────────────────────

    #[test]
    fn ocr_empty_parts_returns_original() {
        let result = append_ocr_text_to_prompt("hello".into(), &[]);
        assert_eq!(result, "hello");
    }

    #[test]
    fn ocr_single_part_appended() {
        let result = append_ocr_text_to_prompt("prompt".into(), &["OCR text".into()]);
        assert!(result.starts_with("prompt"));
        assert!(result.contains(OCR_SECTION_HEADER));
        assert!(result.contains("OCR text"));
    }

    #[test]
    fn ocr_multiple_parts_joined_with_newline() {
        let parts = vec!["line1".into(), "line2".into(), "line3".into()];
        let result = append_ocr_text_to_prompt("q".into(), &parts);
        assert!(result.contains("line1\nline2\nline3"));
    }

    // ── find_chat_session (#1488) ─────────────────────────────────

    #[test]
    fn find_existing_session() {
        let session = ChatSession {
            id: "s1".into(),
            title: "Test".into(),
            ..Default::default()
        };
        let state = ChatState {
            sessions: vec![session],
            current_session_id: "s1".into(),
        };
        assert!(find_chat_session(&state, "s1").is_some());
        assert_eq!(find_chat_session(&state, "s1").unwrap().title, "Test");
    }

    #[test]
    fn find_nonexistent_session_returns_none() {
        let state = ChatState {
            sessions: vec![],
            current_session_id: String::new(),
        };
        assert!(find_chat_session(&state, "missing").is_none());
    }

    #[test]
    fn find_session_wrong_id_returns_none() {
        let session = ChatSession {
            id: "s1".into(),
            ..Default::default()
        };
        let state = ChatState {
            sessions: vec![session],
            current_session_id: "s1".into(),
        };
        assert!(find_chat_session(&state, "s2").is_none());
    }

    // ── new_chat_session (#1488) ──────────────────────────────────

    #[test]
    fn new_session_with_title() {
        let session = new_chat_session(Some("My Title"));
        assert_eq!(session.title, "My Title");
        assert!(session.turns.is_empty());
        assert!(session.summary.is_none());
    }

    #[test]
    fn new_session_empty_title_uses_default() {
        let session = new_chat_session(Some("   "));
        assert_eq!(session.title, "新对话");
    }

    #[test]
    fn new_session_none_title_uses_default() {
        let session = new_chat_session(None);
        assert_eq!(session.title, "新对话");
    }

    #[test]
    fn new_session_has_unique_id() {
        let s1 = new_chat_session(None);
        let s2 = new_chat_session(None);
        assert_ne!(s1.id, s2.id);
    }

    // ── build_chat_attachments (#1488) ────────────────────────────

    #[test]
    fn build_attachments_empty() {
        assert!(build_chat_attachments(&[]).is_empty());
    }

    #[test]
    fn build_attachments_extracts_filename() {
        let paths = vec!["/tmp/photo.jpg".into(), "/docs/report.pdf".into()];
        let atts = build_chat_attachments(&paths);
        assert_eq!(atts.len(), 2);
        assert_eq!(atts[0].name, "photo.jpg");
        assert_eq!(atts[1].name, "report.pdf");
        assert_eq!(atts[0].path, "/tmp/photo.jpg");
    }

    #[test]
    fn build_attachments_path_without_filename() {
        // Path "/" → file_name() returns None → fallback "image"
        let paths = vec!["/".into()];
        let atts = build_chat_attachments(&paths);
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0].name, "image");
    }

    // ── build_chat_session_title CJK (#1488) ─────────────────────

    #[test]
    fn title_pure_cjk_within_limit() {
        let title = build_chat_session_title("今天天气很好");
        assert_eq!(title, "今天天气很好"); // 6 chars, well under 28
    }

    #[test]
    fn title_pure_cjk_exceeds_limit() {
        let long_cjk = "中".repeat(30);
        let title = build_chat_session_title(&long_cjk);
        assert!(title.ends_with("..."));
        assert_eq!(title.chars().count(), 31); // 28 + "..."
    }

    #[test]
    fn title_mixed_cjk_ascii() {
        let title = build_chat_session_title("Hello 世界 test");
        assert_eq!(title, "Hello 世界 test"); // 14 chars
    }

    // ── estimate_session_tokens (#1488) ───────────────────────────

    #[test]
    fn estimate_session_empty_turns() {
        let session = ChatSession {
            turns: vec![],
            ..Default::default()
        };
        assert_eq!(estimate_session_tokens(&session), 0);
    }

    #[test]
    fn estimate_session_with_summary_and_turns() {
        let session = ChatSession {
            summary: Some(crate::models::ConversationSummary {
                text: "hello".into(),
                generated_at: String::new(),
                covered_turn_count: 1,
                compression_count: 1,
            }),
            turns: vec![ChatTurn {
                text: "test".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let tokens = estimate_session_tokens(&session);
        // summary "hello" = 2, turn "test" = 1 → total 3
        assert_eq!(tokens, 3);
    }

    #[test]
    fn estimate_session_with_image_attachment() {
        let session = ChatSession {
            turns: vec![ChatTurn {
                text: "see this".into(),
                attachments: vec![ChatAttachment {
                    path: "a.png".into(),
                    name: "a.png".into(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let tokens = estimate_session_tokens(&session);
        // "see this" = 8 chars → ceil(8/4)=2, + 1200 attachment
        assert_eq!(tokens, 2 + IMAGE_ATTACHMENT_TOKEN_ESTIMATE);
    }

    // ── enrich_turn edge cases (#1488) ────────────────────────────

    #[test]
    fn enrich_turn_empty_attachment_name_filtered() {
        let turn = ChatTurn {
            text: "text".into(),
            attachments: vec![ChatAttachment {
                path: "a.png".into(),
                name: String::new(), // empty name should be filtered
            }],
            ..Default::default()
        };
        let result = enrich_turn_for_compression(&turn);
        assert!(!result.contains("[Attachments:"));
    }

    #[test]
    fn enrich_turn_empty_citation_title_uses_path() {
        let turn = ChatTurn {
            text: "text".into(),
            citations: vec![AnswerCitation {
                title: String::new(),
                path: "notes/tip.md".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let result = enrich_turn_for_compression(&turn);
        assert!(result.contains("[Citations: notes/tip.md]"));
    }

    #[test]
    fn enrich_turn_all_metadata_combined() {
        let turn = ChatTurn {
            text: "complete".into(),
            attachments: vec![ChatAttachment {
                path: "a".into(),
                name: "img.png".into(),
            }],
            citations: vec![AnswerCitation {
                title: "Ref".into(),
                path: "p".into(),
                ..Default::default()
            }],
            saved_note: Some(NoteMeta {
                title: "Note".into(),
                ..Default::default()
            }),
            thinking_trace: Some(ThinkingTrace {
                summary: "Thought".into(),
                steps: vec![],
            }),
            ..Default::default()
        };
        let result = enrich_turn_for_compression(&turn);
        assert!(result.contains("[Attachments: img.png]"));
        assert!(result.contains("[Citations: Ref]"));
        assert!(result.contains("[Saved note: Note]"));
        assert!(result.contains("[Thinking trace summary: Thought]"));
    }

    #[test]
    fn resolve_create_new_session_when_empty() {
        let mut state = ChatState {
            sessions: vec![],
            current_session_id: String::new(),
        };
        let (id, is_new) = resolve_or_create_chat_session(&mut state, None, false).unwrap();
        assert!(is_new);
        assert!(!id.is_empty());
        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.current_session_id, id);
    }

    #[test]
    fn resolve_create_new_session_when_forced() {
        let existing = ChatSession {
            id: "s1".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![existing],
            current_session_id: "s1".into(),
        };
        let (id, is_new) = resolve_or_create_chat_session(&mut state, None, true).unwrap();
        assert!(is_new);
        assert_ne!(id, "s1");
        assert_eq!(state.sessions.len(), 2);
    }

    #[test]
    fn resolve_find_existing_session_by_id() {
        let existing = ChatSession {
            id: "s1".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![existing],
            current_session_id: "s1".into(),
        };
        let (id, is_new) = resolve_or_create_chat_session(&mut state, Some("s1"), false).unwrap();
        assert!(!is_new);
        assert_eq!(id, "s1");
    }

    #[test]
    fn resolve_nonexistent_session_id_returns_error() {
        let existing = ChatSession {
            id: "s1".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![existing],
            current_session_id: "s1".into(),
        };
        let result = resolve_or_create_chat_session(&mut state, Some("missing"), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn resolve_falls_back_to_first_session() {
        let s1 = ChatSession {
            id: "s1".into(),
            ..Default::default()
        };
        let s2 = ChatSession {
            id: "s2".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![s1, s2],
            current_session_id: "nonexistent".into(), // invalid current
        };
        let (id, is_new) = resolve_or_create_chat_session(&mut state, None, false).unwrap();
        assert!(!is_new);
        assert_eq!(id, "s1"); // falls back to first
    }

    #[test]
    fn resolve_empty_session_id_ignores_and_uses_current() {
        let s1 = ChatSession {
            id: "s1".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![s1],
            current_session_id: "s1".into(),
        };
        let (id, is_new) = resolve_or_create_chat_session(&mut state, Some("  "), false).unwrap();
        assert!(!is_new);
        assert_eq!(id, "s1");
    }

    #[test]
    fn append_turn_updates_title_from_default() {
        let s1 = ChatSession {
            id: "s1".into(),
            title: "新对话".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![s1],
            current_session_id: "s1".into(),
        };
        let turn = ChatTurn {
            id: "t1".into(),
            role: "user".into(),
            text: "Tell me about Rust programming".into(),
            ..Default::default()
        };
        append_turn_to_session(&mut state, "s1", turn).unwrap();
        // Title should be updated from "新对话" to first message's text
        let session = find_chat_session(&state, "s1").unwrap();
        assert_ne!(session.title, "新对话");
        assert_eq!(session.turns.len(), 1);
    }

    #[test]
    fn append_turn_preserves_non_default_title() {
        let s1 = ChatSession {
            id: "s1".into(),
            title: "My Chat".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![s1],
            current_session_id: "s1".into(),
        };
        let turn = ChatTurn {
            id: "t1".into(),
            role: "user".into(),
            text: "hello".into(),
            ..Default::default()
        };
        append_turn_to_session(&mut state, "s1", turn).unwrap();
        let session = find_chat_session(&state, "s1").unwrap();
        assert_eq!(session.title, "My Chat"); // preserved
    }

    #[test]
    fn append_turn_nonexistent_session_errors() {
        let mut state = ChatState {
            sessions: vec![],
            current_session_id: String::new(),
        };
        let turn = ChatTurn {
            id: "t1".into(),
            role: "user".into(),
            text: "hi".into(),
            ..Default::default()
        };
        let result = append_turn_to_session(&mut state, "missing", turn);
        assert!(result.is_err());
    }

    #[test]
    fn append_turn_assistant_does_not_change_title() {
        let s1 = ChatSession {
            id: "s1".into(),
            title: "新对话".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![s1],
            current_session_id: "s1".into(),
        };
        let turn = ChatTurn {
            id: "t1".into(),
            role: "assistant".into(),
            text: "Here's my answer".into(),
            ..Default::default()
        };
        append_turn_to_session(&mut state, "s1", turn).unwrap();
        let session = find_chat_session(&state, "s1").unwrap();
        assert_eq!(session.title, "新对话"); // assistant turn doesn't change title
    }

    #[test]
    fn replace_existing_session() {
        let s1 = ChatSession {
            id: "s1".into(),
            title: "Old".into(),
            ..Default::default()
        };
        let mut state = ChatState {
            sessions: vec![s1],
            current_session_id: "s1".into(),
        };
        let updated = ChatSession {
            id: "s1".into(),
            title: "Updated".into(),
            ..Default::default()
        };
        replace_chat_session(&mut state, updated).unwrap();
        assert_eq!(find_chat_session(&state, "s1").unwrap().title, "Updated");
        assert_eq!(state.sessions.len(), 1);
    }

    #[test]
    fn replace_nonexistent_session_errors() {
        let mut state = ChatState {
            sessions: vec![],
            current_session_id: String::new(),
        };
        let updated = ChatSession {
            id: "missing".into(),
            ..Default::default()
        };
        let result = replace_chat_session(&mut state, updated);
        assert!(result.is_err());
    }

    #[test]
    fn history_with_summary_and_turns() {
        let s1 = ChatSession {
            id: "s1".into(),
            summary: Some(crate::models::ConversationSummary {
                text: "Discussed Rust".into(),
                generated_at: String::new(),
                covered_turn_count: 2,
                compression_count: 1,
            }),
            turns: vec![
                ChatTurn {
                    id: "t1".into(),
                    role: "user".into(),
                    text: "hi".into(),
                    ..Default::default()
                },
                ChatTurn {
                    id: "t2".into(),
                    role: "assistant".into(),
                    text: "hello".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let state = ChatState {
            sessions: vec![s1],
            current_session_id: "s1".into(),
        };
        let history = current_session_history(&state, "s1").unwrap();
        // Summary turn + 2 message turns = 3
        assert_eq!(history.len(), 3);
        assert!(history[0].text.contains("摘要"));
        assert_eq!(history[1].role, "user");
        assert_eq!(history[2].role, "assistant");
    }

    #[test]
    fn history_empty_summary_no_system_turn() {
        let s1 = ChatSession {
            id: "s1".into(),
            summary: Some(crate::models::ConversationSummary {
                text: "  ".into(), // empty-ish summary
                generated_at: String::new(),
                covered_turn_count: 0,
                compression_count: 1,
            }),
            turns: vec![ChatTurn {
                id: "t1".into(),
                role: "user".into(),
                text: "hi".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let state = ChatState {
            sessions: vec![s1],
            current_session_id: "s1".into(),
        };
        let history = current_session_history(&state, "s1").unwrap();
        assert_eq!(history.len(), 1); // only the user turn, no summary turn
    }

    #[test]
    fn history_filters_empty_text_turns() {
        let s1 = ChatSession {
            id: "s1".into(),
            turns: vec![
                ChatTurn {
                    id: "t1".into(),
                    role: "user".into(),
                    text: "hi".into(),
                    ..Default::default()
                },
                ChatTurn {
                    id: "t2".into(),
                    role: "assistant".into(),
                    text: "  ".into(),
                    ..Default::default()
                },
                ChatTurn {
                    id: "t3".into(),
                    role: "user".into(),
                    text: "bye".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let state = ChatState {
            sessions: vec![s1],
            current_session_id: "s1".into(),
        };
        let history = current_session_history(&state, "s1").unwrap();
        assert_eq!(history.len(), 2); // empty-text turn filtered out
    }

    #[test]
    fn history_nonexistent_session_errors() {
        let state = ChatState {
            sessions: vec![],
            current_session_id: String::new(),
        };
        let result = current_session_history(&state, "missing");
        assert!(result.is_err());
    }

    #[test]
    fn estimate_turn_no_text_no_attachments() {
        assert_eq!(estimate_turn_tokens("", &[]), 0);
    }

    #[test]
    fn estimate_turn_multiple_attachments() {
        let attachments = vec![
            ChatAttachment {
                path: "a.png".into(),
                name: "a.png".into(),
            },
            ChatAttachment {
                path: "b.png".into(),
                name: "b.png".into(),
            },
            ChatAttachment {
                path: "c.png".into(),
                name: "c.png".into(),
            },
        ];
        let tokens = estimate_turn_tokens("text", &attachments);
        // "text" = 4 chars → 1 token, + 3 × 1200 = 3601
        assert_eq!(tokens, 1 + 3 * IMAGE_ATTACHMENT_TOKEN_ESTIMATE);
    }

    #[test]
    fn estimate_turn_cjk_with_attachments() {
        let attachments = vec![ChatAttachment {
            path: "a".into(),
            name: "a".into(),
        }];
        let tokens = estimate_turn_tokens("你好世界", &attachments);
        // 4 CJK chars = 8 tokens, + 1200
        assert_eq!(tokens, 8 + IMAGE_ATTACHMENT_TOKEN_ESTIMATE);
    }

    // ── effective_compression_threshold (#1928) ────────────────────

    fn make_settings_for_compression(enabled: bool, threshold: f32) -> AppSettings {
        AppSettings {
            context_compression: enabled,
            compression_threshold: threshold,
            ..AppSettings::default()
        }
    }

    #[test]
    fn compression_threshold_disabled_returns_none() {
        // Default: compression off → never compress.
        let settings = AppSettings::default();
        assert_eq!(effective_compression_threshold(&settings), None);
    }

    #[test]
    fn compression_threshold_disabled_even_when_threshold_set() {
        // Toggle off wins regardless of the threshold value.
        let settings = make_settings_for_compression(false, 0.5);
        assert_eq!(effective_compression_threshold(&settings), None);
    }

    #[test]
    fn compression_threshold_enabled_uses_configured_value() {
        let settings = make_settings_for_compression(true, 0.8);
        // f32 → f64 promotion is not exact; compare against the same promotion.
        assert_eq!(
            effective_compression_threshold(&settings),
            Some(f64::from(0.8_f32))
        );
    }

    #[test]
    fn compression_threshold_clamps_below_minimum() {
        // A value below 10% would compress nearly every turn; clamp it.
        let settings = make_settings_for_compression(true, 0.01);
        assert_eq!(
            effective_compression_threshold(&settings),
            Some(MIN_COMPRESSION_THRESHOLD)
        );
    }

    #[test]
    fn compression_threshold_clamps_above_maximum() {
        let settings = make_settings_for_compression(true, 2.5);
        assert_eq!(
            effective_compression_threshold(&settings),
            Some(MAX_COMPRESSION_THRESHOLD)
        );
    }

    #[test]
    fn compression_threshold_negative_clamps_to_minimum() {
        let settings = make_settings_for_compression(true, -0.5);
        assert_eq!(
            effective_compression_threshold(&settings),
            Some(MIN_COMPRESSION_THRESHOLD)
        );
    }

    #[test]
    fn compression_threshold_at_bounds_preserved() {
        let min = make_settings_for_compression(true, MIN_COMPRESSION_THRESHOLD as f32);
        // f32 → f64 promotion of 0.1 is not exact; compare via the same path.
        assert_eq!(
            effective_compression_threshold(&min),
            Some(f64::from(MIN_COMPRESSION_THRESHOLD as f32))
        );
        let max = make_settings_for_compression(true, MAX_COMPRESSION_THRESHOLD as f32);
        assert_eq!(
            effective_compression_threshold(&max),
            Some(MAX_COMPRESSION_THRESHOLD)
        );
    }

    #[test]
    fn compression_threshold_non_finite_falls_back_to_default() {
        // NaN / infinity must not silently disable or runaway-compress.
        let settings = make_settings_for_compression(true, f32::NAN);
        assert_eq!(
            effective_compression_threshold(&settings),
            Some(DEFAULT_COMPRESSION_THRESHOLD)
        );
        let settings = make_settings_for_compression(true, f32::INFINITY);
        assert_eq!(
            effective_compression_threshold(&settings),
            Some(DEFAULT_COMPRESSION_THRESHOLD)
        );
    }

    #[test]
    fn compression_threshold_zero_clamps_to_minimum() {
        // 0.0 is finite, so it is clamped (not treated as "unset").
        let settings = make_settings_for_compression(true, 0.0);
        assert_eq!(
            effective_compression_threshold(&settings),
            Some(MIN_COMPRESSION_THRESHOLD)
        );
    }
}
