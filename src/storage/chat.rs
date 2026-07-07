//! Chat session persistence — load, save, normalize, and derive titles.
//!
//! Extracted from `mod.rs` as Phase 2 of the incremental storage split (#1197).

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::models::{ChatSession, ChatState};

use super::atomic_write;
use super::session_export::export_sessions_to_markdown;

/// Maximum number of chat sessions to retain in the persisted state.
/// Older sessions beyond this limit are pruned during normalization.
pub(super) const MAX_SESSIONS: usize = 50;

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyChatState {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    turns: Vec<crate::models::ChatTurn>,
    #[serde(default)]
    summary: Option<crate::models::ConversationSummary>,
}

pub fn load_chat_state_with_context(context: &super::StorageContext) -> Result<ChatState> {
    let paths = &context.paths;
    if let Some(parent) = paths.chat_state_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    match std::fs::read_to_string(&paths.chat_state_path) {
        Ok(raw) => {
            let normalized = raw.trim_start_matches('\u{feff}');
            let state = parse_chat_state(normalized)
                .with_context(|| format!("failed to parse {}", paths.chat_state_path.display()))?;
            Ok(state)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let state = default_chat_state();
            save_chat_state_with_context(context, &state)?;
            Ok(state)
        }
        Err(e) => Err(anyhow::Error::from(e))
            .with_context(|| format!("failed to read {}", paths.chat_state_path.display())),
    }
}

pub fn save_chat_state_with_context(
    context: &super::StorageContext,
    state: &ChatState,
) -> Result<ChatState> {
    let paths = &context.paths;
    if let Some(parent) = paths.chat_state_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let normalized = normalize_chat_state(state.clone());

    let content = serde_json::to_string_pretty(&normalized)?;
    atomic_write(&paths.chat_state_path, content.as_bytes())
        .with_context(|| format!("failed to write {}", paths.chat_state_path.display()))?;

    // After writing chat state, also export sessions to markdown files if
    // the feature is enabled in settings (#1944).
    if let Err(e) = export_sessions_to_markdown(context) {
        tracing::warn!(
            error = %e,
            "failed to export sessions to markdown (session_export feature)"
        );
    }

    Ok(normalized)
}

fn default_chat_state() -> ChatState {
    let session = default_chat_session();
    ChatState {
        current_session_id: session.id.clone(),
        sessions: vec![session],
    }
}

fn default_chat_session() -> ChatSession {
    let now = Utc::now().to_rfc3339();
    ChatSession {
        id: Uuid::new_v4().to_string(),
        title: "新对话".to_string(),
        turns: Vec::new(),
        summary: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn parse_chat_state(raw: &str) -> Result<ChatState> {
    if let Ok(state) = serde_json::from_str::<ChatState>(raw) {
        return Ok(normalize_chat_state(state));
    }

    if let Ok(legacy) = serde_json::from_str::<LegacyChatState>(raw) {
        let now = Utc::now().to_rfc3339();
        let session = ChatSession {
            id: if legacy.session_id.trim().is_empty() {
                Uuid::new_v4().to_string()
            } else {
                legacy.session_id
            },
            title: derive_chat_title(&legacy.turns),
            turns: legacy.turns,
            summary: legacy.summary,
            created_at: now.clone(),
            updated_at: now,
        };
        return Ok(ChatState {
            current_session_id: session.id.clone(),
            sessions: vec![session],
        });
    }

    Err(anyhow!("unsupported chat state schema"))
}

fn normalize_chat_state(mut state: ChatState) -> ChatState {
    if state.sessions.is_empty() {
        return default_chat_state();
    }

    let now = Utc::now().to_rfc3339();
    for session in &mut state.sessions {
        if session.id.trim().is_empty() {
            session.id = Uuid::new_v4().to_string();
        }
        if session.title.trim().is_empty() {
            session.title = derive_chat_title(&session.turns);
        }
        if session.created_at.trim().is_empty() {
            session.created_at = now.clone();
        }
        if session.updated_at.trim().is_empty() {
            session.updated_at = session
                .turns
                .last()
                .and_then(|turn| {
                    let created_at = turn.created_at.trim();
                    if created_at.is_empty() {
                        None
                    } else {
                        Some(created_at.to_string())
                    }
                })
                .unwrap_or_else(|| now.clone());
        }
    }

    state
        .sessions
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    // Prune the oldest sessions when the count exceeds MAX_SESSIONS to prevent
    // unbounded growth of chat-state.json.
    if state.sessions.len() > MAX_SESSIONS {
        let pruned = state.sessions.len() - MAX_SESSIONS;
        info!(pruned, limit = MAX_SESSIONS, "pruning old chat session(s)");
        state.sessions.truncate(MAX_SESSIONS);
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

    state
}

fn derive_chat_title(turns: &[crate::models::ChatTurn]) -> String {
    let text = turns
        .iter()
        .find(|turn| turn.role == "user" && !turn.text.trim().is_empty())
        .map(|turn| turn.text.trim())
        .unwrap_or("新对话");

    let title = text.chars().take(22).collect::<String>().trim().to_string();
    if title.is_empty() {
        "新对话".to_string()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ChatTurn;

    // ── parse_chat_state ──

    #[test]
    fn parse_chat_state_modern_format() {
        let json = r#"{"currentSessionId":"s1","sessions":[{"id":"s1","title":"Hello","turns":[],"summary":null,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}]}"#;
        let state = parse_chat_state(json).expect("parse");
        assert_eq!(state.current_session_id, "s1");
        assert_eq!(state.sessions.len(), 1);
    }

    #[test]
    fn parse_chat_state_legacy_format_migrated_to_empty_sessions() {
        let json = r#"{"sessionId":"legacy1","turns":[{"id":"t1","role":"user","text":"hello","citations":[],"savedNote":null,"thinkingTrace":null,"attachments":[],"createdAt":"2026-01-01T00:00:00Z"}]}"#;
        let state = parse_chat_state(json).expect("parse");
        assert_eq!(state.sessions.len(), 1);
    }

    #[test]
    fn parse_chat_state_invalid_returns_err() {
        assert!(parse_chat_state("not json at all").is_err());
    }

    // ── normalize_chat_state ──

    #[test]
    fn normalize_chat_state_fills_empty_ids() {
        let state = ChatState {
            current_session_id: String::new(),
            sessions: vec![ChatSession {
                id: String::new(),
                title: "Test".to_string(),
                turns: vec![],
                summary: None,
                created_at: String::new(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        };
        let result = normalize_chat_state(state);
        assert!(!result.sessions[0].id.is_empty());
    }

    #[test]
    fn normalize_chat_state_empty_sessions_returns_default() {
        let state = ChatState {
            current_session_id: String::new(),
            sessions: vec![],
        };
        let result = normalize_chat_state(state);
        assert!(!result.sessions.is_empty());
    }

    #[test]
    fn normalize_chat_state_sorts_by_updated_at_desc() {
        let state = ChatState {
            current_session_id: "a".to_string(),
            sessions: vec![
                ChatSession {
                    id: "a".to_string(),
                    title: "Old".to_string(),
                    turns: vec![],
                    summary: None,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                },
                ChatSession {
                    id: "b".to_string(),
                    title: "New".to_string(),
                    turns: vec![],
                    summary: None,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-06-01T00:00:00Z".to_string(),
                },
            ],
        };
        let result = normalize_chat_state(state);
        assert_eq!(result.sessions[0].id, "b");
        assert_eq!(result.sessions[1].id, "a");
    }

    #[test]
    fn normalize_chat_state_fixes_invalid_current_session() {
        let state = ChatState {
            current_session_id: "ghost".to_string(),
            sessions: vec![ChatSession {
                id: "real".to_string(),
                title: "Real".to_string(),
                turns: vec![],
                summary: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        };
        let result = normalize_chat_state(state);
        assert_eq!(result.current_session_id, "real");
    }

    // ── derive_chat_title ──

    #[test]
    fn derive_chat_title_from_user_turn() {
        let turns = vec![ChatTurn {
            role: "user".to_string(),
            text: "mmc超时怎么处理比较好".to_string(),
            ..Default::default()
        }];
        let title = derive_chat_title(&turns);
        assert!(title.contains("mmc"));
    }

    #[test]
    fn derive_chat_title_no_user_turn() {
        let turns = vec![ChatTurn {
            role: "assistant".to_string(),
            text: "hi".to_string(),
            ..Default::default()
        }];
        assert_eq!(derive_chat_title(&turns), "新对话");
    }

    #[test]
    fn derive_chat_title_empty_turns() {
        assert_eq!(derive_chat_title(&[]), "新对话");
    }

    // ── Session pruning ──

    fn make_session(id: &str, updated_at: &str) -> ChatSession {
        ChatSession {
            id: id.to_string(),
            title: format!("Session {id}"),
            turns: Vec::new(),
            summary: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: updated_at.to_string(),
        }
    }

    #[test]
    fn normalize_prunes_sessions_beyond_limit() {
        let sessions: Vec<ChatSession> = (0..MAX_SESSIONS + 10)
            .map(|i| {
                make_session(
                    &format!("s{i}"),
                    &format!("2026-01-{:02}T00:00:00Z", (i % 28) + 1),
                )
            })
            .collect();
        let state = ChatState {
            current_session_id: "s0".to_string(),
            sessions,
        };
        let normalized = normalize_chat_state(state);
        assert_eq!(normalized.sessions.len(), MAX_SESSIONS);
    }

    #[test]
    fn normalize_keeps_newest_sessions() {
        let mut sessions: Vec<ChatSession> = Vec::new();
        for i in 0..40 {
            sessions.push(make_session(&format!("old{i}"), "2020-01-01T00:00:00Z"));
        }
        for i in 0..20 {
            sessions.push(make_session(
                &format!("new{i}"),
                &format!("2026-06-{:02}T00:00:00Z", i + 1),
            ));
        }
        let state = ChatState {
            current_session_id: "new0".to_string(),
            sessions,
        };
        let normalized = normalize_chat_state(state);
        assert_eq!(normalized.sessions.len(), MAX_SESSIONS);
        for i in 0..20 {
            assert!(normalized
                .sessions
                .iter()
                .any(|s| s.id == format!("new{i}")));
        }
    }

    #[test]
    fn normalize_does_not_prune_when_under_limit() {
        let sessions: Vec<ChatSession> = (0..10)
            .map(|i| make_session(&format!("s{i}"), "2026-06-01T00:00:00Z"))
            .collect();
        let state = ChatState {
            current_session_id: "s0".to_string(),
            sessions,
        };
        let normalized = normalize_chat_state(state);
        assert_eq!(normalized.sessions.len(), 10);
    }

    #[test]
    fn normalize_updates_current_session_if_pruned() {
        let mut sessions: Vec<ChatSession> = Vec::new();
        sessions.push(make_session("old-current", "2020-01-01T00:00:00Z"));
        for i in 0..MAX_SESSIONS {
            sessions.push(make_session(
                &format!("s{i}"),
                &format!("2026-06-{:02}T00:00:00Z", (i % 28) + 1),
            ));
        }
        let state = ChatState {
            current_session_id: "old-current".to_string(),
            sessions,
        };
        let normalized = normalize_chat_state(state);
        assert_eq!(normalized.sessions.len(), MAX_SESSIONS);
        assert!(normalized.current_session_id != "old-current");
        assert!(normalized
            .sessions
            .iter()
            .any(|s| s.id == normalized.current_session_id));
    }
}
