//! Mindmap backend: parse Markdown heading hierarchy into a node tree (#3420).
//!
//! Extracts `#`-prefixed ATX headings (h1-h6) from Markdown text and builds a
//! nested tree suitable for rendering as an interactive mindmap.  This is a
//! pure-Rust, dependency-free parser that works on the raw string content —
//! no full Markdown AST is needed for heading extraction.
//!
//! # Example
//! ```text
//! # Root                     → MindmapNode { level:1, title:"Root", children:[...] }
//! ## Child A                 →   child of Root
//! ### Grandchild             →     child of Child A
//! ## Child B                 →   child of Root
//! ```
//!
//! Non-heading lines (paragraphs, code fences, lists, etc.) are ignored.
//! Setext-style headings (`===` / `---` underlines) are deliberately not
//! supported — they are ambiguous in practice and rarely used in knowledge
//! vault notes.

use serde::{Deserialize, Serialize};

/// A single node in the heading tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MindmapNode {
    /// Heading level: 1 = `#`, 2 = `##`, …, 6 = `######`.
    /// Range is clamped to 1..=6 by the parser.
    pub level: u8,
    /// Heading text after stripping leading `#` chars and surrounding whitespace.
    pub title: String,
    /// 0-based line number in the source Markdown text where this heading starts.
    pub line: usize,
    /// Children of this node — all headings with level > `self.level` that appear
    /// after this heading and before the next heading at `self.level` or higher.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<MindmapNode>,
}

/// Parse `text` (Markdown source) into a forest of top-level heading trees.
///
/// Returns a `Vec<MindmapNode>` containing every h1 (`#`) heading as a root.
/// If the document has no h1 but starts with e.g. h2, those h2 nodes become
/// the roots.  Lower-level headings are nested inside their nearest higher-level
/// ancestor.
///
/// # Algorithm
/// Uses a stack-based approach: maintain a stack of ancestor nodes at each
/// level.  When a new heading arrives, pop the stack until we find a node whose
/// level is *strictly less* than the incoming heading, then push the new node
/// as a child of that ancestor (or as a root if no suitable ancestor exists).
pub fn parse_markdown_headings(text: &str) -> Vec<MindmapNode> {
    let mut roots: Vec<MindmapNode> = Vec::new();
    // Stack of (depth_index, node_ref).  depth_index points into the children
    // chain: stack[0] is a root, stack[1] is a child of stack[0], etc.
    // We can't store references (borrow checker), so we store the node by
    // navigating the tree at insertion time.
    let mut stack: Vec<(u8, usize)> = Vec::new(); // (level, index in parent's children)
    let mut in_code_fence = false;
    let mut fence_char: u8 = 0; // 0 = none, b'`' = backtick, b'~' = tilde
    let mut fence_len: usize = 3;

    for (line_num, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim_end_matches('\r');

        // Count leading spaces — CommonMark allows 0-3 before headings & fence markers
        let leading_spaces = line.chars().take_while(|&c| c == ' ').count();
        let trimmed = &line[leading_spaces..];

        if in_code_fence {
            // Inside a code fence: check for matching closing fence
            // (same fence char, length >= opening length, 0-3 leading spaces)
            if leading_spaces <= 3 {
                if let Some(f_char) = trimmed.chars().next() {
                    if f_char as u8 == fence_char {
                        let count = trimmed.chars().take_while(|&c| c == f_char).count();
                        if count >= fence_len {
                            in_code_fence = false;
                        }
                    }
                }
            }
            continue; // All lines inside a fence are code content
        }

        // Not inside a code fence: 4+ leading spaces = indented code block
        if leading_spaces > 3 {
            continue;
        }

        // Track code fences with correct char and length matching (CommonMark)
        if let Some(f_char) = trimmed.chars().next() {
            if f_char == '`' || f_char == '~' {
                let count = trimmed.chars().take_while(|&c| c == f_char).count();
                if count >= 3 {
                    in_code_fence = true;
                    fence_char = f_char as u8;
                    fence_len = count;
                    continue;
                }
            }
        }

        // Must start with '#' (up to 3 leading spaces already stripped)
        if !trimmed.starts_with('#') {
            continue;
        }

        // Count leading '#' characters
        let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
        // Clamp: 1..=6 per CommonMark ATX spec.  7+ '#' is a paragraph.
        if hash_count == 0 || hash_count > 6 {
            continue;
        }
        let level = hash_count as u8;

        // After the #s, optional space(s), then the heading text
        let after_hashes = &trimmed[hash_count..];
        // CommonMark: at least one space required after #s, unless heading is empty
        let title_raw = if after_hashes.is_empty() {
            String::new()
        } else if after_hashes.starts_with(' ') || after_hashes.starts_with('\t') {
            // Strip leading whitespace and trailing `#` sequences + whitespace
            let inner = after_hashes.trim_start_matches([' ', '\t']);
            strip_trailing_heading_markers(inner)
        } else {
            // No space after #s → not a valid ATX heading (e.g. `##not heading`)
            continue;
        };

        let node = MindmapNode {
            level,
            title: title_raw,
            line: line_num,
            children: Vec::new(),
        };

        // Find insertion point: pop stack until ancestor level < node.level
        while let Some(&(ancestor_level, _)) = stack.last() {
            if ancestor_level < level {
                break;
            }
            stack.pop();
        }

        if stack.is_empty() {
            // No suitable ancestor → this is a root
            let idx = roots.len();
            roots.push(node);
            stack.push((level, idx));
        } else {
            // Insert as child of the last ancestor on the stack.
            // Navigate the tree to find the right children vec.
            let parent_children: &mut Vec<MindmapNode> = if stack.len() == 1 {
                // Parent is a root
                &mut roots[stack[0].1].children
            } else {
                // Navigate through multiple levels
                let mut children = &mut roots[stack[0].1].children;
                for (_, idx) in &stack[1..] {
                    children = &mut children[*idx].children;
                }
                children
            };

            let child_idx = parent_children.len();
            parent_children.push(node);
            stack.push((level, child_idx));
        }
    }

    roots
}

/// Strip optional trailing `#` sequences and whitespace from heading text.
///
/// CommonMark allows `## My heading ##` where trailing `#`s are decorative.
/// Any number of trailing `#`s (not part of a sequence mixing with text)
/// is stripped, along with trailing whitespace.
fn strip_trailing_heading_markers(s: &str) -> String {
    let trimmed = s.trim_end_matches([' ', '\t']);
    // Check for trailing # sequence separated by at least one space
    if let Some(last_space) = trimmed.rfind([' ', '\t']) {
        let after_space = &trimmed[last_space..].trim_start_matches([' ', '\t']);
        if !after_space.is_empty()
            && after_space.chars().all(|c| c == '#')
            && !trimmed[..last_space].ends_with('#')
        {
            return trimmed[..last_space]
                .trim_end_matches([' ', '\t'])
                .to_string();
        }
    }
    trimmed.to_string()
}

/// Output format for the mindmap CLI command (#3430).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum MindmapFormat {
    /// Human-readable indented text tree (default).
    Text,
    /// `MindmapNode` JSON tree (for frontend consumption).
    Json,
    /// Mermaid `mindmap` diagram syntax.
    Mermaid,
}

/// Render the heading tree as a human-readable indented text outline.
///
/// Each node is printed on its own line with an indentation proportional to its
/// depth in the tree.  This is the default output format for `vp mindmap`.
pub fn render_text(nodes: &[MindmapNode]) -> String {
    let mut buf = String::new();
    for root in nodes {
        render_text_node(root, 0, &mut buf);
    }
    // Trim trailing newline
    if buf.ends_with('\n') {
        buf.pop();
    }
    buf
}

fn render_text_node(node: &MindmapNode, depth: usize, buf: &mut String) {
    let indent = "  ".repeat(depth);
    buf.push_str(&format!("{}{}\n", indent, node.title));
    for child in &node.children {
        render_text_node(child, depth + 1, buf);
    }
}

/// Render the heading tree as a [Mermaid `mindmap` diagram](https://mermaid.js.org/syntax/mindmap.html).
///
/// Produces a complete ```` ```mermaid ```` fenced block ready for Markdown embedding.
/// Mermind mindmap syntax uses indentation to indicate nesting.
pub fn render_mermaid(nodes: &[MindmapNode]) -> String {
    let mut buf = String::new();
    buf.push_str("```mermaid\nmindmap\n");
    for root in nodes {
        render_mermaid_node(root, 1, &mut buf);
    }
    buf.push_str("```\n");
    buf
}

fn render_mermaid_node(node: &MindmapNode, depth: usize, buf: &mut String) {
    let indent = "  ".repeat(depth);
    buf.push_str(&format!(
        "{}({})\n",
        indent,
        sanitize_mermaid_label(&node.title)
    ));
    for child in &node.children {
        render_mermaid_node(child, depth + 1, buf);
    }
}

/// Escape characters that are problematic inside Mermaid mindmap node labels.
/// Mermaid uses `()` for rounded nodes; parentheses in the title need stripping
/// to avoid breaking the diagram syntax.
fn sanitize_mermaid_label(s: &str) -> String {
    // Mermaid mindmap node labels can't contain unescaped parentheses or brackets.
    // Replace them with visually similar unicode characters.
    s.replace('(', "（")
        .replace(')', "）")
        .replace('[', "【")
        .replace(']', "】")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify basic heading parsing with no nesting.
    #[test]
    fn single_root() {
        let md = "# Hello\n\nSome paragraph.\n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].level, 1);
        assert_eq!(nodes[0].title, "Hello");
        assert_eq!(nodes[0].line, 0);
        assert!(nodes[0].children.is_empty());
    }

    /// Verify nested headings build correct tree.
    #[test]
    fn nested_headings() {
        let md = "\
# Root
## Child
### Grandchild
## Child B
";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1, "should have one root");
        let root = &nodes[0];
        assert_eq!(root.title, "Root");
        assert_eq!(root.children.len(), 2, "root should have 2 children");

        let child_a = &root.children[0];
        assert_eq!(child_a.title, "Child");
        assert_eq!(child_a.level, 2);
        assert_eq!(child_a.children.len(), 1);

        let grandchild = &child_a.children[0];
        assert_eq!(grandchild.title, "Grandchild");
        assert_eq!(grandchild.level, 3);
        assert!(grandchild.children.is_empty());

        let child_b = &root.children[1];
        assert_eq!(child_b.title, "Child B");
        assert_eq!(child_b.level, 2);
        assert!(child_b.children.is_empty());
    }

    /// h1 → h3 → h2: h2 should be child of h1 (not h3), because h3 is deeper.
    /// This is the standard "h2 closes the h3 section" behaviour.
    #[test]
    fn level_jump_up_closes_subtree() {
        let md = "\
# Root
### Deep
## Shallow
";
        let nodes = parse_markdown_headings(md);
        let root = &nodes[0];
        assert_eq!(root.children.len(), 2);

        // "Deep" is h3 child of h1
        assert_eq!(root.children[0].title, "Deep");
        assert_eq!(root.children[0].level, 3);
        assert!(root.children[0].children.is_empty());

        // "Shallow" is h2 child of h1 (sibling to Deep, not child of Deep)
        assert_eq!(root.children[1].title, "Shallow");
        assert_eq!(root.children[1].level, 2);
        assert!(root.children[1].children.is_empty());
    }

    /// Document with no h1: h2 nodes become roots.
    #[test]
    fn no_h1_h2_becomes_root() {
        let md = "\
## A
### A.1
## B
";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].title, "A");
        assert_eq!(nodes[0].level, 2);
        assert_eq!(nodes[0].children.len(), 1);
        assert_eq!(nodes[0].children[0].title, "A.1");

        assert_eq!(nodes[1].title, "B");
        assert_eq!(nodes[1].level, 2);
        assert!(nodes[1].children.is_empty());
    }

    /// Non-heading lines are ignored.
    #[test]
    fn ignores_non_headings() {
        let md = "\
Some text
```
# code fence heading
```
# Real heading
* list item
";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "Real heading");
    }

    /// `#######` (7 #s) is not a heading → ignored.
    #[test]
    fn seven_hashes_not_heading() {
        let md = "####### not a heading\n# Real\n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "Real");
    }

    /// Trailing `#` markers are stripped.
    #[test]
    fn trailing_hashes_stripped() {
        let md = "## My Section ##\n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes[0].title, "My Section");
    }

    /// `##not heading` — no space after #s → not a heading.
    #[test]
    fn no_space_after_hashes_not_heading() {
        let md = "##not heading\n# Real\n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "Real");
    }

    /// Up to 3 leading spaces before #s is valid (CommonMark).
    #[test]
    fn leading_spaces_allowed() {
        let md = "   # Spaced heading\n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes[0].title, "Spaced heading");
    }

    /// Mixed whitespace (tabs) after #s.
    #[test]
    fn tab_after_hashes() {
        let md = "#\tTab after hash\n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes[0].title, "Tab after hash");
    }

    /// Empty document.
    #[test]
    fn empty_document() {
        let nodes = parse_markdown_headings("");
        assert!(nodes.is_empty());
    }

    /// Only non-heading lines.
    #[test]
    fn no_headings() {
        let nodes = parse_markdown_headings("Just text.\nNo headings here.\n");
        assert!(nodes.is_empty());
    }

    /// Regression: heading with only # and nothing else.
    #[test]
    fn empty_heading_title() {
        let md = "## \n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "");
    }

    /// 4+ leading spaces before `#` is an indented code block, not a heading (#3432 Bug 1).
    #[test]
    fn four_plus_indent_not_heading() {
        let md = "    # Indented code comment\n# Real heading\n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "Real heading");
    }

    /// 4+ leading spaces before ``` is an indented code block, not a fence open (#3432 Bug 1).
    #[test]
    fn four_plus_indent_skips_fence() {
        let md = "    ```\n    # code\n    ```\n# Real heading\n";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "Real heading");
    }

    /// Mismatched fence chars: `~~~` with inner ``` should not toggle (#3432 Bug 2).
    #[test]
    fn mismatched_fence_chars() {
        let md = "\
~~~~
use ```python to open a block
~~~~
# After the fence
";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "After the fence");
    }

    /// Shorter fence does not close a longer one (#3432 Bug 2).
    #[test]
    fn shorter_fence_does_not_close_longer() {
        let md = "\
`````
```
content
`````
# Real heading
";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "Real heading");
    }

    /// Inside a matching fence, headings are correctly skipped.
    #[test]
    fn headings_inside_fence_skipped() {
        let md = "\
# Outer
```
## Inside code
```
# After
";
        let nodes = parse_markdown_headings(md);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].title, "Outer");
        assert_eq!(nodes[1].title, "After");
    }

    /// Serialization round-trip via serde_json.
    #[test]
    fn serialize_deserialize_round_trip() {
        let md = "# Root\n## Child\n### Grandchild\n";
        let nodes = parse_markdown_headings(md);
        let json = serde_json::to_string(&nodes).expect("serialize");
        let parsed: Vec<MindmapNode> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, nodes);
    }

    // ── render_text tests (#3430) ──────────────────────────────────

    /// Text rendering produces an indented outline.
    #[test]
    fn render_text_basic_tree() {
        let md = "# Root\n## Child A\n### Grandchild\n## Child B\n";
        let nodes = parse_markdown_headings(md);
        let text = render_text(&nodes);
        let expected = "Root\n  Child A\n    Grandchild\n  Child B";
        assert_eq!(text, expected);
    }

    /// Text rendering handles multiple roots.
    #[test]
    fn render_text_multiple_roots() {
        let md = "# Root1\n## Sub1\n# Root2\n";
        let nodes = parse_markdown_headings(md);
        let text = render_text(&nodes);
        let expected = "Root1\n  Sub1\nRoot2";
        assert_eq!(text, expected);
    }

    /// Text rendering of empty input is empty string.
    #[test]
    fn render_text_empty() {
        let text = render_text(&[]);
        assert_eq!(text, "");
    }

    // ── render_mermaid tests (#3430) ──────────────────────────────

    /// Mermaid output is well-formed for a simple tree.
    #[test]
    fn render_mermaid_basic_tree() {
        let md = "# Root\n## Child\n### Grandchild\n";
        let nodes = parse_markdown_headings(md);
        let out = render_mermaid(&nodes);
        assert!(out.starts_with("```mermaid\nmindmap\n"));
        assert!(out.ends_with("```\n"));
        assert!(out.contains("  (Root)"), "root should be at depth 1");
        assert!(out.contains("    (Child)"), "child at depth 2");
        assert!(out.contains("      (Grandchild)"), "grandchild at depth 3");
    }

    /// Parentheses in heading titles are sanitized for Mermaid.
    #[test]
    fn render_mermaid_sanitizes_parens() {
        let md = "# API (v2)\n";
        let nodes = parse_markdown_headings(md);
        let out = render_mermaid(&nodes);
        assert!(
            out.contains("（v2）"),
            "parens should be replaced with fullwidth"
        );
        assert!(
            !out.contains("(v2)"),
            "no halfwidth parens should remain in label"
        );
    }

    /// Mermaid output for empty input is just the empty block.
    #[test]
    fn render_mermaid_empty() {
        let out = render_mermaid(&[]);
        assert_eq!(out, "```mermaid\nmindmap\n```\n");
    }

    // ── MindmapFormat enum smoke test ─────────────────────────────

    /// MindmapFormat derives PartialEq and has exactly 3 variants.
    #[test]
    fn mindmap_format_variants() {
        assert_eq!(MindmapFormat::Text, MindmapFormat::Text);
        assert_ne!(MindmapFormat::Text, MindmapFormat::Json);
        assert_ne!(MindmapFormat::Json, MindmapFormat::Mermaid);
    }
}
