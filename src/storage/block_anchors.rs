//! Block anchor generation and resolution.
//!
//! Block anchors are stable `^blockid` markers that can be attached to any
//! block-level element (heading, paragraph, list item, code block) in a note.
//! They enable:
//!   - Block-level references: `[[note#^blockid]]`
//!   - Block-level transclusion (Phase 2)
//!   - AI agent block-granularity RAG
//!
//! Anchor ID format: `^` + 8-char lowercase hex from SHA-256 of block content.
//! This is deterministic — same content → same anchor — so re-saving a note
//! with unchanged content does not churn anchor IDs.
//! Compatible with Obsidian block reference syntax: `[[note#^blockid]]`

use sha2::{Digest, Sha256};
use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{params, Connection};
use tracing::instrument;

/// A single block anchor extracted from a note.
#[derive(Debug, Clone)]
pub struct BlockAnchor {
    /// Note ID that owns this anchor.
    pub note_id: String,
    /// The anchor ID (without `^` prefix, e.g. `"a1b2c3d4"`).
    pub block_id: String,
    /// Type of block: `"heading1"`, `"heading2"`, `"heading3"`, `"paragraph"`,
    /// `"list_item"`, `"code_block"`, `"blockquote"`, `"hr"`, `"thematic_break"`.
    pub block_type: String,
    /// Text content of this block (first 200 chars for display/preview).
    pub content: String,
    /// 0-based start line in the note file.
    pub line_start: i32,
    /// 0-based end line (exclusive) in the note file.
    pub line_end: i32,
}

// ────────────────────────────────────────────
// Anchor generation helpers
// ────────────────────────────────────────────

/// Simple hex encoding of a byte slice (avoids adding `hex` crate dependency).
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Generate a deterministic 8-char anchor ID from `content`.
fn content_anchor(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    hex_encode(&result[..4]) // first 4 bytes → 8 hex chars
}

/// Generate a human-friendly heading anchor (slugified).
/// Falls back to content hash if slug is empty.
fn heading_anchor(text: &str) -> String {
    let slug: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .collect::<String>()
        .trim()
        .to_lowercase()
        .replace(' ', "-");
    if slug.is_empty() {
        content_anchor(text)
    } else {
        slug
    }
}

/// Determine block type from a line of text and its heading level.
fn classify_block_type(line: &str, heading_level: Option<usize>) -> &'static str {
    if let Some(level) = heading_level {
        match level {
            1 => "heading1",
            2 => "heading2",
            3 => "heading3",
            _ => "heading",
        }
    } else if line.trim_start().starts_with('-')
        || line.trim_start().starts_with('*')
        || line.trim_start().starts_with('+')
    {
        "list_item"
    } else if line.trim_start().starts_with('>') {
        "blockquote"
    } else if line.trim_start().starts_with("```") {
        "code_block"
    } else if line.trim_start().starts_with("---")
        || line.trim_start().starts_with("***")
        || line.trim_start().starts_with("___")
    {
        "thematic_break"
    } else {
        "paragraph"
    }
}

/// Check if a line looks like an ordered list item (starts with digit + period).
fn is_ordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    i > 0 && i < bytes.len() && bytes[i] == b'.'
}

/// Parse markdown body and extract block anchors.
///
/// Returns a list of `BlockAnchor` values. Heading blocks get human-friendly
/// slug-based IDs; other blocks get deterministic content-hash-based IDs.
///
/// Compatible with Obsidian's block reference syntax `^blockid`.
/// If a line already has an explicit anchor (`^custom-id`), that is preserved.
pub fn extract_block_anchors(body: &str, note_id: &str) -> Vec<BlockAnchor> {
    let lines: Vec<&str> = body.lines().collect();
    let mut anchors = Vec::new();
    let mut seen_ids = HashSet::new();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Skip frontmatter
        if i == 0 && line.trim() == "---" {
            i += 1;
            while i < lines.len() && lines[i].trim() != "---" {
                i += 1;
            }
            i += 1; // skip closing ---
            continue;
        }

        // Blank lines are not blocks
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // Collect the current block content
        let mut block_lines = vec![line];
        let is_code_block = line.trim_start().starts_with("```");
        if is_code_block {
            // Consume until closing ```
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                block_lines.push(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                block_lines.push(lines[i]); // include closing ```
                i += 1;
            }
        } else {
            i += 1;
            // For paragraph blocks, continue collecting until we hit a blank
            // line, heading, or block-level marker
            let is_heading = line.starts_with('#');
            let is_list_item = is_ordered_list_item(line)
                || line.trim_start().starts_with('-')
                || line.trim_start().starts_with('*')
                || line.trim_start().starts_with('+');
            let is_blockquote = line.trim_start().starts_with('>');
            let is_hr = line.trim_start().starts_with("---")
                || line.trim_start().starts_with("***");

            if is_heading || is_hr {
                // Headings and horizontal rules are single-line blocks
            } else if is_list_item {
                // Consume continuation lines of the same list item
                while i < lines.len() {
                    let next = lines[i];
                    if next.trim().is_empty()
                        || next.starts_with('#')
                        || next.trim_start().starts_with("```")
                    {
                        break;
                    }
                    if next.trim_start().starts_with('-')
                        || next.trim_start().starts_with('*')
                        || next.trim_start().starts_with('+')
                        || is_ordered_list_item(next)
                    {
                        // New list item starts
                        break;
                    }
                    // Continuation (indented text after list marker)
                    block_lines.push(next);
                    i += 1;
                }
            } else if is_blockquote {
                // Consume continuation blockquote lines
                while i < lines.len() && lines[i].trim_start().starts_with('>') {
                    block_lines.push(lines[i]);
                    i += 1;
                }
            } else {
                // Paragraph-style: consume until blank line or heading
                while i < lines.len() {
                    let next = lines[i];
                    if next.trim().is_empty()
                        || next.starts_with('#')
                        || next.trim_start().starts_with("```")
                        || next.trim_start().starts_with("---")
                    {
                        break;
                    }
                    block_lines.push(next);
                    i += 1;
                }
            }
        }

        if block_lines.is_empty() {
            continue;
        }

        // Build block text (strip trailing whitespace, join)
        let block_text = block_lines.join("\n").trim().to_string();
        if block_text.is_empty() {
            continue;
        }

        let first_line = block_lines[0];

        // Determine heading level
        let heading_level = if first_line.starts_with('#') {
            let level = first_line.chars().take_while(|c| *c == '#').count();
            Some(level)
        } else {
            None
        };

        // Generate block ID
        let block_id = if let Some(rest) = block_text.strip_suffix(']') {
            // Check for explicit anchor: `content [^custom-id]`
            if let Some(open) = rest.rfind("[^") {
                let explicit_id = rest[open + 2..].trim().to_string();
                if !explicit_id.contains(' ') && !explicit_id.is_empty() {
                    explicit_id // valid explicit anchor
                } else {
                    // Not a valid anchor marker; fall through
                    if let Some(level) = heading_level {
                        let heading_text = first_line[level..].trim();
                        heading_anchor(heading_text)
                    } else {
                        content_anchor(&block_text)
                    }
                }
            } else {
                // Ends with ] but no [^ — treat as regular block
                if let Some(level) = heading_level {
                    let heading_text = first_line[level..].trim();
                    heading_anchor(heading_text)
                } else {
                    content_anchor(&block_text)
                }
            }
        } else if let Some(rest) = block_text.strip_suffix(')') {
            // Alternative format: `content (^custom-id)`
            if let Some(open) = rest.rfind("(^") {
                let explicit_id = rest[open + 2..].trim().to_string();
                if !explicit_id.contains(' ') && !explicit_id.is_empty() {
                    explicit_id
                } else {
                    if let Some(level) = heading_level {
                        let heading_text = first_line[level..].trim();
                        heading_anchor(heading_text)
                    } else {
                        content_anchor(&block_text)
                    }
                }
            } else {
                if let Some(level) = heading_level {
                    let heading_text = first_line[level..].trim();
                    heading_anchor(heading_text)
                } else {
                    content_anchor(&block_text)
                }
            }
        } else if let Some(level) = heading_level {
            // Heading: slugify the heading text
            let heading_text = first_line[level..].trim();
            heading_anchor(heading_text)
        } else {
            // Other blocks: deterministic hash
            content_anchor(&block_text)
        };

        // Deduplicate within the same note (last one wins for same block_id)
        if seen_ids.contains(&block_id) {
            // Remove previous occurrence so we can replace it
            if let Some(pos) = anchors.iter().position(|a: &BlockAnchor| a.block_id == block_id) {
                anchors.remove(pos);
            }
        }
        seen_ids.insert(block_id.clone());

        let block_type = classify_block_type(first_line, heading_level);

        // Content preview: first 200 chars
        let content_preview: String = block_text.chars().take(200).collect();

        // Line range (0-based) - find the actual line index
        let line_start = lines
            .iter()
            .position(|l| std::ptr::eq(*l, block_lines[0]))
            .unwrap_or(0) as i32;
        let line_end = line_start + block_lines.len() as i32;

        anchors.push(BlockAnchor {
            note_id: note_id.to_string(),
            block_id,
            block_type: block_type.to_string(),
            content: content_preview,
            line_start,
            line_end,
        });
    }

    anchors
}

// ────────────────────────────────────────────
// Database operations
// ────────────────────────────────────────────

/// Ensure the `block_anchors` table exists.
pub fn ensure_block_anchors_table(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS block_anchors (
            note_id TEXT NOT NULL,
            block_id TEXT NOT NULL,
            block_type TEXT NOT NULL DEFAULT 'paragraph',
            content TEXT NOT NULL DEFAULT '',
            line_start INTEGER NOT NULL DEFAULT 0,
            line_end INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (note_id, block_id),
            FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_block_anchors_note_id
            ON block_anchors(note_id);
        "#,
    )?;
    Ok(())
}

/// Replace all block anchors for a given note with fresh ones.
///
/// Call this after `index_note_file_with_connection()` completes.
#[instrument(skip(connection))]
pub fn update_block_anchors(connection: &Connection, note_id: &str, body: &str) -> Result<()> {
    // Delete existing anchors for this note
    connection.execute("DELETE FROM block_anchors WHERE note_id = ?1", params![note_id])?;

    // Extract fresh anchors from the body
    let anchors = extract_block_anchors(body, note_id);

    // Insert new anchors
    let mut stmt = connection.prepare(
        "INSERT INTO block_anchors (note_id, block_id, block_type, content, line_start, line_end)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    for anchor in &anchors {
        stmt.execute(params![
            anchor.note_id,
            anchor.block_id,
            anchor.block_type,
            anchor.content,
            anchor.line_start,
            anchor.line_end,
        ])?;
    }

    Ok(())
}

/// Retrieve a specific block anchor by note ID and block ID.
pub fn get_block_anchor(
    connection: &Connection,
    note_id: &str,
    block_id: &str,
) -> Result<Option<BlockAnchor>> {
    let mut stmt = connection.prepare(
        "SELECT note_id, block_id, block_type, content, line_start, line_end
         FROM block_anchors
         WHERE note_id = ?1 AND block_id = ?2",
    )?;

    let mut rows = stmt.query(params![note_id, block_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(BlockAnchor {
            note_id: row.get(0)?,
            block_id: row.get(1)?,
            block_type: row.get(2)?,
            content: row.get(3)?,
            line_start: row.get(4)?,
            line_end: row.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

/// Retrieve all block anchors for a given note.
pub fn get_block_anchors_for_note(
    connection: &Connection,
    note_id: &str,
) -> Result<Vec<BlockAnchor>> {
    let mut stmt = connection.prepare(
        "SELECT note_id, block_id, block_type, content, line_start, line_end
         FROM block_anchors
         WHERE note_id = ?1
         ORDER BY line_start ASC",
    )?;

    let anchors = stmt
        .query_map(params![note_id], |row| {
            Ok(BlockAnchor {
                note_id: row.get(0)?,
                block_id: row.get(1)?,
                block_type: row.get(2)?,
                content: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(anchors)
}

/// Parse a block reference string like `"Note Name"` or `"Note#^blockid"`
/// into `(note_identifier, block_id_option)`.
///
/// Returns `(Some("Note Name"), Some("blockid"))` for `[[Note Name#^blockid]]`
/// Returns `(Some("Note Name"), None)` for `[[Note Name]]`
/// Returns `(None, Some("blockid"))` for `[[#^blockid]]` (current note)
pub fn parse_block_reference(ref_str: &str) -> (Option<String>, Option<String>) {
    // Strip surrounding [[ ]]
    let inner = ref_str.trim();
    let inner = inner
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(inner);

    if let Some(pos) = inner.find("#^") {
        let note_part = if pos == 0 {
            None
        } else {
            Some(inner[..pos].trim().to_string())
        };
        let block_part = Some(inner[pos + 2..].trim().to_string());
        (note_part, block_part)
    } else if let Some(pos) = inner.find('#') {
        // `#` without `^` — could be a heading anchor
        let note_part = if pos == 0 {
            None
        } else {
            Some(inner[..pos].trim().to_string())
        };
        let block_part = Some(inner[pos + 1..].trim().to_string());
        (note_part, block_part)
    } else {
        (Some(inner.trim().to_string()), None)
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_heading_anchors() {
        let body = "# Introduction\n\nThis is the intro paragraph.\n\n## Details\n\nSome details here.\n";
        let anchors = extract_block_anchors(body, "note-1");
        assert!(!anchors.is_empty(), "expected some anchors");

        let heading_anchor = anchors.iter().find(|a| a.block_type == "heading1");
        assert!(heading_anchor.is_some(), "expected a heading1 anchor");
        assert_eq!(heading_anchor.unwrap().block_id, "introduction");

        let h2_anchor = anchors.iter().find(|a| a.block_type == "heading2");
        assert!(h2_anchor.is_some(), "expected a heading2 anchor");
        assert_eq!(h2_anchor.unwrap().block_id, "details");
    }

    #[test]
    fn test_extract_paragraph_anchors() {
        let body = "First paragraph content here.\n\nSecond paragraph with more text.\n";
        let anchors = extract_block_anchors(body, "note-1");
        let paragraphs: Vec<_> = anchors.iter().filter(|a| a.block_type == "paragraph").collect();
        assert_eq!(paragraphs.len(), 2, "expected 2 paragraph anchors");
        assert_eq!(paragraphs[0].line_start, 0);
        assert_eq!(paragraphs[1].line_start, 2);
    }

    #[test]
    fn test_extract_list_anchors() {
        let body = "- Item one\n- Item two\n- Item three\n";
        let anchors = extract_block_anchors(body, "note-1");
        let list_items: Vec<_> = anchors.iter().filter(|a| a.block_type == "list_item").collect();
        assert_eq!(list_items.len(), 3, "expected 3 list item anchors");
    }

    #[test]
    fn test_explicit_anchor_preserved() {
        let body = "Some paragraph [^my-custom-id]\n\nAnother paragraph.\n";
        let anchors = extract_block_anchors(body, "note-1");
        let explicit = anchors.iter().find(|a| a.block_id == "my-custom-id");
        assert!(explicit.is_some(), "expected explicit anchor to be preserved");
        assert!(explicit.unwrap().content.contains("my-custom-id"));
    }

    #[test]
    fn test_parse_block_reference() {
        let (note, block) = parse_block_reference("[[Note Name#^blockid]]");
        assert_eq!(note, Some("Note Name".to_string()));
        assert_eq!(block, Some("blockid".to_string()));

        let (note, block) = parse_block_reference("[[Note Name]]");
        assert_eq!(note, Some("Note Name".to_string()));
        assert_eq!(block, None);

        let (note, block) = parse_block_reference("[[#^blockid]]");
        assert_eq!(note, None);
        assert_eq!(block, Some("blockid".to_string()));

        let (note, block) = parse_block_reference("[[Note Name#Heading]]");
        assert_eq!(note, Some("Note Name".to_string()));
        assert_eq!(block, Some("Heading".to_string()));
    }

    #[test]
    fn test_anchor_determinism() {
        let body = "Some content.";
        let anchors1 = extract_block_anchors(body, "note-1");
        let anchors2 = extract_block_anchors(body, "note-1");
        assert_eq!(anchors1.len(), anchors2.len());
        assert_eq!(anchors1[0].block_id, anchors2[0].block_id);
    }

    #[test]
    fn test_code_block_anchor() {
        let body = "Some text.\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n\nMore text.\n";
        let anchors = extract_block_anchors(body, "note-1");
        let code_blocks: Vec<_> = anchors.iter().filter(|a| a.block_type == "code_block").collect();
        assert_eq!(code_blocks.len(), 1, "expected 1 code block anchor");
        assert!(code_blocks[0].content.contains("fn main()"));
        assert_eq!(code_blocks[0].line_start, 2);
    }

    #[test]
    fn test_frontmatter_skipped() {
        let body = "---\ntitle: Test\ntags: [a, b]\n---\n\n# Real content\n\nParagraph.\n";
        let anchors = extract_block_anchors(body, "note-1");
        // First anchor should be heading1, not frontmatter
        assert!(anchors[0].block_type == "heading1" || anchors[0].block_type == "paragraph");
        // There should be no frontmatter-derived anchors
        assert!(anchors.iter().all(|a| a.block_id != "title"));
    }

    #[test]
    fn test_update_and_get_block_anchor() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // Create block_anchors table without FK constraint for in-memory test
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS block_anchors (
                note_id TEXT NOT NULL,
                block_id TEXT NOT NULL,
                block_type TEXT NOT NULL DEFAULT 'paragraph',
                content TEXT NOT NULL DEFAULT '',
                line_start INTEGER NOT NULL DEFAULT 0,
                line_end INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (note_id, block_id)
            );
            CREATE INDEX IF NOT EXISTS idx_block_anchors_note_id ON block_anchors(note_id);"
        ).unwrap();

        let body = "# Title\n\nContent here.\n";
        update_block_anchors(&conn, "test-note", body).unwrap();

        let retrieved = get_block_anchor(&conn, "test-note", "title").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().block_type, "heading1");

        let all = get_block_anchors_for_note(&conn, "test-note").unwrap();
        assert_eq!(all.len(), 2); // heading + paragraph
    }
}
