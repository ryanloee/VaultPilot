//! Vault Markdown Mirror (#2859)
//!
//! Projects the SQLite-backed vault onto disk as plain Markdown files so the
//! notes can be grepped, git-versioned, and opened in any editor.
//!
//! Each note is written to `<mirror_dir>/<note_id>.md` containing the original
//! frontmatter + body plus a stable `<!-- vaultpilot-note-id: <id> -->` anchor
//! comment used for traceability.
//!
//! Persistence is *real* (#2884): a `.vp-mirror-state.json` file records the
//! `updated_at` of every mirrored note. On every run — including the very first
//! run of `vp mirror --watch` after a restart — that file is **read back** and
//! used to perform an incremental sync, so unchanged notes are skipped instead
//! of re-exported. The original implementation wrote the state file but never
//! read it, making it dead write-only code.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use anyhow::Context as _;

use crate::models::NoteMeta;
use crate::storage::{
    export_note_markdown_with_context, list_all_notes_with_context, StorageContext,
};

/// Name of the per-directory state file that records mirrored-note metadata.
pub const MIRROR_STATE_FILE: &str = ".vp-mirror-state.json";

/// Prefix of the stable inline anchor comment embedded in every mirror file.
pub const NOTE_ID_ANCHOR_PREFIX: &str = "<!-- vaultpilot-note-id: ";

/// Suffix that closes the inline anchor comment.
pub const NOTE_ID_ANCHOR_SUFFIX: &str = " -->";

/// One entry in the mirror state map, keyed by note id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MirrorStateEntry {
    /// Last-known `updated_at` of the note, used for incremental diffing.
    pub updated_at: String,
    /// Last-known title (kept for diagnostics / mirror listing).
    pub title: String,
    /// Relative path of the mirror file inside the mirror directory.
    pub path: String,
}

/// Persisted mirror state. Serialized to [`MIRROR_STATE_FILE`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MirrorState {
    /// Schema version, for forward-compatible migrations.
    #[serde(default)]
    pub version: u32,
    /// Map of note id -> recorded mirror entry.
    #[serde(default)]
    pub entries: HashMap<String, MirrorStateEntry>,
}

impl MirrorState {
    /// Create an empty state with the current schema version.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Read the mirror state file. Returns `None` when the file is missing or
/// corrupt — the caller should fall back to a fresh (full) sync.
///
/// This is the read side that was missing in the original implementation
/// (#2884): the state file is now genuinely consumed.
pub fn read_mirror_state(path: &Path) -> Option<MirrorState> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Persist the mirror state. Safe to call after every sync cycle.
pub fn write_mirror_state(path: &Path, state: &MirrorState) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// The set of changes needed to bring the mirror in line with the vault.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MirrorDiff {
    /// Note ids that have no mirror file yet.
    pub to_create: Vec<String>,
    /// Note ids whose `updated_at` diverged from the recorded state.
    pub to_update: Vec<String>,
    /// Note ids present in state but no longer in the vault (deleted notes).
    pub to_delete: Vec<String>,
}

/// Compute the incremental diff between the current vault notes and the
/// persisted mirror state. Pure function — no I/O.
pub fn compute_mirror_diff(current: &[NoteMeta], state: &MirrorState) -> MirrorDiff {
    let mut diff = MirrorDiff::default();

    for meta in current {
        match state.entries.get(&meta.id) {
            None => diff.to_create.push(meta.id.clone()),
            Some(entry) => {
                if entry.updated_at != meta.updated_at {
                    diff.to_update.push(meta.id.clone());
                }
            }
        }
    }

    let current_ids: std::collections::HashSet<&String> = current.iter().map(|m| &m.id).collect();
    for id in state.entries.keys() {
        if !current_ids.contains(id) {
            diff.to_delete.push(id.clone());
        }
    }

    diff
}

/// Compose the final Markdown content for a mirror file: the original
/// frontmatter + body, followed by the stable note-id anchor comment.
pub fn compose_mirror_markdown(body_with_frontmatter: &str, note_id: &str) -> String {
    format!(
        "{}\n\n{}{}{}\n",
        body_with_frontmatter.trim_end(),
        NOTE_ID_ANCHOR_PREFIX,
        note_id,
        NOTE_ID_ANCHOR_SUFFIX
    )
}

/// Extract the note id from a mirror file's anchor comment.
/// Used for disk-scan fallback / diagnostics. Returns `None` when absent.
pub fn extract_note_id_anchor(content: &str) -> Option<String> {
    let start = content.find(NOTE_ID_ANCHOR_PREFIX)?;
    let rest = &content[start + NOTE_ID_ANCHOR_PREFIX.len()..];
    let end = rest.find(NOTE_ID_ANCHOR_SUFFIX)?;
    let id = rest[..end].trim().to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// Absolute path of a note's mirror file inside `mirror_dir`.
pub fn mirror_file_path(mirror_dir: &Path, note_id: &str) -> PathBuf {
    mirror_dir.join(format!("{note_id}.md"))
}

/// Result of a single sync cycle.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MirrorResult {
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
}

/// Perform one incremental sync of the vault into `mirror_dir`.
///
/// Reads the persisted state (if any), diffs it against the current vault,
/// writes/updates/deletes mirror files accordingly, then rewrites the state
/// file. This is the function that gives `vp mirror --watch` a real fast-start:
/// on restart the saved state is read back and unchanged notes are skipped.
pub fn mirror_sync_with_context(
    context: &StorageContext,
    mirror_dir: &Path,
) -> anyhow::Result<MirrorResult> {
    std::fs::create_dir_all(mirror_dir)
        .with_context(|| format!("failed to create mirror directory {}", mirror_dir.display()))?;

    let state_path = mirror_dir.join(MIRROR_STATE_FILE);
    let mut state = read_mirror_state(&state_path).unwrap_or_default();

    let current = list_all_notes_with_context(context)?;
    let diff = compute_mirror_diff(&current, &state);

    let mut result = MirrorResult::default();

    for id in diff.to_create.iter().chain(diff.to_update.iter()) {
        let (markdown, _filename) = export_note_markdown_with_context(context, id)?;
        let path = mirror_file_path(mirror_dir, id);
        std::fs::write(&path, compose_mirror_markdown(&markdown, id))?;
        if let Some(meta) = current.iter().find(|m| &m.id == id) {
            state.entries.insert(
                id.clone(),
                MirrorStateEntry {
                    updated_at: meta.updated_at.clone(),
                    title: meta.title.clone(),
                    path: path.display().to_string(),
                },
            );
        }
        if diff.to_create.contains(id) {
            result.created += 1;
        } else {
            result.updated += 1;
        }
    }

    for id in &diff.to_delete {
        let path = mirror_file_path(mirror_dir, id);
        let _ = std::fs::remove_file(&path);
        state.entries.remove(id);
        result.deleted += 1;
    }

    result.unchanged = current.len() - diff.to_create.len() - diff.to_update.len();

    write_mirror_state(&state_path, &state)?;
    Ok(result)
}

/// Run `vp mirror --watch`: sync once, then re-sync every `interval_secs`
/// until the process is terminated. Each cycle is an incremental sync that
/// reads back the persisted state.
pub fn mirror_watch_with_context(
    context: &StorageContext,
    mirror_dir: &Path,
    interval_secs: u64,
) -> anyhow::Result<()> {
    loop {
        let result = mirror_sync_with_context(context, mirror_dir)?;
        // Emit a JSON status line per cycle so logs are machine-readable.
        println!(
            "{}",
            serde_json::json!({
                "event": "mirror_sync",
                "created": result.created,
                "updated": result.updated,
                "deleted": result.deleted,
                "unchanged": result.unchanged,
            })
        );
        std::thread::sleep(std::time::Duration::from_secs(interval_secs.max(1)));
    }
}
