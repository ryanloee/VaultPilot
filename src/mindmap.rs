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

    for (line_num, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim_end_matches('\r');

        // Track code fences (``` or ~~~) to skip headings inside code blocks
        let trimmed = line.trim_start_matches(' ');
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_fence = !in_code_fence;
            continue;
        }
        if in_code_fence {
            continue;
        }

        // Must start with '#' (possibly preceded by up to 3 spaces per CommonMark)
        let trimmed_start = line.trim_start_matches(' ');
        if !trimmed_start.starts_with('#') {
            continue;
        }

        // Count leading '#' characters
        let hash_count = trimmed_start.chars().take_while(|&c| c == '#').count();
        // Clamp: 1..=6 per CommonMark ATX spec.  7+ '#' is a paragraph.
        if hash_count == 0 || hash_count > 6 {
            continue;
        }
        let level = hash_count as u8;

        // After the #s, optional space(s), then the heading text
        let after_hashes = &trimmed_start[hash_count..];
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

    /// Serialization round-trip via serde_json.
    #[test]
    fn serialize_deserialize_round_trip() {
        let md = "# Root\n## Child\n### Grandchild\n";
        let nodes = parse_markdown_headings(md);
        let json = serde_json::to_string(&nodes).expect("serialize");
        let parsed: Vec<MindmapNode> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, nodes);
    }
}
