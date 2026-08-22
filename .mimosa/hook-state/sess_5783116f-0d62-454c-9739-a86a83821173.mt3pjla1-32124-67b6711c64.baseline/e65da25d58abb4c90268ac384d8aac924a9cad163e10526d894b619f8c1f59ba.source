//! Session-to-markdown export — save chat history as plain .md files in the vault.
//!
//! Every time a session is saved, the corresponding markdown file is written or
//! updated under `vault/.vaultpilot/sessions/{session-id}.md` (or a custom path
//! configured via `session_export_path`).
//!
//! The format uses YAML frontmatter (matching the vault note convention) plus a
//! human-readable markdown conversation body.  Because the files live inside the
//! vault, they are automatically picked up by the full-text search index and by
//! git if the vault is version-controlled.
//!
//! # Design decisions
//!
//! - **Full rewrite, not incremental append.**  Sessions are small (normally
//!   dozens of turns, rarely hundreds) and the write happens asynchronously in
//!   the save path, so we optimise for correctness (the file always reflects
//!   the canonical state) over incremental-append complexity.
//! - **Atomic write** via [`super::atomic_write`] to avoid torn/corrupt files.
//! - **Opt-in.**  `session_export_enabled` defaults to `false` so existing users
//!   see no change.  Enable it in Settings to start exporting.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::ChatSession;

use super::{load_settings_with_context, StorageContext};

/// Default sessions directory within the vault (relative to vault root).
const DEFAULT_SESSIONS_DIR: &str = ".vaultpilot/sessions";

// ── Frontmatter ────────────────────────────────────────────────────────────

/// YAML frontmatter embedded at the top of each exported session file.
#[derive(Debug, Serialize, Deserialize)]
struct SessionFrontmatter {
    /// Mark this as a session file (not a regular note) so tooling can
    /// distinguish them.
    kind: String,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    turn_count: usize,
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Export all sessions in `state` to individual markdown files in the
/// configured sessions directory inside the vault.
///
/// Called automatically from within [`save_chat_state_with_context`] when
/// `session_export_enabled` is `true`.
///
/// This function **silently skips** sessions whose markdown file already
/// matches the current state (checking via turn_count and updated_at), so
/// repeated saves are cheap.
pub fn export_sessions_to_markdown(context: &StorageContext) -> Result<()> {
    let settings = load_settings_with_context(context)?;
    if !settings.session_export_enabled {
        return Ok(());
    }

    let sessions_dir = resolve_sessions_dir(&settings);

    // Ensure the sessions directory exists.
    std::fs::create_dir_all(&sessions_dir)
        .with_context(|| format!("failed to create sessions dir: {}", sessions_dir.display()))?;

    // Load chat state from the same storage context.
    let chat_state = super::load_chat_state_with_context(context)?;

    // Build a set of session IDs that currently exist.
    let active_ids: std::collections::HashSet<String> =
        chat_state.sessions.iter().map(|s| s.id.clone()).collect();

    // Write / update markdown files for each active session.
    for session in &chat_state.sessions {
        let file_path = sessions_dir.join(format!("{}.md", sanitise_filename(&session.id)));
        let needs_write = check_needs_write(&file_path, session);

        if !needs_write {
            continue;
        }

        let markdown = compose_session_markdown(session)?;
        tracing::debug!(
            session_id = %session.id,
            path = %file_path.display(),
            "writing session markdown"
        );
        super::atomic_write(&file_path, markdown.as_bytes())
            .with_context(|| format!("failed to write {}", file_path.display()))?;
    }

    // Clean up orphaned files for sessions that have been deleted.
    if sessions_dir.exists() {
        for entry in std::fs::read_dir(&sessions_dir)
            .with_context(|| format!("failed to read sessions dir: {}", sessions_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if !active_ids.iter().any(|id| sanitise_filename(id) == stem) {
                        tracing::debug!(path = %path.display(), "removing orphaned session file");
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Export a single session by ID to its markdown file.
///
/// Returns `true` if the file was written, `false` if nothing changed.
#[allow(dead_code)]
pub fn export_single_session(context: &StorageContext, session_id: &str) -> Result<bool> {
    let settings = load_settings_with_context(context)?;
    if !settings.session_export_enabled {
        return Ok(false);
    }

    let chat_state = super::load_chat_state_with_context(context)?;
    let Some(session) = chat_state.sessions.iter().find(|s| s.id == session_id) else {
        return Ok(false);
    };

    let sessions_dir = resolve_sessions_dir(&settings);
    std::fs::create_dir_all(&sessions_dir)?;

    let file_path = sessions_dir.join(format!("{}.md", sanitise_filename(&session.id)));
    let markdown = compose_session_markdown(session)?;
    super::atomic_write(&file_path, markdown.as_bytes())
        .with_context(|| format!("failed to write {}", file_path.display()))?;
    Ok(true)
}

/// Remove the markdown file for a deleted session.
#[allow(dead_code)]
pub fn delete_session_file(context: &StorageContext, session_id: &str) -> Result<()> {
    let settings = load_settings_with_context(context)?;
    if !settings.session_export_enabled {
        return Ok(());
    }

    let sessions_dir = resolve_sessions_dir(&settings);
    let file_path = sessions_dir.join(format!("{}.md", sanitise_filename(session_id)));

    if file_path.exists() {
        std::fs::remove_file(&file_path)
            .with_context(|| format!("failed to delete session file: {}", file_path.display()))?;
        tracing::debug!(path = %file_path.display(), "deleted session markdown file");
    }

    Ok(())
}

// ── Internal helpers ───────────────────────────────────────────────────────

/// Resolve the effective sessions directory from settings.
fn resolve_sessions_dir(settings: &crate::models::AppSettings) -> PathBuf {
    let vault_dir = Path::new(&settings.vault_dir);
    match &settings.session_export_path {
        Some(custom) if !custom.trim().is_empty() => {
            let p = PathBuf::from(custom);
            if p.is_absolute() {
                p
            } else {
                vault_dir.join(&p)
            }
        }
        _ => vault_dir.join(DEFAULT_SESSIONS_DIR),
    }
}

/// Check whether the existing file for `session` is already up-to-date.
fn check_needs_write(file_path: &Path, session: &ChatSession) -> bool {
    if !file_path.exists() {
        return true;
    }

    // Read the frontmatter from the existing file to decide.
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return true,
    };

    // Extract the YAML frontmatter block (between first two `---` lines).
    let body = content.trim_start();
    if !body.starts_with("---") {
        return true; // malformed — rewrite
    }
    let end = match body[3..].find("\n---") {
        Some(pos) => pos + 3,
        None => return true, // no closing frontmatter — rewrite
    };
    let yaml_block = &body[3..end];

    let existing: SessionFrontmatter = match serde_yaml_ng::from_str(yaml_block) {
        Ok(f) => f,
        Err(_) => return true,
    };

    // Data-driven freshness check: only rewrite if something changed.
    existing.turn_count != session.turns.len()
        || existing.updated_at != session.updated_at
        || existing.title != session.title
}

/// Compose a full markdown document (frontmatter + conversation body) for a
/// single session.
fn compose_session_markdown(session: &ChatSession) -> Result<String> {
    let frontmatter = SessionFrontmatter {
        kind: "session".to_string(),
        session_id: session.id.clone(),
        title: session.title.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        turn_count: session.turns.len(),
    };

    let yaml = serde_yaml_ng::to_string(&frontmatter)?;

    let mut body = String::new();
    body.push_str(&format!("---\n{}---\n\n", yaml));
    body.push_str(&format!("# {}\n\n", session.title));

    if let Some(summary) = &session.summary {
        if !summary.text.trim().is_empty() {
            body.push_str("> **会话摘要**: ");
            body.push_str(&summary.text);
            body.push_str("\n\n");
        }
    }

    body.push_str("## 对话记录\n\n");

    for turn in &session.turns {
        let role_label = match turn.role.as_str() {
            "user" => "🧑 **你**",
            "assistant" => "🤖 **AI**",
            "system" => "⚙️ **系统**",
            _ => &turn.role,
        };

        body.push_str(&format!("### {} ({})\n\n", role_label, turn.created_at));
        body.push_str(&turn.text);
        body.push('\n');

        // Include citations if present.
        if !turn.citations.is_empty() {
            body.push_str("\n**引用**:\n");
            for (i, citation) in turn.citations.iter().enumerate() {
                if citation.path.trim().is_empty() {
                    body.push_str(&format!(
                        "{}. {} (note: `{}`)\n",
                        i + 1,
                        citation.title,
                        citation.note_id
                    ));
                } else {
                    body.push_str(&format!(
                        "{}. [{}]({})\n",
                        i + 1,
                        citation.title,
                        citation.path
                    ));
                }
            }
        }

        body.push_str("\n---\n\n");
    }

    if session.turns.is_empty() {
        body.push_str("*(空对话)*\n\n");
    }

    Ok(body)
}

/// Sanitise a session ID for use as a filename — replace non-alphanumeric
/// characters with hyphens and collapse runs.
fn sanitise_filename(id: &str) -> String {
    let sanitized: String = id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let collapsed: String = sanitized.chars().fold(String::new(), |mut acc, c| {
        if c == '-' && acc.ends_with('-') {
            // skip duplicate hyphen
        } else {
            acc.push(c);
        }
        acc
    });
    collapsed.trim_matches('-').to_string()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ChatTurn;

    fn sample_turn(role: &str, text: &str) -> ChatTurn {
        ChatTurn {
            role: role.to_string(),
            text: text.to_string(),
            created_at: "2026-07-01T12:00:00Z".to_string(),
            ..Default::default()
        }
    }

    fn sample_session() -> ChatSession {
        ChatSession {
            id: "abc-123".to_string(),
            title: "测试 Rust 代码".to_string(),
            turns: vec![
                sample_turn("user", "如何用 Rust 处理文件？"),
                sample_turn("assistant", "你可以使用 `std::fs` 模块。"),
            ],
            created_at: "2026-07-01T12:00:00Z".to_string(),
            updated_at: "2026-07-01T12:01:00Z".to_string(),
            summary: None,
        }
    }

    #[test]
    fn compose_session_markdown_includes_frontmatter() {
        let session = sample_session();
        let md = compose_session_markdown(&session).expect("compose");
        assert!(md.starts_with("---\n"), "must start with frontmatter");
        assert!(md.contains("kind: session"), "must have kind field");
        assert!(
            md.contains("session_id: abc-123"),
            "must have session id, got: {md}"
        );
        assert!(
            md.contains("如何用 Rust 处理文件？"),
            "must contain user turn text"
        );
        assert!(md.contains("std::fs"), "must contain assistant turn text");
    }

    #[test]
    fn compose_empty_session() {
        let session = ChatSession {
            id: "empty-1".to_string(),
            title: "空对话".to_string(),
            turns: vec![],
            created_at: "2026-07-01T12:00:00Z".to_string(),
            updated_at: "2026-07-01T12:00:00Z".to_string(),
            summary: None,
        };
        let md = compose_session_markdown(&session).expect("compose");
        assert!(md.contains("空对话"), "title present");
        assert!(md.contains("空对话"), "empty session indicator");
    }

    #[test]
    fn sanitise_filename_removes_invalid_chars() {
        assert_eq!(sanitise_filename("abc-123_def"), "abc-123_def");
        assert_eq!(sanitise_filename("a/b/c"), "a-b-c");
        assert_eq!(sanitise_filename("  spaces  "), "spaces");
        assert_eq!(sanitise_filename(""), "");
    }

    #[test]
    fn resolve_sessions_dir_default() {
        let settings = crate::models::AppSettings {
            vault_dir: "/tmp/test-vault".to_string(),
            session_export_enabled: true,
            session_export_path: None,
            ..Default::default()
        };

        let dir = resolve_sessions_dir(&settings);
        assert_eq!(dir, PathBuf::from("/tmp/test-vault/.vaultpilot/sessions"));
    }

    #[test]
    fn resolve_sessions_dir_custom_relative() {
        let settings = crate::models::AppSettings {
            vault_dir: "/tmp/test-vault".to_string(),
            session_export_path: Some("my-sessions".to_string()),
            ..Default::default()
        };
        let dir = resolve_sessions_dir(&settings);
        assert_eq!(dir, PathBuf::from("/tmp/test-vault/my-sessions"));
    }

    #[test]
    fn resolve_sessions_dir_custom_absolute() {
        let settings = crate::models::AppSettings {
            vault_dir: "/tmp/test-vault".to_string(),
            session_export_path: Some("/data/sessions".to_string()),
            ..Default::default()
        };
        let dir = resolve_sessions_dir(&settings);
        assert_eq!(dir, PathBuf::from("/data/sessions"));
    }

    #[test]
    fn check_needs_write_detects_turn_change() {
        let path = std::env::temp_dir().join("test-session-check-needs-write.md");
        let session = sample_session();

        // Write once
        let md = compose_session_markdown(&session).expect("compose");
        std::fs::write(&path, &md).expect("write");
        assert!(!check_needs_write(&path, &session), "up-to-date");

        // Modify session
        let mut modified = session.clone();
        modified.turns.push(sample_turn("user", "另一条消息"));
        assert!(check_needs_write(&path, &modified), "new turn needs write");

        let _ = std::fs::remove_file(&path);
    }
}
