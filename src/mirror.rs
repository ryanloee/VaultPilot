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
//! Two-way sync (#2924): after export, mirror files are scanned for external
//! changes (detected via content hash). When a mirror file has been edited
//! outside VaultPilot, the changes flow back into the vault:
//!   - If the vault note hasn't changed since the last sync, the external edit
//!     replaces the vault note directly.
//!   - If both the mirror and vault have changed (concurrent edit), both
//!     versions are preserved and a conflict marker is appended.

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
    /// SHA-256 hash of the mirror file content as last written by VaultPilot.
    /// Used to detect external edits (#2924). `None` for entries created
    /// before this field was added (backward-compatible).
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

/// Compute the SHA-256 hex digest of a string. Used to detect external
/// edits to mirror files (#2924).
pub fn hash_content(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Strip the anchor comment line(s) from mirror file content, returning
/// the original frontmatter + body as written by VaultPilot. This is needed
/// when an externally-edited mirror file is read back for vault import: the
/// anchor must be removed before the content is parsed as a note.
pub fn strip_anchor_from_content(content: &str) -> String {
    if let Some(pos) = content.find(NOTE_ID_ANCHOR_PREFIX) {
        let anchor_start = content[..pos].rfind('\n').map(|n| n + 1).unwrap_or(0);
        let after_prefix = &content[pos + NOTE_ID_ANCHOR_PREFIX.len()..];
        if let Some(suffix_pos) = after_prefix.find(NOTE_ID_ANCHOR_SUFFIX) {
            let anchor_end =
                pos + NOTE_ID_ANCHOR_PREFIX.len() + suffix_pos + NOTE_ID_ANCHOR_SUFFIX.len();
            let end = if anchor_end < content.len() && content.as_bytes()[anchor_end] == b'\n' {
                anchor_end + 1
            } else {
                anchor_end
            };
            let mut result = String::with_capacity(content.len());
            result.push_str(&content[..anchor_start]);
            if end < content.len() {
                result.push_str(&content[end..]);
            }
            return result.trim_end().to_string();
        }
    }
    content.trim_end().to_string()
}

/// Result of a single sync cycle.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MirrorResult {
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
    /// Number of mirror files whose external edits were flowed back into
    /// the vault (#2924).
    pub backflow: usize,
}

/// Detect mirror files that have been edited externally since the last
/// VaultPilot sync. Compares the current on-disk content hash against the
/// stored `content_hash` in the mirror state entry.
///
/// Returns a list of note_ids whose mirror hash differs from the recorded
/// value. Pure function — no I/O.
pub fn detect_external_changes(
    disk_hashes: &HashMap<String, String>,
    state: &MirrorState,
) -> Vec<String> {
    let mut changed = Vec::new();
    for (id, hash) in disk_hashes {
        if let Some(entry) = state.entries.get(id) {
            match &entry.content_hash {
                Some(stored) if stored == hash => {} // unchanged
                _ => changed.push(id.clone()),
            }
        }
    }
    changed
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
        } else {
            // #2935: an external editor may have deleted or corrupted the anchor
            // comment. Such a file would otherwise be silently skipped and never
            // reach `orphan_mirror_files`, leaving it as a permanent orphan on
            // disk. VaultPilot names mirror files `<note_id>.md` (#2859), so fall
            // back to the filename stem as the id — mirroring Logseq's
            // filename-based attribution — so the file can still be reconciled.
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if !stem.is_empty() {
                map.insert(stem.to_string(), path);
            }
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

/// Perform one incremental sync of the vault into `mirror_dir`.
///
/// Reads the persisted state (if any), diffs it against the current vault,
/// writes/updates/deletes mirror files accordingly, then rewrites the state
/// file. This is the function that gives `vp mirror --watch` a real fast-start:
/// on restart the saved state is read back and unchanged notes are skipped.
///
/// Two-way sync (#2924): after the export phase, scans mirror files for
/// external edits and flows them back into the vault.
pub fn mirror_sync_with_context(
    context: &StorageContext,
    mirror_dir: &Path,
) -> anyhow::Result<MirrorResult> {
    std::fs::create_dir_all(mirror_dir)
        .with_context(|| format!("failed to create mirror directory {}", mirror_dir.display()))?;

    let state_path = mirror_dir.join(MIRROR_STATE_FILE);
    let mut state = read_mirror_state(&state_path).unwrap_or_default();

    let current = list_all_notes_with_context(context)?;

    let disk_files = disk_scan_mirror_files(mirror_dir)?;
    let current_ids: std::collections::HashSet<&String> = current.iter().map(|m| &m.id).collect();

    let mut result = MirrorResult::default();

    // Phase 1: remove orphan mirror files.
    for path in orphan_mirror_files(&disk_files, &current_ids, &state) {
        let _ = std::fs::remove_file(path);
        result.deleted += 1;
    }

    let diff = compute_mirror_diff(&current, &state);

    for id in diff.to_create.iter().chain(diff.to_update.iter()) {
        let (markdown, _filename) = export_note_markdown_with_context(context, id)?;
        let composed = compose_mirror_markdown(&markdown, id);
        let file_hash = hash_content(&composed);
        let path = mirror_file_path(mirror_dir, id);
        std::fs::write(&path, &composed)?;
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
                    content_hash: Some(file_hash),
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

    // Phase 2: two-way sync — flow external edits back into the vault (#2924)
    let mut disk_hashes: HashMap<String, String> = HashMap::new();
    for (id, disk_path) in &disk_files {
        if state.entries.contains_key(id) {
            if let Ok(content) = std::fs::read_to_string(disk_path) {
                disk_hashes.insert(id.clone(), hash_content(&content));
            }
        }
    }

    let external_changes = detect_external_changes(&disk_hashes, &state);

    for note_id in &external_changes {
        let mirror_path = mirror_file_path(mirror_dir, note_id);
        let last_synced_at = state
            .entries
            .get(note_id.as_str())
            .map(|e| e.updated_at.as_str())
            .unwrap_or("");
        match flow_back_external_change(context, &mirror_path, note_id, last_synced_at) {
            Ok(true) => {
                result.backflow += 1;
                if let Some(entry) = state.entries.get_mut(note_id.as_str()) {
                    if let Ok(content) = std::fs::read_to_string(&mirror_path) {
                        entry.content_hash = Some(hash_content(&content));
                    }
                    if let Ok((markdown, _)) = export_note_markdown_with_context(context, note_id) {
                        let composed = compose_mirror_markdown(&markdown, note_id);
                        let _ = std::fs::write(&mirror_path, &composed);
                        if let Some(entry) = state.entries.get_mut(note_id.as_str()) {
                            entry.content_hash = Some(hash_content(&composed));
                        }
                    }
                }
            }
            Ok(false) => {
                if let Some(entry) = state.entries.get_mut(note_id.as_str()) {
                    if let Ok(content) = std::fs::read_to_string(&mirror_path) {
                        entry.content_hash = Some(hash_content(&content));
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "mirror backflow: failed to merge external changes for note {}: {:#}",
                    note_id, e
                );
            }
        }
    }

    write_mirror_state(&state_path, &state)?;
    Ok(result)
}

/// Flow an externally-edited mirror file back into the vault (#2924).
///
/// Reads the mirror file, strips the anchor comment, and determines whether
/// the vault note has also been modified since the last sync by comparing
/// the current vault `updated_at` against `last_synced_at` from the mirror
/// state.
///
/// Returns `Ok(true)` if the vault was updated, `Ok(false)` if the mirror
/// content is identical to the vault (no change needed).
pub fn flow_back_external_change(
    context: &StorageContext,
    mirror_path: &Path,
    note_id: &str,
    last_synced_at: &str,
) -> anyhow::Result<bool> {
    use crate::storage::load_note_with_context;
    use crate::storage::save_note_with_context;

    let mirror_raw = std::fs::read_to_string(mirror_path)
        .with_context(|| format!("failed to read mirror file {}", mirror_path.display()))?;
    let mirror_with_frontmatter = strip_anchor_from_content(&mirror_raw);

    let vault_note = match load_note_with_context(context, note_id) {
        Ok(note) => note,
        Err(_) => return Ok(false),
    };

    let vault_changed = vault_note.meta.updated_at != last_synced_at;

    // #2941: the mirror file is `compose_markdown(meta, body)` + anchor, i.e. it
    // already contains the YAML frontmatter block. `strip_anchor_from_content`
    // only removes the anchor comment, leaving `frontmatter + body`. Storing that
    // whole string as `NoteDocument.body` would nest the frontmatter inside the
    // body and duplicate it on the next re-export. Split the frontmatter off and
    // keep only the body slice — preserving the existing vault frontmatter fields.
    let (mirror_fm, mirror_body) =
        match crate::storage::notes::split_frontmatter(&mirror_with_frontmatter) {
            Ok(parts) => parts,
            Err(_) => (
                crate::storage::Frontmatter::default(),
                mirror_with_frontmatter.as_str(),
            ),
        };

    if !vault_changed {
        let mut updated_note = vault_note;
        updated_note.body = mirror_body.to_string();
        // Prefer the externally-edited title from the mirror frontmatter; fall
        // back to deriving it from the (now frontmatter-stripped) body.
        if !mirror_fm.title.is_empty() {
            updated_note.meta.title = mirror_fm.title.clone();
        } else if let Some(title) = extract_title_from_markdown(mirror_body) {
            updated_note.meta.title = title;
        }
        save_note_with_context(context, updated_note)?;
        return Ok(true);
    }

    // Conflict: both mirror and vault changed.
    let vault_markdown = {
        let mut md = String::new();
        md.push_str("---\n");
        md.push_str(&format!("title: {}\n", vault_note.meta.title));
        md.push_str(&format!("id: {}\n", vault_note.meta.id));
        md.push_str("---\n\n");
        md.push_str(&vault_note.body);
        md
    };

    let merged_body = build_conflict_merge_body(&vault_markdown, mirror_body);

    let mut merged_note = vault_note;
    merged_note.body = merged_body;
    save_note_with_context(context, merged_note)?;

    Ok(true)
}

/// Build the merged note body used when both the vault note and the mirror
/// file changed concurrently (#2924 conflict branch).
///
/// The result contains exactly two labelled sections — `## Vault version
/// (auto-saved)` with the vault content and `## Mirror version (external edit)`
/// with the externally-edited mirror content — wrapped in a conflict banner.
/// The mirror content must appear **once**, under its own heading; it must not
/// leak above the banner. Regression guard for #2945.
pub(crate) fn build_conflict_merge_body(vault_markdown: &str, mirror_body: &str) -> String {
    format!(
        "<!-- ===== CONFLICT: vault and mirror both changed ===== -->\n\n\
         ## Vault version (auto-saved)\n\n{}\n\n\
         ## Mirror version (external edit)\n\n{}\n\n\
         <!-- ===== END CONFLICT ===== -->\n",
        vault_markdown, mirror_body
    )
}

/// Extract the title from YAML frontmatter in a markdown string.
fn extract_title_from_markdown(content: &str) -> Option<String> {
    let body = content.trim();
    if !body.starts_with("---") {
        return None;
    }
    let after_first = &body[3..];
    let end = after_first.find("\n---")?;
    let frontmatter = &after_first[..end];
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("title:") {
            let title = value.trim().trim_matches('"').trim_matches('\'');
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
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
        println!(
            "{}",
            serde_json::json!({
                "event": "mirror_sync",
                "created": result.created,
                "updated": result.updated,
                "deleted": result.deleted,
                "unchanged": result.unchanged,
                "backflow": result.backflow,
            })
        );
        std::thread::sleep(std::time::Duration::from_secs(interval_secs.max(1)));
    }
}

/// Result of a `vp mirror import` operation.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MirrorImportResult {
    /// Number of new notes created in the vault.
    pub imported: usize,
    /// Number of existing vault notes updated from mirror files.
    pub updated: usize,
    /// Number of mirror files skipped (unchanged or anchor missing).
    pub skipped: usize,
}

/// Import mirror `.md` files from `mirror_dir` into the vault (#3605).
///
/// For each `.md` file in the directory:
/// 1. Reads the file and extracts the `<!-- vaultpilot-note-id: <id> -->` anchor.
/// 2. If an anchor exists and the note already exists in the vault, updates the
///    vault note with the mirror content (assuming `--force` or if the mirror is
///    newer). This is the "mirror → vault" import direction.
/// 3. If no anchor exists, or the referenced note does not exist, the file is
///    treated as a new note and imported into the vault.
/// 4. When `force` is `false`, files whose body is identical to their vault
///    counterpart are skipped (counted in `skipped`).
///
/// The `force` flag controls whether to always overwrite vault content. When
/// `true`, every existing note is updated unconditionally. When `false`
/// (default), notes whose body hasn't changed are skipped.
pub fn mirror_import_with_context(
    context: &StorageContext,
    mirror_dir: &Path,
    force: bool,
) -> anyhow::Result<MirrorImportResult> {
    use crate::models::NoteDocument;
    use crate::models::NoteMeta;
    use crate::storage::{load_note_with_context, save_note_with_context};

    let mut result = MirrorImportResult::default();

    let entries = match std::fs::read_dir(mirror_dir) {
        Ok(e) => e,
        Err(_) => return Ok(result),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == MIRROR_STATE_FILE || !name.ends_with(".md") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let vault_note_id = extract_note_id_anchor(&content);

        let body_without_anchor = strip_anchor_from_content(&content);
        let (frontmatter, body) =
            match crate::storage::notes::split_frontmatter(&body_without_anchor) {
                Ok((fm, b)) => (Some(fm), b.to_string()),
                Err(_) => (None, body_without_anchor),
            };

        let title = frontmatter
            .as_ref()
            .and_then(|fm| {
                if fm.title.is_empty() {
                    None
                } else {
                    Some(fm.title.clone())
                }
            })
            .or_else(|| {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if stem.is_empty() {
                    None
                } else {
                    Some(stem.to_string())
                }
            });

        if let Some(ref note_id) = vault_note_id {
            match load_note_with_context(context, note_id) {
                Ok(existing) => {
                    // When force is false and content is identical, skip.
                    // The storage layer prepends a "## 摘要\n\n{summary}\n\n" section,
                    // so we strip that from existing.body before comparing.
                    if !force {
                        let existing_core =
                            if let Some(rest) = existing.body.trim().strip_prefix("## 摘要\n\n") {
                                if let Some(idx) = rest.find("\n\n") {
                                    rest[idx + 2..].trim()
                                } else {
                                    existing.body.trim()
                                }
                            } else {
                                existing.body.trim()
                            };
                        if existing_core == body.trim() {
                            result.skipped += 1;
                            continue;
                        }
                    }
                    let mut updated = existing;
                    updated.body = body.to_string();
                    if let Some(ref t) = title {
                        updated.meta.title = t.clone();
                    }
                    save_note_with_context(context, updated)?;
                    result.updated += 1;
                }
                Err(_) => {
                    // Note referenced by anchor not found in vault — treat as new.
                    let note = NoteDocument {
                        meta: NoteMeta {
                            id: note_id.clone(),
                            title: title.clone().unwrap_or_else(|| note_id.clone()),
                            ..Default::default()
                        },
                        body: body.to_string(),
                        ..Default::default()
                    };
                    save_note_with_context(context, note)?;
                    result.imported += 1;
                }
            }
        } else {
            // No anchor — create a new vault note.
            let new_id = uuid::Uuid::new_v4().to_string();
            let note = NoteDocument {
                meta: NoteMeta {
                    id: new_id,
                    title: title.unwrap_or_else(|| name.trim_end_matches(".md").to_string()),
                    ..Default::default()
                },
                body: body.to_string(),
                ..Default::default()
            };
            save_note_with_context(context, note)?;
            result.imported += 1;
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::build_conflict_merge_body;

    #[test]
    fn conflict_merge_body_has_each_section_once() {
        // Regression test for #2945: the mirror (external) content must appear
        // exactly once, under its own heading, and must NOT leak above the
        // conflict banner.
        let vault = "# Vault content\nvault-only";
        let mirror = "# Mirror content\nmirror-only";
        let merged = build_conflict_merge_body(vault, mirror);

        // Banner appears exactly once, at the very start.
        let banner = "<!-- ===== CONFLICT: vault and mirror both changed ===== -->";
        assert!(
            merged.starts_with(banner),
            "merged body must start with banner"
        );
        assert_eq!(merged.matches(banner).count(), 1);

        // Each labelled section appears once.
        assert_eq!(merged.matches("## Vault version (auto-saved)").count(), 1);
        assert_eq!(
            merged.matches("## Mirror version (external edit)").count(),
            1
        );

        // Mirror content appears exactly once (no triple duplication).
        assert_eq!(merged.matches("mirror-only").count(), 1);
        // Vault content appears exactly once.
        assert_eq!(merged.matches("vault-only").count(), 1);

        // Mirror content is present *after* the banner, not before it.
        let banner_pos = merged.find(banner).unwrap();
        let mirror_pos = merged.find("mirror-only").unwrap();
        assert!(mirror_pos > banner_pos);
    }
}
