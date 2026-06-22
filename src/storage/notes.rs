//! Note CRUD, import/export, and OCR operations.
//!
//! Extracted from `mod.rs` to keep the storage module focused (#1280).

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
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

use crate::models::{ExportResult, ImportResult, NoteDocument, NoteMeta, VaultExportResult};

use super::search::{
    build_attachment_semantic_text, build_text_semantic_vector, derived_note_id, fallback_source,
    fallback_title, hash_content, is_markdown_file, list_all_note_metas, load_note_meta_by_id,
    rank_documents, rank_note_metas, sanitize_terms, serialize_semantic_vector, slugify,
};
use super::{
    atomic_write, load_chat_state_with_context, load_settings_with_context, open_connection,
    Frontmatter, StorageContext, MAX_NOTE_FILE_SIZE,
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
    let id = if is_new {
        Uuid::new_v4().to_string()
    } else {
        note.meta.id.clone()
    };
    let created_at = if note.meta.created_at.trim().is_empty() {
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
    };

    let serialized = compose_markdown(&meta, &body_with_images)?;
    atomic_write(&path, serialized.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    index_note_file_with_connection(&connection, &path)?;
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
        .ok_or_else(|| anyhow!("note not found: {note_id}"))?;
    parse_markdown_note(Path::new(&path), "manual")
}

pub fn delete_note_with_context(context: &StorageContext, note_id: &str) -> Result<bool> {
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
        "DELETE FROM attachments WHERE note_id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM notes WHERE id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.commit()?;

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
                let id_prefix = sanitize_id_prefix(&meta.id);
                let path = output_dir.join(format!("{}-{}.md", filename, id_prefix));
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
                let id_prefix = sanitize_id_prefix(&meta.id);
                let entry_name = format!("notes/{}-{}.md", filename, id_prefix);
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
            if index_note_file_with_connection(&tx, entry.path()).is_ok() {
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
        .ok_or_else(|| anyhow!("note not found: {note_id}"))?;
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

pub(super) fn split_frontmatter(content: &str) -> Result<(Frontmatter, &str)> {
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
    if let Some(end_index) = inner.rfind("\n---") {
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

fn detect_title(body: &str, path: &Path) -> String {
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
    compact.chars().take(180).collect()
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

/// Sanitize a note ID prefix for safe use in file/ZIP entry names (#901).
/// Strips path traversal characters (`.`, `/`, `\`) to prevent Zip Slip attacks.
fn sanitize_id_prefix(id: &str) -> String {
    id.chars()
        .take(8)
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
        .unwrap_or_else(|_| PathBuf::from(&settings.vault_dir));
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    if canonical.starts_with(&vault_dir) {
        index_note_file_with_connection(connection, &canonical)?;
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
        },
        body: parsed.body,
        search_snippet: None,
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
        },
        body: body.trim().to_string(),
        search_snippet: None,
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

fn index_note_file_with_connection(connection: &Connection, path: &Path) -> Result<()> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let document = parse_markdown_note(&canonical, "manual")?;
    let body_hash = hash_content(&document.body);
    connection.execute_batch("SAVEPOINT sp_index_note")?;
    let result: Result<()> = (|| {
        connection.execute(
            "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary, body_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
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
               body_hash = excluded.body_hash",
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
                body_hash
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
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => connection.execute_batch("RELEASE SAVEPOINT sp_index_note")?,
        Err(e) => {
            let _ = connection.execute_batch("ROLLBACK TO SAVEPOINT sp_index_note");
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
) -> Result<()> {
    connection.execute("DELETE FROM attachment_fts WHERE note_id = ?1", [note_id])?;
    connection.execute("DELETE FROM attachments WHERE note_id = ?1", [note_id])?;

    if image_refs.is_empty() {
        return Ok(());
    }

    let note_dir = Path::new(note_path)
        .parent()
        .ok_or_else(|| anyhow!("note path has no parent: {note_path}"))?;
    let now = Utc::now().to_rfc3339();

    for relative in image_refs {
        let absolute = note_dir.join(relative);
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
    }

    Ok(())
}

fn extract_note_image_refs(body: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    let mut offset = 0usize;

    while let Some(start) = body[offset..].find("![") {
        let absolute_start = offset + start;
        let Some(open) = body[absolute_start..].find("](") else {
            break;
        };
        let path_start = absolute_start + open + 2;
        let Some(close) = body[path_start..].find(')') else {
            break;
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
pub async fn delete_note_async(ctx: &StorageContext, note_id: &str) -> Result<bool> {
    let ctx = ctx.clone();
    let note_id = note_id.to_owned();
    tokio::task::spawn_blocking(move || delete_note_with_context(&ctx, &note_id))
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
    tokio::task::spawn_blocking(move || ocr_image_text(&path))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
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
}
