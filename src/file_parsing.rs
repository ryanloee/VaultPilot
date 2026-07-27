//! Multi-format file parsing pipeline (issue #1987: 多格式文件支持).
//!
//! A unified, dependency-free parsing pipeline that extracts plain text from
//! the file formats most commonly found in a knowledge vault, so the result can
//! feed full-text search / RAG.  Pure-Rust parsers are provided for the text
//! family (txt/log/text), Markdown (with YAML frontmatter), and delimited
//! tabular data (CSV/TSV).
//!
//! # Honest scope (v1)
//! Binary container formats — **PDF** and **Office OOXML** (docx/xlsx/pptx) plus
//! **EPUB** — cannot be decoded correctly with only the standard library.  They
//! are detected and reported via dedicated parsers that set
//! [`ParsedFile::needs_native_parser`] = `true` and return an *empty* text body
//! with a clear explanatory note in metadata.  They deliberately do **not**
//! fake-extract text.  Wiring a native backend (pdfium / poppler / an OOXML
//! unzip+XML parser) is the orchestrator's follow-up work; the trait, cache, and
//! dispatcher are already in place to receive it.
//!
//! # Caching
//! Parsed results are cached in the SQLite table `parsed_files`, keyed by the
//! file path and content-addressed by a SHA-256 hash of the raw bytes.  See
//! [`parse_and_cache`] for the cache-hit semantics.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::instrument;

use crate::storage::StorageContext;

/// Get a pooled SQLite connection from the storage context.
fn db_conn(
    context: &StorageContext,
) -> Result<r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>> {
    context
        .get_connection()
        .context("failed to get database connection")
}

// ─── Data types ───────────────────────────────────────────────────

/// The result of parsing a single file.
///
/// `metadata` is a free-form JSON object whose shape depends on the parser used
/// (e.g. `line_count` for text, `row_count`/`column_count` for CSV, a `note`
/// explaining a skipped binary format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedFile {
    /// Absolute or relative path of the source file, as given to the parser.
    pub path: String,
    /// Lower-cased file extension without the leading dot (e.g. `md`, `csv`).
    pub extension: String,
    /// Best-effort MIME type derived from the extension.
    pub mime_hint: String,
    /// Extracted plain text (UTF-8). Empty for honest stubs of binary formats.
    pub text: String,
    /// Size of the source file in bytes.
    pub byte_size: u64,
    /// Parser-specific metadata (line counts, row/column counts, notes, …).
    pub metadata: serde_json::Value,
    /// Name of the parser that produced this result (`txt`, `markdown`, …).
    pub parser_used: String,
    /// `true` for formats we cannot yet decode in pure Rust (PDF/Office/EPUB).
    pub needs_native_parser: bool,
}

/// A parser that knows how to turn a file on disk into a [`ParsedFile`].
pub trait FileParser {
    /// Parse the file at `path`.
    fn parse(&self, path: &Path) -> Result<ParsedFile>;
    /// Whether this parser claims to handle the given lower-cased extension.
    fn supports(&self, ext: &str) -> bool;
}

// ─── Helpers ──────────────────────────────────────────────────────

/// Lower-cased extension of `path` without the leading dot (`""` if none).
fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Lossy string copy of a path (non-UTF-8 bytes become U+FFFD).
fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Best-effort MIME type for an extension.
fn mime_hint_for(ext: &str) -> String {
    match ext {
        "txt" | "log" | "text" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "epub" => "application/epub+zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Formats whose contents we cannot decode in pure Rust and therefore defer to
/// a native backend.  PDF is now handled by `pdf-extract` — removed from this list.
fn is_native_format(ext: &str) -> bool {
    matches!(ext, "docx" | "xlsx" | "pptx" | "epub")
}

/// SHA-256 hex digest of `bytes` (content-addressing for the parse cache).
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// ─── Concrete parsers ─────────────────────────────────────────────

/// Plain-text parser: txt / log / text, and the catch-all fallback.
pub struct TxtParser;

impl FileParser for TxtParser {
    fn parse(&self, path: &Path) -> Result<ParsedFile> {
        let bytes =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let ext = extension_of(path);
        let line_count = text.lines().count();
        Ok(ParsedFile {
            path: path_string(path),
            extension: ext.clone(),
            mime_hint: mime_hint_for(&ext),
            text,
            byte_size: bytes.len() as u64,
            metadata: serde_json::json!({
                "line_count": line_count,
                "encoding": "utf-8(lossy)",
            }),
            parser_used: "txt".to_string(),
            needs_native_parser: is_native_format(&ext),
        })
    }

    fn supports(&self, ext: &str) -> bool {
        matches!(ext, "txt" | "log" | "text")
    }
}

/// Markdown parser: strips a leading YAML frontmatter block into metadata.
pub struct MarkdownParser;

/// Split a leading `---\n...\n---\n` YAML frontmatter block from `content`.
///
/// Returns `(frontmatter, body)` where `frontmatter` is `Some(raw_yaml)` when a
/// block was found (without the surrounding `---` fences) and `body` is the
/// remaining text.  When no frontmatter is present, `body` is the original
/// content untouched.
fn split_frontmatter(content: &str) -> (Option<String>, String) {
    let lines: Vec<&str> = content.lines().collect();
    let is_fence = |l: &str| l.trim_end_matches('\r') == "---";
    if lines.first().is_some_and(|l| is_fence(l)) {
        for (i, line) in lines.iter().enumerate().skip(1) {
            if is_fence(line) {
                let frontmatter = lines[1..i].join("\n");
                let body = lines[i + 1..]
                    .join("\n")
                    .trim_start_matches('\n')
                    .to_string();
                return (Some(frontmatter), body);
            }
        }
    }
    (None, content.to_string())
}

impl FileParser for MarkdownParser {
    fn parse(&self, path: &Path) -> Result<ParsedFile> {
        let bytes =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        let ext = extension_of(path);
        let (frontmatter, body) = split_frontmatter(&raw);
        let line_count = body.lines().count();
        let metadata = match &frontmatter {
            Some(fm) => serde_json::json!({
                "line_count": line_count,
                "encoding": "utf-8(lossy)",
                "has_frontmatter": true,
                "frontmatter": fm,
            }),
            None => serde_json::json!({
                "line_count": line_count,
                "encoding": "utf-8(lossy)",
                "has_frontmatter": false,
            }),
        };
        Ok(ParsedFile {
            path: path_string(path),
            extension: ext.clone(),
            mime_hint: mime_hint_for(&ext),
            text: body,
            byte_size: bytes.len() as u64,
            metadata,
            parser_used: "markdown".to_string(),
            needs_native_parser: is_native_format(&ext),
        })
    }

    fn supports(&self, ext: &str) -> bool {
        matches!(ext, "md" | "markdown")
    }
}

/// Delimited-table parser for CSV (`separator = ','`) and TSV (`'\t'`).
pub struct CsvParser {
    separator: char,
}

impl CsvParser {
    /// Build a parser that splits fields on `separator`.
    pub fn new(separator: char) -> Self {
        Self { separator }
    }
}

/// Tolerant RFC-4180-ish record reader: supports quoted fields, doubled-quote
/// escaping (`""`), embedded separators/newlines inside quotes, and CRLF.
/// Blank lines are skipped.
fn parse_delimited(text: &str, separator: char) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_quotes {
            if c == '"' {
                if i + 1 < chars.len() && chars[i + 1] == '"' {
                    field.push('"');
                    i += 2;
                    continue;
                }
                in_quotes = false;
                i += 1;
                continue;
            }
            field.push(c);
            i += 1;
            continue;
        }
        if c == '"' {
            in_quotes = true;
        } else if c == separator {
            current.push(std::mem::take(&mut field));
        } else if c == '\r' {
            // Swallow CR; the following LF ends the record.
        } else if c == '\n' {
            if !field.is_empty() || !current.is_empty() {
                current.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut current));
            }
        } else {
            field.push(c);
        }
        i += 1;
    }
    if !field.is_empty() || !current.is_empty() {
        current.push(field);
        records.push(current);
    }
    records
}

impl FileParser for CsvParser {
    fn parse(&self, path: &Path) -> Result<ParsedFile> {
        let bytes =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        let records = parse_delimited(&raw, self.separator);
        let row_count = records.len();
        let column_count = records.iter().map(Vec::len).max().unwrap_or(0);

        // Compact markdown-ish table preview (header + up to PREVIEW_ROWS).
        const PREVIEW_ROWS: usize = 50;
        let mut text = String::new();
        for (idx, rec) in records.iter().enumerate().take(PREVIEW_ROWS) {
            if rec.is_empty() {
                continue;
            }
            text.push_str("| ");
            text.push_str(&rec.join(" | "));
            text.push_str(" |\n");
            if idx == 0 {
                text.push_str(&"| --- ".repeat(rec.len()));
                text.push_str("|\n");
            }
        }

        let ext = extension_of(path);
        let separator_str = if self.separator == '\t' { "\t" } else { "," };
        let parser_used = if self.separator == '\t' { "tsv" } else { "csv" };
        Ok(ParsedFile {
            path: path_string(path),
            extension: ext.clone(),
            mime_hint: mime_hint_for(&ext),
            text,
            byte_size: bytes.len() as u64,
            metadata: serde_json::json!({
                "row_count": row_count,
                "column_count": column_count,
                "separator": separator_str,
                "line_count": raw.lines().count(),
                "encoding": "utf-8(lossy)",
            }),
            parser_used: parser_used.to_string(),
            needs_native_parser: is_native_format(&ext),
        })
    }

    fn supports(&self, ext: &str) -> bool {
        if self.separator == '\t' {
            ext == "tsv"
        } else {
            ext == "csv"
        }
    }
}

/// PDF parser backed by `pdf-extract` (#1571 Phase 1).
///
/// Reads the file bytes and extracts plain text via `pdf_extract::extract_text_from_mem`.
/// Falls back to the honest stub on extraction failure (malformed/corrupt PDFs).
pub struct PdfParser;

impl FileParser for PdfParser {
    fn parse(&self, path: &Path) -> Result<ParsedFile> {
        let ext = extension_of(path);
        let bytes =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        let byte_size = bytes.len() as u64;

        let (text, metadata) = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pdf_extract::extract_text_from_mem(&bytes)
        })) {
            Ok(Ok(extracted)) => {
                let line_count = extracted.lines().count();
                (
                    extracted,
                    serde_json::json!({
                        "line_count": line_count,
                        "parser_backend": "pdf-extract",
                    }),
                )
            }
            Ok(Err(output_err)) => {
                let msg = format!("pdf-extract error: {output_err}");
                tracing::warn!(
                    "pdf-extract failed for {}: {msg}; returning empty stub",
                    path.display()
                );
                (
                    String::new(),
                    serde_json::json!({
                        "note": format!("PDF text extraction failed: {msg}"),
                        "parser_backend": "pdf-extract(stub-fallback)",
                    }),
                )
            }
            Err(panic_payload) => {
                let msg = if let Some(s) = panic_payload.downcast_ref::<String>() {
                    format!("pdf-extract panic: {s}")
                } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    format!("pdf-extract panic: {s}")
                } else {
                    "pdf-extract panic (unknown)".to_string()
                };
                tracing::warn!(
                    "pdf-extract panicked for {}: {msg}; returning empty stub",
                    path.display()
                );
                (
                    String::new(),
                    serde_json::json!({
                        "note": format!("PDF text extraction failed: {msg}"),
                        "parser_backend": "pdf-extract(stub-fallback)",
                    }),
                )
            }
        };

        Ok(ParsedFile {
            path: path_string(path),
            extension: ext.clone(),
            mime_hint: mime_hint_for(&ext),
            text,
            byte_size,
            metadata,
            parser_used: "pdf".to_string(),
            needs_native_parser: false,
        })
    }

    fn supports(&self, ext: &str) -> bool {
        ext == "pdf"
    }
}

/// Honest stub for Office OOXML (docx/xlsx/pptx) and EPUB — zip + XML formats
/// that need native decompression/parsing support.
pub struct OfficeParser;

impl FileParser for OfficeParser {
    fn parse(&self, path: &Path) -> Result<ParsedFile> {
        let ext = extension_of(path);
        let byte_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        Ok(ParsedFile {
            path: path_string(path),
            extension: ext.clone(),
            mime_hint: mime_hint_for(&ext),
            text: String::new(),
            byte_size,
            metadata: serde_json::json!({
                "note": "Office/EPUB text extraction (docx/xlsx/pptx/epub) requires a native backend (unzip + OOXML parsing); not available in this build.",
            }),
            parser_used: "office".to_string(),
            needs_native_parser: true,
        })
    }

    fn supports(&self, ext: &str) -> bool {
        matches!(ext, "docx" | "xlsx" | "pptx" | "epub")
    }
}

// ─── Dispatcher ───────────────────────────────────────────────────

/// Parse a file by dispatching on its (lower-cased) extension.
///
/// Unknown extensions fall back to [`TxtParser`]; recognized binary extensions
/// (pdf/docx/xlsx/pptx/epub) route to their honest stubs.
pub fn parse_file(path: &Path) -> Result<ParsedFile> {
    let ext = extension_of(path);
    match ext.as_str() {
        "md" | "markdown" => MarkdownParser.parse(path),
        "csv" => CsvParser::new(',').parse(path),
        "tsv" => CsvParser::new('\t').parse(path),
        "pdf" => PdfParser.parse(path),
        "docx" | "xlsx" | "pptx" | "epub" => OfficeParser.parse(path),
        _ => TxtParser.parse(path),
    }
}

// ─── Storage cache ────────────────────────────────────────────────

/// DDL for the `parsed_files` cache table.  Idempotent; safe to re-run.
pub(crate) const FILE_PARSING_SCHEMA_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS parsed_files (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    extension TEXT,
    text TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    parser_used TEXT NOT NULL,
    parsed_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_parsed_files_content_hash ON parsed_files(content_hash);
"#;

/// Create the `parsed_files` cache table if it does not yet exist.
///
/// Called lazily by every public DB-touching function in this module so that
/// callers (and tests) need not wire it into the central schema bootstrap.
pub(crate) fn ensure_parsing_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(FILE_PARSING_SCHEMA_DDL)?;
    Ok(())
}

/// Parse `path` and upsert the result into the `parsed_files` cache.
///
/// # Cache-hit behaviour
/// The file's raw bytes are read and SHA-256 hashed.  If a cached row exists for
/// this path **and** its `content_hash` matches, the previously extracted text
/// and metadata are returned directly and the (potentially expensive) parser
/// text-extraction step is **skipped**.  Only the byte size, which is derived
/// from the bytes we already read to hash, and the extension-derived hints are
/// refreshed.  On a cache miss (new path or changed content) the file is parsed
/// and the row is upserted.
#[instrument(skip(context))]
pub fn parse_and_cache(context: &StorageContext, path: &Path) -> Result<ParsedFile> {
    let conn = db_conn(context)?;
    ensure_parsing_tables(&conn)?;

    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let hash = content_hash(&bytes);
    let key = path_string(path);

    // Cache-hit probe: reuse cached text when the content hash is unchanged.
    let cached: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT content_hash, text, metadata_json, parser_used
             FROM parsed_files WHERE path = ?1",
            rusqlite::params![key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    if let Some((cached_hash, cached_text, cached_metadata_json, cached_parser_used)) = cached {
        if cached_hash == hash {
            let ext = extension_of(path);
            let metadata = serde_json::from_str(&cached_metadata_json)
                .unwrap_or_else(|_| serde_json::json!({}));
            return Ok(ParsedFile {
                path: key,
                extension: ext.clone(),
                mime_hint: mime_hint_for(&ext),
                text: cached_text,
                byte_size: bytes.len() as u64,
                metadata,
                parser_used: cached_parser_used,
                needs_native_parser: is_native_format(&ext),
            });
        }
    }

    // Cache miss: parse and upsert.
    let parsed = parse_file(path)?;
    let metadata_json = serde_json::to_string(&parsed.metadata)
        .context("failed to serialize parsed file metadata")?;
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    conn.execute(
        "INSERT INTO parsed_files
            (path, content_hash, extension, text, metadata_json, parser_used, parsed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(path) DO UPDATE SET
            content_hash = excluded.content_hash,
            extension    = excluded.extension,
            text         = excluded.text,
            metadata_json= excluded.metadata_json,
            parser_used  = excluded.parser_used,
            parsed_at    = excluded.parsed_at",
        rusqlite::params![
            key,
            hash,
            parsed.extension,
            parsed.text,
            metadata_json,
            parsed.parser_used,
            now,
        ],
    )?;
    Ok(parsed)
}

/// Return a previously cached parse for `path` without parsing.
///
/// Reads only from the `parsed_files` cache table; returns `None` if the path
/// has not been cached.  `byte_size` is recovered from the file's current
/// metadata (a cheap `stat`) since the cache table does not store it.
#[instrument(skip(context))]
pub fn cached_parse(context: &StorageContext, path: &Path) -> Result<Option<ParsedFile>> {
    let conn = db_conn(context)?;
    ensure_parsing_tables(&conn)?;
    let key = path_string(path);

    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT extension, text, metadata_json, parser_used
             FROM parsed_files WHERE path = ?1",
            rusqlite::params![key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    match row {
        Some((ext, text, metadata_json, parser_used)) => {
            let metadata =
                serde_json::from_str(&metadata_json).unwrap_or_else(|_| serde_json::json!({}));
            let byte_size = std::fs::metadata(path)
                .map(|m| m.len())
                .unwrap_or(text.len() as u64);
            Ok(Some(ParsedFile {
                path: key,
                extension: ext.clone(),
                mime_hint: mime_hint_for(&ext),
                text,
                byte_size,
                metadata,
                parser_used,
                needs_native_parser: is_native_format(&ext),
            }))
        }
        None => Ok(None),
    }
}

/// Delete cached parse result(s).
///
/// If `path_opt` is `Some(p)`, deletes only the row for that path; if `None`,
/// clears the entire cache.
#[instrument(skip(context))]
pub fn clear_cache(context: &StorageContext, path_opt: Option<&str>) -> Result<()> {
    let conn = db_conn(context)?;
    ensure_parsing_tables(&conn)?;
    match path_opt {
        Some(path) => {
            conn.execute(
                "DELETE FROM parsed_files WHERE path = ?1",
                rusqlite::params![path],
            )?;
        }
        None => {
            conn.execute("DELETE FROM parsed_files", [])?;
        }
    }
    Ok(())
}

/// Parsed image-dimension suffix of an Obsidian/Logseq-style `![alt|WxH](url)`
/// image, plus the original alt text and URL.  CommonMark does not natively
/// support sizing, but the `|WxH` convention is widely used by Obsidian, Logseq
/// and Notion exports.  VaultPilot keeps this information so the front-ends
/// (WinUI drag-resize, Android pinch-zoom — issue #2934) can render and
/// round-trip a user-chosen display size without losing data on re-save.
///
/// `width`/`height` are in pixels.  Either dimension may be omitted
/// (`|300` = width only, `|x200` = height only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSize {
    /// Alt text without the `|WxH` suffix (may be empty).
    pub alt: String,
    /// Display width in pixels, if specified.
    pub width: Option<u32>,
    /// Display height in pixels, if specified.
    pub height: Option<u32>,
    /// The image target (relative path, vault link, or URL).
    pub url: String,
}

/// Try to parse `s` as a bare dimension specifier (no alt text, no `|`).
///
/// This handles the Obsidian 1.13.2 external-image syntax `![200](url)` and
/// `![200x150](url)` where the alt position carries pure dimension metadata.
/// Returns `Some(dimension_str)` only when `s` is a valid `W` or `WxH` string
/// (both parts parse as `u32`, or at least one does); otherwise `None`.
///
/// A plain number `"200"` is accepted as width-only. Mixed text like
/// `"my photo"` or `"note|300"` is rejected and treated as alt text.
fn _try_parse_alt_as_size(s: &str) -> Option<&str> {
    if s.is_empty() {
        return None;
    }
    // Must be purely numeric or numeric-x-numeric — no stray words.
    let has_x = s.contains('x');
    if has_x {
        let parts: Vec<&str> = s.split('x').collect();
        if parts.len() != 2 {
            return None;
        }
        let w = parts[0].trim();
        let h = parts[1].trim();
        let w_ok = !w.is_empty() && w.parse::<u32>().is_ok();
        let h_ok = !h.is_empty() && h.parse::<u32>().is_ok();
        if w_ok || h_ok {
            Some(s)
        } else {
            None
        }
    } else {
        // No 'x' → must be a pure number.
        if s.parse::<u32>().is_ok() {
            Some(s)
        } else {
            None
        }
    }
}

/// Parse a markdown image written in the `![alt|WxH](url)` convention
/// (Obsidian/Logseq size syntax).  Returns `Some` for **any** valid image
/// token — including a bare `![alt](url)` with no `|WxH` suffix, in which case
/// `width`/`height` are `None`.  Returns `None` only when the input is not a
/// valid markdown image at all (plain text, a link, or empty alt+url).
///
/// As of Obsidian 1.13.2, the `![WxH](url)` syntax (no `|`, no alt text) is
/// also supported for external images — the alt position holds pure dimension
/// metadata.  See #3221.
///
/// The parser is tolerant of:
/// * whitespace around the `|` (`![alt | 300x200 ](url)`)
/// * missing width or height (`|300`, `|x200`)
/// * non-numeric junk after `|` — treated as "no size" (`None` dims)
pub fn parse_image_size(token: &str) -> Option<ImageSize> {
    // Expect exactly the shape ![...](...)
    let trimmed = token.trim();
    if !trimmed.starts_with("![") || !trimmed.ends_with(')') {
        return None;
    }
    let inner = &trimmed[2..trimmed.len() - 1];
    // Split on the first ']' to separate alt from url.
    let (alt_part, url_part) = match inner.split_once(']') {
        Some((a, u)) => (a, u.strip_prefix('(').unwrap_or(u)),
        None => return None,
    };
    let url = url_part.trim().to_string();
    if url.is_empty() {
        return None;
    }

    // The alt may carry a `|WxH` suffix (Obsidian/Logseq convention).
    // Additionally, Obsidian 1.13.2 added the `![WxH](url)` syntax for
    // *external* images where the alt position holds pure dimension metadata
    // (no `|` separator, no alt text).  See #3221.
    let (alt, size) = match alt_part.split_once('|') {
        Some((a, s)) => (a.trim().to_string(), Some(s.trim())),
        None => {
            // No `|` → check whether alt_part is a pure dimension
            // (width-only "200" or width×height "200x150"). If it parses as
            // a dimension, treat it as the size suffix with empty alt.
            // Otherwise treat the whole string as alt text.
            _try_parse_alt_as_size(alt_part.trim())
                .map(|size_str| ("".to_string(), Some(size_str)))
                .unwrap_or_else(|| (alt_part.trim().to_string(), None))
        }
    };

    let (width, height) = match size {
        None => (None, None),
        Some("") => (None, None),
        Some(s) => {
            let (w, h) = match s.split_once('x') {
                Some((w, h)) => (w.trim(), h.trim()),
                None => (s, ""),
            };
            let width = if w.is_empty() {
                None
            } else {
                w.parse::<u32>().ok()
            };
            let height = if h.is_empty() {
                None
            } else {
                h.parse::<u32>().ok()
            };
            // If neither side parsed, there was no real size suffix.
            if width.is_none() && height.is_none() {
                (None, None)
            } else {
                (width, height)
            }
        }
    };

    Some(ImageSize {
        alt,
        width,
        height,
        url,
    })
}

/// Serialize an [`ImageSize`] back into the markdown token.
///
/// Dimension rendering rules (matches Obsidian convention):
/// * alt + both      → `![alt|WxH](url)`  (Obsidian/Logseq convention)
/// * alt + width only → `![alt|W](url)`
/// * alt + height only → `![alt|xH](url)`
/// * no alt + sized   → `![WxH](url)`  (Obsidian 1.13.2 external image syntax, #3221)
/// * no alt + neither → `![](url)`  (bare image)
pub fn serialize_image_size(img: &ImageSize) -> String {
    let has_alt = !img.alt.is_empty();
    let has_size = img.width.is_some() || img.height.is_some();
    let size = match (img.width, img.height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        (Some(w), None) => w.to_string(),
        (None, Some(h)) => format!("x{h}"),
        (None, None) => String::new(),
    };
    if has_alt && has_size {
        format!("![{}|{}]({})", img.alt, size, img.url)
    } else if !has_alt && has_size {
        format!("![{}]({})", size, img.url)
    } else if has_alt {
        format!("![{}]({})", img.alt, img.url)
    } else {
        format!("![]({})", img.url)
    }
}

/// Rewrite every `![alt|WxH](url)` (or `![alt](url)`) token in `markdown` so
/// that images matching `target_url` get the supplied `width`/`height`
/// applied/cleared.  Useful when a front-end reports a new user-chosen size and
/// the note body must be persisted.  Non-matching images are left untouched,
/// and tokens that are not valid images pass through unchanged.
///
/// Passing `width = None` and `height = None` *clears* any existing size on the
/// matching image (reset to original dimensions), enabling the "double-click to
/// reset" behavior from issue #2934.
pub fn set_image_size(
    markdown: &str,
    target_url: &str,
    width: Option<u32>,
    height: Option<u32>,
) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("![") {
        // Emit everything before this token verbatim.
        out.push_str(&rest[..start]);
        // Find the matching closing ')'. Markdown image tokens cannot be
        // nested, so the first ')' after the '(' ends the token.
        let after = &rest[start..];
        let end = match after.find(']') {
            Some(i) => match after[i..].find(')') {
                Some(j) => start + i + j + 1,
                None => rest.len(),
            },
            None => rest.len(),
        };
        let token = &rest[start..end];
        if let Some(mut img) = parse_image_size(token) {
            if img.url == target_url {
                img.width = width;
                img.height = height;
                out.push_str(&serialize_image_size(&img));
            } else {
                out.push_str(token);
            }
        } else {
            out.push_str(token);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

// ── Image Lightbox support (#3469) ───────────────────────────────────────

/// A reference to an image embedded in a markdown note.
///
/// Used by the Image Lightbox feature (#3469) to build the list of images
/// available for fullscreen viewing and keyboard navigation (arrow keys,
/// pinch-zoom, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// The image target — relative path, vault link, or URL.
    pub url: String,
    /// Alt text (without the `|WxH` size suffix).
    pub alt: String,
    /// Byte offset of the `![` token in the source markdown.
    pub byte_offset: usize,
    /// Whether this is an Obsidian wikilink embed (`![[file]]`) rather than
    /// a standard markdown image (`![alt](url)`).
    pub is_wikilink: bool,
}

/// Collect all image references from a markdown body.
///
/// Supports both standard markdown images (`![alt](url)`) and Obsidian
/// wikilink embeds (`![[file.png]]`).  Returns references in source order
/// (the order they appear in the document).
///
/// Non-image content (links, plain text) is ignored.  Image file extensions
/// recognized for wikilink embeds: `.png`, `.jpg`, `.jpeg`, `.gif`, `.svg`,
/// `.webp`, `.avif`, `.bmp`, `.ico`.
pub fn collect_image_references(markdown: &str) -> Vec<ImageRef> {
    let mut refs = Vec::new();
    let bytes = markdown.as_bytes();
    let mut i = 0;

    while i < markdown.len() {
        // Look for `![` at the current position.
        if i + 1 < bytes.len() && bytes[i] == b'!' && bytes[i + 1] == b'[' {
            // Check for Obsidian wikilink embed: `![[...]]`
            if i + 2 < bytes.len() && bytes[i + 2] == b'[' {
                if let Some(ref_url) = parse_wikilink_embed(&markdown[i..]) {
                    if is_image_extension(&ref_url) {
                        refs.push(ImageRef {
                            url: ref_url,
                            alt: String::new(),
                            byte_offset: i,
                            is_wikilink: true,
                        });
                    }
                    // Skip past the wikilink regardless of whether it's an image.
                    // Find the closing `]]` and advance.
                    if let Some(close) = markdown[i..].find("]]") {
                        i += close + 2;
                        continue;
                    }
                }
            }

            // Standard markdown image: `![alt](url)`
            // Find the matching closing ')' for this token.
            if let Some(img) = parse_image_token_at(markdown, i) {
                refs.push(ImageRef {
                    url: img.0,
                    alt: img.1,
                    byte_offset: i,
                    is_wikilink: false,
                });
                i += img.2; // advance past the token
                continue;
            }
        }
        i += 1;
    }

    refs
}

/// Parse an Obsidian wikilink embed `![[target]]` and return the target string.
///
/// Returns `None` if the text doesn't start with `![[` or doesn't contain `]]`.
fn parse_wikilink_embed(text: &str) -> Option<String> {
    let rest = text.strip_prefix("![[")?;
    let end = rest.find("]]")?;
    let target = &rest[..end];
    if target.is_empty() {
        return None;
    }
    // Strip any alias or heading: `![[file#heading]]` → `file`, `![[file|alias]]` → `file`
    let target = target.split(['#', '|']).next().unwrap_or(target);
    Some(target.trim().to_string())
}

/// Check whether a filename/path has an image extension.
fn is_image_extension(path: &str) -> bool {
    let lower = path.to_lowercase();
    const EXTS: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".avif", ".bmp", ".ico",
    ];
    EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// Parse a standard markdown image token `![alt](url)` starting at byte offset `start`.
///
/// Returns `(url, alt, token_length)` or `None` if not a valid image token.
fn parse_image_token_at(markdown: &str, start: usize) -> Option<(String, String, usize)> {
    let rest = &markdown[start..];
    // Must start with `![`
    if !rest.starts_with("![") {
        return None;
    }
    // Find the closing `]`.
    let bracket_end = rest.find(']')?;
    let alt_part = &rest[2..bracket_end];
    // Must be followed by `(`
    let after_bracket = &rest[bracket_end + 1..];
    let paren_start = after_bracket.strip_prefix('(').ok_or(()).ok()?;
    // Find the *matching* closing `)`, tracking depth so URLs that contain
    // parentheses (e.g. Windows screenshot filenames like "Screenshot (1).png")
    // are not truncated.
    let mut depth = 0u32;
    let mut paren_end: Option<usize> = None;
    for (j, c) in paren_start.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    paren_end = Some(j);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let paren_end = paren_end?;
    let url = paren_start[..paren_end].trim();

    if url.is_empty() {
        return None;
    }

    // Separate alt text from size suffix.
    let alt = if let Some(bar_pos) = alt_part.find('|') {
        alt_part[..bar_pos].trim()
    } else {
        alt_part.trim()
    };

    let token_len = bracket_end + 1 + 1 + paren_end + 1; // `]`+`(`+url+`)`
    Some((url.to_string(), alt.to_string(), token_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_and_hex() {
        let h1 = content_hash(b"hello");
        let h2 = content_hash(b"hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(content_hash(b"hello"), content_hash(b"world"));
    }

    #[test]
    fn split_frontmatter_strips_and_preserves() {
        let (fm, body) = split_frontmatter("---\ntitle: Hi\n---\n\nbody");
        assert_eq!(fm.as_deref(), Some("title: Hi"));
        assert_eq!(body, "body");

        let (fm, body) = split_frontmatter("# no frontmatter\n");
        assert!(fm.is_none());
        assert_eq!(body, "# no frontmatter\n");
    }

    #[test]
    fn csv_handles_quoted_comma_and_crlf() {
        let recs = parse_delimited("a,b\r\nc,\"x, y\"\r\n", ',');
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0], vec!["a".to_string(), "b".to_string()]);
        assert_eq!(recs[1], vec!["c".to_string(), "x, y".to_string()]);
    }

    #[test]
    fn tsv_uses_tab_separator() {
        let recs = parse_delimited("a\tb\tc\n1\t2\t3\n", '\t');
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].len(), 3);
        assert_eq!(
            recs[1],
            vec!["1".to_string(), "2".to_string(), "3".to_string()]
        );
    }

    #[test]
    fn txt_parser_supports() {
        let p = TxtParser;
        assert!(p.supports("txt"));
        assert!(p.supports("log"));
        assert!(p.supports("text"));
        assert!(!p.supports("md"));
    }

    // ── issue #2934: image size attribute (Obsidian `![alt|WxH](url)`) ──────

    #[test]
    fn image_size_full_dimensions() {
        let img = parse_image_size("![photo|300x200](img/cat.png)").unwrap();
        assert_eq!(img.alt, "photo");
        assert_eq!(img.width, Some(300));
        assert_eq!(img.height, Some(200));
        assert_eq!(img.url, "img/cat.png");
    }

    #[test]
    fn image_size_width_only_and_height_only() {
        let w = parse_image_size("![a|300](x.png)").unwrap();
        assert_eq!((w.width, w.height), (Some(300), None));

        let h = parse_image_size("![a|x200](x.png)").unwrap();
        assert_eq!((h.width, h.height), (None, Some(200)));
    }

    #[test]
    fn image_size_tolerates_whitespace() {
        let img = parse_image_size("![alt | 300x200 ](url)").unwrap();
        assert_eq!(img.width, Some(300));
        assert_eq!(img.height, Some(200));
        assert_eq!(img.alt, "alt");
    }

    #[test]
    fn image_size_plain_image_has_no_dims() {
        // A bare image with no `|WxH` suffix parses to Some with None dims.
        let img = parse_image_size("![alt](img/cat.png)").unwrap();
        assert_eq!(img.width, None);
        assert_eq!(img.height, None);
        assert_eq!(img.url, "img/cat.png");
    }

    #[test]
    fn image_size_junk_suffix_yields_no_dims() {
        // Non-numeric junk after `|` is not a real size → Some with None dims.
        let img = parse_image_size("![alt|garbage](x.png)").unwrap();
        assert_eq!(img.width, None);
        assert_eq!(img.height, None);
    }

    #[test]
    fn image_size_rejects_non_image_tokens() {
        assert!(parse_image_size("just text").is_none());
        assert!(parse_image_size("[link](url)").is_none());
        assert!(parse_image_size("![]()").is_none());
    }

    #[test]
    fn image_size_roundtrip_full() {
        let token = "![photo|300x200](img/cat.png)";
        let img = parse_image_size(token).unwrap();
        assert_eq!(serialize_image_size(&img), token);
    }

    #[test]
    fn image_size_roundtrip_width_only() {
        let img = parse_image_size("![a|300](x.png)").unwrap();
        assert_eq!(serialize_image_size(&img), "![a|300](x.png)");
    }

    #[test]
    fn image_size_roundtrip_height_only() {
        let img = parse_image_size("![a|x200](x.png)").unwrap();
        assert_eq!(serialize_image_size(&img), "![a|x200](x.png)");
    }

    #[test]
    fn set_image_size_applies_to_matching_url() {
        let md = "intro\n\n![photo|100x50](img/cat.png)\n\noutro ![other](img/dog.png)";
        let out = set_image_size(md, "img/cat.png", Some(400), Some(300));
        assert!(out.contains("![photo|400x300](img/cat.png)"));
        // Non-matching image untouched.
        assert!(out.contains("![other](img/dog.png)"));
        // Surrounding text preserved.
        assert!(out.starts_with("intro"));
        assert!(out.contains("outro"));
    }

    #[test]
    fn set_image_size_can_clear_to_reset() {
        // Passing None/None clears size → simulates "double-click to reset".
        let md = "![photo|300x200](img/cat.png)";
        let out = set_image_size(md, "img/cat.png", None, None);
        assert_eq!(out, "![photo](img/cat.png)");
    }

    #[test]
    fn set_image_size_leaves_non_matching_untouched() {
        let md = "![a|300x200](img/cat.png)";
        let out = set_image_size(md, "img/dog.png", Some(10), None);
        assert_eq!(out, md);
    }

    // ── #3221: Obsidian 1.13.2 external image size syntax ──────

    #[test]
    fn obsidian_external_image_width_only() {
        // ![200](url) → width=200, height=None, alt=""
        let img = parse_image_size("![200](https://example.com/img.png)").unwrap();
        assert_eq!(img.width, Some(200));
        assert_eq!(img.height, None);
        assert_eq!(img.alt, "");
        assert_eq!(img.url, "https://example.com/img.png");
    }

    #[test]
    fn obsidian_external_image_full_dims() {
        let img = parse_image_size("![200x150](https://example.com/img.png)").unwrap();
        assert_eq!(img.width, Some(200));
        assert_eq!(img.height, Some(150));
        assert_eq!(img.alt, "");
        assert_eq!(img.url, "https://example.com/img.png");
    }

    #[test]
    fn obsidian_external_image_roundtrip_width_only() {
        let token = "![200](https://example.com/img.png)";
        let img = parse_image_size(token).unwrap();
        assert_eq!(serialize_image_size(&img), token);
    }

    #[test]
    fn obsidian_external_image_roundtrip_full() {
        let token = "![200x150](https://example.com/img.png)";
        let img = parse_image_size(token).unwrap();
        assert_eq!(serialize_image_size(&img), token);
    }

    #[test]
    fn obsidian_external_image_alt_with_pipe_still_works() {
        // Alt text with `|` separator should still work (existing Obsidian/Logseq style)
        let img = parse_image_size("![photo|300x200](img/cat.png)").unwrap();
        assert_eq!(img.alt, "photo");
        assert_eq!(img.width, Some(300));
        assert_eq!(img.height, Some(200));
    }

    #[test]
    fn obsidian_external_image_numeric_alt_but_with_pipe_is_alt() {
        // "![300|200](url)" — the | means "300" is alt text, "200" is width.
        // This differs from Obsidian where | separates alt from size.
        // We preserve backwards compat: alt text before | is kept.
        let img = parse_image_size("![300|200](url)").unwrap();
        assert_eq!(img.alt, "300");
        assert_eq!(img.width, Some(200));
        assert_eq!(img.height, None);
    }

    // ── collect_image_references tests (#3469) ─────────────────────────────

    #[test]
    fn test_collect_images_markdown_standard() {
        let md = "Some text\n![cat photo](assets/cat.png)\nMore text";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "assets/cat.png");
        assert_eq!(refs[0].alt, "cat photo");
        assert!(!refs[0].is_wikilink);
    }

    #[test]
    fn test_collect_images_wikilink_embed() {
        let md = "Text\n![[dog.jpg]]\nEnd";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "dog.jpg");
        assert!(refs[0].is_wikilink);
    }

    #[test]
    fn test_collect_images_mixed_syntax() {
        let md = "![first](a.png)\n![[second.jpg]]\n![third](c.gif)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].url, "a.png");
        assert_eq!(refs[1].url, "second.jpg");
        assert_eq!(refs[2].url, "c.gif");
        assert!(!refs[0].is_wikilink);
        assert!(refs[1].is_wikilink);
        assert!(!refs[2].is_wikilink);
    }

    #[test]
    fn test_collect_images_no_images() {
        let md = "Just text\n[a link](page.md)\nNo images here";
        let refs = collect_image_references(md);
        assert!(refs.is_empty());
    }

    #[test]
    fn test_collect_images_ignores_non_image_wikilinks() {
        let md = "![[note.md]]\n![[image.png]]";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "image.png");
    }

    #[test]
    fn test_collect_images_wikilink_with_heading() {
        let md = "![[page.md#section]]\n![[photo.jpg#center]]";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "photo.jpg");
    }

    #[test]
    fn test_collect_images_wikilink_with_alias() {
        let md = "![[photo.jpg|My Photo]]";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "photo.jpg");
    }

    #[test]
    fn test_collect_images_preserves_source_order() {
        let md = "![z](z.png)\n![a](a.png)\n![m](m.png)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].url, "z.png");
        assert_eq!(refs[1].url, "a.png");
        assert_eq!(refs[2].url, "m.png");
    }

    #[test]
    fn test_collect_images_byte_offsets() {
        let md = "xx![cat](c.png)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].byte_offset, 2);
    }

    #[test]
    fn test_collect_images_size_suffix_stripped_from_alt() {
        let md = "![photo|300x200](c.png)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].alt, "photo");
        assert_eq!(refs[0].url, "c.png");
    }

    #[test]
    fn test_collect_images_all_extensions() {
        let md = "![](a.png)![](b.jpg)![](c.jpeg)![](d.gif)![](e.svg)![](f.webp)![](g.avif)![](h.bmp)![](i.ico)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 9);
    }

    #[test]
    fn test_collect_images_empty_body() {
        assert!(collect_image_references("").is_empty());
        assert!(collect_image_references("no images").is_empty());
    }

    #[test]
    fn test_collect_images_obsidian_external_syntax() {
        // Obsidian 1.13.2 external image syntax: ![200](url) or ![200x150](url)
        let md = "![200](https://example.com/img.png)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "https://example.com/img.png");
    }
}
