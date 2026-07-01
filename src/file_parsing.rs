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
/// a native backend.
fn is_native_format(ext: &str) -> bool {
    matches!(ext, "pdf" | "docx" | "xlsx" | "pptx" | "epub")
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

/// Honest stub for PDF.  Detects the format, reports its size, but does not
/// extract text — that requires a native backend (pdfium/poppler).
pub struct PdfParser;

impl FileParser for PdfParser {
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
                "note": "PDF text extraction requires a native backend (pdfium/poppler); not available in this build.",
            }),
            parser_used: "pdf".to_string(),
            needs_native_parser: true,
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
}
