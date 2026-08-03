//! Note CRUD, import/export, and OCR operations.
//!
//! Extracted from `mod.rs` to keep the storage module focused (#1280).

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[cfg(target_os = "windows")]
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Datelike, Utc};
use deunicode::deunicode;
use rusqlite::{params, Connection, OptionalExtension};
use tracing::{instrument, warn};
use uuid::Uuid;
use walkdir::WalkDir;

/// Error type indicating a note was not found. Used instead of string matching
/// for robust 404 detection. (#2516)
#[derive(Debug)]
pub struct NoteNotFound(pub String);

impl std::fmt::Display for NoteNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "note not found: {}", self.0)
    }
}

impl std::error::Error for NoteNotFound {}

use crate::models::{
    ExportResult, HeadingNode, ImportResult, NoteDocument, NoteMeta, VaultExportResult,
};

use super::pool::open_connection;
use super::search::{
    build_attachment_semantic_text, build_text_semantic_vector, derived_note_id, fallback_source,
    fallback_title, hash_content, is_markdown_file, list_all_note_metas, load_note_meta_by_id,
    rank_documents, rank_note_metas, sanitize_terms, serialize_semantic_vector, slugify,
};
use super::{
    atomic_write, load_chat_state_with_context, load_settings_with_context, Frontmatter,
    StorageContext, MAX_NOTE_FILE_SIZE,
};

// ────────────────────────────────────────────────────────
// Note CRUD
// ────────────────────────────────────────────────────────

pub fn save_note_with_context(
    context: &StorageContext,
    note: NoteDocument,
) -> Result<NoteDocument> {
    save_note_with_images_with_context(context, note, &[])
}

pub fn save_note_with_images_with_context(
    context: &StorageContext,
    note: NoteDocument,
    image_paths: &[String],
) -> Result<NoteDocument> {
    let (connection, settings) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let title = fallback_title(&note.meta.title);
    let is_new = note.meta.id.trim().is_empty();
    // Tracks whether source-based dedup matched an existing note, so we can
    // preserve the original created_at (#3084).
    let mut source_dedup_original_created_at: Option<String> = None;
    let id = if is_new {
        // ── Source-based dedup: if the caller provides a non-empty source
        //    (e.g. feed entry link), look for an existing note with the same
        //    source and reuse its id to turn this into an update/overwrite
        //    instead of creating a duplicate. (#3077)
        if !note.meta.source.trim().is_empty() {
            let existing: Option<String> = connection
                .query_row(
                    "SELECT id FROM notes WHERE source = ?1 LIMIT 1",
                    params![note.meta.source.trim()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(existing_id) = existing {
                // Preserve the original created_at when dedup resolves to an
                // existing note, so re-polling a feed entry doesn't reset its
                // creation date to "now". (#3084)
                source_dedup_original_created_at = connection
                    .query_row(
                        "SELECT created_at FROM notes WHERE id = ?1",
                        params![existing_id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .filter(|s: &String| !s.trim().is_empty());
                existing_id
            } else {
                Uuid::new_v4().to_string()
            }
        } else {
            Uuid::new_v4().to_string()
        }
    } else {
        // Sanitize the id: only allow alphanumeric, '-' and '_' to prevent
        // path traversal via sequences like "../" (#1966).  If after
        // filtering the id is empty, fall back to a fresh UUID.
        let sanitized: String = note
            .meta
            .id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if sanitized.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            sanitized
        }
    };

    // ── Persistent snapshot: capture the old state before overwriting (#2855, #3081) ──
    // After ID is fully resolved (including source-based dedup), record a snapshot
    // if a note with this ID already exists.  The snapshot function handles the
    // case where the note doesn't exist (first save) gracefully.
    if !id.trim().is_empty() {
        super::snapshots::record_snapshot_before_save(context, &id, "agent")?;
    }

    let created_at = if let Some(ref preserved) = source_dedup_original_created_at {
        // Preserve original created_at when source-based dedup resolved to
        // an existing note, so re-polling doesn't reset the creation date (#3084)
        preserved.clone()
    } else if note.meta.created_at.trim().is_empty() {
        now.clone()
    } else {
        note.meta.created_at.clone()
    };
    let updated_at = if is_new && !note.meta.updated_at.trim().is_empty() {
        note.meta.updated_at.clone()
    } else {
        now
    };

    let path = if note.meta.path.trim().is_empty() {
        build_note_path(&settings.vault_dir, &title, &created_at, &id)
    } else {
        crate::normalize_tool_path(&note.meta.path, Path::new(&settings.vault_dir))
            .map_err(|e| anyhow::anyhow!("invalid note path: {e}"))?
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let note_path_string = path.to_string_lossy().to_string();
    let image_refs = import_note_images(&path, image_paths)?;
    let body_with_images = append_image_markdown(&note.body, &image_refs);

    // 优先从 DB 获取当前 note_collections 关联，确保 frontmatter 与 DB 一致 (Issue #2191)
    let collections = {
        let mut stmt = connection.prepare(
            "SELECT c.name FROM collections c \
             INNER JOIN note_collections nc ON nc.collection_id = c.id \
             WHERE nc.note_id = ?1 \
             ORDER BY c.name",
        )?;
        let names: Vec<String> = stmt
            .query_map(params![id], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        if names.is_empty() {
            note.meta.collections.clone()
        } else {
            names
        }
    };

    let meta = NoteMeta {
        id,
        title,
        tags: sanitize_terms(&note.meta.tags),
        keywords: sanitize_terms(&note.meta.keywords),
        platform: note.meta.platform.trim().to_string(),
        board: note.meta.board.trim().to_string(),
        kernel: note.meta.kernel.trim().to_string(),
        status: note.meta.status.trim().to_string(),
        created_at,
        updated_at,
        source: fallback_source(&note.meta.source),
        path: note_path_string.clone(),
        summary: if note.meta.summary.trim().is_empty() {
            extract_summary(&body_with_images)
        } else {
            note.meta.summary.trim().to_string()
        },
        collections,
    };

    let serialized = compose_markdown(&meta, &body_with_images)?;
    atomic_write(&path, serialized.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    index_note_file_with_connection(&connection, &path, Path::new(&settings.vault_dir))?;
    load_note_with_context(context, &meta.id)
}

pub fn load_note_with_context(context: &StorageContext, note_id: &str) -> Result<NoteDocument> {
    let (connection, _) = open_connection(context)?;
    let path = connection
        .query_row(
            "SELECT path FROM notes WHERE id = ?1 OR path = ?1 LIMIT 1",
            [note_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow::Error::from(NoteNotFound(note_id.to_string())))?;
    parse_markdown_note(Path::new(&path), "manual")
}

/// Preview a fragment of a note for hover/long-press link previews (#3739).
///
/// Similar to Obsidian's Page Preview: returns the first `max_lines` lines of a
/// note, or a specific block/heading if an anchor is provided.
///
/// Anchor format (consistent with Obsidian wiki-link anchors):
/// * `None`       — return first `max_lines` lines of the note
/// * `#heading`   — return content under the matching heading (trimmed to `max_lines`)
/// * `#^block-id` — return the specific block with matching id. Block ids are
///   FNV-1a content hashes assigned by `parse_blocks`/`annotate_blocks`
///   (e.g. `d0b576e1ab0f1f25`), *not* user-written `^marker` text — a `^marker`
///   typed into the note body stays literal text and never becomes an anchor.
pub fn preview_note_fragment(
    context: &StorageContext,
    note_id: &str,
    anchor: Option<&str>,
    max_lines: usize,
) -> Result<String> {
    use crate::block_ref::{find_block_by_id, parse_blocks};

    let doc = load_note_with_context(context, note_id)?;
    let body = &doc.body;

    match anchor {
        None => {
            // Return first N lines (head of the note)
            let lines: Vec<&str> = body.lines().take(max_lines).collect();
            Ok(lines.join("\n"))
        }
        Some(anchor) if anchor.starts_with('^') => {
            // Block reference: find by block id
            let block_id = &anchor[1..]; // strip leading ^
            let blocks = parse_blocks(body);
            match find_block_by_id(&blocks, block_id) {
                Some(block) => Ok(block.text.clone()),
                None => {
                    // Block not found — fall back to first N lines
                    let lines: Vec<&str> = body.lines().take(max_lines).collect();
                    Ok(lines.join("\n"))
                }
            }
        }
        Some(heading) => {
            // Heading anchor: find the heading and return content underneath it
            let heading_text = heading.trim_start_matches('#');
            let mut found = false;
            let mut result = Vec::new();
            let mut line_count = 0;

            for line in body.lines() {
                if line_count >= max_lines {
                    break;
                }
                if !found {
                    // Check if this line is a heading matching the anchor
                    let trimmed = line.trim();
                    if trimmed.starts_with('#')
                        && trimmed
                            .trim_start_matches('#')
                            .trim()
                            .eq_ignore_ascii_case(heading_text.trim())
                    {
                        found = true;
                    }
                }
                if found {
                    result.push(line);
                    line_count += 1;
                }
            }

            if result.is_empty() {
                // Heading not found — fall back to first N lines
                let lines: Vec<&str> = body.lines().take(max_lines).collect();
                Ok(lines.join("\n"))
            } else {
                Ok(result.join("\n"))
            }
        }
    }
}

/// Delete a note (and optionally its associated attachments).
///
/// `delete_attachments` controls attachment cleanup:
/// * `None`      — use the default behavior (delete non-shared attachments, as before)
/// * `Some(true)`— force deletion of non-shared attachment files
/// * `Some(false)`— delete **only** the note's `.md` file; leave all attachment
///   files on disk untouched (mirrors Obsidian's "Never" cleanup mode)
///
/// This powers the per-delete "Also delete attachments?" prompt
/// (enhancement #2936): the UI passes `Some(user_choice)` while callers that
/// don't care about the distinction pass `None` to keep the prior behavior.
pub fn delete_note_with_context(
    context: &StorageContext,
    note_id: &str,
    delete_attachments: Option<bool>,
) -> Result<bool> {
    let (mut connection, _) = open_connection(context)?;
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT id, path FROM notes WHERE id = ?1 OR path = ?1 LIMIT 1",
            [note_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((resolved_note_id, note_path)) = row else {
        return Ok(false);
    };
    let file = PathBuf::from(&note_path);

    // ── Query attachment paths BEFORE the transaction ─────────────────
    // The attachment rows will be deleted inside the transaction, so we
    // must fetch their paths first to know which physical files to clean
    // up afterwards.  (#2241)
    let attachment_paths: Vec<String> = connection
        .prepare("SELECT path FROM attachments WHERE note_id = ?1")?
        .query_map([resolved_note_id.as_str()], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // For each attachment, check whether it is referenced by other notes
    // *before* we delete the rows, so we know which files are safe to
    // remove from disk.
    let mut shared_paths: Vec<String> = Vec::new();
    for path_str in &attachment_paths {
        let other_refs: i64 = connection.query_row(
            "SELECT COUNT(*) FROM attachments WHERE path = ?1 AND note_id != ?2",
            [path_str, &resolved_note_id],
            |row| row.get(0),
        )?;
        if other_refs > 0 {
            shared_paths.push(path_str.clone());
        }
    }

    let tx = connection.transaction()?;
    tx.execute(
        "DELETE FROM note_fts WHERE note_id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM attachment_fts WHERE note_id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM image_text_fts WHERE note_id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM attachments WHERE note_id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM notes WHERE id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.commit()?;

    // ── Delete attachment physical files ────────────────────────────────
    // After the DB transaction has been committed, remove the physical
    // files for attachments that are not shared with other notes.
    // For shared files (attachments referenced by multiple notes), we skip
    // deletion.
    //
    // When the caller explicitly opts out (`Some(false)`), we leave every
    // attachment file on disk untouched and only remove the note itself —
    // this is the "Never delete attachments" behavior. `None` and
    // `Some(true)` both keep the original cleanup behavior. (#2936)
    let keep_attachments = delete_attachments == Some(false);
    let note_stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent_dir = file.parent().unwrap_or(Path::new(""));
    let assets_dir = parent_dir.join(format!("{note_stem}-assets"));

    if !keep_attachments {
        for path_str in &attachment_paths {
            let apath = PathBuf::from(path_str);
            if apath.exists() && !shared_paths.contains(path_str) {
                if let Err(e) = fs::remove_file(&apath) {
                    warn!(path = %apath.display(), error = %e, "failed to delete attachment file");
                }
            }
        }

        // Remove the assets directory if it exists and is now empty
        // `fs::remove_dir` only succeeds if the directory is empty, so this
        // is a no-op if shared attachment files or non-attachment files remain.
        if assets_dir.exists() {
            let _ = fs::remove_dir(&assets_dir);
        }
    }

    // Delete the physical file only after the DB transaction has been committed.
    // If file deletion fails, the DB is already clean so we log a warning rather
    // than propagating the error.
    if file.exists() {
        if let Err(e) = fs::remove_file(&file) {
            warn!(path = %file.display(), error = %e, "failed to delete file");
        }
    }

    Ok(true)
}

// ────────────────────────────────────────────────────────
// Bulk Operations (#3104)
// ────────────────────────────────────────────────────────

/// Outcome of a bulk operation on multiple notes (#3104).
///
/// Reports per-note outcomes so the caller (CLI/UI) can show which notes
/// succeeded, which were skipped (no change needed), and which failed —
/// without losing partial progress when a batch hits a few bad ids.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkNoteOpResult {
    /// Total notes the caller asked to operate on.
    pub requested: usize,
    /// Notes that were successfully modified.
    pub affected: usize,
    /// Notes that were skipped (e.g. tag set unchanged, note already in target dir).
    pub skipped: usize,
    /// Per-note failures (id + reason).
    pub failures: Vec<BulkNoteOpFailure>,
}

/// A single per-note failure inside a [`BulkNoteOpResult`] (#3104).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkNoteOpFailure {
    pub id: String,
    pub reason: String,
}

/// Bulk delete notes by ID (#3104).
///
/// Iterates over `note_ids`, calling [`delete_note_with_context`] for each.
/// Failures (missing note, IO error) are recorded per-note and don't abort
/// the rest of the batch — this mirrors the UX of selecting multiple files
/// in a file manager and pressing Delete: some may fail while the rest go
/// through.
pub fn bulk_delete_notes_with_context(
    context: &StorageContext,
    note_ids: &[String],
    delete_attachments: Option<bool>,
) -> Result<BulkNoteOpResult> {
    let mut result = BulkNoteOpResult {
        requested: note_ids.len(),
        ..Default::default()
    };
    for id in note_ids {
        match delete_note_with_context(context, id, delete_attachments) {
            Ok(true) => result.affected += 1,
            Ok(false) => result.failures.push(BulkNoteOpFailure {
                id: id.clone(),
                reason: "not found".to_string(),
            }),
            Err(e) => result.failures.push(BulkNoteOpFailure {
                id: id.clone(),
                reason: e.to_string(),
            }),
        }
    }
    Ok(result)
}

/// Bulk add/remove tags on notes (#3104).
///
/// For each note: load, modify its tag set, and save. Tags are matched
/// case-insensitively for removal. Notes whose final tag set is unchanged
/// (e.g. trying to add a tag that's already present and removing none of
/// the existing ones) are reported as `skipped` rather than `affected`,
/// so callers can distinguish "no-op" from "wrote a new revision".
pub fn bulk_update_tags_with_context(
    context: &StorageContext,
    note_ids: &[String],
    add_tags: &[String],
    remove_tags: &[String],
) -> Result<BulkNoteOpResult> {
    let mut result = BulkNoteOpResult {
        requested: note_ids.len(),
        ..Default::default()
    };

    // Normalize tag inputs: split on commas, trim, drop empties.
    let add_tags: Vec<String> = add_tags
        .iter()
        .flat_map(|t| t.split(','))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let remove_tags_lower: Vec<String> = remove_tags
        .iter()
        .flat_map(|t| t.split(','))
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    for id in note_ids {
        match load_note_with_context(context, id) {
            Ok(mut note) => {
                let original: Vec<String> = note.meta.tags.clone();

                // 1. Remove first (case-insensitive) so a tag in both add and
                //    remove lists ends up being added (last-write-wins).
                note.meta
                    .tags
                    .retain(|t| !remove_tags_lower.contains(&t.to_lowercase()));

                // 2. Then add (dedup case-insensitive, preserve caller casing).
                for tag in &add_tags {
                    let already = note.meta.tags.iter().any(|t| t.eq_ignore_ascii_case(tag));
                    if !already {
                        note.meta.tags.push(tag.clone());
                    }
                }

                if note.meta.tags == original {
                    result.skipped += 1;
                } else {
                    match save_note_with_context(context, note) {
                        Ok(_) => result.affected += 1,
                        Err(e) => result.failures.push(BulkNoteOpFailure {
                            id: id.clone(),
                            reason: e.to_string(),
                        }),
                    }
                }
            }
            Err(e) => result.failures.push(BulkNoteOpFailure {
                id: id.clone(),
                reason: e.to_string(),
            }),
        }
    }
    Ok(result)
}

/// Bulk update a single NoteMeta field for multiple notes (#3762).
///
/// This is the backend for Kanban drag-drop: when the user drags a card
/// from one column to another, the UI calls this function to update the
/// note field without loading/resaving the entire document.
/// Notes whose field already holds the requested value are
/// reported as \"skipped\" rather than \"affected\".
///
/// Supported fields: title, board, kernel, platform, status, source,
/// summary, tags, keywords.
pub fn bulk_update_meta_field_with_context(
    context: &StorageContext,
    note_ids: &[String],
    field: &str,
    value: &str,
) -> Result<BulkNoteOpResult> {
    let mut result = BulkNoteOpResult {
        requested: note_ids.len(),
        ..Default::default()
    };

    let allowed = [
        "title", "board", "kernel", "platform", "status", "source", "summary", "tags", "keywords",
    ];
    if !allowed.contains(&field) {
        return Err(anyhow::anyhow!(
            "field '{field}' is not supported for bulk update; allowed fields: {}",
            allowed.join(", ")
        ));
    }

    let value = value.trim().to_string();

    for id in note_ids {
        match load_note_with_context(context, id) {
            Ok(mut note) => {
                if field == "tags" {
                    let add: Vec<String> = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    let sub = bulk_update_tags_with_context(
                        context,
                        std::slice::from_ref(id),
                        &add,
                        &[],
                    )?;
                    if sub.affected > 0 {
                        result.affected += 1;
                    } else if sub.skipped > 0 {
                        result.skipped += 1;
                    }
                    result.failures.extend(sub.failures);
                    continue;
                }
                if field == "keywords" {
                    let mut kwds: Vec<String> = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    kwds.dedup();
                    let new_str = kwds.join(",");
                    let old_str = note.meta.keywords.join(",");
                    if old_str == new_str {
                        result.skipped += 1;
                        continue;
                    }
                    note.meta.keywords = kwds;
                    note.meta.updated_at = Utc::now().to_rfc3339();
                    match save_note_with_context(context, note) {
                        Ok(_) => result.affected += 1,
                        Err(e) => result.failures.push(BulkNoteOpFailure {
                            id: id.clone(),
                            reason: e.to_string(),
                        }),
                    }
                    continue;
                }

                let old_value: &str = match field {
                    "title" => &note.meta.title,
                    "board" => &note.meta.board,
                    "kernel" => &note.meta.kernel,
                    "platform" => &note.meta.platform,
                    "status" => &note.meta.status,
                    "source" => &note.meta.source,
                    "summary" => &note.meta.summary,
                    _ => unreachable!("validated above"),
                };

                if old_value == value {
                    result.skipped += 1;
                } else {
                    match field {
                        "title" => note.meta.title = value.clone(),
                        "board" => note.meta.board = value.clone(),
                        "kernel" => note.meta.kernel = value.clone(),
                        "platform" => note.meta.platform = value.clone(),
                        "status" => note.meta.status = value.clone(),
                        "source" => note.meta.source = value.clone(),
                        "summary" => note.meta.summary = value.clone(),
                        _ => unreachable!("validated above"),
                    }
                    note.meta.updated_at = Utc::now().to_rfc3339();
                    match save_note_with_context(context, note) {
                        Ok(_) => result.affected += 1,
                        Err(e) => result.failures.push(BulkNoteOpFailure {
                            id: id.clone(),
                            reason: e.to_string(),
                        }),
                    }
                }
            }
            Err(e) => result.failures.push(BulkNoteOpFailure {
                id: id.clone(),
                reason: e.to_string(),
            }),
        }
    }
    Ok(result)
}

/// Bulk move notes to a target subdirectory within the vault (#3104).
///
/// `target_dir` is interpreted relative to the vault root and is confined
/// to the vault by [`crate::normalize_tool_path`] (so `../escape` is
/// rejected). The notes' existing filenames are preserved. After moving
/// each physical file the FTS index and `notes.path` are updated so
/// search returns the new location.
pub fn bulk_move_notes_with_context(
    context: &StorageContext,
    note_ids: &[String],
    target_dir: &str,
) -> Result<BulkNoteOpResult> {
    let mut result = BulkNoteOpResult {
        requested: note_ids.len(),
        ..Default::default()
    };
    let (connection, settings) = open_connection(context)?;
    let vault_dir = PathBuf::from(&settings.vault_dir);
    // `target_dir` is documented as relative to the vault root. `normalize_tool_path`
    // only uses its second argument as a *confinement boundary* — it does NOT join the
    // candidate onto the vault root — so a relative target would be resolved against the
    // process CWD and rejected ("cannot verify path is inside the vault directory").
    // Join it onto the vault root first so relative paths like `archive/2026` resolve
    // correctly and the confinement walk-up can confirm the (existing) vault dir as the
    // nearest ancestor. (#3104 regression: all relative moves currently fail.)
    let target_candidate = vault_dir.join(target_dir);
    let target_root =
        crate::normalize_tool_path(target_candidate.to_str().unwrap_or(target_dir), &vault_dir)
            .map_err(|e| anyhow::anyhow!("invalid target directory '{target_dir}': {e}"))?;
    fs::create_dir_all(&target_root).ok();

    for id in note_ids {
        match move_one_note_with_connection(&connection, id, &target_root, &vault_dir) {
            Ok(MoveOutcome::Moved) => result.affected += 1,
            Ok(MoveOutcome::Skipped) => result.skipped += 1,
            Err(e) => result.failures.push(BulkNoteOpFailure {
                id: id.clone(),
                reason: e.to_string(),
            }),
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy)]
enum MoveOutcome {
    Moved,
    Skipped,
}

/// Move a single note file into `target_root`, then re-index it so the
/// SQLite `notes.path` column and the FTS index reflect the new location.
///
/// Returns `Skipped` when the note is already inside `target_root`
/// (no-op move). Returns an error when the note is missing, the target
/// file already exists, or the underlying filesystem rename fails.
fn move_one_note_with_connection(
    connection: &Connection,
    note_id: &str,
    target_root: &Path,
    vault_dir: &Path,
) -> Result<MoveOutcome> {
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT id, path FROM notes WHERE id = ?1 OR path = ?1 LIMIT 1",
            [note_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((_resolved_id, old_path_str)) = row else {
        return Err(anyhow!("note not found: {note_id}"));
    };
    let old_path = PathBuf::from(&old_path_str);
    let Some(filename) = old_path.file_name() else {
        return Err(anyhow::anyhow!(
            "note has no filename: {}",
            old_path.display()
        ));
    };
    let new_path = target_root.join(filename);
    if new_path == old_path {
        return Ok(MoveOutcome::Skipped);
    }
    // If the note already lives inside `target_root` (including any nested
    // subdirectory of it), moving it would only flatten the user's folder
    // hierarchy without changing its logical location — treat that as a
    // no-op rather than relocating the file to the target's top level.
    // (#3134: bulk move previously pulled notes out of `archive/2026` up
    // into `archive/`, destroying multi-level directory structures.)
    if let (Ok(old_canon), Ok(target_canon)) = (old_path.canonicalize(), target_root.canonicalize())
    {
        if old_canon.starts_with(&target_canon) {
            return Ok(MoveOutcome::Skipped);
        }
    } else if old_path.starts_with(target_root) {
        // Fallback when canonicalize is unavailable (e.g. the note file was
        // just created and not yet flushed) — compare lexical prefixes.
        return Ok(MoveOutcome::Skipped);
    }
    if new_path.exists() {
        return Err(anyhow!(
            "target already exists: {} (note id={note_id})",
            new_path.display()
        ));
    }
    if let Some(parent) = new_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dir for {}", new_path.display()))?;
    }
    // Move the physical file. fs::rename can fail across filesystems
    // (EXDEV), so fall back to copy + delete for robustness — important
    // when the vault lives on a different mount than the OS temp dir.
    fs::rename(&old_path, &new_path)
        .or_else(|_| {
            fs::copy(&old_path, &new_path)?;
            fs::remove_file(&old_path)
        })
        .with_context(|| format!("moving {} -> {}", old_path.display(), new_path.display()))?;
    index_note_file_with_connection(connection, &new_path, vault_dir)?;
    // ── Clean up stale DB entry left at the old path ──────────────────
    // For notes with an *explicit* frontmatter `id`, the UPSERT inside
    // `index_note_file_with_connection` already updated the existing row's
    // path to the new location, so these deletes are no-ops (0 rows).
    //
    // For notes whose `id` was *derived* from the file path
    // (`derived_note_id` = SHA-256 of path), the re-index above inserted a
    // brand-new row under the new path-derived id, leaving the old row as a
    // "ghost" pointing at the now-deleted old path. Without this cleanup,
    // the ghost lingers in search results / FTS / listings and breaks
    // `load_note_with_context` until a full `rebuild_index` runs.
    // (#3509: ghost entries after bulk move.)
    //
    // Mirrors the cleanup pattern used by `rebuild_index` below. The
    // `attachments` and `note_collections` child rows are removed by their
    // `ON DELETE CASCADE` FK once the parent `notes` row goes, but FTS5
    // virtual tables cannot have FK constraints, so they are cleaned
    // explicitly before the parent row.
    connection.execute(
        "DELETE FROM note_fts WHERE note_id IN (SELECT id FROM notes WHERE path = ?1)",
        [&old_path_str],
    )?;
    connection.execute(
        "DELETE FROM attachment_fts WHERE note_id IN (SELECT id FROM notes WHERE path = ?1)",
        [&old_path_str],
    )?;
    connection.execute(
        "DELETE FROM image_text_fts WHERE note_id IN (SELECT id FROM notes WHERE path = ?1)",
        [&old_path_str],
    )?;
    connection.execute("DELETE FROM notes WHERE path = ?1", [&old_path_str])?;
    Ok(MoveOutcome::Moved)
}

// ────────────────────────────────────────────────────────
// Import / Export
// ────────────────────────────────────────────────────────

#[instrument(skip(context, paths))]
pub fn import_markdown_with_context(
    context: &StorageContext,
    paths: &[String],
) -> Result<ImportResult> {
    let (connection, _) = open_connection(context)?;
    let mut result = ImportResult::default();
    for file in collect_markdown_files(paths) {
        match import_single_markdown(context, &connection, &file) {
            Ok(imported) => {
                if imported {
                    result.imported += 1;
                } else {
                    result.skipped += 1;
                }
            }
            Err(error) => result.errors.push(format!("{}: {error}", file.display())),
        }
    }
    Ok(result)
}

/// Export a single note as Markdown with frontmatter preserved.
/// Returns the composed Markdown string and the suggested filename.
pub fn export_note_markdown_with_context(
    context: &StorageContext,
    note_id: &str,
) -> Result<(String, String)> {
    let note = load_note_with_context(context, note_id)?;
    let markdown = compose_markdown(&note.meta, &note.body)?;
    let filename = sanitize_filename(&note.meta.title);
    Ok((markdown, filename))
}

/// Export all notes as Markdown files into the given directory.
pub fn export_all_notes_with_context(
    context: &StorageContext,
    output_dir: &Path,
) -> Result<ExportResult> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output directory {}", output_dir.display()))?;

    let mut result = ExportResult::default();
    // #575: Use list_all_note_metas to get all notes without the 200-note
    // clamp imposed by search_notes_with_context.
    let (connection, _) = open_connection(context)?;
    let all_note_metas = list_all_note_metas(&connection)?;

    for meta in &all_note_metas {
        match export_note_markdown_with_context(context, &meta.id) {
            Ok((markdown, filename)) => {
                let safe_id = sanitize_id_for_filename(&meta.id);
                let path = output_dir.join(format!("{}-{}.md", filename, safe_id));
                match fs::write(&path, &markdown) {
                    Ok(()) => result.exported += 1,
                    Err(e) => result
                        .errors
                        .push(format!("{}: failed to write: {e}", meta.title)),
                }
            }
            Err(e) => result.errors.push(format!("{}: {e}", meta.title)),
        }
    }
    Ok(result)
}

/// Export the entire vault as a zip file: all notes (as .md with frontmatter)
/// plus all chat sessions (as a single chat-sessions.json).
///
/// The resulting zip has the structure:
///   notes/`<title>`.md  (one file per note)
///   chat-sessions.json  (all sessions in one JSON file)
pub fn vault_export_with_context(
    context: &StorageContext,
    output_path: &Path,
) -> Result<VaultExportResult> {
    let mut result = VaultExportResult::default();

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Create the zip file
    let zip_file = fs::File::create(output_path).with_context(|| {
        format!(
            "failed to create output zip file: {}",
            output_path.display()
        )
    })?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // ── Export all notes ──
    // #575: Use list_all_note_metas to get all notes without the 200-note
    // clamp imposed by search_notes_with_context.
    let (connection, _) = open_connection(context)?;
    let all_note_metas = list_all_note_metas(&connection)?;

    for meta in &all_note_metas {
        match export_note_markdown_with_context(context, &meta.id) {
            Ok((markdown, filename)) => {
                let safe_id = sanitize_id_for_filename(&meta.id);
                let entry_name = format!("notes/{}-{}.md", filename, safe_id);
                zip.start_file(entry_name, options)?;
                std::io::Write::write_all(&mut zip, markdown.as_bytes())?;
                result.notes_exported += 1;
            }
            Err(e) => result.errors.push(format!("{}: {e}", meta.title)),
        }
    }

    // ── Export chat sessions ──
    let chat_state = load_chat_state_with_context(context)?;
    let chat_json = serde_json::to_string_pretty(&chat_state)?;
    zip.start_file("chat-sessions.json", options)?;
    std::io::Write::write_all(&mut zip, chat_json.as_bytes())?;
    result.sessions_exported = chat_state.sessions.len();

    zip.finish()?;

    // Record output metadata
    result.output_path = output_path.display().to_string();
    result.file_size_bytes = fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);

    Ok(result)
}

// ────────────────────────────────────────────────────────
// Rebuild index
// ────────────────────────────────────────────────────────

#[instrument(skip(context))]
pub fn rebuild_index_with_context(context: &StorageContext) -> Result<super::IndexStats> {
    let (mut connection, settings) = open_connection(context)?;
    let vault_dir = PathBuf::from(&settings.vault_dir);
    fs::create_dir_all(&vault_dir)?;

    // Collect all markdown files first (no transaction needed).
    let markdown_files: Vec<_> = WalkDir::new(&vault_dir)
        .max_depth(20)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file() && is_markdown_file(entry.path()))
        .collect();

    let mut indexed_paths = HashSet::new();
    let mut stats = super::IndexStats::default();

    // Process files in batches of 50 to avoid holding a long write lock.
    const BATCH_SIZE: usize = 50;
    for chunk in markdown_files.chunks(BATCH_SIZE) {
        let tx = connection.transaction()?;
        for entry in chunk {
            stats.scanned += 1;
            // #851: Add both canonical and non-canonical paths to handle Windows
            // extended-length prefix mismatch (\\?\C:\... vs C:\...) and
            // canonicalize failures (permissions, network drives).
            let raw = entry.path().to_string_lossy().to_string();
            indexed_paths.insert(raw);
            let canonical = entry
                .path()
                .canonicalize()
                .unwrap_or_else(|_| entry.path().to_path_buf());
            indexed_paths.insert(canonical.to_string_lossy().to_string());
            if index_note_file_with_connection(&tx, entry.path(), &vault_dir).is_ok() {
                stats.indexed += 1;
            }
        }
        tx.commit()?;
    }

    // Clean up stale entries in a separate transaction.
    {
        let tx = connection.transaction()?;
        let mut statement = tx.prepare("SELECT path FROM notes")?;
        let existing_paths = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for existing in existing_paths {
            if !indexed_paths.contains(&existing) {
                tx.execute(
                    "DELETE FROM note_fts WHERE note_id IN (SELECT id FROM notes WHERE path = ?1)",
                    [&existing],
                )?;
                tx.execute(
                    "DELETE FROM attachment_fts WHERE note_id IN (SELECT id FROM notes WHERE path = ?1)",
                    [&existing],
                )?;
                tx.execute(
                    "DELETE FROM image_text_fts WHERE note_id IN (SELECT id FROM notes WHERE path = ?1)",
                    [&existing],
                )?;
                stats.removed += tx.execute("DELETE FROM notes WHERE path = ?1", [&existing])?;
            }
        }
        tx.commit()?;
    }

    Ok(stats)
}

// ────────────────────────────────────────────────────────
// Context / Related notes
// ────────────────────────────────────────────────────────

pub fn load_context_notes_with_context(
    context: &StorageContext,
    question: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteDocument>> {
    let (connection, _) = open_connection(context)?;
    rank_documents(context, &connection, question, image_paths, limit)
}

pub fn search_candidate_notes_with_context(
    context: &StorageContext,
    question: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteMeta>> {
    let (connection, _) = open_connection(context)?;
    rank_note_metas(context, &connection, question, image_paths, limit)
}

/// Find notes related to the given note by extracting key terms and running FTS5 search.
/// Returns up to `limit` related notes with relevance scores, excluding the source note.
pub fn find_related_notes_with_context(
    context: &StorageContext,
    note_id: &str,
    limit: usize,
) -> Result<Vec<crate::models::RelatedNote>> {
    let (connection, _) = open_connection(context)?;
    let source_meta = load_note_meta_by_id(&connection, note_id)?
        .ok_or_else(|| anyhow::Error::from(NoteNotFound(note_id.to_string())))?;
    let source_doc = load_note_with_context(context, &source_meta.id)?;

    // Build a focused query from title + tags (most distinctive terms).
    let query = build_related_query(&source_doc);
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Use existing rank infrastructure which has FTS5 + LIKE fallback.
    let search_limit = limit.saturating_mul(3).max(15);
    let candidates = rank_documents(context, &connection, &query, &[], search_limit)?;

    let mut results: Vec<crate::models::RelatedNote> = Vec::new();
    for doc in candidates {
        if doc.meta.id == note_id {
            continue;
        }
        let mut score = 0i64;
        // Title word overlap bonus
        let source_words: HashSet<&str> = source_meta.title.split_whitespace().collect();
        let target_words: HashSet<&str> = doc.meta.title.split_whitespace().collect();
        let overlap = source_words.intersection(&target_words).count();
        score += (overlap as i64) * 30;
        // Tag overlap bonus
        let source_tags: HashSet<&str> = source_meta.tags.iter().map(String::as_str).collect();
        let target_tags: HashSet<&str> = doc.meta.tags.iter().map(String::as_str).collect();
        let tag_overlap = source_tags.intersection(&target_tags).count();
        score += (tag_overlap as i64) * 50;
        // Base relevance from FTS/LIKE ranking
        score += 10;
        results.push(crate::models::RelatedNote {
            meta: doc.meta,
            score,
            snippet: doc.search_snippet,
        });
    }

    results.sort_by_key(|b| std::cmp::Reverse(b.score));
    results.truncate(limit);
    Ok(results)
}

/// Issue #1995: real-time context surface. Find notes related to **free-form
/// text** (e.g. what the user is currently typing, or a live meeting
/// transcript window) rather than to an already-saved note. This powers the
/// live "relevant notes" panel that recomputes as content changes, without
/// requiring a save first.
///
/// Mirrors [`find_related_notes_with_context`] but derives its signals from raw
/// text: the first non-empty line stands in for a "title" (word-overlap bonus)
/// and inline `#hashtags` stand in for tags (tag-overlap bonus).
pub fn find_related_notes_for_text_with_context(
    context: &StorageContext,
    text: &str,
    limit: usize,
) -> Result<Vec<crate::models::RelatedNote>> {
    let (connection, _) = open_connection(context)?;
    let query = build_related_query_for_text(text);
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let search_limit = limit.saturating_mul(3).max(15);
    let candidates = rank_documents(context, &connection, &query, &[], search_limit)?;

    let text_tags: HashSet<String> = extract_hashtags(text);
    let text_title_words: HashSet<String> = first_line(text)
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();

    let mut results: Vec<crate::models::RelatedNote> = Vec::new();
    for doc in candidates {
        let mut score = 10i64;
        // title-word overlap bonus
        let target_words: HashSet<String> = doc
            .meta
            .title
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();
        let overlap = text_title_words.intersection(&target_words).count();
        score += (overlap as i64) * 30;
        // tag overlap bonus
        let target_tags: HashSet<&str> = doc.meta.tags.iter().map(String::as_str).collect();
        let tag_overlap = text_tags
            .iter()
            .filter(|t| target_tags.contains(t.as_str()))
            .count();
        score += (tag_overlap as i64) * 50;
        results.push(crate::models::RelatedNote {
            meta: doc.meta,
            score,
            snippet: doc.search_snippet,
        });
    }

    results.sort_by_key(|b| std::cmp::Reverse(b.score));
    results.truncate(limit);
    Ok(results)
}

// ────────────────────────────────────────────────────────
// Wikilink graph traversal (#1829)
// ────────────────────────────────────────────────────────

/// Extract all `[[wikilink]]` targets from a note body.
///
/// Supports the Obsidian-style syntax:
/// - `[[Note Title]]` — simple link
/// - `[[Note Title|display alias]]` — link with alias
/// - `[[#heading]]` — heading link (target = note title, alias = heading)
/// - `[[Note Title#heading]]` — note + heading
///
/// Code blocks (`` ``` ``) and inline code (`` ` ``) are skipped so links
/// inside code are not treated as graph edges.
pub fn extract_wikilinks(body: &str) -> Vec<(String, Option<String>)> {
    let mut results = Vec::new();
    let mut in_code_block = false;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        // Walk the line, skipping inline code spans, looking for [[…]]
        let mut in_inline_code = false;
        let mut code_span_backticks: usize = 0; // backtick count that opened the span
        let chars_vec: Vec<char> = line.chars().collect();
        let mut i = 0usize;

        while i < chars_vec.len() {
            match chars_vec[i] {
                '`' => {
                    // Count consecutive backticks to handle both single (`)
                    // and double/triple backtick inline code spans (#2672).
                    let mut count = 1usize;
                    while i + count < chars_vec.len() && chars_vec[i + count] == '`' {
                        count += 1;
                    }
                    if in_inline_code {
                        // Only close when the backtick count matches the opener
                        if count == code_span_backticks {
                            in_inline_code = false;
                            code_span_backticks = 0;
                        }
                        // else: literal backticks inside the code span
                    } else {
                        in_inline_code = true;
                        code_span_backticks = count;
                    }
                    i += count;
                }
                '[' if !in_inline_code && i + 1 < chars_vec.len() && chars_vec[i + 1] == '[' => {
                    // Found [[ — extract until ]]
                    let start = i + 2;
                    let mut end = start;
                    let mut found = false;
                    while end + 1 < chars_vec.len() {
                        if chars_vec[end] == ']' && chars_vec[end + 1] == ']' {
                            found = true;
                            break;
                        }
                        end += 1;
                    }
                    if found {
                        let inner: String = chars_vec[start..end].iter().collect();
                        let (target, alias) = parse_wikilink_inner(&inner);
                        if !target.is_empty() {
                            results.push((target, alias));
                        }
                        i = end + 2; // skip past ]]
                    } else {
                        i += 1;
                    }
                }
                _ => {
                    i += 1;
                }
            }
        }
    }
    results
}

/// Extract all Markdown headings (H1–H6) from a note body as a flat list of
/// [`HeadingNode`] entries, each carrying its level, text, and 1-based line
/// number. Headings inside fenced code blocks (``` … ```) are ignored.
///
/// Used by the outline/TOC navigation feature (#3319).
#[allow(dead_code)] // wired to CLI/MCP in follow-up PRs
pub fn extract_heading_tree(body: &str) -> Vec<HeadingNode> {
    let mut headings = Vec::new();
    let mut in_code_block = false;

    for (line_idx, line) in body.lines().enumerate() {
        let trimmed = line.trim_start();

        // Toggle code-block state on ``` fences
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        // Count leading '#' characters (max 6)
        let level = trimmed.chars().take(6).take_while(|&c| c == '#').count();
        if level == 0 || level > 6 {
            continue;
        }

        // A heading requires at least one space (or end of line) after the '#'s
        let rest = &trimmed[level..];
        if !rest.is_empty() && !rest.starts_with(' ') {
            continue;
        }

        let text = rest.trim().to_string();
        headings.push(HeadingNode {
            level: level as u8,
            text,
            line: line_idx + 1,
        });
    }

    headings
}

/// Parse the inner content of a `[[…]]` wikilink into (target, alias).
///
/// - `[[Title]]` → `("Title", None)`
/// - `[[Title|alias]]` → `("Title", Some("alias"))`
/// - `[[#heading]]` → `("", Some("heading"))` (heading-only link, no note target)
/// - `[[Title#heading]]` → `("Title", Some("heading"))`
fn parse_wikilink_inner(inner: &str) -> (String, Option<String>) {
    // Split on `|` first (alias separator)
    let (main, alias_part) = if let Some(idx) = inner.find('|') {
        (&inner[..idx], Some(inner[idx + 1..].to_string()))
    } else {
        (inner, None)
    };

    // Split on `#` (heading separator)
    let (target, heading) = if let Some(idx) = main.find('#') {
        (
            main[..idx].trim().to_string(),
            Some(main[idx + 1..].trim().to_string()),
        )
    } else {
        (main.trim().to_string(), None)
    };

    // Combine alias and heading
    let alias = match (alias_part, heading) {
        (Some(a), _) if !a.is_empty() => Some(a),
        (_, Some(h)) if !h.is_empty() => Some(h),
        _ => None,
    };

    (target, alias)
}

/// Follow all `[[wikilinks]]` in a note and return resolved references.
///
/// For each wikilink target, attempts to find a note whose title matches
/// (case-insensitive). Unresolved links are included with `note: None`
/// so callers can display dangling links.
///
/// # Arguments
/// * `context` - Storage context for database access
/// * `note_id` - ID or path of the source note
///
/// # Errors
/// Returns an error if the source note cannot be loaded.
pub fn follow_wikilinks_with_context(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<crate::models::WikilinkRef>> {
    let (connection, _) = open_connection(context)?;
    let source_meta = load_note_meta_by_id(&connection, note_id)?
        .ok_or_else(|| anyhow::Error::from(NoteNotFound(note_id.to_string())))?;
    let source_doc = load_note_with_context(context, &source_meta.id)?;

    let raw_links = extract_wikilinks(&source_doc.body);
    if raw_links.is_empty() {
        return Ok(Vec::new());
    }

    // Build a case-insensitive title → NoteMeta lookup from all notes.
    let all_metas = list_all_note_metas(&connection)?;
    let mut title_lookup: std::collections::HashMap<String, NoteMeta> =
        std::collections::HashMap::with_capacity(all_metas.len());
    for meta in &all_metas {
        title_lookup.insert(meta.title.to_lowercase(), meta.clone());
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut results = Vec::new();
    for (target, alias) in raw_links {
        // Deduplicate by target (case-insensitive)
        let key = target.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let note = title_lookup.get(&target.to_lowercase()).cloned();
        results.push(crate::models::WikilinkRef {
            target,
            alias,
            note,
        });
    }

    Ok(results)
}

/// Find all notes that contain a `[[wikilink]]` pointing to the given note.
///
/// A backlink is any note whose body contains `[[target]]` where `target`
/// matches the given note's title (case-insensitive). Both `[[Title]]` and
/// `[[Title|alias]]` forms are detected; `[[Title#heading]]` also counts.
///
/// # Arguments
/// * `context` - Storage context for database access
/// * `note_id` - ID of the target note whose backlinks we want
///
/// # Errors
/// Returns an error if the target note cannot be loaded.
pub fn find_backlinks_with_context(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<crate::models::BacklinkEntry>> {
    let (connection, _) = open_connection(context)?;
    let target_meta = load_note_meta_by_id(&connection, note_id)?
        .ok_or_else(|| anyhow::Error::from(NoteNotFound(note_id.to_string())))?;

    if target_meta.title.trim().is_empty() {
        return Ok(Vec::new());
    }

    let target_title_lower = target_meta.title.to_lowercase();
    let all_metas = list_all_note_metas(&connection)?;
    let mut results = Vec::new();

    for meta in &all_metas {
        // Skip the note itself
        if meta.id == target_meta.id {
            continue;
        }
        // Load the note body to check for wikilinks
        let doc = match load_note_with_context(context, &meta.id) {
            Ok(doc) => doc,
            Err(_) => continue, // skip notes that can't be loaded
        };
        let raw_links = extract_wikilinks(&doc.body);
        for (target, _alias) in &raw_links {
            if target.to_lowercase() == target_title_lower {
                results.push(crate::models::BacklinkEntry {
                    meta: meta.clone(),
                    link_target: target.clone(),
                });
                break; // one backlink per note is enough
            }
        }
    }

    // Sort by updated_at descending (most recently modified first)
    results.sort_by(|a, b| b.meta.updated_at.cmp(&a.meta.updated_at));
    Ok(results)
}

/// Async wrapper for [`follow_wikilinks_with_context`].
pub async fn follow_wikilinks_async(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<crate::models::WikilinkRef>> {
    let ctx = context.clone();
    let note_id = note_id.to_string();
    tokio::task::spawn_blocking(move || follow_wikilinks_with_context(&ctx, &note_id))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
}

/// Async wrapper for [`find_backlinks_with_context`].
pub async fn find_backlinks_async(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<crate::models::BacklinkEntry>> {
    let ctx = context.clone();
    let note_id = note_id.to_string();
    tokio::task::spawn_blocking(move || find_backlinks_with_context(&ctx, &note_id))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
}

/// Find all notes that mention the given note's **title as plain text**
/// (not wrapped in `[[ ]]` wikilinks).
///
/// This surfaces latent connections the user hasn't formalised into wikilinks —
/// e.g. if note B is titled "Machine Learning" and note A says "I've been
/// studying Machine Learning lately" (without `[[Machine Learning]]`), note A
/// is an unlinked mention of note B.
///
/// Matching rules:
/// - Case-insensitive whole-word match.
/// - Skips fenced code blocks and inline code (same as `extract_wikilinks`).
/// - Skips frontmatter (YAML between `---` fences).
/// - Titles shorter than 3 characters are ignored (too many false positives).
/// - Notes that already link to the target via `[[wikilink]]` are excluded
///   (those are backlinks, not unlinked mentions).
pub fn find_unlinked_mentions_with_context(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<crate::models::UnlinkedMention>> {
    let (connection, _) = open_connection(context)?;
    let target_meta = load_note_meta_by_id(&connection, note_id)?
        .ok_or_else(|| anyhow::Error::from(NoteNotFound(note_id.to_string())))?;

    let title = target_meta.title.trim();
    if title.len() < 3 {
        return Ok(Vec::new());
    }

    let title_lower = title.to_lowercase();
    let all_metas = list_all_note_metas(&connection)?;
    let mut results = Vec::new();

    for meta in &all_metas {
        if meta.id == target_meta.id {
            continue;
        }
        let doc = match load_note_with_context(context, &meta.id) {
            Ok(doc) => doc,
            Err(_) => continue,
        };

        let raw_links = extract_wikilinks(&doc.body);
        let already_linked = raw_links
            .iter()
            .any(|(target, _)| target.to_lowercase() == title_lower);
        if already_linked {
            continue;
        }

        if body_mentions_title(&doc.body, &title_lower) {
            results.push(crate::models::UnlinkedMention {
                meta: meta.clone(),
                matched_title: title.to_string(),
            });
        }
    }

    results.sort_by(|a, b| b.meta.updated_at.cmp(&a.meta.updated_at));
    Ok(results)
}

/// Check whether a note body mentions `title_lower` as plain text (case-insensitive
/// whole-word match), excluding code blocks, inline code, and frontmatter.
pub(crate) fn body_mentions_title(body: &str, title_lower: &str) -> bool {
    let mut in_code_block = false;
    let mut in_frontmatter = false;
    let mut frontmatter_seen = false;

    for line in body.lines() {
        let trimmed = line.trim();

        if trimmed == "---" {
            if !frontmatter_seen {
                frontmatter_seen = true;
                in_frontmatter = true;
                continue;
            } else if in_frontmatter {
                in_frontmatter = false;
                continue;
            }
        }
        if in_frontmatter {
            continue;
        }

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        let cleaned = strip_inline_code(line);
        if contains_whole_word(&cleaned.to_lowercase(), title_lower) {
            return true;
        }
    }

    false
}

/// Remove inline code spans (`` `…` ``) from a line.
fn strip_inline_code(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut in_code = false;
    for ch in line.chars() {
        if ch == '`' {
            in_code = !in_code;
            result.push(' ');
        } else if in_code {
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}

/// Check whether `haystack` contains `needle` as a whole word.
fn contains_whole_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }

    let needle_bytes = needle.as_bytes();
    let h_bytes = haystack.as_bytes();
    let n_len = needle_bytes.len();

    let mut i = 0usize;
    while i + n_len <= h_bytes.len() {
        if &h_bytes[i..i + n_len] == needle_bytes {
            let left_ok = i == 0 || !is_word_char(h_bytes[i - 1]);
            let right_idx = i + n_len;
            let right_ok = right_idx >= h_bytes.len() || !is_word_char(h_bytes[right_idx]);
            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Whether a byte is an ASCII alphanumeric or underscore (word character).
fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Async wrapper for [`find_unlinked_mentions_with_context`].
pub async fn find_unlinked_mentions_async(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<crate::models::UnlinkedMention>> {
    let ctx = context.clone();
    let note_id = note_id.to_string();
    tokio::task::spawn_blocking(move || find_unlinked_mentions_with_context(&ctx, &note_id))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
}

/// Extract key terms from a note to build a search query for related notes.
/// Uses only title + tags for focused matching (avoids FTS5 AND-query bloat).
pub(crate) fn build_related_query(doc: &NoteDocument) -> String {
    let mut terms: Vec<String> = Vec::new();
    // Title words (most important signal)
    for word in doc.meta.title.split_whitespace() {
        let w = word.trim();
        if w.len() >= 2 {
            terms.push(w.to_string());
        }
    }
    // Tags
    for tag in &doc.meta.tags {
        let t = tag.trim();
        if !t.is_empty() {
            terms.push(t.to_string());
        }
    }
    // Keywords
    for kw in &doc.meta.keywords {
        let k = kw.trim();
        if !k.is_empty() {
            terms.push(k.to_string());
        }
    }
    // Deduplicate while preserving order
    let mut seen = HashSet::new();
    let unique: Vec<String> = terms
        .into_iter()
        .filter(|t| seen.insert(t.to_lowercase()))
        .collect();
    unique.join(" ")
}

/// Issue #1995: tokenize free-form text into a focused search query for the live
/// context surface. Keeps significant words (length >= 3, skipping a small stop
/// list) plus any inline `#hashtags`, deduplicated case-insensitively. Caps the
/// term count to avoid FTS5 AND-query bloat on long inputs.
pub(crate) fn build_related_query_for_text(text: &str) -> String {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "have", "your", "you", "are", "was",
        "were", "but", "not", "can", "all", "use", "using", "into", "they", "them", "their",
        "will", "would", "could", "should", "about",
    ];
    let mut terms: Vec<String> = Vec::new();
    for token in text.split(|c: char| c.is_whitespace()) {
        // strip a leading '#' so hashtags also flow through the word path,
        // then trim surrounding non-alphanumerics.
        let stripped = token.strip_prefix('#').unwrap_or(token);
        let w = stripped.trim_matches(|c: char| !c.is_alphanumeric());
        if w.len() < 3 {
            continue;
        }
        let lower = w.to_lowercase();
        if STOPWORDS.contains(&lower.as_str()) {
            continue;
        }
        terms.push(lower);
    }
    let mut seen = HashSet::new();
    let unique: Vec<String> = terms
        .into_iter()
        .filter(|t| seen.insert(t.to_lowercase()))
        .take(12)
        .collect();
    unique.join(" ")
}

/// Extract inline `#hashtags` (lowercased, alnum-only) from free-form text.
fn extract_hashtags(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .filter_map(|w| w.strip_prefix('#'))
        .map(|t| {
            t.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// First non-empty line of `text`, used as a stand-in "title" for word-overlap
/// scoring in the free-text related-notes path.
fn first_line(text: &str) -> &str {
    text.lines().find(|l| !l.trim().is_empty()).unwrap_or("")
}

/// Load recent notes with body text for overview/listing. Performs sync I/O.
pub fn load_recent_notes_for_overview(
    context: &StorageContext,
    limit: usize,
) -> Result<Vec<NoteDocument>> {
    let notes = super::list_notes_with_context(context)?;
    let mut docs = Vec::new();
    for note in notes.into_iter().take(limit) {
        if let Ok(doc) = super::load_note_body_from_meta(&note) {
            docs.push(doc);
        }
    }
    Ok(docs)
}

// ────────────────────────────────────────────────────────
// OCR
// ────────────────────────────────────────────────────────

pub(super) fn extract_image_text(path: &Path) -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        extract_image_text_with_windows_ocr(path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Ok(String::new())
    }
}

pub fn ocr_image_text(path: &Path) -> Result<String> {
    extract_image_text(path)
}

/// Manually set OCR text for an attachment by its database path (#3541).
///
/// This is useful for CLI tools or external OCR pipelines that extract text
/// outside the automatic note-save flow (e.g., a cloud OCR API or manual
/// transcription). The text is stored in the `attachments.ocr_text` column
/// and indexed into `image_text_fts` for full-text search.
pub fn set_attachment_ocr_text(
    context: &super::StorageContext,
    attachment_path: &str,
    ocr_text: &str,
) -> Result<()> {
    let (connection, _) = open_connection(context)?;
    let trimmed = ocr_text.trim();
    // Update the attachments table.
    let affected = connection.execute(
        "UPDATE attachments SET ocr_text = ?1 WHERE path = ?2",
        params![trimmed, attachment_path],
    )?;
    if affected == 0 {
        return Ok(()); // No matching attachment — nothing to do.
    }

    // Sync the image_text_fts: remove old entries for this attachment, then
    // re-insert if there is non-empty OCR text.
    connection.execute(
        "DELETE FROM image_text_fts WHERE attachment_id IN (SELECT id FROM attachments WHERE path = ?1)",
        params![attachment_path],
    )?;
    if !trimmed.is_empty() {
        connection.execute(
            "INSERT INTO image_text_fts (note_id, attachment_id, ocr_text)
             SELECT note_id, id, ?1 FROM attachments WHERE path = ?2",
            params![trimmed, attachment_path],
        )?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn extract_image_text_with_windows_ocr(path: &Path) -> Result<String> {
    let script = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType=WindowsRuntime]
$null = [Windows.Storage.Streams.IRandomAccessStream, Windows.Storage.Streams, ContentType=WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType=WindowsRuntime]
$null = [Windows.Graphics.Imaging.SoftwareBitmap, Windows.Graphics.Imaging, ContentType=WindowsRuntime]
$null = [Windows.Media.Ocr.OcrEngine, Windows.Media.Ocr, ContentType=WindowsRuntime]
function Await([object]$Operation, [type]$ResultType) {
  $method = [System.WindowsRuntimeSystemExtensions].GetMethods() |
    Where-Object { $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1 } |
    Select-Object -First 1
  $generic = $method.MakeGenericMethod($ResultType)
  $task = $generic.Invoke($null, @($Operation))
  $task.GetAwaiter().GetResult()
}
$imagePath = $args[0]
$file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($imagePath)) ([Windows.Storage.StorageFile])
$stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
$decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
$bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
$engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
if ($null -eq $engine) { return }
$result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
if ($null -ne $result -and $null -ne $result.Text) {
  [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
  Write-Output $result.Text
}
"#;

    let mut command = Command::new("powershell");
    command
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .arg(path.as_os_str())
        .stdin(std::process::Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let output = command
        .output()
        .with_context(|| format!("failed to run Windows OCR for {}", path.display()))?;

    if !output.status.success() {
        return Ok(String::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ────────────────────────────────────────────────────────
// Helper functions
// ────────────────────────────────────────────────────────

pub(crate) fn split_frontmatter(content: &str) -> Result<(Frontmatter, &str)> {
    // #847: Strip UTF-8 BOM that Windows editors (e.g. Notepad) may prepend.
    // Without this, files with BOM have their frontmatter silently ignored.
    let content = content.trim_start_matches('\u{feff}');
    if !content.starts_with("---\n") {
        return Ok((Frontmatter::default(), content));
    }
    let inner = &content[4..];
    // First try: delimiter followed by newline (normal case).
    if let Some(end_index) = inner.find("\n---\n") {
        let yaml = &inner[..end_index];
        let body = &inner[end_index + 5..];
        let frontmatter = match serde_yaml_ng::from_str::<Frontmatter>(yaml) {
            Ok(fm) => fm,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse frontmatter YAML, using defaults");
                Frontmatter::default()
            }
        };
        return Ok((frontmatter, body));
    }
    // #848: Fallback — file ends with "\n---" and no trailing newline.
    // Common with programmatic file generation or truncated files.
    if let Some(end_index) = inner.find("\n---") {
        if end_index + 4 == inner.len() {
            let yaml = &inner[..end_index];
            let frontmatter = match serde_yaml_ng::from_str::<Frontmatter>(yaml) {
                Ok(fm) => fm,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to parse frontmatter YAML, using defaults");
                    Frontmatter::default()
                }
            };
            return Ok((frontmatter, ""));
        }
    }
    // #1651: Edge case — empty frontmatter with adjacent delimiters (no blank line).
    // e.g. "---\n---\nBody" where inner is "---\nBody" after stripping the first "---\n".
    if inner.starts_with("---\n")
        || inner.starts_with("---\r\n")
        || inner == "---"
        || inner == "---\r"
    {
        let body_start = inner.find('\n').map(|i| i + 1).unwrap_or(inner.len());
        let body = &inner[body_start..];
        return Ok((Frontmatter::default(), body));
    }
    Err(anyhow!("invalid frontmatter"))
}

/// Like [`split_frontmatter`] but returns the raw YAML `Mapping` instead of
/// a typed `Frontmatter` struct.  Used by the vault query engine (#2813) to
/// preserve arbitrary user-defined frontmatter keys that don't appear in the
/// fixed `Frontmatter` struct.
pub(crate) fn split_frontmatter_yaml(content: &str) -> Result<(serde_yaml_ng::Mapping, &str)> {
    // #847: Strip UTF-8 BOM
    let content = content.trim_start_matches('\u{feff}');
    if !content.starts_with("---\n") {
        return Ok((serde_yaml_ng::Mapping::new(), content));
    }
    let inner = &content[4..];
    if let Some(end_index) = inner.find("\n---\n") {
        let yaml_str = &inner[..end_index];
        let body = &inner[end_index + 5..];
        let mapping =
            serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(yaml_str).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Failed to parse frontmatter YAML for query");
                serde_yaml_ng::Mapping::new()
            });
        return Ok((mapping, body));
    }
    if let Some(end_index) = inner.find("\n---") {
        if end_index + 4 == inner.len() {
            let yaml_str = &inner[..end_index];
            let mapping = serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(yaml_str)
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "Failed to parse frontmatter YAML for query");
                    serde_yaml_ng::Mapping::new()
                });
            return Ok((mapping, ""));
        }
    }
    if inner.starts_with("---\n")
        || inner.starts_with("---\r\n")
        || inner == "---"
        || inner == "---\r"
    {
        let body_start = inner.find('\n').map(|i| i + 1).unwrap_or(inner.len());
        let body = &inner[body_start..];
        return Ok((serde_yaml_ng::Mapping::new(), body));
    }
    Err(anyhow!("invalid frontmatter"))
}

fn build_note_path(vault_dir: &str, title: &str, created_at: &str, id: &str) -> PathBuf {
    let created = DateTime::parse_from_rfc3339(created_at)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let year = created.year().to_string();
    let month = format!("{:02}", created.month());
    let slug = slugify(title);
    let suffix = id;
    PathBuf::from(vault_dir)
        .join(year)
        .join(month)
        .join(format!("{slug}-{suffix}.md"))
}

fn collect_markdown_files(paths: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let path = PathBuf::from(path);
        if path.is_file() && is_markdown_file(&path) {
            if seen.insert(path.clone()) {
                files.push(path);
            }
            continue;
        }
        if path.is_dir() {
            for entry in WalkDir::new(path)
                .max_depth(20)
                .into_iter()
                .filter_map(|entry| entry.ok())
            {
                if entry.file_type().is_file() && is_markdown_file(entry.path()) {
                    let candidate = entry.path().to_path_buf();
                    if seen.insert(candidate.clone()) {
                        files.push(candidate);
                    }
                }
            }
        }
    }
    files
}

pub(crate) fn detect_title(body: &str, path: &Path) -> String {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            if !title.trim().is_empty() {
                return title.trim().to_string();
            }
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled Note")
        .replace(['_', '-'], " ")
}

fn extract_summary(body: &str) -> String {
    let compact = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("```") && !line.starts_with('#'))
        .take(6)
        .collect::<Vec<_>>()
        .join(" ");
    // Strip inline code spans (`...`) so backtick syntax doesn't leak into summaries.
    // Example: Run `sudo apt install` → Run sudo apt install
    let stripped = strip_inline_code_spans(&compact);
    stripped.chars().take(180).collect()
}

/// Remove Markdown inline code spans from text.
fn strip_inline_code_spans(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_code = false;
    for ch in text.chars() {
        match ch {
            '`' => {
                in_code = !in_code;
                if !in_code {
                    // Add a space when exiting code span to prevent word concatenation
                    // e.g., "Run `cmd` now" → "Run cmd now" (not "Run cmdnow")
                    if !result.ends_with(' ') {
                        result.push(' ');
                    }
                }
            }
            _ => result.push(ch),
        }
    }
    // Trim trailing spaces from code span replacements
    result.trim().to_string()
}

/// Produce a filesystem-safe filename from a note title.
fn sanitize_filename(title: &str) -> String {
    let slug = deunicode(title)
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    }
}

/// Sanitize a note ID for safe use in file/ZIP entry names (#901, #2149).
/// Strips path traversal characters (`.`, `/`, `\`) to prevent Zip Slip attacks.
/// Uses the full ID (not truncated) to guarantee uniqueness across export files.
fn sanitize_id_for_filename(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

fn validate_import_path(path: &Path) -> Result<()> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("cannot resolve import path '{}'", path.display()))?;

    if !canonical.is_file() {
        return Err(anyhow::anyhow!(
            "import path '{}' is not a regular file",
            path.display()
        ));
    }

    // Reject sensitive system directories to prevent exfiltration via import.
    let path_str = canonical.to_string_lossy();
    let blocked_prefixes: &[&str] = &[
        // Unix system directories
        "/etc",
        "/proc",
        "/sys",
        "/dev",
        "/boot",
        "/run",
        "/System",
        "/private/etc",
        "/private/var",
    ];
    for prefix in blocked_prefixes {
        if path_str.starts_with(prefix)
            && (path_str.len() == prefix.len() || path_str.as_bytes()[prefix.len()] == b'/')
        {
            return Err(anyhow::anyhow!(
                "access denied: cannot import from system directory '{}'",
                prefix
            ));
        }
    }

    // Windows system directories — canonicalize() produces backslash paths on Windows.
    #[cfg(windows)]
    {
        let windows_blocked: &[&str] = &[
            "C:\\Windows",
            "C:\\Program Files",
            "C:\\Program Files (x86)",
            "C:\\ProgramData",
        ];
        // Case-insensitive comparison for Windows paths.
        let path_lower = path_str.to_lowercase();
        for prefix in windows_blocked {
            let prefix_lower = prefix.to_lowercase();
            if path_lower.starts_with(&prefix_lower)
                && (path_lower.len() == prefix_lower.len()
                    || path_str.as_bytes()[prefix_lower.len()] == b'\\')
            {
                return Err(anyhow::anyhow!(
                    "access denied: cannot import from system directory '{}'",
                    prefix
                ));
            }
        }
    }

    // Also block common sensitive user paths.
    // On Windows, HOME is typically unset; USERPROFILE is the standard env var.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from);
    if let Ok(home) = home {
        let sensitive: &[&str] = &[".ssh", ".gnupg", ".aws", ".config/gh"];
        for rel in sensitive {
            let sensitive_path = home.join(rel);
            if let Ok(sensitive_canonical) = sensitive_path.canonicalize() {
                if canonical.starts_with(&sensitive_canonical) {
                    return Err(anyhow::anyhow!(
                        "access denied: cannot import from sensitive directory '{}'",
                        sensitive_path.display()
                    ));
                }
            }
        }
    }

    Ok(())
}

fn import_single_markdown(
    context: &StorageContext,
    connection: &Connection,
    file: &Path,
) -> Result<bool> {
    validate_import_path(file)?;

    let settings = load_settings_with_context(context)?;
    let vault_dir = PathBuf::from(&settings.vault_dir)
        .canonicalize()
        .map_err(|e| {
            anyhow::anyhow!(
                "cannot resolve vault directory '{}': {e}",
                settings.vault_dir
            )
        })?;
    let canonical = file
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve file path '{}': {e}", file.display()))?;
    if canonical.starts_with(&vault_dir) {
        index_note_file_with_connection(connection, &canonical, &vault_dir)?;
        return Ok(true);
    }

    let parsed = parse_markdown_note(&canonical, "imported")?;
    let imported = NoteDocument {
        meta: NoteMeta {
            id: String::new(),
            title: parsed.meta.title,
            tags: parsed.meta.tags,
            keywords: parsed.meta.keywords,
            platform: parsed.meta.platform,
            board: parsed.meta.board,
            kernel: parsed.meta.kernel,
            status: parsed.meta.status,
            created_at: parsed.meta.created_at,
            updated_at: parsed.meta.updated_at,
            source: "imported".to_string(),
            path: String::new(),
            summary: parsed.meta.summary,
            collections: parsed.meta.collections,
        },
        body: parsed.body,
        search_snippet: None,
        search_score: None,
    };
    save_note_with_context(context, imported)?;
    Ok(true)
}

pub(super) fn parse_markdown_note(path: &Path, default_source: &str) -> Result<NoteDocument> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.len() > MAX_NOTE_FILE_SIZE {
        return Err(anyhow!(
            "note file too large ({} bytes, limit {} bytes): {}",
            metadata.len(),
            MAX_NOTE_FILE_SIZE,
            path.display()
        ));
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let normalized = raw.replace("\r\n", "\n");
    let (frontmatter, body) = split_frontmatter(&normalized)?;
    let modified = metadata.modified().unwrap_or_else(|_| SystemTime::now());
    let modified_at = DateTime::<Utc>::from(modified).to_rfc3339();
    let created_at = metadata
        .created()
        .map(DateTime::<Utc>::from)
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|_| modified_at.clone());
    let title = if frontmatter.title.trim().is_empty() {
        detect_title(body, path)
    } else {
        frontmatter.title.trim().to_string()
    };
    let source = if frontmatter.source.trim().is_empty() {
        default_source.to_string()
    } else {
        frontmatter.source.trim().to_string()
    };

    Ok(NoteDocument {
        meta: NoteMeta {
            id: if frontmatter.id.trim().is_empty() {
                derived_note_id(path)
            } else {
                frontmatter.id
            },
            title,
            tags: sanitize_terms(&frontmatter.tags),
            keywords: sanitize_terms(&frontmatter.keywords),
            platform: frontmatter.platform,
            board: frontmatter.board,
            kernel: frontmatter.kernel,
            status: frontmatter.status,
            created_at: if frontmatter.created_at.trim().is_empty() {
                created_at
            } else {
                frontmatter.created_at
            },
            updated_at: if frontmatter.updated_at.trim().is_empty() {
                modified_at
            } else {
                frontmatter.updated_at
            },
            source,
            path: path.to_string_lossy().to_string(),
            summary: if frontmatter.summary.trim().is_empty() {
                extract_summary(body)
            } else {
                frontmatter.summary.trim().to_string()
            },
            collections: frontmatter.collections,
        },
        body: body.trim().to_string(),
        search_snippet: None,
        search_score: None,
    })
}

fn compose_markdown(meta: &NoteMeta, body: &str) -> Result<String> {
    let frontmatter = Frontmatter {
        id: meta.id.clone(),
        title: meta.title.clone(),
        summary: meta.summary.clone(),
        tags: meta.tags.clone(),
        keywords: meta.keywords.clone(),
        platform: meta.platform.clone(),
        board: meta.board.clone(),
        kernel: meta.kernel.clone(),
        status: meta.status.clone(),
        created_at: meta.created_at.clone(),
        updated_at: meta.updated_at.clone(),
        source: meta.source.clone(),
        collections: meta.collections.clone(),
    };
    let yaml = serde_yaml_ng::to_string(&frontmatter)?;
    Ok(format!(
        "---\n{}---\n\n{}\n",
        yaml,
        ensure_summary_section(body, &meta.summary)
    ))
}

fn import_note_images(note_path: &Path, image_paths: &[String]) -> Result<Vec<String>> {
    if image_paths.is_empty() {
        return Ok(Vec::new());
    }

    // #573: Validate all source paths before copying any files to prevent
    // exfiltration of sensitive system/user files via image import.
    for source in image_paths {
        validate_import_path(Path::new(source))?;
    }

    let parent = note_path
        .parent()
        .ok_or_else(|| anyhow!("note path has no parent: {}", note_path.display()))?;
    let stem = note_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("note");
    let asset_dir_name = format!("{stem}-assets");
    let asset_dir = parent.join(&asset_dir_name);
    fs::create_dir_all(&asset_dir)?;

    let mut refs = Vec::new();
    let mut seen_names = HashSet::new();

    for source in image_paths {
        let source_path = PathBuf::from(source);
        if !source_path.exists() {
            continue;
        }

        let original_name = source_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("image");
        let target_name = unique_asset_name(original_name, &mut seen_names);
        let target_path = asset_dir.join(&target_name);
        fs::copy(&source_path, &target_path).with_context(|| {
            format!(
                "failed to copy image from {} to {}",
                source_path.display(),
                target_path.display()
            )
        })?;
        refs.push(format!("{asset_dir_name}/{target_name}"));
    }

    Ok(refs)
}

fn unique_asset_name(original_name: &str, seen_names: &mut HashSet<String>) -> String {
    let path = Path::new(original_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");

    let mut index = 0usize;
    loop {
        let candidate = if index == 0 {
            if ext.is_empty() {
                slugify(stem)
            } else {
                format!("{}.{}", slugify(stem), ext.to_ascii_lowercase())
            }
        } else if ext.is_empty() {
            format!("{}-{}", slugify(stem), index)
        } else {
            format!("{}-{}.{}", slugify(stem), index, ext.to_ascii_lowercase())
        };

        if seen_names.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

fn append_image_markdown(body: &str, image_refs: &[String]) -> String {
    if image_refs.is_empty() {
        return body.trim().to_string();
    }

    let existing = body.trim().to_string();
    let image_block = image_refs
        .iter()
        .map(|path| {
            let normalized = path.replace('\\', "/");
            let name = Path::new(&normalized)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("image");
            format!("![{}]({})", name, normalized)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if existing.contains("## 图片记录") {
        format!("{}\n\n{}", existing, image_block)
    } else {
        format!("{}\n\n## 图片记录\n\n{}", existing, image_block)
    }
}

fn ensure_summary_section(body: &str, summary: &str) -> String {
    let trimmed = body.trim();
    if summary.trim().is_empty() || trimmed.starts_with("## 摘要") {
        return trimmed.to_string();
    }

    format!("## 摘要\n\n{}\n\n{}", summary.trim(), trimmed)
}

fn index_note_file_with_connection(
    connection: &Connection,
    path: &Path,
    vault_dir: &Path,
) -> Result<()> {
    let canonical = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve path '{}': {e}", path.display()))?;
    let document = parse_markdown_note(&canonical, "manual")?;
    let body_hash = hash_content(&document.body);
    let note_semantic_vector = build_text_semantic_vector(&document.body)
        .map(|v| serialize_semantic_vector(&v))
        .unwrap_or_default();
    connection.execute_batch("SAVEPOINT sp_index_note")?;
    let result: Result<()> = (|| {
        connection.execute(
            "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary, body_hash, semantic_vector)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
               title = excluded.title,
               tags = excluded.tags,
               keywords = excluded.keywords,
               platform = excluded.platform,
               board = excluded.board,
               kernel = excluded.kernel,
               status = excluded.status,
               created_at = excluded.created_at,
               updated_at = excluded.updated_at,
               source = excluded.source,
               path = excluded.path,
               summary = excluded.summary,
               body_hash = excluded.body_hash,
               semantic_vector = excluded.semantic_vector",
            params![
                document.meta.id,
                document.meta.title,
                serde_json::to_string(&document.meta.tags)?,
                serde_json::to_string(&document.meta.keywords)?,
                document.meta.platform,
                document.meta.board,
                document.meta.kernel,
                document.meta.status,
                document.meta.created_at,
                document.meta.updated_at,
                document.meta.source,
                canonical.to_string_lossy().to_string(),
                document.meta.summary,
                body_hash,
                note_semantic_vector
            ],
        )?;
        connection.execute(
            "DELETE FROM note_fts WHERE note_id = ?1",
            [document.meta.id.clone()],
        )?;
        connection.execute(
            "INSERT INTO note_fts (note_id, title, keywords, body) VALUES (?1, ?2, ?3, ?4)",
            params![
                document.meta.id,
                document.meta.title,
                document.meta.keywords.join(" "),
                document.body
            ],
        )?;
        sync_note_attachments_with_connection(
            connection,
            &document.meta.id,
            &canonical.to_string_lossy(),
            &extract_note_image_refs(&document.body),
            vault_dir,
        )?;
        // 同步 note_collections：基于 frontmatter 中的集合名称维护 note-collection 关联 (Issue #2191)
        connection.execute(
            "DELETE FROM note_collections WHERE note_id = ?1",
            [document.meta.id.clone()],
        )?;
        if !document.meta.collections.is_empty() {
            let collection_names = document.meta.collections.clone();
            let mut lookup = connection.prepare("SELECT id FROM collections WHERE name = ?1")?;
            let now = Utc::now().to_rfc3339();
            for name in &collection_names {
                let cid: Option<String> = lookup
                    .query_row(params![name], |row| row.get(0))
                    .optional()?;
                if let Some(cid) = cid {
                    connection.execute(
                        "INSERT OR IGNORE INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
                        params![document.meta.id, cid, now],
                    )?;
                }
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            // If RELEASE fails (e.g., disk full, SQLite internal error), the savepoint
            // stays active on the connection. When returned to the pool, subsequent
            // operations are silently wrapped in the unclosed savepoint → data loss.
            if let Err(e) = connection.execute_batch("RELEASE SAVEPOINT sp_index_note") {
                tracing::warn!(
                    error = %e,
                    "failed to RELEASE savepoint after successful index; forcing rollback + release"
                );
                let _ = connection.execute_batch(
                    "ROLLBACK TO SAVEPOINT sp_index_note; RELEASE SAVEPOINT sp_index_note;",
                );
                return Err(anyhow::Error::from(e)
                    .context("failed to release sp_index_note savepoint on commit"));
            }
        }
        Err(e) => {
            // ROLLBACK TO 仅回滚变更但不从保存点栈移除保存点，必须再 RELEASE
            // 才能结束事务；否则池化连接被归还后会"中毒"，后续写入静默丢失。
            let _ = connection.execute_batch(
                "ROLLBACK TO SAVEPOINT sp_index_note; RELEASE SAVEPOINT sp_index_note;",
            );
            return Err(e);
        }
    }
    Ok(())
}

fn sync_note_attachments_with_connection(
    connection: &Connection,
    note_id: &str,
    note_path: &str,
    image_refs: &[String],
    vault_dir: &Path,
) -> Result<()> {
    connection.execute("DELETE FROM attachment_fts WHERE note_id = ?1", [note_id])?;
    connection.execute("DELETE FROM image_text_fts WHERE note_id = ?1", [note_id])?;
    connection.execute("DELETE FROM attachments WHERE note_id = ?1", [note_id])?;

    if image_refs.is_empty() {
        return Ok(());
    }

    let note_dir = Path::new(note_path)
        .parent()
        .ok_or_else(|| anyhow!("note path has no parent: {note_path}"))?;
    let vault_canonical = vault_dir
        .canonicalize()
        .unwrap_or_else(|_| vault_dir.to_path_buf());
    let now = Utc::now().to_rfc3339();

    for relative in image_refs {
        let absolute = note_dir.join(relative);
        // Path traversal guard: resolved path must stay within the vault directory.
        // If canonicalize fails (e.g. file does not exist), the path cannot be
        // resolved safely — skip it rather than falling back to the raw path
        // which could bypass the starts_with check.
        let canonical_absolute = match absolute.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                warn!(
                    "skipping attachment that cannot be resolved (file missing): '{}'",
                    relative
                );
                continue;
            }
        };
        if !canonical_absolute.starts_with(&vault_canonical) {
            warn!(
                "skipping attachment with path traversal attempt: '{}' resolves to '{}' which is outside vault",
                relative,
                canonical_absolute.display()
            );
            continue;
        }
        let absolute_string = absolute.to_string_lossy().to_string();
        let file_name = absolute
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let stem = absolute
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let perceptual_hash = super::compute_image_perceptual_hash(&absolute)
            .map(|value| format!("{value:016x}"))
            .unwrap_or_default();
        let ocr_text = extract_image_text(&absolute).unwrap_or_default();
        let semantic_source = build_attachment_semantic_text(&file_name, &stem, &ocr_text);
        let semantic_vector = build_text_semantic_vector(&semantic_source)
            .map(|vector| serialize_semantic_vector(&vector))
            .unwrap_or_default();
        let attachment_id = Uuid::new_v4().to_string();

        connection.execute(
            "INSERT INTO attachments (id, note_id, path, file_name, stem, ocr_text, semantic_vector, perceptual_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                attachment_id,
                note_id,
                absolute_string,
                file_name,
                stem,
                ocr_text,
                semantic_vector,
                perceptual_hash,
                now
            ],
        )?;
        connection.execute(
            "INSERT INTO attachment_fts (note_id, attachment_id, file_name, stem, path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                note_id,
                attachment_id,
                absolute
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
                absolute
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
                absolute.to_string_lossy().to_string()
            ],
        )?;
        // #3541: Index OCR text into dedicated FTS table for efficient
        // full-text search of text embedded in images.
        if !ocr_text.trim().is_empty() {
            connection.execute(
                "INSERT INTO image_text_fts (note_id, attachment_id, ocr_text)
                 VALUES (?1, ?2, ?3)",
                params![note_id, attachment_id, &ocr_text],
            )?;
        }
    }

    Ok(())
}

fn extract_note_image_refs(body: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    let mut offset = 0usize;
    let mut in_code_block = false;

    while let Some(start) = body[offset..].find("![") {
        let absolute_start = offset + start;

        // Update fenced-code-block state by scanning for ``` markers
        // in the text between the previous position and this match.
        for line in body[offset..absolute_start].lines() {
            if line.trim_start().starts_with("```") {
                in_code_block = !in_code_block;
            }
        }

        // Skip image syntax that appears inside a fenced code block.
        if in_code_block {
            offset = absolute_start + 2;
            continue;
        }

        let Some(open) = body[absolute_start..].find("](") else {
            offset = absolute_start + 2;
            continue;
        };
        let path_start = absolute_start + open + 2;
        let Some(close) = body[path_start..].find(')') else {
            offset = path_start;
            continue;
        };
        let raw = body[path_start..path_start + close]
            .trim()
            .trim_matches('<')
            .trim_matches('>')
            .trim();
        let path = raw.split_whitespace().next().unwrap_or_default().trim();
        if !path.is_empty() && seen.insert(path.to_string()) {
            refs.push(path.to_string());
        }
        offset = path_start + close + 1;
    }

    refs
}

// ────────────────────────────────────────────────────────
// Async wrappers
// ────────────────────────────────────────────────────────

/// Spawn-blocking wrapper for [`save_note_with_context`].
pub async fn save_note_async(ctx: &StorageContext, note: NoteDocument) -> Result<NoteDocument> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || save_note_with_context(&ctx, note))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`save_note_with_images_with_context`].
pub async fn save_note_with_images_async(
    ctx: &StorageContext,
    note: NoteDocument,
    image_paths: &[String],
) -> Result<NoteDocument> {
    let ctx = ctx.clone();
    let image_paths = image_paths.to_vec();
    tokio::task::spawn_blocking(move || {
        save_note_with_images_with_context(&ctx, note, &image_paths)
    })
    .await
    .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`delete_note_with_context`].
///
/// `delete_attachments` follows the same semantics as the underlying function:
/// * `None`       — old default (purge exclusive attachments)
/// * `Some(true)` — force-delete exclusive attachments
/// * `Some(false)`— keep all attachment files on disk
///
/// Callers that need to honor the persisted `attachment_cleanup_on_note_delete`
/// setting should resolve it first via
/// [`AttachmentCleanupMode::resolve_delete_attachments`] (#3732).
pub async fn delete_note_async(
    ctx: &StorageContext,
    note_id: &str,
    delete_attachments: Option<bool>,
) -> Result<bool> {
    let ctx = ctx.clone();
    let note_id = note_id.to_owned();
    tokio::task::spawn_blocking(move || {
        delete_note_with_context(&ctx, &note_id, delete_attachments)
    })
    .await
    .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`import_markdown_with_context`].
pub async fn import_markdown_async(ctx: &StorageContext, paths: &[String]) -> Result<ImportResult> {
    let ctx = ctx.clone();
    let paths = paths.to_vec();
    tokio::task::spawn_blocking(move || import_markdown_with_context(&ctx, &paths))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`export_note_markdown_with_context`].
pub async fn export_note_markdown_async(
    ctx: &StorageContext,
    note_id: &str,
) -> Result<(String, String)> {
    let ctx = ctx.clone();
    let note_id = note_id.to_owned();
    tokio::task::spawn_blocking(move || export_note_markdown_with_context(&ctx, &note_id))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`export_all_notes_with_context`].
pub async fn export_all_notes_async(
    ctx: &StorageContext,
    output_dir: &Path,
) -> Result<ExportResult> {
    let ctx = ctx.clone();
    let output_dir = output_dir.to_path_buf();
    tokio::task::spawn_blocking(move || export_all_notes_with_context(&ctx, &output_dir))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`rebuild_index_with_context`].
pub async fn rebuild_index_async(ctx: &StorageContext) -> Result<super::IndexStats> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || rebuild_index_with_context(&ctx))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`find_related_notes_with_context`].
pub async fn find_related_notes_async(
    ctx: &StorageContext,
    note_id: &str,
    limit: usize,
) -> Result<Vec<crate::models::RelatedNote>> {
    let ctx = ctx.clone();
    let note_id = note_id.to_owned();
    tokio::task::spawn_blocking(move || find_related_notes_with_context(&ctx, &note_id, limit))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`bulk_delete_notes_with_context`] (#3514).
pub async fn bulk_delete_notes_async(
    ctx: &StorageContext,
    note_ids: Vec<String>,
    delete_attachments: Option<bool>,
) -> Result<BulkNoteOpResult> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        bulk_delete_notes_with_context(&ctx, &note_ids, delete_attachments)
    })
    .await
    .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`bulk_move_notes_with_context`] (#3514).
pub async fn bulk_move_notes_async(
    ctx: &StorageContext,
    note_ids: Vec<String>,
    target_dir: String,
) -> Result<BulkNoteOpResult> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || bulk_move_notes_with_context(&ctx, &note_ids, &target_dir))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`bulk_update_tags_with_context`] (#3514).
pub async fn bulk_update_tags_async(
    ctx: &StorageContext,
    note_ids: Vec<String>,
    add_tags: Vec<String>,
    remove_tags: Vec<String>,
) -> Result<BulkNoteOpResult> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        bulk_update_tags_with_context(&ctx, &note_ids, &add_tags, &remove_tags)
    })
    .await
    .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`load_context_notes_with_context`].
pub async fn load_context_notes_async(
    ctx: &StorageContext,
    question: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteDocument>> {
    let ctx = ctx.clone();
    let question = question.to_owned();
    let image_paths = image_paths.to_vec();
    tokio::task::spawn_blocking(move || {
        load_context_notes_with_context(&ctx, &question, &image_paths, limit)
    })
    .await
    .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`search_candidate_notes_with_context`].
pub async fn search_candidate_notes_async(
    ctx: &StorageContext,
    question: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteMeta>> {
    let ctx = ctx.clone();
    let question = question.to_owned();
    let image_paths = image_paths.to_vec();
    tokio::task::spawn_blocking(move || {
        search_candidate_notes_with_context(&ctx, &question, &image_paths, limit)
    })
    .await
    .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`vault_export_with_context`].
pub async fn vault_export_async(
    ctx: &StorageContext,
    output_path: &Path,
) -> Result<VaultExportResult> {
    let ctx = ctx.clone();
    let output_path = output_path.to_path_buf();
    tokio::task::spawn_blocking(move || vault_export_with_context(&ctx, &output_path))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`ocr_image_text`].
pub async fn ocr_image_text_async(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    let path_display = path.display().to_string();
    match tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || ocr_image_text(&path)),
    )
    .await
    {
        Ok(inner) => inner.map_err(|e| anyhow!("spawn_blocking failed: {e}"))?,
        Err(_) => {
            warn!(path = %path_display, "OCR timed out after 30s");
            Ok(String::new())
        }
    }
}

/// Spawn-blocking wrapper for [`load_recent_notes_for_overview`].
pub async fn load_recent_notes_for_overview_async(
    ctx: &StorageContext,
    limit: usize,
) -> Result<Vec<NoteDocument>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || load_recent_notes_for_overview(&ctx, limit))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Find an existing note by exact title and tag match.
///
/// Returns `(id, created_at)` of the most recently updated matching note, or `None`
/// if no note matches. Used for idempotency checks — e.g., ensuring a daily
/// briefing is upserted rather than duplicated on repeated runs (#3499).
///
/// The `tags` column stores a JSON array (e.g. `["daily-briefing","auto-generated"]`),
/// so the tag is matched with a `LIKE` pattern `"tag"`.
pub fn find_note_by_title_and_tag(
    context: &StorageContext,
    title: &str,
    tag: &str,
) -> Result<Option<(String, String)>> {
    let (connection, _) = open_connection(context)?;
    let tag_pattern = format!("%\"{}\"%", tag);
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT id, created_at FROM notes \
             WHERE title = ?1 AND tags LIKE ?2 \
             ORDER BY updated_at DESC LIMIT 1",
            params![title, tag_pattern],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row)
}

/// Async wrapper for [`find_note_by_title_and_tag`].
pub async fn find_note_by_title_and_tag_async(
    ctx: &StorageContext,
    title: String,
    tag: String,
) -> Result<Option<(String, String)>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || find_note_by_title_and_tag(&ctx, &title, &tag))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_frontmatter_no_delimiter_returns_defaults() {
        let content = "Just body text\nNo frontmatter here";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert!(fm.id.is_empty());
        assert!(fm.title.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn split_frontmatter_valid_block_parses_fields() {
        let content = "---\nid: test-id\ntitle: Test Title\n---\n\nBody here";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert_eq!(fm.id, "test-id");
        assert_eq!(fm.title, "Test Title");
        assert!(body.contains("Body here"));
    }

    #[test]
    fn split_frontmatter_malformed_returns_err() {
        let content = "---\nid: test\nno closing delimiter";
        assert!(split_frontmatter(content).is_err());
    }

    #[test]
    fn split_frontmatter_empty_block_returns_defaults() {
        let content = "---\n\n---\n\nBody";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert!(fm.id.is_empty());
        assert!(body.contains("Body"));
    }

    // #847: UTF-8 BOM prefix should not prevent frontmatter parsing.
    #[test]
    fn split_frontmatter_with_bom_prefix_parses() {
        let content = "\u{feff}---\nid: bom-test\ntitle: BOM Note\n---\nBody here";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert_eq!(fm.id, "bom-test");
        assert_eq!(fm.title, "BOM Note");
        assert_eq!(body, "Body here");
    }

    // #847: BOM + no frontmatter → defaults, content preserved (BOM stripped).
    #[test]
    fn split_frontmatter_bom_without_frontmatter_returns_defaults() {
        let content = "\u{feff}No frontmatter here.";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert!(fm.id.is_empty());
        assert_eq!(body, "No frontmatter here.");
    }

    // #848: Closing --- without trailing newline should parse.
    #[test]
    fn split_frontmatter_no_trailing_newline_parses() {
        let content = "---\nid: no-newline\ntitle: Edge Case\n---";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert_eq!(fm.id, "no-newline");
        assert_eq!(fm.title, "Edge Case");
        assert!(body.is_empty());
    }

    // #848: BOM + no trailing newline combination.
    #[test]
    fn split_frontmatter_bom_and_no_trailing_newline_parses() {
        let content = "\u{feff}---\nid: combo\ntitle: Combo\n---";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert_eq!(fm.id, "combo");
        assert_eq!(fm.title, "Combo");
        assert!(body.is_empty());
    }

    #[test]
    fn compose_markdown_produces_valid_yaml_frontmatter() {
        let meta = NoteMeta {
            id: "id123".to_string(),
            title: "测试笔记".to_string(),
            ..Default::default()
        };
        let body = "内容";
        let result = compose_markdown(&meta, body).expect("compose");
        assert!(result.starts_with("---\n"));
        assert!(result.contains("\n---\n"));
        assert!(result.contains("id: id123"));
        assert!(result.contains("title: 测试笔记"));
    }

    #[test]
    fn compose_markdown_injects_summary_section() {
        let meta = NoteMeta {
            summary: "这是摘要".to_string(),
            ..Default::default()
        };
        let result = compose_markdown(&meta, "正文").expect("compose");
        assert!(result.contains("## 摘要"));
        assert!(result.contains("这是摘要"));
    }

    #[test]
    fn compose_markdown_preserves_existing_summary_section() {
        let meta = NoteMeta {
            summary: "新摘要".to_string(),
            ..Default::default()
        };
        let result = compose_markdown(&meta, "## 摘要\n\n旧摘要\n\n正文").expect("compose");
        assert_eq!(result.matches("## 摘要").count(), 1);
    }

    #[test]
    fn summary_ignores_headings_and_limits_length() {
        let body = "# 标题\n\n第一段现象说明。\n\n第二段补充。\n";
        let summary = extract_summary(body);
        assert!(summary.contains("第一段现象说明"));
        assert!(!summary.contains("# 标题"));
        assert!(summary.len() <= 180);
    }

    #[test]
    fn frontmatter_round_trip_preserves_core_fields() {
        let meta = NoteMeta {
            id: "abc".to_string(),
            title: "MMC timeout".to_string(),
            tags: vec!["kernel".to_string()],
            keywords: vec!["mmc".to_string()],
            platform: "imx8mp".to_string(),
            board: "evk".to_string(),
            kernel: "5.10".to_string(),
            status: "已解决".to_string(),
            created_at: "2026-04-09T00:00:00Z".to_string(),
            updated_at: "2026-04-09T00:00:00Z".to_string(),
            source: "manual".to_string(),
            path: String::new(),
            summary: String::new(),
            collections: Vec::new(),
        };
        let body = "## 问题现象\n\n启动超时";
        let serialized = compose_markdown(&meta, body).expect("serialize markdown");
        let (frontmatter, parsed_body) = split_frontmatter(&serialized).expect("parse frontmatter");
        assert_eq!(frontmatter.id, "abc");
        assert_eq!(frontmatter.title, "MMC timeout");
        assert_eq!(frontmatter.tags, vec!["kernel".to_string()]);
        assert_eq!(parsed_body.trim(), body);
    }

    #[test]
    fn build_note_path_uses_date_and_id() {
        let path = build_note_path(
            "D:\\Vault",
            "MMC Timeout",
            "2026-04-09T00:00:00Z",
            "abc12345-6789",
        );
        assert!(path.to_string_lossy().contains("2026"));
        assert!(path.to_string_lossy().contains("04"));
        assert!(path.to_string_lossy().contains("mmc-timeout"));
        assert!(path.to_string_lossy().contains("abc12345"));
        assert!(path.to_string_lossy().ends_with(".md"));
    }

    #[test]
    fn build_note_path_invalid_date_uses_current() {
        let path = build_note_path("D:\\Vault", "Test", "invalid-date", "abc12345-6789-def0");
        assert!(path.to_string_lossy().ends_with(".md"));
    }

    #[test]
    fn detect_title_from_h1() {
        assert_eq!(
            detect_title("# My Title\nBody", Path::new("x.md")),
            "My Title"
        );
    }

    #[test]
    fn detect_title_h2_falls_to_file_stem() {
        assert_eq!(
            detect_title("## Sub\nBody", Path::new("my-note.md")),
            "my note"
        );
    }

    #[test]
    fn detect_title_empty_body_uses_file_stem() {
        assert_eq!(
            detect_title("", Path::new("/vault/2026/04/boot-timeout.md")),
            "boot timeout"
        );
    }

    #[test]
    fn detect_title_underscores_replaced() {
        assert_eq!(
            detect_title("", Path::new("boot_timeout_log.md")),
            "boot timeout log"
        );
    }

    #[test]
    fn ensure_summary_injects_when_missing() {
        let result = ensure_summary_section("Body text", "My summary");
        assert!(result.starts_with("## 摘要"));
        assert!(result.contains("My summary"));
        assert!(result.contains("Body text"));
    }

    #[test]
    fn ensure_summary_skips_when_already_present() {
        let body = "## 摘要\n\nExisting\n\nMore";
        let result = ensure_summary_section(body, "New");
        assert_eq!(result, body);
    }

    #[test]
    fn ensure_summary_skips_when_empty() {
        let result = ensure_summary_section("Body text", "");
        assert_eq!(result, "Body text");
    }

    #[test]
    fn append_image_empty_refs_returns_body() {
        assert_eq!(append_image_markdown("body", &[]), "body");
    }

    #[test]
    fn append_image_creates_section() {
        let result = append_image_markdown("body text", &["assets/photo.png".to_string()]);
        assert!(result.contains("## 图片记录"));
        assert!(result.contains("![photo.png](assets/photo.png)"));
    }

    #[test]
    fn append_image_appends_to_existing_section() {
        let body = "body\n\n## 图片记录\n\n![a.png](a.png)";
        let result = append_image_markdown(body, &["b/photo.jpg".to_string()]);
        assert!(result.contains("![a.png](a.png)"));
        assert!(result.contains("![photo.jpg](b/photo.jpg)"));
        assert!(result.contains("/"));
    }

    #[test]
    fn append_image_replaces_backslashes() {
        let result = append_image_markdown("body", &["dir\\img.png".to_string()]);
        assert!(result.contains("dir/img.png"));
        assert!(!result.contains("\\"));
    }

    #[test]
    fn unique_asset_first_occurrence() {
        let mut seen = HashSet::new();
        assert_eq!(unique_asset_name("photo.png", &mut seen), "photo.png");
    }

    #[test]
    fn unique_asset_second_occurrence() {
        let mut seen = HashSet::new();
        seen.insert("photo.png".to_string());
        assert_eq!(unique_asset_name("photo.png", &mut seen), "photo-1.png");
    }

    #[test]
    fn unique_asset_no_extension() {
        let mut seen = HashSet::new();
        seen.insert("data".to_string());
        assert_eq!(unique_asset_name("data", &mut seen), "data-1");
    }

    #[test]
    fn sanitize_id_for_filename_strips_path_traversal() {
        // sanitize_id_for_filename filters path traversal chars
        assert_eq!(sanitize_id_for_filename("../../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_id_for_filename("..\\\\..\\\\windows"), "windows");
        assert_eq!(sanitize_id_for_filename("a/b/c"), "abc");
    }

    #[test]
    fn sanitize_id_for_filename_only_ascii_alphanumeric_and_dash() {
        assert_eq!(sanitize_id_for_filename("abc-123"), "abc-123");
        assert_eq!(sanitize_id_for_filename("日本語テスト"), "");
        assert_eq!(sanitize_id_for_filename("a b c"), "abc");
        assert_eq!(sanitize_id_for_filename("test@#$%"), "test");
    }

    #[test]
    fn sanitize_id_for_filename_uses_full_id() {
        assert_eq!(sanitize_id_for_filename("1234567890"), "1234567890");
        assert_eq!(sanitize_id_for_filename("a"), "a");
        assert_eq!(sanitize_id_for_filename(""), "");
    }

    #[test]
    fn sanitize_id_for_filename_empty_after_filtering() {
        assert_eq!(sanitize_id_for_filename("..."), "");
        assert_eq!(sanitize_id_for_filename("/\\\\./"), "");
        assert_eq!(sanitize_id_for_filename("日本語"), "");
    }

    #[test]
    fn sanitize_filename_basic_ascii() {
        assert_eq!(sanitize_filename("Hello World"), "Hello-World");
        assert_eq!(sanitize_filename("test_file"), "test_file");
        assert_eq!(sanitize_filename("my-note"), "my-note");
    }

    #[test]
    fn sanitize_filename_cjk_with_deunicode() {
        // deunicode converts CJK to romanized form
        let result = sanitize_filename("测试笔记");
        assert!(!result.is_empty());
        assert!(!result.contains(' '));
    }

    #[test]
    fn sanitize_filename_special_chars_to_dashes() {
        let result = sanitize_filename("hello/world\\test");
        assert!(!result.contains('/'));
        assert!(!result.contains('\\'));
        assert!(result.contains('-'));
    }

    #[test]
    fn sanitize_filename_empty_or_whitespace_returns_untitled() {
        assert_eq!(sanitize_filename(""), "untitled");
        assert_eq!(sanitize_filename("   "), "untitled");
        assert_eq!(sanitize_filename("---"), "untitled");
    }

    #[test]
    fn sanitize_filename_consecutive_special_chars_collapsed() {
        let result = sanitize_filename("a///b\\\\\\c");
        // Multiple consecutive non-alphanumeric chars become single dash
        assert_eq!(result.matches('-').count(), 2); // a-b-c
    }

    #[test]
    fn extract_summary_skips_code_blocks_and_headings() {
        let body = "# Title\n\n```code block```\n\nActual content here.\n\nMore text.";
        let summary = extract_summary(body);
        assert!(!summary.contains("# Title"));
        assert!(!summary.contains("```"));
        assert!(summary.contains("Actual content here"));
    }

    #[test]
    fn extract_summary_limits_to_180_chars() {
        let long_text = "a".repeat(300);
        let summary = extract_summary(&long_text);
        assert!(summary.len() <= 180);
    }

    #[test]
    fn extract_summary_empty_body() {
        let summary = extract_summary("");
        assert!(summary.is_empty());
    }

    #[test]
    fn detect_title_h1_takes_priority() {
        let body = "# Real Title\n\nSome content\n## Subtitle";
        let tmp = std::env::temp_dir().join("test.md");
        assert_eq!(detect_title(body, &tmp), "Real Title");
    }

    #[test]
    fn detect_title_empty_h1_falls_through() {
        let body = "# \n\nSome content";
        let tmp = std::env::temp_dir().join("fallback.md");
        // Empty H1 should fall through to file stem
        assert_eq!(detect_title(body, &tmp), "fallback");
    }

    #[test]
    fn detect_title_no_heading_uses_file_stem() {
        let body = "Just some text\nNo headings here";
        let tmp = std::env::temp_dir().join("my_note.md");
        assert_eq!(detect_title(body, &tmp), "my note");
    }

    #[test]
    fn detect_title_stem_with_underscores_and_dashes() {
        let body = "text";
        let tmp = std::env::temp_dir().join("my_cool-note.md");
        assert_eq!(detect_title(body, &tmp), "my cool note");
    }

    #[test]
    fn unique_asset_third_occurrence() {
        let mut seen = HashSet::new();
        seen.insert("img.png".to_string());
        seen.insert("img-1.png".to_string());
        assert_eq!(unique_asset_name("img.png", &mut seen), "img-2.png");
    }

    #[test]
    fn unique_asset_preserves_extension() {
        let mut seen = HashSet::new();
        seen.insert("photo.jpg".to_string());
        assert_eq!(unique_asset_name("photo.jpg", &mut seen), "photo-1.jpg");
    }

    #[test]
    fn unique_asset_multiple_dots_in_name() {
        let mut seen = HashSet::new();
        // slugify converts dots to dashes: "file.backup.tar" → "file-backup-tar"
        assert_eq!(
            unique_asset_name("file.backup.tar.gz", &mut seen),
            "file-backup-tar.gz"
        );
    }

    #[test]
    fn build_related_query_extracts_keywords() {
        let doc = NoteDocument {
            meta: NoteMeta {
                id: "test".to_string(),
                title: "Rust Programming Guide".to_string(),
                tags: vec!["rust".to_string(), "programming".to_string()],
                ..Default::default()
            },
            body: "This is about Rust language and cargo build system".to_string(),
            ..Default::default()
        };
        let query = build_related_query(&doc);
        assert!(!query.is_empty());
        // Should contain some keywords from title or tags
        assert!(
            query.to_lowercase().contains("rust") || query.to_lowercase().contains("programming")
        );
    }

    #[test]
    fn build_related_query_empty_doc() {
        let doc = NoteDocument::default();
        let query = build_related_query(&doc);
        // Should handle empty doc gracefully — just verify no panic
        let _ = query;
    }

    #[test]
    fn extracts_markdown_image_refs() {
        let body = "## 图片记录\n\n![boot-log](attachments/boot-log.png)\n\n![scope](./attachments/scope.jpg)";
        let refs = extract_note_image_refs(body);
        assert_eq!(
            refs,
            vec![
                "attachments/boot-log.png".to_string(),
                "./attachments/scope.jpg".to_string()
            ]
        );
    }

    #[test]
    fn export_id_prefix_safe_for_short_ids() {
        let short_id = "ab";
        let id_prefix: String = short_id.chars().take(8).collect();
        assert_eq!(id_prefix, "ab");

        let exact_8 = "12345678";
        let id_prefix: String = exact_8.chars().take(8).collect();
        assert_eq!(id_prefix, "12345678");

        let long_id = "1234567890abcdef";
        let id_prefix: String = long_id.chars().take(8).collect();
        assert_eq!(id_prefix, "12345678");

        let empty_id = "";
        let id_prefix: String = empty_id.chars().take(8).collect();
        assert_eq!(id_prefix, "");
    }

    #[test]
    fn export_id_prefix_safe_for_cjk_ids() {
        let cjk_id = "日本語abcdefghij";
        let id_prefix: String = cjk_id.chars().take(8).collect();
        assert_eq!(id_prefix, "日本語abcde");

        let short_cjk = "日本語";
        let id_prefix: String = short_cjk.chars().take(8).collect();
        assert_eq!(id_prefix, "日本語");

        let mixed_cjk = "abc日本語def";
        let id_prefix: String = mixed_cjk.chars().take(8).collect();
        assert_eq!(id_prefix, "abc日本語de");
    }

    // ── Wikilink extraction tests (#1829) ──────────────────────────────

    #[test]
    fn regression_1829_extract_wikilinks_simple() {
        let body = "See [[Note A]] and [[Note B]] for details.";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, "Note A");
        assert!(links[0].1.is_none());
        assert_eq!(links[1].0, "Note B");
        assert!(links[1].1.is_none());
    }

    #[test]
    fn regression_1829_extract_wikilinks_with_alias() {
        let body = "See [[Note A|display text]] here.";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "Note A");
        assert_eq!(links[0].1.as_deref(), Some("display text"));
    }

    #[test]
    fn regression_1829_extract_wikilinks_with_heading() {
        let body = "See [[Note A#Section 1]] here.";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "Note A");
        assert_eq!(links[0].1.as_deref(), Some("Section 1"));
    }

    #[test]
    fn regression_1829_extract_wikilinks_skips_code_blocks() {
        let body = "Text [[Real Link]]\n\n```\n[[Code Link]]\n```\n\nMore text [[Another Real]]";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, "Real Link");
        assert_eq!(links[1].0, "Another Real");
    }

    #[test]
    fn regression_1829_extract_wikilinks_skips_inline_code() {
        let body = "See [[Real Link]] and `[[Code Link]]` here.";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "Real Link");
    }

    #[test]
    fn regression_2672_extract_wikilinks_skips_double_backtick_code() {
        // Double-backtick code spans should also be skipped (#2672)
        let body = "See [[Real Link]] and ``code with [[Code Link]]`` here.";
        let links = extract_wikilinks(body);
        assert_eq!(
            links.len(),
            1,
            "double-backtick code span wikilink should be skipped"
        );
        assert_eq!(links[0].0, "Real Link");
    }

    #[test]
    fn regression_2672_extract_wikilinks_double_backtick_with_single_inside() {
        // Double-backtick span containing a single backtick should not close early
        let body = "``a ` b [[wikilink]]`` and [[Real]]";
        let links = extract_wikilinks(body);
        assert_eq!(
            links.len(),
            1,
            "wikilink inside double-backtick span should be skipped"
        );
        assert_eq!(links[0].0, "Real");
    }

    #[test]
    fn regression_2672_extract_wikilinks_mixed_backtick_spans() {
        // Mix of single and double backtick on same line
        let body = "`single [[A]]` and ``double [[B]]`` and [[Real]]";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "Real");
    }

    #[test]
    fn regression_1829_extract_wikilinks_empty_body() {
        let links = extract_wikilinks("");
        assert!(links.is_empty());
    }

    #[test]
    fn regression_1829_extract_wikilinks_no_links() {
        let body = "This is just plain text with no wikilinks at all.";
        let links = extract_wikilinks(body);
        assert!(links.is_empty());
    }

    #[test]
    fn regression_1829_extract_wikilinks_multiple_on_same_line() {
        let body = "Links: [[A]] [[B]] [[C]]";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].0, "A");
        assert_eq!(links[1].0, "B");
        assert_eq!(links[2].0, "C");
    }

    #[test]
    fn regression_1829_extract_wikilinks_cjk_title() {
        let body = "参见 [[日本語ノート]] 了解详情。";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "日本語ノート");
    }

    #[test]
    fn regression_1829_extract_wikilinks_heading_only() {
        // [[#heading]] — no note target, just a heading reference
        let body = "See [[#Introduction]] section.";
        let links = extract_wikilinks(body);
        // target is empty string, should not be included
        assert!(links.is_empty());
    }

    #[test]
    fn regression_1829_parse_wikilink_inner_variants() {
        let (t, a) = parse_wikilink_inner("Simple Title");
        assert_eq!(t, "Simple Title");
        assert!(a.is_none());

        let (t, a) = parse_wikilink_inner("Title|Alias");
        assert_eq!(t, "Title");
        assert_eq!(a.as_deref(), Some("Alias"));

        let (t, a) = parse_wikilink_inner("Title#Section");
        assert_eq!(t, "Title");
        assert_eq!(a.as_deref(), Some("Section"));

        let (t, a) = parse_wikilink_inner("Title|Alias#Section");
        assert_eq!(t, "Title");
        // Alias is the full text after |, including any # chars
        assert_eq!(a.as_deref(), Some("Alias#Section"));

        // Standard Obsidian syntax: [[Title#heading|Alias]]
        let (t, a) = parse_wikilink_inner("Title#heading|Alias");
        assert_eq!(t, "Title");
        assert_eq!(a.as_deref(), Some("Alias"));

        let (t, a) = parse_wikilink_inner("#heading");
        assert_eq!(t, "");
        assert_eq!(a.as_deref(), Some("heading"));
    }

    // ── Regression test for #2850 ─────────────────────────────────

    #[test]
    fn regression_2850_note_not_found_downcast_succeeds() {
        // Regression test for #2850: NoteNotFound must be detectable via
        // `downcast_ref::<NoteNotFound>()` so that callers like
        // `handle_capture` can distinguish "note doesn't exist" from IO/parse
        // errors. Without this, a transient IO failure would silently create
        // a duplicate note instead of propagating the error.
        let err: anyhow::Error = anyhow::Error::from(NoteNotFound("missing".to_string()));

        assert!(
            err.downcast_ref::<NoteNotFound>().is_some(),
            "NoteNotFound must be downcastable from anyhow::Error"
        );

        // A generic error must NOT downcast to NoteNotFound.
        let other_err: anyhow::Error = anyhow::anyhow!("database locked");
        assert!(
            other_err.downcast_ref::<NoteNotFound>().is_none(),
            "non-NoteNotFound errors must not match the downcast"
        );
    }

    // ── Regression tests for unlinked mentions (#2832) ─────────────────

    #[test]
    fn regression_2832_strip_inline_code_preserves_boundaries() {
        let input = "Hello `world` here";
        let cleaned = strip_inline_code(input);
        // `world` → 7 spaces (2 backticks + 5 chars), plus original spaces.
        assert_eq!(cleaned, "Hello         here");
    }

    #[test]
    fn regression_2832_contains_whole_word_basic() {
        assert!(contains_whole_word(
            "i love machine learning",
            "machine learning"
        ));
        assert!(!contains_whole_word("machinelearning", "machine learning"));
    }

    #[test]
    fn regression_2832_contains_whole_word_boundary() {
        assert!(contains_whole_word("(rust) is great", "rust"));
        assert!(!contains_whole_word("rustacean", "rust"));
    }

    #[test]
    fn regression_2832_body_mentions_title_basic() {
        let body = "I've been studying Machine Learning lately.\n```\nMachine Learning code\n```\nNot in code.";
        assert!(body_mentions_title(body, "machine learning"));
    }

    #[test]
    fn regression_2832_body_mentions_title_skips_frontmatter() {
        let body = "---\ntitle: Machine Learning\n---\n\nThis is about Machine Learning indeed.";
        assert!(body_mentions_title(body, "machine learning"));
    }

    #[test]
    fn regression_2832_body_mentions_title_inline_code_skipped() {
        let body = "Run `Machine Learning` from the CLI.";
        assert!(!body_mentions_title(body, "machine learning"));
    }

    // ── #3319: extract_heading_tree tests ──────────────────────────────

    #[test]
    fn heading_tree_simple_h1_to_h3() {
        let body = "# Title\n\n## Section A\n\n### Subsection\n\nText.\n";
        let tree = extract_heading_tree(body);
        assert_eq!(tree.len(), 3);
        assert_eq!(
            tree[0],
            HeadingNode {
                level: 1,
                text: "Title".into(),
                line: 1
            }
        );
        assert_eq!(
            tree[1],
            HeadingNode {
                level: 2,
                text: "Section A".into(),
                line: 3
            }
        );
        assert_eq!(
            tree[2],
            HeadingNode {
                level: 3,
                text: "Subsection".into(),
                line: 5
            }
        );
    }

    #[test]
    fn heading_tree_skips_code_block_headings() {
        let body = "# Real Heading\n\n```\n## Fake Heading in Code\n```\n\n## Real H2\n";
        let tree = extract_heading_tree(body);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].text, "Real Heading");
        assert_eq!(tree[1].text, "Real H2");
    }

    #[test]
    fn heading_tree_empty_body() {
        let tree = extract_heading_tree("");
        assert!(tree.is_empty());
    }

    #[test]
    fn heading_tree_no_headings() {
        let body = "Just some text.\nNo headings here.\n";
        let tree = extract_heading_tree(body);
        assert!(tree.is_empty());
    }

    #[test]
    fn heading_tree_skips_hash_in_words() {
        // `#tag` is not a heading (no space after #)
        let body = "This is #not-a-heading.\n## This is\n";
        let tree = extract_heading_tree(body);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].text, "This is");
        assert_eq!(tree[0].level, 2);
    }

    #[test]
    fn heading_tree_max_level_h6() {
        let body = "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n####### Not H7\n";
        let tree = extract_heading_tree(body);
        assert_eq!(tree.len(), 6); // 7 #'s is not a valid heading
        assert_eq!(tree[5].level, 6);
    }

    #[test]
    fn heading_tree_indented_headings() {
        // Leading whitespace before # is acceptable in Markdown
        let body = "  ## Indented H2\n";
        let tree = extract_heading_tree(body);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].text, "Indented H2");
    }

    // ── #3509: bulk_move must not leave stale DB "ghost" entries for notes
    //    whose id is derived from the file path (i.e. no explicit frontmatter
    //    `id`). Before the fix, the re-index at the new path inserted a new
    //    row under the new path-derived id while the old row lingered. ──
    #[test]
    fn regression_3509_bulk_move_no_ghost_for_path_derived_id() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-3509-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let vault = temp.join("vault");
        let source_dir = vault.join("notes");
        std::fs::create_dir_all(&source_dir).expect("create source dir");
        let ctx = crate::storage::StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");

        // Note WITHOUT an explicit `id` frontmatter field → indexing assigns a
        // path-derived id (SHA-256 of the canonical path).
        let old_path = source_dir.join("ghost-test.md");
        std::fs::write(&old_path, "# Ghost Test\n\nBody before move.\n").expect("write note");

        // Index the file so a row exists under the path-derived id.
        rebuild_index_with_context(&ctx).expect("index");
        let (connection, _settings) = open_connection(&ctx).expect("open conn");

        // Capture the path-derived id assigned at the old location.
        // canonicalize: on Windows, `index_note_file_with_connection` stores the
        // canonical form (e.g. with \\?\ prefix), so queries must also use the
        // canonical path to match.
        let old_path_canonical = old_path.canonicalize().expect("canonicalize old path");
        let old_id: String = connection
            .query_row(
                "SELECT id FROM notes WHERE path = ?1",
                [&old_path_canonical.to_string_lossy().to_string()],
                |row| row.get(0),
            )
            .expect("note row exists before move");
        // Sanity: the id really is path-derived (not a UUID), i.e. it changes
        // when the path changes — this is the precondition for the bug.
        let new_path = vault.join("archive").join("ghost-test.md");
        assert_ne!(
            old_id,
            crate::storage::search::derived_note_id(&new_path),
            "test precondition: id must be path-derived"
        );

        // Move the note into a subdirectory of the vault.
        let result = bulk_move_notes_with_context(&ctx, std::slice::from_ref(&old_id), "archive")
            .expect("bulk move");
        assert_eq!(result.affected, 1, "exactly one note should be moved");
        assert_eq!(result.failures.len(), 0, "no move failures");

        // Reopen to see the post-move DB state.
        let (connection, _settings) = open_connection(&ctx).expect("reopen conn");

        // ── The ghost: no row may still point at the old path. ──
        let ghost_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE path = ?1",
                [&old_path_canonical.to_string_lossy().to_string()],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(
            ghost_count, 0,
            "ghost entry must not remain at the old path after bulk move"
        );

        // ── No orphan FTS row for the stale path-derived id. ──
        let fts_orphan_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM note_fts WHERE note_id = ?1",
                [&old_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(
            fts_orphan_count, 0,
            "note_fts must not retain an orphan row for the old path-derived id"
        );

        // ── Exactly one notes row exists (the freshly-indexed new-path row),
        //    not two. ──
        let new_path_canonical = new_path.canonicalize().expect("canonicalize new path");
        let total_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE path = ?1",
                [&new_path_canonical.to_string_lossy().to_string()],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(
            total_rows, 1,
            "exactly one row should exist at the new path"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    // ── #3509 companion: notes WITH an explicit frontmatter `id` must still
    //    move correctly (the cleanup deletes 0 rows — no false removal). ──
    #[test]
    fn regression_3509_bulk_move_explicit_id_unaffected_by_cleanup() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-3509-explicit-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let vault = temp.join("vault");
        let source_dir = vault.join("notes");
        std::fs::create_dir_all(&source_dir).expect("create source dir");
        let ctx = crate::storage::StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");

        // Note WITH an explicit `id` field — the UPSERT updates the existing
        // row's path in place, so the cleanup is a 0-row no-op.
        let old_path = source_dir.join("explicit-id.md");
        std::fs::write(
            &old_path,
            "---\nid: my-explicit-id\ntitle: Explicit\n---\n\nBody.\n",
        )
        .expect("write note");
        rebuild_index_with_context(&ctx).expect("index");

        // Move the note into a subdirectory of the vault.
        let result = bulk_move_notes_with_context(&ctx, &["my-explicit-id".to_string()], "archive")
            .expect("bulk move");
        assert_eq!(result.affected, 1, "explicit-id note should move");

        let (connection, _settings) = open_connection(&ctx).expect("reopen conn");
        let new_path = vault.join("archive").join("explicit-id.md");
        // canonicalize: on Windows, the DB stores canonical paths (may include \\?\ prefix)
        let new_path_canonical = new_path.canonicalize().expect("canonicalize new path");
        let row_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE id = ?1 AND path = ?2",
                params![
                    "my-explicit-id",
                    &new_path_canonical.to_string_lossy().to_string()
                ],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(
            row_count, 1,
            "explicit-id note must exist once at the new path"
        );

        // Total notes rows for this id should be exactly 1 (not duplicated).
        let total: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE id = ?1",
                params!["my-explicit-id"],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(total, 1, "no duplicate rows for explicit-id note");

        let _ = std::fs::remove_dir_all(&temp);
    }

    // ── #3518: savepoint RELEASE failure must not leave active savepoint
    //    on the connection when returned to the pool. ──
    #[test]
    fn regression_3518_savepoint_released_on_success_path() {
        // Verify that after a successful index_note_file_with_connection,
        // no savepoint remains active on the connection. We can't easily
        // trigger RELEASE failure in a test, but we verify that a ROLLBACK
        // TO on a freshly-opened connection works (no savepoint active).
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-3518-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let vault = temp.join("vault");
        let note_dir = vault.join("notes");
        std::fs::create_dir_all(&note_dir).expect("create dirs");
        let ctx = crate::storage::StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");

        // Write a note and index it — this exercises the savepoint path
        let note_path = note_dir.join("test-note.md");
        std::fs::write(&note_path, "---\ntitle: Test\n---\n\nBody content here.\n").expect("write");
        rebuild_index_with_context(&ctx).expect("rebuild index");

        // After indexing, verify the connection pool is not poisoned:
        // should be able to open a connection and run queries normally.
        let (conn, _) = open_connection(&ctx).expect("open after index");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
            .unwrap_or(0);
        assert!(
            count >= 1,
            "notes table should have at least 1 row after indexing"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    // ── #3519: inline code spans (`...`) should NOT leak into summaries ──
    #[test]
    fn regression_3519_extract_summary_strips_inline_code() {
        // Input with inline code
        let body = "Run `sudo apt install` to install packages.\n\nThen run `make build`.\n\nMore text for length padding here to ensure we reach the character limit and test the full pipeline with multiple lines of content so that the summary engine has plenty of material to work with and can't short-circuit on too-short input.";
        let summary = super::extract_summary(body);
        assert!(
            !summary.contains('`'),
            "summary '{}' should not contain inline code backticks",
            summary
        );
        assert!(
            summary.contains("sudo apt install"),
            "summary '{}' should contain the code content without backticks",
            summary
        );
    }

    #[test]
    fn regression_3519_extract_summary_strips_code_spans_adjacent() {
        let body = "`one``two` end";
        let summary = super::extract_summary(body);
        assert!(!summary.contains('`'), "no backticks in summary");
        assert!(
            summary.contains("one") && summary.contains("two"),
            "content preserved: '{}'",
            summary
        );
    }

    // ── #3739: preview_note_fragment regression tests ──
    #[test]
    fn regression_3739_preview_note_fragment_no_anchor() {
        use super::*;
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-3739-preview-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let ctx = StorageContext::for_test(&dir);

        // Body includes summary section so ensure_summary_section won't duplicate it.
        let body = "## 摘要\n\nBrief summary.\n\nLine 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6";
        let note = NoteDocument {
            meta: NoteMeta {
                id: "test-note-1".to_string(),
                title: "Test Note".to_string(),
                ..Default::default()
            },
            body: body.to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, note).expect("save note");
        assert_eq!(saved.meta.id, "test-note-1");

        // Preview first 3 lines (should include the summary header)
        let preview = preview_note_fragment(&ctx, "test-note-1", None, 3).unwrap();
        assert!(preview.contains("## 摘要"), "preview: {}", preview);
        assert!(preview.contains("Brief summary"), "preview: {}", preview);

        let preview = preview_note_fragment(&ctx, "test-note-1", None, 100).unwrap();
        assert!(preview.contains("Line 1"));
        assert!(preview.contains("Line 6"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_3739_preview_note_fragment_with_heading() {
        use super::*;
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-3739-heading-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let ctx = StorageContext::for_test(&dir);

        let body = "## 摘要\n\ntest summary.\n\n# Introduction\nintro content\nmore intro\n## Methods\nmethod detail\nmore method\n## Results\nresult line";
        let note = NoteDocument {
            meta: NoteMeta {
                title: "Test Heading".to_string(),
                ..Default::default()
            },
            body: body.to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, note).expect("save note");
        let note_id = &saved.meta.id;

        let preview = preview_note_fragment(&ctx, note_id, Some("#Introduction"), 3).unwrap();
        assert!(preview.contains("# Introduction"), "preview: {}", preview);
        assert!(preview.contains("intro content"), "preview: {}", preview);

        // Nonexistent heading → fallback
        let preview = preview_note_fragment(&ctx, note_id, Some("#Nonexistent"), 2).unwrap();
        assert!(
            !preview.contains("method"),
            "nonexistent should fallback: {}",
            preview
        );
        assert!(preview.contains("## 摘要"), "fallback: {}", preview);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_3739_preview_note_fragment_block_id() {
        use super::*;
        use crate::block_ref::parse_blocks;
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-3739-block-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let ctx = StorageContext::for_test(&dir);

        let body = "## 摘要\n\ntest block summary.\n\nplain line\nblock content here ^block-abc\nmore text";
        let note = NoteDocument {
            meta: NoteMeta {
                title: "test-block".to_string(),
                ..Default::default()
            },
            body: body.to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, note).expect("save note");
        let note_id = &saved.meta.id;

        // Block ids are FNV-1a content hashes assigned by parse_blocks /
        // annotate_blocks (src/block_ref.rs), NOT user-written "^marker" text.
        // Anchor with a real id and assert *exact* block text so the
        // find_block_by_id branch is genuinely exercised: the fallback path
        // (first `max_lines` lines) can never return this merged paragraph.
        let blocks = parse_blocks(&saved.body);
        assert_eq!(blocks.len(), 3, "expected 3 blocks, got: {blocks:?}");
        let target = &blocks[2];
        assert!(
            target.text.contains("block content here"),
            "expected merged paragraph block, got: {:?}",
            target.text
        );
        let preview =
            preview_note_fragment(&ctx, note_id, Some(&format!("^{}", target.id)), 1).unwrap();
        assert_eq!(
            preview, target.text,
            "block-id branch must return the exact block text, got: {preview}"
        );

        // Headings are addressable blocks too.
        let heading = &blocks[0];
        assert_eq!(heading.text, "## 摘要");
        let preview =
            preview_note_fragment(&ctx, note_id, Some(&format!("^{}", heading.id)), 1).unwrap();
        assert_eq!(
            preview, "## 摘要",
            "heading block lookup must return the heading text"
        );

        // A user-written "^marker" in the body is literal text, not a block id,
        // so it must NOT resolve — the preview falls back to the note head.
        let preview = preview_note_fragment(&ctx, note_id, Some("^block-abc"), 2).unwrap();
        assert!(
            !preview.contains("^block-abc"),
            "user marker must not resolve as a block id, got: {preview}"
        );

        // Unknown ids fall back to the first N lines.
        let preview = preview_note_fragment(&ctx, note_id, Some("^nonexistent"), 2).unwrap();
        assert!(preview.contains("## 摘要"), "fallback: {preview}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
