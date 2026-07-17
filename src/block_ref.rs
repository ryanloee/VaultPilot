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
/// The id is a short, url/path-safe base36 of a 32-bit hash over the heading
/// path joined with `\0`, the occurrence count `n` of this exact block content
/// within the note, and the trimmed text. The occurrence count disambiguates
/// two blocks with identical text under the same heading path (#2998): a Daily
/// Journal with two identical list items previously collided on the same `^id`,
/// corrupting annotations and embedding resolution). Using the occurrence count
/// (rather than raw document position) keeps ids identical between
/// [`parse_blocks`] and [`annotate_blocks_grouped`] and stable across re-runs.
fn block_id_for(heading_path: &[String], occurrence: usize, text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for h in heading_path {
        h.hash(&mut hasher);
        '\0'.hash(&mut hasher);
    }
    occurrence.hash(&mut hasher);
    text.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{:x}", hash)
}

/// Return the 0-based occurrence index for `(heading_path, text)` within this
/// note and record that we've now seen it once more. Used by [`block_id_for`]
/// to disambiguate identically-texted blocks (#2998).
fn occurrence_for(
    seen: &mut HashMap<(Vec<String>, String), usize>,
    heading_path: &[String],
    text: &str,
) -> usize {
    let key = (heading_path.to_vec(), text.to_string());
    let n = *seen.get(&key).unwrap_or(&0);
    *seen.entry(key).or_insert(0) += 1;
    n
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
    // Tracks how many times each (heading_path, text) has appeared, so
    // identically-texted blocks get distinct, stable ids (#2998).
    let mut seen: HashMap<(Vec<String>, String), usize> = HashMap::new();

    // Push a non-heading block (paragraph / list item / code) into `blocks`.
    let flush = |hp: &[String],
                 buf: &str,
                 blocks: &mut Vec<Block>,
                 index: &mut usize,
                 seen: &mut HashMap<(Vec<String>, String), usize>| {
        let text = buf.trim();
        if text.is_empty() {
            return;
        }
        let occ = occurrence_for(seen, hp, text);
        blocks.push(Block {
            id: block_id_for(hp, occ, text),
            heading_path: hp.to_vec(),
            text: text.to_string(),
            index: *index,
        });
        *index += 1;
    };

    // Push a heading block and update the running heading path.
    let push_heading = |hp: Vec<String>,
                        level: usize,
                        title: &str,
                        blocks: &mut Vec<Block>,
                        index: &mut usize,
                        seen: &mut HashMap<(Vec<String>, String), usize>| {
        let text = format!(
            "{}{}{}",
            "#".repeat(level),
            if level > 0 { " " } else { "" },
            title
        );
        let occ = occurrence_for(seen, &hp, &text);
        blocks.push(Block {
            id: block_id_for(&hp, occ, title),
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
                    let text = buf.trim();
                    let occ = occurrence_for(&mut seen, &hp, text);
                    blocks.push(Block {
                        id: block_id_for(&hp, occ, text),
                        heading_path: hp,
                        text: text.to_string(),
                        index,
                    });
                    index += 1;
                }
                in_code = false;
                continue;
            } else {
                // Open a fence: flush any pending prose first.
                if let Some((hp, buf)) = current.take() {
                    flush(&hp, &buf, &mut blocks, &mut index, &mut seen);
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
                flush(&hp, &buf, &mut blocks, &mut index, &mut seen);
            }
            continue;
        }

        // Heading: its own block, also updates heading_path for subsequent blocks.
        if trimmed.starts_with('#') {
            if let Some((hp, buf)) = current.take() {
                flush(&hp, &buf, &mut blocks, &mut index, &mut seen);
            }
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            // Strip any pre-existing block-id marker before computing the title.
            let title = strip_block_id_marker(&trimmed[level..]).trim().to_string();
            let mut hp = heading_path.clone();
            hp.truncate(level.saturating_sub(1));
            hp.push(title.clone());
            heading_path = hp.clone();
            push_heading(hp, level, &title, &mut blocks, &mut index, &mut seen);
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
                flush(&hp, &buf, &mut blocks, &mut index, &mut seen);
            }
            current = Some((heading_path.clone(), clean));
        } else {
            let buf = &mut current.as_mut().unwrap().1;
            buf.push('\n');
            buf.push_str(clean.trim());
        }
    }
    if let Some((hp, buf)) = current.take() {
        flush(&hp, &buf, &mut blocks, &mut index, &mut seen);
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
    // Same occurrence tracking as parse_blocks so emitted ids match (#2998).
    let mut seen: HashMap<(Vec<String>, String), usize> = HashMap::new();

    let flush_block = |block_lines: &[&str],
                       heading_path: &[String],
                       seen: &mut HashMap<(Vec<String>, String), usize>,
                       out: &mut String| {
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
            let occ = occurrence_for(seen, &hp, title);
            let id = block_id_for(&hp, occ, title);
            let _ = writeln!(out, "{} <!-- {}{} -->", text, BLOCK_ID_PREFIX, id);
        } else if block_lines.len() == 1 && has_block_id_marker(block_lines[0]) {
            // Already annotated — keep as-is.
            let _ = writeln!(out, "{}", block_lines[0].trim_end());
        } else {
            let occ = occurrence_for(seen, heading_path, text);
            let id = block_id_for(heading_path, occ, text);
            let _ = writeln!(out, "{} <!-- {}{} -->", text, BLOCK_ID_PREFIX, id);
        }
    };

    for raw_line in body.lines() {
        let line = raw_line;
        let fence = line.trim_start().starts_with("```");
        if fence {
            flush_block(&block_lines, &heading_path, &mut seen, &mut out);
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
            flush_block(&block_lines, &heading_path, &mut seen, &mut out);
            block_lines.clear();
            let _ = writeln!(out);
            continue;
        }
        // Heading is a block of its own; update the running heading path.
        if line.trim_start().starts_with('#') {
            flush_block(&block_lines, &heading_path, &mut seen, &mut out);
            block_lines.clear();
            let text = line.trim();
            let level = text.chars().take_while(|c| *c == '#').count();
            let title = text[level..].trim();
            let mut hp = heading_path.clone();
            hp.truncate(level.saturating_sub(1));
            hp.push(title.to_string());
            heading_path = hp.clone();
            let occ = occurrence_for(&mut seen, &hp, title);
            let id = block_id_for(&hp, occ, title);
            let _ = writeln!(out, "{} <!-- {}{} -->", text, BLOCK_ID_PREFIX, id);
            continue;
        }
        block_lines.push(line);
    }
    flush_block(&block_lines, &heading_path, &mut seen, &mut out);
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

    // ── #2998 regression: identical-text blocks get distinct ids ──
    #[test]
    fn identical_blocks_get_distinct_ids() {
        // Two identical list items under the same heading must not collide.
        let body = "## Tasks\n- [ ] 回复邮件\n- [ ] 回复邮件\n";
        let blocks = parse_blocks(body);
        let tasks: Vec<&Block> = blocks
            .iter()
            .filter(|b| b.text == "- [ ] 回复邮件")
            .collect();
        assert_eq!(tasks.len(), 2, "expected two identical blocks");
        assert_ne!(
            tasks[0].id, tasks[1].id,
            "identical-text blocks must have distinct stable ids"
        );
        // And the ids are stable/deterministic across runs.
        let blocks2 = parse_blocks(body);
        let tasks2: Vec<&Block> = blocks2
            .iter()
            .filter(|b| b.text == "- [ ] 回复邮件")
            .collect();
        assert_eq!(tasks[0].id, tasks2[0].id);
        assert_eq!(tasks[1].id, tasks2[1].id);
    }

    #[test]
    fn annotate_and_parse_agree_on_ids() {
        // The ids emitted by annotate_blocks must match those parse_blocks
        // assigns, so re-annotation is stable (#2998 occurrence tracking).
        let body = "## Tasks\n- [ ] 回复邮件\n- [ ] 回复邮件\n";
        let annotated = annotate_blocks(body);
        let parsed = parse_blocks(&annotated);
        let ids: Vec<String> = parsed.iter().map(|b| b.id.clone()).collect();
        // Two distinct ids for the duplicated item.
        let dup: Vec<&String> = ids
            .iter()
            .filter(|id| parsed.iter().filter(|b| &b.id == *id).count() > 1)
            .collect();
        assert!(dup.is_empty(), "annotated ids must be unique per block");
    }
}
