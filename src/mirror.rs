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
//!
//! **Bidirectional sync (#2924):** The mirror state now also records a content
//! hash (SHA-256) of each mirror file at write time. On the next sync cycle,
//! hashes are re-computed and compared against the stored values to detect
//! external edits. Mirror files that were externally edited but whose vault
//! copy was unchanged are read back and saved to the vault (reverse sync).
//! Notes modified on both sides are flagged as conflicts for manual resolution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use anyhow::Context as _;

use crate::models::NoteMeta;
use crate::storage::{
    export_note_markdown_with_context, list_all_notes_with_context, load_note_with_context,
    save_note_with_context, StorageContext,
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
    /// SHA-256 hash of the mirror file content at the time it was last written
    /// by `mirror_sync_with_context`. Used to detect external edits (#2924).
    /// `None` for legacy state entries migrated from older versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
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

/// Scan `mirror_dir` for existing mirror `.md` files (excluding the state
/// file) and return a map from each file's embedded note id — extracted via
/// [`extract_note_id_anchor`] — to its on-disk path.
///
/// This is the disk-scan fallback that makes [`extract_note_id_anchor`] part
/// of the real reconcile path instead of dead test-only code. It is used by
/// [`mirror_sync_with_context`] to detect and remove orphan mirror files left
/// behind when a note has been deleted from the vault but its `.md` lingers on
/// disk (#2889).
pub fn disk_scan_mirror_files(mirror_dir: &Path) -> anyhow::Result<HashMap<String, PathBuf>> {
    let mut map = HashMap::new();
    let entries = std::fs::read_dir(mirror_dir)
        .with_context(|| format!("failed to read mirror directory {}", mirror_dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Skip the state file and anything that is not a mirror `.md` file.
        if name == MIRROR_STATE_FILE || !name.ends_with(".md") {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        if let Some(id) = extract_note_id_anchor(&content) {
            map.insert(id, path);
        }
    }
    Ok(map)
}

/// Identify orphan mirror files: present on disk but whose note no longer
/// exists in the vault (`current_ids`) and is not recorded in the persisted
/// `state`. These files should be removed during reconcile (#2889).
///
/// Pure function — no I/O — so it is trivially testable and keeps the
/// reconcile policy decoupled from filesystem access.
pub fn orphan_mirror_files<'a>(
    disk_files: &'a HashMap<String, PathBuf>,
    current_ids: &std::collections::HashSet<&String>,
    state: &MirrorState,
) -> Vec<&'a PathBuf> {
    disk_files
        .iter()
        .filter(|(id, _)| !current_ids.contains(id) && !state.entries.contains_key(*id))
        .map(|(_, path)| path)
        .collect()
}

/// Result of a single sync cycle.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MirrorResult {
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
    /// Number of externally-edited mirror files reverse-synced back to vault (#2924).
    pub reverse_synced: usize,
    /// Number of conflicts where both mirror and vault were modified (#2924).
    pub conflicts: usize,
}

/// Compute the SHA-256 hex digest of a file's content.
/// Returns `None` if the file cannot be read.
pub fn compute_file_hash(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// Strip the vaultpilot-note-id anchor comment from mirror file content.
/// Returns the original frontmatter + body without the anchor.
/// Used when reverse-syncing external edits back to the vault (#2924).
pub fn strip_anchor(content: &str) -> &str {
    if let Some(pos) = content.find(NOTE_ID_ANCHOR_PREFIX) {
        content[..pos].trim_end()
    } else {
        content.trim_end()
    }
}

/// Detect externally-edited mirror files: notes where the vault hasn't changed
/// (not in `to_create` or `to_update`) but the mirror file's current hash
/// differs from the last known hash stored in the mirror state (#2924).
///
/// Pure function — no I/O — but the caller must have already computed current
/// file hashes via `compute_file_hash`.
pub fn detect_external_edits(
    current_ids: &std::collections::HashSet<&String>,
    diff: &MirrorDiff,
    state: &MirrorState,
    current_hashes: &HashMap<String, String>,
) -> Vec<String> {
    let mut externally_edited = Vec::new();
    for id in current_ids {
        let id_str: &String = id; // dereference from &&String
                                  // Skip notes that the vault already wants to create or update —
                                  // those will be handled by the forward sync path.
        if diff.to_create.contains(id_str) || diff.to_update.contains(id_str) {
            continue;
        }
        if let Some(entry) = state.entries.get(id_str) {
            if let Some(stored_hash) = &entry.content_hash {
                if let Some(current_hash) = current_hashes.get(id_str) {
                    if stored_hash != current_hash {
                        // Mirror file hash differs from stored — externally edited
                        externally_edited.push(id_str.clone());
                    }
                }
            }
        }
    }
    externally_edited
}

/// Detect conflicts: notes that are in `to_update` (vault changed) AND have
/// an external edit (mirror hash changed). Both sides modified independently
/// — requires manual conflict resolution (#2924).
pub fn detect_conflicts<'a>(
    diff: &'a MirrorDiff,
    external_edits: &std::collections::HashSet<&String>,
) -> Vec<&'a String> {
    diff.to_update
        .iter()
        .filter(|id| external_edits.contains(*id))
        .collect()
}

/// Perform one incremental sync of the vault into `mirror_dir`, with
/// bidirectional reverse-sync for externally-edited mirror files (#2924).
///
/// Reads the persisted state (if any), diffs it against the current vault,
/// writes/updates/deletes mirror files accordingly, then rewrites the state
/// file.
///
/// **Reverse sync (#2924):** Before overwriting mirror files, this function
/// scans existing mirror files for external edits (content hash differs from
/// the last known hash stored in state). Externally-edited notes that were NOT
/// also modified in the vault are read back and saved to the vault (reverse
/// sync). Notes where BOTH sides were modified are counted as conflicts and
/// left for manual resolution (the vault's version wins the mirror file, but
/// the conflict is reported).
pub fn mirror_sync_with_context(
    context: &StorageContext,
    mirror_dir: &Path,
) -> anyhow::Result<MirrorResult> {
    std::fs::create_dir_all(mirror_dir)
        .with_context(|| format!("failed to create mirror directory {}", mirror_dir.display()))?;

    let state_path = mirror_dir.join(MIRROR_STATE_FILE);
    let mut state = read_mirror_state(&state_path).unwrap_or_default();

    let current = list_all_notes_with_context(context)?;

    // Disk-scan fallback (#2889)
    let disk_files = disk_scan_mirror_files(mirror_dir)?;
    let current_ids: std::collections::HashSet<&String> = current.iter().map(|m| &m.id).collect();

    let mut result = MirrorResult::default();

    // Phase 0: remove orphan mirror files whose note no longer exists.
    for path in orphan_mirror_files(&disk_files, &current_ids, &state) {
        let _ = std::fs::remove_file(path);
        result.deleted += 1;
    }

    let diff = compute_mirror_diff(&current, &state);

    // Phase 1: compute current hashes of all existing mirror files
    // (to detect external edits and conflicts — #2924).
    let mut current_hashes: HashMap<String, String> = HashMap::new();
    for id in &current_ids {
        let path = mirror_file_path(mirror_dir, id);
        if path.exists() {
            if let Some(hash) = compute_file_hash(&path) {
                current_hashes.insert((*id).clone(), hash);
            }
        }
    }

    // Phase 2: detect external edits and conflicts (#2924).
    let external_edited = detect_external_edits(&current_ids, &diff, &state, &current_hashes);
    let external_edited_set: std::collections::HashSet<&String> = external_edited.iter().collect();
    let conflicts = detect_conflicts(&diff, &external_edited_set);
    result.conflicts = conflicts.len();

    // Phase 3: reverse-sync externally-edited notes (not in conflict) back to vault.
    for id in &external_edited {
        // Skip conflicted notes — vault version wins mirror, conflict reported separately
        if conflicts.contains(&id) {
            continue;
        }
        let path = mirror_file_path(mirror_dir, id);
        if let Ok(mirror_content) = std::fs::read_to_string(&path) {
            let stripped = strip_anchor(&mirror_content);
            // Load the existing note to preserve its metadata (id, created_at, tags, etc.)
            if let Ok(mut note) = load_note_with_context(context, id) {
                // Only update the body if it actually changed
                if note.body != stripped {
                    note.body = stripped.to_string();
                    // Save back — this will update `updated_at` automatically
                    save_note_with_context(context, note)?;
                    result.reverse_synced += 1;
                }
            }
        }
    }

    // Phase 4: forward sync — create/update mirror files from vault changes.
    for id in diff.to_create.iter().chain(diff.to_update.iter()) {
        let (markdown, _filename) = export_note_markdown_with_context(context, id)?;
        let mirror_content = compose_mirror_markdown(&markdown, id);
        let path = mirror_file_path(mirror_dir, id);
        std::fs::write(&path, &mirror_content)?;

        // Compute hash of the written content for future external edit detection
        let content_hash = compute_file_hash(&path);

        if let Some(meta) = current.iter().find(|m| &m.id == id) {
            let rel = path
                .strip_prefix(mirror_dir)
                .unwrap_or(&path)
                .display()
                .to_string();
            state.entries.insert(
                id.clone(),
                MirrorStateEntry {
                    updated_at: meta.updated_at.clone(),
                    title: meta.title.clone(),
                    path: rel,
                    content_hash,
                },
            );
        }
        if diff.to_create.contains(id) {
            result.created += 1;
        } else {
            // Conflicts are not counted as normal updates — they were already
            // logged separately and the vault version overwrites the mirror.
            if !conflicts.contains(&id) {
                result.updated += 1;
            }
        }
    }

    for id in &diff.to_delete {
        let path = mirror_file_path(mirror_dir, id);
        let _ = std::fs::remove_file(&path);
        state.entries.remove(id);
        result.deleted += 1;
    }

    result.unchanged =
        current.len() - diff.to_create.len() - diff.to_update.len() - external_edited.len();

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
                "reverse_synced": result.reverse_synced,
                "conflicts": result.conflicts,
            })
        );
        std::thread::sleep(std::time::Duration::from_secs(interval_secs.max(1)));
    }
}
