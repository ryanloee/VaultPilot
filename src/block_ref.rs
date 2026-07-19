//! # Block-level references (Logseq-style, #2993)
//!
//! VaultPilot's Daily Journal workflow (#2993) needs the second pillar of the
//! Logseq model: **block-level references**. Every paragraph, list item and
//! heading is addressable by a stable block id, so it can be embedded
//! (`![[note#^blockid]]`) and back-linked independently of the note it lives in.
//!
//! Design goals:
//! * **Stable ids.** A block id is derived from the (trimmed) block text and its
//!   heading path, *not* from line position. Inserting a paragraph elsewhere in
//!   the note does not reshuffle every other block's id.
//! * **Local-markdown compatible.** Block ids are stored as trailing HTML
//!   comments (`... <!-- ^blockid -->`) so they round-trip through plain editors
//!   and do not break existing frontmatter parsing.
//! * **Embed resolution.** `![[note#^blockid]]` is resolved to the referenced
//!   block's text and reported for backlink indexing.

use std::collections::HashMap;
use std::fmt::Write as _;

const BLOCK_ID_PREFIX: &str = "^";

/// A single addressable block extracted from a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Stable, position-independent id (without the leading `^`).
    pub id: String,
    /// Heading path the block sits under, e.g. `["End of Day Reflection"]`.
    pub heading_path: Vec<String>,
    /// The trimmed block text (original, without any trailing block-id comment).
    pub text: String,
    /// 0-based index of the block in document order.
    pub index: usize,
}

/// Compute a stable block id from its canonical content.
///
/// The id is a short, url-safe hex of a 64-bit **FNV-1a** hash over the
/// heading path joined with `\0` and the trimmed text. Deterministic across
/// runs, platforms, and Rust compiler versions; collision probability is
/// negligible for note-scale text.
///
/// ## Why FNV-1a and not `std::collections::hash_map::DefaultHasher`?
///
/// `DefaultHasher`'s algorithm is explicitly **unspecified** per the Rust std
/// docs ("the algorithm ... is not specified"), so block ids computed with it
/// could silently drift after a Rust toolchain upgrade — orphaning every
/// `![[note#^blockid]]` reference in a vault. FNV-1a is a fixed, documented
/// algorithm we control byte-for-byte, so ids stay stable forever. This
/// mirrors the same fix already applied in #3160 (`semantic::stable_hash`),
/// #3166 (`utils::slugify`), and #3169 (`agent::slugify`).
///
/// Note: this function alone only guarantees a *content-derived* id. When the
/// same `(heading_path, text)` appears more than once in a single note (e.g.
/// two identical `- [ ] reply to mail` list items in a Daily Journal), the
/// base id collides. Callers must run the result through
/// [`disambiguate_block_id`] to guarantee document-wide uniqueness (#2998).
fn block_id_for(heading_path: &[String], text: &str) -> String {
    // FNV-1a 64-bit — fixed algorithm (RFC-style), unlike DefaultHasher.
    // Same constants as `semantic::stable_hash` for consistency.
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    // Each heading is folded as: length-prefix (as u64 byte) followed by its
    // UTF-8 bytes. The length prefix makes the encoding unambiguous:
    //   (["a","b"])   -> 1, 'a', 1, 'b'
    //   (["a\0b"])    -> 3, 'a', 0, 'b'
    // which differ, unlike a naive separator scheme (a single 0x00 separator
    // after each heading would collide with embedded NULs).
    for h in heading_path {
        let len = h.len() as u64;
        hash ^= len;
        hash = hash.wrapping_mul(FNV_PRIME);
        for byte in h.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    // Length-prefix the text too so trailing empty text is distinguishable
    // from no text call site (defensive; current callers always pass text).
    let text_len = text.len() as u64;
    hash ^= text_len;
    hash = hash.wrapping_mul(FNV_PRIME);
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:x}", hash)
}

/// Disambiguate block ids so that every block in a single note has a globally
/// unique id, even when the same `(heading_path, text)` is repeated.
///
/// The *first* occurrence of a given content id keeps the bare id (preserving
/// inter-document stability for the common case). Each subsequent occurrence of
/// the same content gets a `-N` suffix where `N` is the 0-based repeat index
/// (`-1`, `-2`, ...). This means inserting unrelated text elsewhere in the note
/// never reshuffles already-unique ids, while deterministic duplicate text is no
/// longer merged into a single ambiguous anchor (#2998).
struct IdDisambiguator {
    seen: HashMap<String, usize>,
}

impl IdDisambiguator {
    fn new() -> Self {
        IdDisambiguator {
            seen: HashMap::new(),
        }
    }

    fn disambiguate(&mut self, base_id: String) -> String {
        let entry = self.seen.entry(base_id.clone()).or_insert(0);
        let count = *entry;
        *entry += 1;
        if count == 0 {
            base_id
        } else {
            format!("{}-{count}", base_id)
        }
    }
}

/// Strip a trailing `<!-- ^id -->` block-id marker (if present) from a line.
fn strip_block_id_marker(line: &str) -> &str {
    let trimmed = line.trim_end();
    if let Some(idx) = trimmed.find("<!--") {
        let tail = &trimmed[idx..];
        let tail = tail.trim();
        if let Some(stripped) = tail
            .strip_prefix("<!--")
            .and_then(|s| s.strip_suffix("-->"))
        {
            let inner = stripped.trim();
            if inner.starts_with(BLOCK_ID_PREFIX) && inner.len() > 1 {
                return trimmed[..idx].trim_end();
            }
        }
    }
    line
}

/// True if `line` carries a trailing block-id marker.
fn has_block_id_marker(line: &str) -> bool {
    let trimmed = line.trim_end();
    if let Some(idx) = trimmed.find("<!--") {
        let tail = trimmed[idx..].trim();
        if let Some(stripped) = tail
            .strip_prefix("<!--")
            .and_then(|s| s.strip_suffix("-->"))
        {
            let inner = stripped.trim();
            return inner.starts_with(BLOCK_ID_PREFIX) && inner.len() > 1;
        }
    }
    false
}

/// Split a markdown body into blocks (headings, paragraphs, list items),
/// preserving any pre-existing block-id markers and assigning ids to blocks
/// that lack them.
///
/// Fenced code blocks (```) are treated as a single opaque block so their
/// content is never re-id'd line-by-line.
pub fn parse_blocks(body: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut heading_path: Vec<String> = Vec::new();
    let mut in_code = false;
    // Pending prose/paragraph buffer: (heading_path, text).
    let mut current: Option<(Vec<String>, String)> = None;
    let mut index = 0usize;

    // Push a non-heading block (paragraph / list item / code) into `blocks`.
    let flush = |hp: &[String], buf: &str, blocks: &mut Vec<Block>, index: &mut usize| {
        let text = buf.trim();
        if text.is_empty() {
            return;
        }
        blocks.push(Block {
            id: block_id_for(hp, text),
            heading_path: hp.to_vec(),
            text: text.to_string(),
            index: *index,
        });
        *index += 1;
    };

    // Push a heading block and update the running heading path.
    let push_heading =
        |hp: Vec<String>, level: usize, title: &str, blocks: &mut Vec<Block>, index: &mut usize| {
            let text = format!(
                "{}{}{}",
                "#".repeat(level),
                if level > 0 { " " } else { "" },
                title
            );
            blocks.push(Block {
                id: block_id_for(&hp, title),
                heading_path: hp,
                text,
                index: *index,
            });
            *index += 1;
        };

    // True for the first line of a list item or blockquote.
    let is_list_or_quote_start = |line: &str| -> bool {
        let t = line.trim_start();
        if t.starts_with(">") {
            return true;
        }
        // unordered: -, *, + followed by space
        if let Some(rest) = t
            .strip_prefix('-')
            .or_else(|| t.strip_prefix('*'))
            .or_else(|| t.strip_prefix('+'))
        {
            return rest.starts_with(' ') || rest.is_empty();
        }
        // ordered: 1. 2) etc.
        if let Some(dot) = t.find(['.', ')']) {
            let num = &t[..dot];
            if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
        false
    };

    for raw_line in body.lines() {
        let line = raw_line;
        let fence = line.trim_start().starts_with("```");
        if fence {
            if in_code {
                // Close the fence: flush the accumulated code block.
                if let Some((hp, buf)) = current.take() {
                    blocks.push(Block {
                        id: block_id_for(&hp, buf.trim()),
                        heading_path: hp,
                        text: buf.trim().to_string(),
                        index,
                    });
                    index += 1;
                }
                in_code = false;
                continue;
            } else {
                // Open a fence: flush any pending prose first.
                if let Some((hp, buf)) = current.take() {
                    flush(&hp, &buf, &mut blocks, &mut index);
                }
                in_code = true;
                current = Some((heading_path.clone(), String::new()));
                continue;
            }
        }

        if in_code {
            let buf = &mut current.as_mut().unwrap().1;
            writeln!(buf, "{line}").ok();
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Blank line ends the current block.
            if let Some((hp, buf)) = current.take() {
                flush(&hp, &buf, &mut blocks, &mut index);
            }
            continue;
        }

        // Heading: its own block, also updates heading_path for subsequent blocks.
        if trimmed.starts_with('#') {
            if let Some((hp, buf)) = current.take() {
                flush(&hp, &buf, &mut blocks, &mut index);
            }
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            // Strip any pre-existing block-id marker before computing the title.
            let title = strip_block_id_marker(&trimmed[level..]).trim().to_string();
            let mut hp = heading_path.clone();
            hp.truncate(level.saturating_sub(1));
            hp.push(title.clone());
            heading_path = hp.clone();
            push_heading(hp, level, &title, &mut blocks, &mut index);
            continue;
        }

        // Strip any existing trailing block-id marker.
        let clean = strip_block_id_marker(line).to_string();

        // Start a new block when:
        //  - nothing pending yet, OR
        //  - this line begins a new list item / blockquote (each is its own block), OR
        //  - the pending block is a list item / blockquote but this line is plain
        //    prose (paragraph boundary), OR
        //  - this line is a continuation (indented) of the current block.
        let start_new = match &current {
            None => true,
            Some((_, buf)) => {
                let first = buf.lines().next().unwrap_or("");
                let prev_is_list = is_list_or_quote_start(first);
                let cur_is_list = is_list_or_quote_start(line);
                // A new list/quote item always starts a fresh block.
                if cur_is_list {
                    true
                } else if prev_is_list {
                    // Previous was a list/quote item and this is plain prose → new block.
                    true
                } else {
                    // Both prose: continuation only if indented (wrapped text).
                    line.starts_with(' ') || line.starts_with('\t')
                }
            }
        };

        if start_new {
            if let Some((hp, buf)) = current.take() {
                flush(&hp, &buf, &mut blocks, &mut index);
            }
            current = Some((heading_path.clone(), clean));
        } else {
            let buf = &mut current.as_mut().unwrap().1;
            buf.push('\n');
            buf.push_str(clean.trim());
        }
    }
    if let Some((hp, buf)) = current.take() {
        flush(&hp, &buf, &mut blocks, &mut index);
    }

    // Guarantee document-wide unique ids: when the same (heading_path, text)
    // appears more than once the bare content-derived id collides, so append a
    // `-N` repeat suffix (#2998). The first occurrence keeps the bare id.
    let mut disambig = IdDisambiguator::new();
    for block in blocks.iter_mut() {
        block.id = disambig.disambiguate(block.id.clone());
    }

    blocks
}

/// Annotate a markdown body so every block carries a stable `<!-- ^id -->`
/// marker. Idempotent: blocks that already have a marker keep it; the rest get
/// one appended. Headings and fenced code blocks are annotated on their own
/// line. Returns the rewritten body.
pub fn annotate_blocks(body: &str) -> String {
    annotate_blocks_grouped(body)
}

/// Group-aware annotation (used by [`annotate_blocks`]).
///
/// Reuses the same block segmentation as [`parse_blocks`] (including the running
/// `heading_path`) so the emitted ids are identical to those `parse_blocks`
/// would assign — keeping annotations stable across re-runs and resolvable by
/// `extract_block_embeds` / `resolve_embeds`.
fn annotate_blocks_grouped(body: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut heading_path: Vec<String> = Vec::new();
    let mut block_lines: Vec<&str> = Vec::new();
    let mut disambig = IdDisambiguator::new();

    let flush_block = |block_lines: &[&str],
                       heading_path: &[String],
                       out: &mut String,
                       disambig: &mut IdDisambiguator| {
        if block_lines.is_empty() {
            return;
        }
        // Reconstruct the block text the way parse_blocks would.
        let mut text = String::new();
        for (i, l) in block_lines.iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            text.push_str(strip_block_id_marker(l));
        }
        let text = text.trim();
        // Heading?
        if let Some(rest) = text.strip_prefix('#') {
            let level = rest.chars().take_while(|c| *c == '#').count();
            let title = rest[level..].trim();
            let mut hp = heading_path.to_vec();
            hp.truncate(level.saturating_sub(1));
            hp.push(title.to_string());
            let id = disambig.disambiguate(block_id_for(&hp, title));
            let _ = writeln!(out, "{} <!-- {}{} -->", text, BLOCK_ID_PREFIX, id);
        } else if block_lines.len() == 1 && has_block_id_marker(block_lines[0]) {
            // Already annotated — keep as-is.
            let _ = writeln!(out, "{}", block_lines[0].trim_end());
        } else {
            let id = disambig.disambiguate(block_id_for(heading_path, text));
            let _ = writeln!(out, "{} <!-- {}{} -->", text, BLOCK_ID_PREFIX, id);
        }
    };

    for raw_line in body.lines() {
        let line = raw_line;
        let fence = line.trim_start().starts_with("```");
        if fence {
            flush_block(&block_lines, &heading_path, &mut out, &mut disambig);
            block_lines.clear();
            let _ = writeln!(out, "{line}");
            in_code = !in_code;
            continue;
        }
        if in_code {
            let _ = writeln!(out, "{line}");
            continue;
        }
        if line.trim().is_empty() {
            flush_block(&block_lines, &heading_path, &mut out, &mut disambig);
            block_lines.clear();
            let _ = writeln!(out);
            continue;
        }
        // Heading is a block of its own; update the running heading path.
        if line.trim_start().starts_with('#') {
            flush_block(&block_lines, &heading_path, &mut out, &mut disambig);
            block_lines.clear();
            let text = line.trim();
            let level = text.chars().take_while(|c| *c == '#').count();
            let title = text[level..].trim();
            let mut hp = heading_path.clone();
            hp.truncate(level.saturating_sub(1));
            hp.push(title.to_string());
            heading_path = hp.clone();
            let id = disambig.disambiguate(block_id_for(&hp, title));
            let _ = writeln!(out, "{} <!-- {}{} -->", text, BLOCK_ID_PREFIX, id);
            continue;
        }
        block_lines.push(line);
    }
    flush_block(&block_lines, &heading_path, &mut out, &mut disambig);
    out
}

/// A resolved block embed `![[note#^blockid]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEmbed {
    /// Target note id (everything before `#`).
    pub note_id: String,
    /// The referenced block id (without `^`).
    pub block_id: String,
}

/// Extract all `![[note#^blockid]]` embeds from a body.
pub fn extract_block_embeds(body: &str) -> Vec<BlockEmbed> {
    let mut embeds = Vec::new();
    let mut pos = 0;
    while let Some(start) = body[pos..].find("![[") {
        let abs = pos + start;
        if let Some(end_rel) = body[abs + 3..].find("]]") {
            let end = abs + 3 + end_rel;
            let inner = &body[abs + 3..end];
            pos = end + 2;
            // Must contain "#^".
            if let Some((note, block_with_marker)) = inner.split_once('#') {
                let block = block_with_marker.trim_start_matches(BLOCK_ID_PREFIX);
                if !block.is_empty() && !note.trim().is_empty() {
                    embeds.push(BlockEmbed {
                        note_id: note.trim().to_string(),
                        block_id: block.to_string(),
                    });
                }
            }
        } else {
            break;
        }
    }
    embeds
}

/// Look up a block by id within a parsed set.
pub fn find_block_by_id<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    blocks.iter().find(|b| b.id == id)
}

/// Resolve embeds against a map of note_id -> body, returning the embed source
/// paired with the resolved block text (empty string if not found).
pub fn resolve_embeds(body: &str, notes: &HashMap<String, String>) -> Vec<(BlockEmbed, String)> {
    let embeds = extract_block_embeds(body);
    let mut resolved = Vec::new();
    for embed in embeds {
        let text = notes
            .get(&embed.note_id)
            .and_then(|b| {
                let blocks = parse_blocks(b);
                find_block_by_id(&blocks, &embed.block_id).map(|bl| bl.text.clone())
            })
            .unwrap_or_default();
        resolved.push((embed, text));
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_id_for_is_fnv1a_deterministic_3173() {
        // #3173 regression: block_id_for must be deterministic across runs,
        // platforms, and Rust compiler versions. The previous implementation
        // used std DefaultHasher whose algorithm is unspecified, risking
        // silent id drift. We pin known FNV-1a values so any future change
        // to the hash constants / algorithm is caught here.
        //
        // Reference values computed by an independent Python FNV-1a impl that
        // mirrors this function's length-prefix encoding.
        let id = block_id_for(&[], "Hello");
        assert_eq!(id, "f37d64b8ed429e18");

        // Empty content folds just the length-prefix byte (0). Not equal to
        // the bare FNV offset basis anymore.
        assert_eq!(block_id_for(&[], ""), "af63bd4c8601b7df");

        // Same input must always produce the same id (regression guard).
        assert_eq!(block_id_for(&[], "Hello"), id);

        // Different inputs must produce different ids.
        let id3 = block_id_for(&[], "Hello!");
        assert_ne!(id, id3);
        assert_eq!(id3, "ede81631820c462");
    }

    #[test]
    fn block_id_for_heading_path_separators_3173() {
        // Length-prefix encoding guarantees (["a","b"]) ≠ (["a\0b"]) — the
        // former encodes as 1,'a',1,'b' while the latter is 3,'a',0,'b'.
        let ab = block_id_for(&["a".to_string(), "b".to_string()], "x");
        let a_null_b = block_id_for(&["a\0b".to_string()], "x");
        assert_ne!(ab, a_null_b);
        assert_eq!(ab, "1be9d399fe4afb1");
        assert_eq!(a_null_b, "31db0044b1d433f2");
    }

    #[test]
    fn block_id_for_unicode_safe_3173() {
        // FNV-1a operates on UTF-8 bytes, so multi-byte content (CJK, emoji)
        // is handled byte-wise — no panics on character boundaries.
        let id = block_id_for(&["日记".to_string()], "今天的工作总结 🚀");
        assert!(!id.is_empty());
        // Deterministic.
        assert_eq!(id, block_id_for(&["日记".to_string()], "今天的工作总结 🚀"));
    }

    #[test]
    fn parse_blocks_assigns_stable_ids() {
        let body = "# Day\n\nFirst paragraph.\n\n- item one\n- item two\n";
        let blocks = parse_blocks(body);
        // heading + paragraph + 2 list items = 4 blocks
        assert_eq!(blocks.len(), 4);
        // Ids are deterministic and non-empty.
        let ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.iter().all(|i| !i.is_empty()));
        // No two different blocks share an id here.
        assert_eq!(
            ids.len(),
            ids.iter().collect::<std::collections::HashSet<_>>().len()
        );
    }

    #[test]
    fn block_id_stable_across_insertion() {
        let a = parse_blocks("# Day\n\nAlpha.\n\nBeta.\n");
        let b = parse_blocks("# Day\n\nGamma inserted.\n\nAlpha.\n\nBeta.\n");
        let alpha_a = a.iter().find(|bl| bl.text == "Alpha.").unwrap();
        let alpha_b = b.iter().find(|bl| bl.text == "Alpha.").unwrap();
        // Inserting a paragraph elsewhere must not change Alpha's id.
        assert_eq!(alpha_a.id, alpha_b.id);
    }

    #[test]
    fn heading_path_tracked() {
        let body = "# Top\n\npara\n\n## Sub\n\nchild\n";
        let blocks = parse_blocks(body);
        let child = blocks.iter().find(|b| b.text == "child").unwrap();
        assert_eq!(
            child.heading_path,
            vec!["Top".to_string(), "Sub".to_string()]
        );
    }

    #[test]
    fn fenced_code_is_single_block() {
        let body = "```\nline1\nline2\n```\n\nafter\n";
        let blocks = parse_blocks(body);
        // code block + "after" = 2 blocks, not 3.
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].text.contains("line1"));
        assert!(blocks[0].text.contains("line2"));
    }

    #[test]
    fn annotate_is_idempotent() {
        let body = "Hello world.\n\nSecond block.\n";
        let once = annotate_blocks(body);
        let twice = annotate_blocks(&once);
        assert_eq!(once, twice);
        // Both blocks now carry markers.
        assert_eq!(once.lines().filter(|l| l.contains("<!-- ^")).count(), 2);
    }

    #[test]
    fn annotate_ids_match_parse_blocks() {
        // The ids emitted by annotate_blocks must be resolvable by parse_blocks,
        // so a `![[note#^id]]` reference survives a round-trip.
        let body = "# Top\n\npara under top.\n\n## Sub\n\nchild block.\n\n- list one\n- list two\n";
        let annotated = annotate_blocks(body);
        let parsed = parse_blocks(&annotated);
        // Every annotated id should appear in the parsed set.
        for line in annotated.lines() {
            if let Some(marker) = line.trim().strip_prefix("<!-- ^") {
                let id = marker.trim_end_matches("-->").trim();
                assert!(
                    parsed.iter().any(|b| b.id == id),
                    "annotated id {id} not found in parsed blocks"
                );
            }
        }
        // And parse_blocks on the annotated text yields the same ids as on the
        // original (idempotency of block identity).
        let orig = parse_blocks(body);
        let original_ids: Vec<String> = orig.iter().map(|b| b.id.clone()).collect();
        let repro_ids: Vec<String> = parsed.iter().map(|b| b.id.clone()).collect();
        assert_eq!(original_ids.len(), repro_ids.len());
        for (a, b) in original_ids.iter().zip(repro_ids.iter()) {
            assert_eq!(a, b, "block id changed after annotation");
        }
    }

    #[test]
    fn extract_and_resolve_embed() {
        let target = "NoteA";
        let target_body = "Unique sentence here.\n\nOther.\n";
        let blocks = parse_blocks(target_body);
        let id = blocks
            .iter()
            .find(|b| b.text == "Unique sentence here.")
            .unwrap()
            .id
            .clone();

        let source = format!("See ![[NoteA#^{}]] for detail.", id);
        let embeds = extract_block_embeds(&source);
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0].note_id, "NoteA");
        assert_eq!(embeds[0].block_id, id);

        let mut notes = HashMap::new();
        notes.insert(target.to_string(), target_body.to_string());
        let resolved = resolve_embeds(&source, &notes);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].1, "Unique sentence here.");
    }

    #[test]
    fn embed_to_missing_note_resolves_empty() {
        let source = "ref ![[Ghost#^abc123]]";
        let notes = HashMap::new();
        let resolved = resolve_embeds(source, &notes);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].1, "");
    }

    #[test]
    fn strip_marker_removes_trailing_id() {
        let line = "Some text <!-- ^deadbeef -->";
        assert_eq!(strip_block_id_marker(line), "Some text");
        assert!(has_block_id_marker(line));
    }

    #[test]
    fn duplicate_blocks_get_unique_ids() {
        // Two identical content blocks under the same heading must not share an id
        // (#2998): embedding `![[note#^id]]` would otherwise be ambiguous.
        // Blank lines keep each list item its own block in parse_blocks.
        let body = "## Tasks\n\n- [ ] reply to mail\n\n- [ ] reply to mail\n";
        let blocks = parse_blocks(body);
        // heading + 2 list items (separated by blank lines)
        assert_eq!(blocks.len(), 3, "expected 3 blocks, got {blocks:?}");
        let ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            "duplicate content blocks collided on the same id"
        );
        // The two list items should carry distinct ids.
        let item_ids: Vec<&str> = blocks
            .iter()
            .filter(|b| b.text == "- [ ] reply to mail")
            .map(|b| b.id.as_str())
            .collect();
        assert_eq!(item_ids.len(), 2);
        assert_ne!(item_ids[0], item_ids[1]);
    }

    #[test]
    fn annotate_disambiguates_duplicate_blocks() {
        // annotate_blocks must emit resolved, unique ids that parse_blocks can
        // resolve back to the correct duplicate block (#2998). Blank lines keep
        // each list item its own block.
        let body = "## Tasks\n\n- [ ] reply to mail\n\n- [ ] reply to mail\n";
        let annotated = annotate_blocks(body);
        let ids: Vec<&str> = annotated
            .lines()
            .filter_map(|l| l.find("<!-- ^").map(|i| &l[i + 5..]))
            .map(|m| m.trim_end_matches("-->").trim())
            .collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(
            ids.len(),
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            "annotate produced duplicate block ids"
        );
        // And those ids must be resolvable by parse_blocks without collision.
        let parsed = parse_blocks(&annotated);
        let resolved_ids: Vec<String> = parsed.iter().map(|b| b.id.clone()).collect();
        assert_eq!(
            resolved_ids.len(),
            resolved_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
    }
}
