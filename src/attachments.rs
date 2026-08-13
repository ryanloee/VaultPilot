//! Orphan attachment scanning & cleanup (#3672).
//!
//! When a note is edited and its image/file references are removed, the
//! referenced attachment files stay on disk forever — `delete_note_with_context`
//! only cleans up when the *note itself* is deleted. This module detects
//! attachments under `<vault>/attachments/` that are not referenced by any
//! note body (markdown `![alt](path)` / `[text](path)` links and `![[path]]`
//! wikilink embeds) or by the `attachments` DB table, and optionally removes
//! them (dry-run by default, mirroring the safety model of #3135).
//!
//! Safety model:
//! - `scan` / `clean` without `--delete` only lists orphans (dry-run).
//! - Deletion requires an explicit `--delete` flag and is logged to stderr.
//! - Files referenced by *any* note body or the attachments table are never
//!   considered orphans, even if the reference syntax is unusual.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::storage::notes::extract_wikilinks;
use crate::storage::StorageContext;

/// Sanitizes a user-supplied filename so it cannot escape the temp directory
/// (no path separators, no `..`, no control characters) and keeps a printable
/// extension when possible.
pub fn sanitize_filename(filename: &str) -> String {
    let name = filename.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = name
        .chars()
        .map(|c| {
            // Control chars + Windows-invalid filename chars are replaced.
            if c.is_control() || matches!(c, ':' | '?' | '*' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() || cleaned == ".." {
        "attachment".to_string()
    } else {
        cleaned.chars().take(120).collect()
    }
}

/// Writes raw attachment bytes (image/audio picked in the UI) to a unique file
/// under the OS temp dir and returns its absolute path. Used by the desktop
/// Tauri shell to bridge `<input type="file">`/MediaRecorder payloads — which
/// the webview only exposes as in-memory bytes — to the disk paths the agent's
/// image/audio pipeline requires (#4074).
///
/// The filename is sanitized and a random UUID prefix guarantees uniqueness, so
/// concurrent sends from multiple sessions can never collide.
pub fn save_temp_attachment(data: &[u8], filename: &str) -> Result<String> {
    let safe_name = sanitize_filename(filename);
    let dir = std::env::temp_dir().join("vaultpilot-attachments");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create temp attachment dir: {}", dir.display()))?;
    let path = dir.join(format!("{}-{}", uuid_like(), safe_name));
    std::fs::write(&path, data)
        .with_context(|| format!("failed to write temp attachment: {}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Unique-ish suffix without pulling uuid into the hot path (tests run in
/// parallel and rely on uniqueness across processes).
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", std::process::id(), nanos)
}

/// A single orphan attachment file found on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanAttachment {
    /// Absolute path of the orphan file on disk.
    pub path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Last-modified timestamp (RFC 3339) if available.
    pub modified_at: Option<String>,
}

/// Result of a cleanup pass (dry-run or real deletion).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentCleanReport {
    /// Whether this was a dry run (no files deleted).
    pub dry_run: bool,
    /// Total orphan files found.
    pub total_orphans: usize,
    /// Number of files actually deleted (0 for dry-run).
    pub deleted: usize,
    /// Total bytes that would be / were freed.
    pub freed_bytes: u64,
    /// The orphan files (truncated to `max_list` for the report).
    pub orphans: Vec<OrphanAttachment>,
}

/// Maximum number of orphan files included in the report list.
const MAX_ORPHANS_IN_REPORT: usize = 1_000;

/// Format epoch seconds + nanoseconds as an RFC 3339 UTC timestamp
/// (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
///
/// Uses `chrono` (already a workspace dependency) for correct calendar
/// arithmetic. This replaces the previous approximate conversion that
/// assumed every year is exactly 365 days and every month 30.42 days
/// (2,628,000 s), which produced dates 12–14 days ahead of reality
/// (#3727).
///
/// Returns the bare epoch seconds as a fallback string for timestamps that
/// cannot be represented (e.g. far-future / out-of-range values).
fn format_epoch_rfc3339(secs: i64, nanos: u32) -> String {
    match DateTime::<Utc>::from_timestamp(secs, nanos) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        None => format!("{secs}"),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan the vault's `attachments/` directory and return files that are not
/// referenced by any note body or the `attachments` DB table.
pub fn scan_orphan_attachments(context: &StorageContext) -> Result<Vec<OrphanAttachment>> {
    let vault_dir = context.vault_dir().to_path_buf();
    let attachments_root = vault_dir.join("attachments");
    if !attachments_root.exists() {
        return Ok(Vec::new());
    }

    // 1. Collect every file under <vault>/attachments/ (including subdirs
    //    like attachments/audio/).
    let disk_files = collect_attachment_files(&attachments_root)?;

    // 2. Build the set of referenced attachment paths.
    let referenced = collect_referenced_paths(context, &vault_dir)?;

    // 3. Diff: disk files not referenced anywhere are orphans.
    let mut orphans = Vec::new();
    for abs in disk_files {
        if referenced.contains(&abs) {
            continue;
        }
        let meta = std::fs::metadata(&abs).ok();
        let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_at = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| format_epoch_rfc3339(d.as_secs() as i64, d.subsec_nanos()));
        orphans.push(OrphanAttachment {
            path: abs.to_string_lossy().to_string(),
            size_bytes,
            modified_at,
        });
    }

    // Deterministic ordering by path for stable output.
    orphans.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(orphans)
}

/// Clean up orphan attachments.
///
/// When `delete` is `false` (default) this is a dry run: orphans are listed
/// but nothing is removed. When `delete` is `true` the orphan files are
/// deleted from disk. Empty directories left behind are pruned.
pub fn clean_orphan_attachments(
    context: &StorageContext,
    delete: bool,
) -> Result<AttachmentCleanReport> {
    let orphans = scan_orphan_attachments(context)?;
    let total_orphans = orphans.len();
    let mut deleted = 0usize;
    let mut freed_bytes = 0u64;

    if delete {
        for orphan in &orphans {
            let path = PathBuf::from(&orphan.path);
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    deleted += 1;
                    freed_bytes += orphan.size_bytes;
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to delete orphan attachment");
                }
            }
        }
        // Prune now-empty directories under attachments/ (deepest first).
        let attachments_root = context.vault_dir().join("attachments");
        prune_empty_dirs(&attachments_root);
    } else {
        freed_bytes = orphans.iter().map(|o| o.size_bytes).sum();
    }

    let report_orphans: Vec<OrphanAttachment> =
        orphans.into_iter().take(MAX_ORPHANS_IN_REPORT).collect();

    Ok(AttachmentCleanReport {
        dry_run: !delete,
        total_orphans,
        deleted,
        freed_bytes,
        orphans: report_orphans,
    })
}

// ---------------------------------------------------------------------------
// Note-scoped attachment cleanup (#3718)
// ---------------------------------------------------------------------------

/// An attachment file referenced by a note being deleted, classified by
/// whether it is safe to remove from disk.
///
/// Returned by [`list_attachments_exclusive_to_note`]. Used by platform UIs
/// to preview "these files will also be deleted" before the user confirms
/// note deletion (#3718, parity with Obsidian 1.12.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteAttachment {
    /// Absolute path of the attachment file on disk (if it exists).
    pub path: String,
    /// File name (basename) for display.
    pub file_name: String,
    /// File size in bytes (0 if the file is missing from disk).
    pub size_bytes: u64,
    /// `true` when no other note references this file — i.e. deleting the
    /// current note would orphan it, so it is safe to delete. `false` when
    /// at least one other note also references the file (shared).
    pub exclusive_to_note: bool,
}

/// List every attachment referenced by `note_id`, classifying each as
/// "exclusive to this note" (no other note references it) or "shared".
///
/// This is the preview used by the delete-note confirmation prompt (#3718):
/// the platform UI shows the exclusive attachments as "files that will also
/// be removed". The actual deletion is performed by
/// [`crate::storage::notes::delete_note_with_context`] when called with
/// `delete_attachments = Some(true)`; this helper only computes the list
/// without modifying anything.
///
/// Returns an empty vector when the note does not exist or has no
/// attachments. The vector is sorted by file name for stable display.
pub fn list_attachments_exclusive_to_note(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<NoteAttachment>> {
    let conn = context.get_connection()?;

    // Resolve the note id (the caller may pass a path).
    let resolved_id: Option<String> = conn
        .query_row(
            "SELECT id FROM notes WHERE id = ?1 OR path = ?1 LIMIT 1",
            [note_id],
            |row| row.get(0),
        )
        .ok();
    let Some(resolved_id) = resolved_id else {
        return Ok(Vec::new());
    };

    // Fetch this note's attachment rows.
    let mut stmt = conn
        .prepare("SELECT path, file_name FROM attachments WHERE note_id = ?1 ORDER BY file_name")?;
    let rows = stmt.query_map([&resolved_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    drop(stmt);

    let mut result = Vec::with_capacity(entries.len());
    for (path, file_name) in entries {
        // Count how many OTHER notes reference the same path.
        let other_refs: i64 = conn.query_row(
            "SELECT COUNT(*) FROM attachments WHERE path = ?1 AND note_id != ?2",
            [&path, &resolved_id],
            |row| row.get(0),
        )?;
        let exclusive = other_refs == 0;
        let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        result.push(NoteAttachment {
            path,
            file_name,
            size_bytes,
            exclusive_to_note: exclusive,
        });
    }

    // Deterministic ordering: exclusive first, then by file name.
    result.sort_by(|a, b| {
        b.exclusive_to_note
            .cmp(&a.exclusive_to_note)
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    Ok(result)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Recursively collect all regular files under `root`, returning absolute paths.
fn collect_attachment_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read attachments directory {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                files.push(normalize_abs(&path));
            }
        }
    }
    Ok(files)
}

/// Build the set of absolute paths that are referenced by any note.
///
/// Sources:
/// 1. The `attachments` DB table (synced from note bodies on save).
/// 2. Every note body in `note_fts`: markdown image/link destinations and
///    `![[path]]` wikilink embeds, resolved against the vault root and the
///    note's own directory.
fn collect_referenced_paths(
    context: &StorageContext,
    vault_dir: &Path,
) -> Result<HashSet<PathBuf>> {
    let conn = context.get_connection()?;
    let mut referenced: HashSet<PathBuf> = HashSet::new();

    // Source 1: attachments DB table — paths are stored absolute.
    {
        let mut stmt = conn.prepare("SELECT path FROM attachments")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            let path = row?;
            let p = PathBuf::from(&path);
            if p.is_absolute() {
                referenced.insert(normalize_abs(&p));
            } else {
                // Relative path in the table — resolve against vault root.
                let abs = vault_dir.join(&p);
                referenced.insert(normalize_abs(&abs));
            }
        }
    }

    // Source 2: note bodies + note directories (for relative resolution).
    {
        let mut stmt = conn.prepare(
            "SELECT n.path, f.body FROM notes n \
             LEFT JOIN note_fts f ON f.note_id = n.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (note_path, body) = row?;
            let note_dir = Path::new(&note_path)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();

            // Markdown links: ![alt](dest) and [text](dest)
            for dest in extract_markdown_destinations(body.as_deref().unwrap_or("")) {
                if let Some(abs) = resolve_reference(&dest, vault_dir, &note_dir) {
                    referenced.insert(abs);
                }
            }
            // Wikilink embeds: ![[path]] and plain [[path]] with an extension.
            for (target, _alias) in extract_wikilinks(body.as_deref().unwrap_or("")) {
                // Only treat as a file reference if it looks like a path with
                // an extension or an attachments/ prefix (note wikilinks are
                // matched by title, not path).
                if looks_like_file_path(&target) {
                    if let Some(abs) = resolve_reference(&target, vault_dir, &note_dir) {
                        referenced.insert(abs);
                    }
                }
            }
        }
    }

    Ok(referenced)
}

/// Extract markdown link/image destinations from a note body:
/// `![alt](dest)` and `[text](dest)`. Skips fenced code blocks.
fn extract_markdown_destinations(body: &str) -> Vec<String> {
    let mut destinations = Vec::new();
    let mut in_code_block = false;
    let mut offset = 0usize;

    while offset < body.len() {
        // Find the next '[' that opens a link or image.
        let Some(rel) = body[offset..].find('[') else {
            break;
        };
        let start = offset + rel;

        // Update fenced-code state from the skipped text.
        for line in body[offset..start].lines() {
            if line.trim_start().starts_with("```") {
                in_code_block = !in_code_block;
            }
        }
        if in_code_block {
            offset = start + 1;
            continue;
        }

        // Find the matching '](' after this '['.
        let Some(close_bracket) = body[start..].find("](") else {
            break;
        };
        let dest_start = start + close_bracket + 2;
        let Some(close_paren) = body[dest_start..].find(')') else {
            break;
        };
        let raw = &body[dest_start..dest_start + close_paren];
        let dest = raw
            .trim_matches('<')
            .trim_matches('>')
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        if !dest.is_empty() && !dest.starts_with("http://") && !dest.starts_with("https://") {
            destinations.push(dest);
        }
        offset = dest_start + close_paren + 1;
    }

    destinations
}

/// True if a wikilink target looks like a file reference rather than a note
/// title: it has a file extension or an `attachments/` prefix.
fn looks_like_file_path(target: &str) -> bool {
    let cleaned = target.trim().trim_start_matches("./");
    if cleaned.starts_with("attachments/") {
        return true;
    }
    // Has an extension (e.g. image.png, doc.pdf) — note titles rarely do.
    cleaned
        .rsplit('/')
        .next()
        .map(|name| name.contains('.'))
        .unwrap_or(false)
}

/// Resolve a markdown/wikilink reference to an absolute path.
///
/// Tries in order:
/// 1. As an absolute path (already absolute).
/// 2. Relative to the note's directory.
/// 3. Relative to the vault root.
fn resolve_reference(reference: &str, vault_dir: &Path, note_dir: &Path) -> Option<PathBuf> {
    let reference = reference.trim();
    if reference.is_empty() || reference.starts_with('#') {
        return None;
    }
    let p = Path::new(reference);
    if p.is_absolute() {
        return Some(normalize_abs(p));
    }
    // Try note-relative first, then vault-relative.
    for base in [note_dir, vault_dir] {
        let candidate = base.join(p);
        if candidate.exists() {
            return Some(normalize_abs(&candidate));
        }
    }
    // Even if the file doesn't exist on disk (broken reference), resolve it
    // lexically so we never flag a referenced-but-missing file as an orphan.
    let lexical = normalize_abs(&vault_dir.join(p));
    Some(lexical)
}

/// Normalize an absolute path: canonicalize when possible, otherwise remove
/// `.` / `..` components lexically.
fn normalize_abs(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Remove empty directories under `root`, deepest first. Returns the number
/// of directories removed.
fn prune_empty_dirs(root: &Path) -> usize {
    let mut removed = 0usize;
    if !root.exists() {
        return 0;
    }
    // Collect all dirs, then remove deepest-first so nested empty dirs are
    // pruned bottom-up (and the root is only removed if it ended up empty).
    let mut dirs = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    stack.push(entry.path());
                }
            }
            dirs.push(dir);
        }
    }
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in dirs {
        let is_empty = std::fs::read_dir(&dir)
            .map(|mut e| e.next().is_none())
            .unwrap_or(false);
        if is_empty && std::fs::remove_dir(&dir).is_ok() {
            removed += 1;
        }
    }
    removed
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── save_temp_attachment / sanitize_filename (#4074) ──────────────────

    #[test]
    fn sanitize_filename_strips_path_traversal_and_controls() {
        // Path separators are stripped to the basename — traversal is impossible.
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("..\\..\\evil.png"), "evil.png");
        assert_eq!(sanitize_filename(""), "attachment");
        assert_eq!(sanitize_filename("..."), "attachment");
        assert_eq!(sanitize_filename("a b.png"), "a b.png");
        assert_eq!(sanitize_filename("你好.png"), "你好.png");
        assert_eq!(sanitize_filename("na:me?.png"), "na_me_.png");
    }

    #[test]
    fn save_temp_attachment_writes_unique_file_in_temp_dir() {
        let data = b"fake-image-bytes";
        let p1 = save_temp_attachment(data, "photo.png").unwrap();
        let p2 = save_temp_attachment(data, "photo.png").unwrap();
        // Unique paths for concurrent sends…
        assert_ne!(p1, p2);
        let path = std::path::Path::new(&p1);
        // …same parent (OS temp dir), same content round-trip.
        assert!(path.starts_with(std::env::temp_dir()));
        assert_eq!(fs::read(&p1).unwrap(), data);
        assert_eq!(fs::read(&p2).unwrap(), data);
        assert!(p1.ends_with("photo.png"), "got: {p1}");
        // Sanitized names cannot escape the temp dir.
        let p3 = save_temp_attachment(data, "../../evil.png").unwrap();
        assert!(std::path::Path::new(&p3).starts_with(std::env::temp_dir()));
        fs::remove_file(&p1).ok();
        fs::remove_file(&p2).ok();
        fs::remove_file(&p3).ok();
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Create a fresh test context with initialized storage schema.
    fn make_context() -> (std::path::PathBuf, StorageContext) {
        let temp =
            std::env::temp_dir().join(format!("vp-orphan-{}-{}", std::process::id(), uuid_like()));
        fs::create_dir_all(&temp).unwrap();
        let ctx = StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");
        (temp, ctx)
    }

    /// Insert a note with the given body into the DB and return its id.
    fn insert_note(context: &StorageContext, id: &str, note_path: &str, body: &str) {
        let conn = context.get_connection().unwrap();
        conn.execute(
            "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, status, \
             created_at, updated_at, source, path, summary, body_hash) \
             VALUES (?1, ?2, '[]', '[]', '', '', '', '', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z', 'manual', ?3, '', '')",
            rusqlite::params![id, id, note_path],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO note_fts (note_id, title, keywords, body) VALUES (?1, ?2, '', ?3)",
            rusqlite::params![id, id, body],
        )
        .unwrap();
    }

    #[test]
    fn extract_markdown_destinations_handles_images_links_and_code() {
        let body = "# Title\n\n![boot-log](attachments/boot-log.png)\n\n\
                    [spec](./attachments/spec.pdf)\n\n\
                    ```\n![ignored](attachments/nope.png)\n```\n\n\
                    [web](https://example.com/x.png)";
        let dests = extract_markdown_destinations(body);
        assert!(dests.contains(&"attachments/boot-log.png".to_string()));
        assert!(dests.contains(&"./attachments/spec.pdf".to_string()));
        assert!(!dests.contains(&"attachments/nope.png".to_string()));
        assert!(!dests.iter().any(|d| d.starts_with("http")));
    }

    #[test]
    fn looks_like_file_path_detects_embeds_only() {
        assert!(looks_like_file_path("attachments/photo.png"));
        assert!(looks_like_file_path("images/report.pdf"));
        assert!(looks_like_file_path("audio.mp3"));
        assert!(!looks_like_file_path("Meeting Notes"));
        assert!(!looks_like_file_path("Project Alpha"));
    }

    #[test]
    fn scan_finds_orphan_but_keeps_referenced_files() {
        let (temp, ctx) = make_context();
        let vault = ctx.vault_dir().to_path_buf();
        fs::create_dir_all(vault.join("attachments/audio")).unwrap();

        // Referenced via markdown image (relative to vault root).
        write_file(&vault.join("attachments/used.png"), "png-bytes");
        // Referenced via markdown link.
        write_file(&vault.join("attachments/notes.pdf"), "pdf-bytes");
        // Referenced via wikilink embed.
        write_file(&vault.join("attachments/audio/voice.mp3"), "mp3-bytes");
        // Orphans.
        write_file(&vault.join("attachments/orphan.png"), "orphan-bytes");
        write_file(&vault.join("attachments/audio/old.wav"), "old-bytes");

        insert_note(
            &ctx,
            "note-1",
            "note-1.md",
            "![used](attachments/used.png)\n[pdf](attachments/notes.pdf)\n![[attachments/audio/voice.mp3]]\n",
        );

        let orphans = scan_orphan_attachments(&ctx).unwrap();
        let paths: Vec<&str> = orphans.iter().map(|o| o.path.as_str()).collect();

        assert!(
            paths.iter().any(|p| p.ends_with("orphan.png")),
            "orphan.png should be flagged: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("old.wav")),
            "old.wav should be flagged: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.ends_with("used.png")),
            "used.png is referenced and must not be flagged: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.ends_with("notes.pdf")),
            "notes.pdf is referenced and must not be flagged: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.ends_with("voice.mp3")),
            "voice.mp3 is referenced and must not be flagged: {paths:?}"
        );

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn clean_dry_run_does_not_delete_files() {
        let (temp, ctx) = make_context();
        let vault = ctx.vault_dir().to_path_buf();
        write_file(&vault.join("attachments/stale.png"), "stale");

        let report = clean_orphan_attachments(&ctx, false).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.total_orphans, 1);
        assert_eq!(report.deleted, 0);
        assert!(vault.join("attachments/stale.png").exists());

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn clean_with_delete_removes_files_and_prunes_dirs() {
        let (temp, ctx) = make_context();
        let vault = ctx.vault_dir().to_path_buf();
        write_file(&vault.join("attachments/audio/stale.mp3"), "stale");
        write_file(&vault.join("attachments/keep.png"), "keep");
        insert_note(&ctx, "n1", "n1.md", "![keep](attachments/keep.png)");

        let report = clean_orphan_attachments(&ctx, true).unwrap();
        assert!(!report.dry_run);
        assert_eq!(report.total_orphans, 1);
        assert_eq!(report.deleted, 1);
        assert!(!vault.join("attachments/audio/stale.mp3").exists());
        assert!(vault.join("attachments/keep.png").exists());

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn attachments_table_paths_are_never_orphans() {
        let (temp, ctx) = make_context();
        let vault = ctx.vault_dir().to_path_buf();
        write_file(&vault.join("attachments/db-tracked.png"), "db");

        // Note with an empty body — the file is only tracked via the DB table.
        insert_note(&ctx, "n1", "n1.md", "");
        let abs = vault.join("attachments/db-tracked.png");
        let conn = ctx.get_connection().unwrap();
        conn.execute(
            "INSERT INTO attachments (id, note_id, path, created_at) \
             VALUES ('a1', 'n1', ?1, '2026-01-01T00:00:00Z')",
            rusqlite::params![abs.to_string_lossy()],
        )
        .unwrap();

        let orphans = scan_orphan_attachments(&ctx).unwrap();
        assert!(
            !orphans.iter().any(|o| o.path.ends_with("db-tracked.png")),
            "DB-table path must not be flagged: {:?}",
            orphans.iter().map(|o| &o.path).collect::<Vec<_>>()
        );

        fs::remove_dir_all(&temp).ok();
    }

    /// Deterministic-ish unique suffix for temp dirs (tests run in parallel).
    fn uuid_like() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!("{:016x}", COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Regression test for #3718: `list_attachments_exclusive_to_note` must
    /// classify a note's attachments as "exclusive" (safe to delete) or
    /// "shared" (referenced by another note) so the delete-note UI can show
    /// an accurate preview before purging.
    #[test]
    fn regression_3718_exclusive_vs_shared_attachments() {
        let (temp, ctx) = make_context();
        let vault = ctx.vault_dir().to_path_buf();

        // Two notes; note A owns two attachment files, note B shares one of them.
        write_file(&vault.join("attachments/exclusive.png"), "png-a");
        write_file(&vault.join("attachments/shared.png"), "png-shared");

        insert_note(&ctx, "note-a", "note-a.md", "body a");
        insert_note(&ctx, "note-b", "note-b.md", "body b");

        let conn = ctx.get_connection().unwrap();
        let excl_path = vault.join("attachments/exclusive.png");
        let shared_path = vault.join("attachments/shared.png");
        // exclusive.png → only note-a
        conn.execute(
            "INSERT INTO attachments (id, note_id, path, file_name, created_at) \
             VALUES ('a1', 'note-a', ?1, 'exclusive.png', '2026-01-01T00:00:00Z')",
            rusqlite::params![excl_path.to_string_lossy()],
        )
        .unwrap();
        // shared.png → note-a AND note-b
        conn.execute(
            "INSERT INTO attachments (id, note_id, path, file_name, created_at) \
             VALUES ('a2', 'note-a', ?1, 'shared.png', '2026-01-01T00:00:00Z')",
            rusqlite::params![shared_path.to_string_lossy()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO attachments (id, note_id, path, file_name, created_at) \
             VALUES ('b1', 'note-b', ?1, 'shared.png', '2026-01-01T00:00:00Z')",
            rusqlite::params![shared_path.to_string_lossy()],
        )
        .unwrap();

        let attachments = list_attachments_exclusive_to_note(&ctx, "note-a").unwrap();

        // Both attachments belong to note-a.
        assert_eq!(
            attachments.len(),
            2,
            "expected both of note-a's attachments"
        );

        let excl = attachments
            .iter()
            .find(|a| a.file_name == "exclusive.png")
            .expect("exclusive.png should be listed");
        assert!(
            excl.exclusive_to_note,
            "exclusive.png is referenced only by note-a → exclusive_to_note must be true"
        );

        let shared = attachments
            .iter()
            .find(|a| a.file_name == "shared.png")
            .expect("shared.png should be listed");
        assert!(
            !shared.exclusive_to_note,
            "shared.png is also referenced by note-b → exclusive_to_note must be false"
        );

        // Non-existent note → empty list (no panic).
        assert!(
            list_attachments_exclusive_to_note(&ctx, "does-not-exist")
                .unwrap()
                .is_empty(),
            "non-existent note should yield an empty attachment list"
        );

        fs::remove_dir_all(&temp).ok();
    }

    /// Regression test for #3718: the new settings field round-trips through
    /// serialization and defaults to `Ask` (matching Obsidian 1.12.0).
    #[test]
    fn regression_3718_attachment_cleanup_mode_default_and_roundtrip() {
        use crate::models::AttachmentCleanupMode;
        use std::str::FromStr;

        // Default is "Ask".
        assert_eq!(AttachmentCleanupMode::default(), AttachmentCleanupMode::Ask);
        assert_eq!(AttachmentCleanupMode::default().as_str(), "ask");

        // as_str round trip.
        for mode in [
            AttachmentCleanupMode::Always,
            AttachmentCleanupMode::Ask,
            AttachmentCleanupMode::Never,
        ] {
            let s = mode.as_str();
            assert_eq!(AttachmentCleanupMode::from_str(s).unwrap(), mode);
        }

        // Serde round trip preserves the value (camelCase keys).
        let json = serde_json::to_string(&AttachmentCleanupMode::Always).unwrap();
        assert_eq!(json, "\"always\"");
        let parsed: AttachmentCleanupMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AttachmentCleanupMode::Always);

        // Invalid input errors clearly.
        assert!(AttachmentCleanupMode::from_str("sometimes").is_err());
    }

    #[test]
    fn test_format_epoch_rfc3339_known_dates() {
        // 2024-06-15T12:30:45.123Z corresponds to epoch 1718454645 + 123ms.
        // The OLD approximate math reported this as ~2024-06-27 (+12 days).
        // (#3727)
        let s = format_epoch_rfc3339(1_718_454_645, 123_000_000);
        assert_eq!(s, "2024-06-15T12:30:45.123Z");

        // 2024-12-15T00:00:00.000Z → epoch 1734220800.
        let s = format_epoch_rfc3339(1_734_220_800, 0);
        assert_eq!(s, "2024-12-15T00:00:00.000Z");
    }

    #[test]
    fn test_format_epoch_rfc3339_epoch_zero() {
        // Unix epoch: 1970-01-01T00:00:00.000Z.
        assert_eq!(format_epoch_rfc3339(0, 0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn test_format_epoch_rfc3339_no_future_drift() {
        // 2026-08-02T04:00:00Z → the approximate conversion drifted this
        // ~12 days ahead. With chrono it must land exactly on Aug 2.
        // epoch = 178_...; compute via a sanity anchor: 2026-01-01T00:00:00Z
        // = 1767225600; Aug 2 is day-of-year 214 → 1767225600 + 213*86400.
        let aug2 = 1_767_225_600 + 213 * 86_400;
        let s = format_epoch_rfc3339(aug2, 0);
        assert_eq!(s, "2026-08-02T00:00:00.000Z");
    }
}
