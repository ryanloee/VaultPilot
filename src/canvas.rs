//! Canvas — free-form infinite whiteboard stored as Obsidian-compatible
//! `.canvas` JSON files inside the vault (#3000).
//!
//! Unlike the auto-generated knowledge graph (`crate::knowledge_graph`), a
//! canvas is a user-authored layout: nodes (notes, text, images, links)
//! manually placed at x/y coordinates with optional edges between them.
//!
//! This module provides the **backend foundation**: parsing, validation,
//! serialization, vault-wide discovery, and a Markdown outline export. The
//! interactive editor UI (WinUI/Mobile) is tracked separately; the CLI
//! (`vp canvas list|show|export`) exposes the same primitives so they can
//! be scripted and tested without a UI.
//!
//! File format
//! -----------
//! The format intentionally mirrors Obsidian Canvas
//! (<https://help.obsidian.md/Canvas>) so `.canvas` files can be exchanged
//! between tools. Unknown fields are preserved on round-trip via
//! `#[serde(flatten)]` catch-alls.
//!
//! ```jsonc
//! {
//!   "nodes": [
//!     { "id": "n1", "type": "text",
//!       "x": 0, "y": 0, "width": 250, "height": 60,
//!       "text": "# Hello\nWorld" },
//!     { "id": "n2", "type": "file",
//!       "x": 300, "y": 0, "width": 400, "height": 400,
//!       "file": "notes/project-plan.md" }
//!   ],
//!   "edges": [
//!     { "id": "e1", "fromNode": "n1", "toNode": "n2", "label": "plans" }
//!   ]
//! }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable file extension for canvas documents in the vault.
pub const CANVAS_EXT: &str = "canvas";

/// A complete canvas document — the on-disk `.canvas` JSON payload.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CanvasFile {
    /// All nodes placed on the canvas.
    #[serde(default)]
    pub nodes: Vec<CanvasNode>,
    /// All edges between nodes.
    #[serde(default)]
    pub edges: Vec<CanvasEdge>,
    /// Catch-all for forward-compatible fields (Obsidian sometimes adds
    /// metadata like `"schema_version"`). Preserved verbatim on round-trip.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// The kind of content a node holds. Matches the Obsidian type strings so
/// files round-trip cleanly between tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CanvasNodeKind {
    /// Free-form Markdown text.
    #[default]
    Text,
    /// A reference to another vault file (e.g. `notes/foo.md`, `img.png`).
    File,
    /// An external hyperlink.
    Link,
    /// A grouping rectangle that other nodes live inside.
    Group,
}

/// A single placed object on the canvas.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CanvasNode {
    /// Stable, canvas-unique node id (any client-chosen string).
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: CanvasNodeKind,
    /// Canvas X coordinate (pixels, top-left origin).
    #[serde(default)]
    pub x: f64,
    /// Canvas Y coordinate (pixels, top-left origin).
    #[serde(default)]
    pub y: f64,
    /// Node width (pixels).
    #[serde(default)]
    pub width: f64,
    /// Node height (pixels).
    #[serde(default)]
    pub height: f64,
    /// Markdown text body (only for `Text` nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Vault-relative path to the embedded file (only for `File` nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// External URL (only for `Link` nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Group label (only for `Group` nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional node colour token (Obsidian uses `"1"`..`"6"` or hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Forward-compatible catch-all.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A directional (or undirected) connection between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CanvasEdge {
    pub id: String,
    #[serde(rename = "fromNode")]
    pub from_node: String,
    #[serde(rename = "toNode")]
    pub to_node: String,
    /// Optional visible label on the arrow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional colour token (same scheme as nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Forward-compatible catch-all.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl CanvasFile {
    /// Look up a node by id.
    pub fn node(&self, id: &str) -> Option<&CanvasNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Count nodes by kind — used for the summary and tests.
    pub fn count_by_kind(&self) -> (usize, usize, usize, usize) {
        let mut text = 0;
        let mut file = 0;
        let mut link = 0;
        let mut group = 0;
        for n in &self.nodes {
            match n.kind {
                CanvasNodeKind::Text => text += 1,
                CanvasNodeKind::File => file += 1,
                CanvasNodeKind::Link => link += 1,
                CanvasNodeKind::Group => group += 1,
            }
        }
        (text, file, link, group)
    }
}

/// Parse a `.canvas` JSON document.
pub fn parse_canvas(content: &str) -> Result<CanvasFile> {
    let file: CanvasFile = serde_json::from_str(content).context("invalid .canvas JSON payload")?;
    validate_canvas(&file)?;
    Ok(file)
}

/// Serialize a canvas document back to pretty JSON suitable for writing to disk.
pub fn serialize_canvas(file: &CanvasFile) -> Result<String> {
    serde_json::to_string_pretty(file).context("failed to serialize canvas JSON")
}

/// Validate structural invariants:
///   - node ids are unique
///   - every edge references existing `from`/`to` nodes
///
/// Returns `Ok(())` on success or an error describing the first violation.
pub fn validate_canvas(file: &CanvasFile) -> Result<()> {
    // Unique node ids.
    let mut seen = std::collections::HashSet::new();
    for n in &file.nodes {
        if !seen.insert(n.id.as_str()) {
            bail!("duplicate node id: {}", n.id);
        }
    }
    // Edge endpoints must exist.
    for e in &file.edges {
        if !seen.contains(e.from_node.as_str()) {
            bail!("edge {} references unknown fromNode {}", e.id, e.from_node);
        }
        if !seen.contains(e.to_node.as_str()) {
            bail!("edge {} references unknown toNode {}", e.id, e.to_node);
        }
    }
    Ok(())
}

/// Discover every `.canvas` file under `dir` (recursive), sorted by path
/// for deterministic output.
pub fn list_canvas_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_canvas(dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_canvas(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    for entry in rd {
        let entry = entry?;
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            // Skip hidden / VCS directories to avoid surprises.
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
            }
            walk_canvas(&path, out)?;
        } else if ft.is_file() && path.extension().and_then(|s| s.to_str()) == Some(CANVAS_EXT) {
            out.push(path);
        }
    }
    Ok(())
}

/// Render a canvas as a Markdown outline — one bullet per node, with edges
/// listed at the bottom. Useful for diffing, accessibility, and piping into
/// other tools (search index, AI agent context).
pub fn export_canvas_to_markdown(file: &CanvasFile) -> Result<String> {
    let mut md = String::new();
    md.push_str("# Canvas export\n\n");
    let (text, file_n, link, group) = file.count_by_kind();
    md.push_str(&format!(
        "_Nodes:_ {} text · {} file · {} link · {} group  \n_Edges:_ {}\n\n",
        text,
        file_n,
        link,
        group,
        file.edges.len()
    ));

    md.push_str("## Nodes\n\n");
    if file.nodes.is_empty() {
        md.push_str("_(none)_\n");
    }
    for n in &file.nodes {
        let kind_label = match n.kind {
            CanvasNodeKind::Text => "text",
            CanvasNodeKind::File => "file",
            CanvasNodeKind::Link => "link",
            CanvasNodeKind::Group => "group",
        };
        let title: String = match n.kind {
            CanvasNodeKind::Text => n
                .text
                .as_deref()
                .map(first_heading_or_truncate)
                .unwrap_or_default(),
            CanvasNodeKind::File => n.file.clone().unwrap_or_default(),
            CanvasNodeKind::Link => n.url.clone().unwrap_or_default(),
            CanvasNodeKind::Group => n.label.clone().unwrap_or_default(),
        };
        md.push_str(&format!(
            "- `{id}` [{kind}] {title}\n",
            id = n.id,
            kind = kind_label,
            title = title,
        ));
    }

    if !file.edges.is_empty() {
        md.push_str("\n## Edges\n\n");
        for e in &file.edges {
            let label = e
                .label
                .as_deref()
                .map(|l| format!(" _{}_ ", l))
                .unwrap_or_default();
            md.push_str(&format!("- `{}` →{} `{}`\n", e.from_node, label, e.to_node));
        }
    }
    Ok(md.trim_end().to_string() + "\n")
}

fn first_heading_or_truncate(s: &str) -> String {
    for line in s.lines() {
        let t = line.trim_start();
        if let Some(title) = parse_atx_heading(t) {
            return title;
        }
    }
    // No heading — take first non-empty line, capped.
    for line in s.lines() {
        let t = line.trim();
        if !t.is_empty() {
            return truncate(t, 80).to_string();
        }
    }
    String::new()
}

/// Parse a CommonMark ATX heading line (`#`..`######` followed by whitespace)
/// and return the inner title text. Strips an optional closing sequence of
/// `#`s (e.g. `# Title #`). Returns `None` if the line is not a heading or
/// exceeds the 6-`#` maximum. Per CommonMark, at least one space (or end of
/// line) is required after the leading `#`s, so `#Foo` is *not* a heading.
fn parse_atx_heading(line: &str) -> Option<String> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let after = &line[hashes..];
    // Must be followed by a space, tab, or end-of-line. `#Foo` is not a
    // heading (CommonMark §4.2, "the opening sequence … must be followed by a
    // space or by the end of line").
    let body = match after.chars().next() {
        None => "",
        Some(' ') | Some('\t') => after[1..].trim_start(),
        _ => return None,
    };
    // Strip optional closing `#` sequence (CommonMark: the closing sequence
    // must be preceded by a space and may be followed by spaces/tabs only).
    let body = strip_closing_hashes(body);
    Some(body.trim().to_string())
}

/// Strip a trailing `#`-only sequence that is preceded by whitespace.
fn strip_closing_hashes(s: &str) -> &str {
    let trimmed_end = s.trim_end_matches([' ', '\t']);
    if trimmed_end.is_empty() {
        return s;
    }
    let n_trailing = trimmed_end.chars().rev().take_while(|c| *c == '#').count();
    if n_trailing == 0 {
        return s;
    }
    let cut = trimmed_end.len() - n_trailing;
    if cut == 0 {
        // Whole string was `#`s with nothing before — the closing sequence
        // needs a preceding space, so keep as-is.
        return s;
    }
    // The char just before the trailing `#`s must be a space/tab.
    let before = &trimmed_end[..cut];
    match before.chars().next_back() {
        Some(' ') | Some('\t') => before.trim_end_matches([' ', '\t']),
        _ => s,
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// A short human-readable summary line for a canvas document.
pub fn canvas_summary(file: &CanvasFile) -> String {
    let (text, file_n, link, group) = file.count_by_kind();
    format!(
        "{} nodes ({} text, {} file, {} link, {} group), {} edges",
        file.nodes.len(),
        text,
        file_n,
        link,
        group,
        file.edges.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAII guard that wipes a temp directory on drop. Mirrors the pattern
    /// used elsewhere in the crate (e.g. `capability_registry::tests`) so we
    /// don't pull in an external `tempfile` dev-dependency.
    struct TempDirGuard(std::path::PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_canvas_dir(name: &str) -> (PathBuf, TempDirGuard) {
        let dir =
            std::env::temp_dir().join(format!("vp-canvas-test-{}-{}", std::process::id(), name));
        // Start from a clean slate in case a prior run was killed mid-test.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let guard = TempDirGuard(dir.clone());
        (dir, guard)
    }

    /// Minimal but representative Obsidian-compatible sample used across tests.
    /// Uses `r##"…"##` because the content includes `"#` sequences (e.g. the
    /// Markdown heading `# Project plan` inside a JSON string literal) which
    /// would otherwise terminate the more common `r#"…"` raw string early.
    const SAMPLE: &str = r##"{
        "nodes": [
            { "id": "n1", "type": "text",
              "x": 0, "y": 0, "width": 250, "height": 60,
              "text": "# Project plan\nBody..." },
            { "id": "n2", "type": "file",
              "x": 300, "y": 0, "width": 400, "height": 400,
              "file": "notes/design.md" },
            { "id": "n3", "type": "link",
              "x": 0, "y": 200, "width": 250, "height": 60,
              "url": "https://example.com" },
            { "id": "g1", "type": "group",
              "x": -50, "y": -50, "width": 800, "height": 600,
              "label": "Sprint scope" }
        ],
        "edges": [
            { "id": "e1", "fromNode": "n1", "toNode": "n2", "label": "expands" },
            { "id": "e2", "fromNode": "n1", "toNode": "n3" }
        ]
    }"##;

    #[test]
    fn parse_and_round_trip_preserves_structure() {
        let parsed = parse_canvas(SAMPLE).expect("sample must parse");
        assert_eq!(parsed.nodes.len(), 4);
        assert_eq!(parsed.edges.len(), 2);

        let re_json = serialize_canvas(&parsed).expect("serialize");
        let reparsed = parse_canvas(&re_json).expect("reparse");
        assert_eq!(reparsed.nodes.len(), parsed.nodes.len());
        assert_eq!(reparsed.edges.len(), parsed.edges.len());
        // Kinds preserved.
        assert_eq!(reparsed.node("n2").unwrap().kind, CanvasNodeKind::File);
        assert_eq!(reparsed.node("g1").unwrap().kind, CanvasNodeKind::Group);
        // Edge endpoints preserved.
        assert_eq!(reparsed.edges[0].from_node, "n1");
        assert_eq!(reparsed.edges[0].to_node, "n2");
        assert_eq!(reparsed.edges[0].label.as_deref(), Some("expands"));
    }

    #[test]
    fn preserves_unknown_top_level_fields() {
        // Forward-compat: an unknown field must not break parsing and must
        // survive a round-trip.
        let json = r#"{
            "schema_version": 42,
            "nodes": [{ "id": "x", "type": "text", "text": "hi" }],
            "edges": []
        }"#;
        let parsed = parse_canvas(json).expect("parses with unknown field");
        assert_eq!(
            parsed.extra.get("schema_version").and_then(|v| v.as_u64()),
            Some(42)
        );
        let re = parse_canvas(&serialize_canvas(&parsed).unwrap()).unwrap();
        assert_eq!(
            re.extra.get("schema_version").and_then(|v| v.as_u64()),
            Some(42)
        );
    }

    #[test]
    fn rejects_duplicate_node_id() {
        let json = r#"{
            "nodes": [
                { "id": "dup", "type": "text" },
                { "id": "dup", "type": "text" }
            ],
            "edges": []
        }"#;
        let err = parse_canvas(json).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("duplicate node id"), "got: {msg}");
    }

    #[test]
    fn rejects_dangling_edge_endpoint() {
        let json = r#"{
            "nodes": [ { "id": "n1", "type": "text" } ],
            "edges": [ { "id": "e1", "fromNode": "n1", "toNode": "ghost" } ]
        }"#;
        let err = parse_canvas(json).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown toNode"), "got: {msg}");
    }

    #[test]
    fn empty_canvas_is_valid() {
        let json = r#"{ "nodes": [], "edges": [] }"#;
        let parsed = parse_canvas(json).unwrap();
        assert!(parsed.nodes.is_empty());
        assert!(parsed.edges.is_empty());
        assert_eq!(
            canvas_summary(&parsed),
            "0 nodes (0 text, 0 file, 0 link, 0 group), 0 edges"
        );
    }

    #[test]
    fn implicit_empty_arrays_when_fields_missing() {
        // A bare `{}` should still parse — default to empty canvas.
        let parsed = parse_canvas("{}").unwrap();
        assert!(parsed.nodes.is_empty());
        assert!(parsed.edges.is_empty());
    }

    #[test]
    fn node_kind_defaults_to_text() {
        // Obsidian omits `type` for plain text cards in some older exports;
        // serde default should make those parse as Text.
        let json = r#"{ "nodes": [ { "id": "a", "text": "no type field" } ] }"#;
        let parsed = parse_canvas(json).unwrap();
        assert_eq!(parsed.nodes[0].kind, CanvasNodeKind::Text);
    }

    #[test]
    fn list_canvas_files_walks_recursively_skipping_hidden() {
        use std::io::Write as _;
        let (root, _guard) = temp_canvas_dir("list_walk");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let mk = |rel: &str, body: &str| {
            let p = root.join(rel);
            let mut f = std::fs::File::create(&p).unwrap();
            f.write_all(body.as_bytes()).unwrap();
            p
        };
        let a = mk("a.canvas", r#"{ "nodes": [], "edges": [] }"#);
        let b = mk("sub/b.canvas", r#"{ "nodes": [], "edges": [] }"#);
        mk("not-a-canvas.md", "# nope");
        mk(".git/ignored.canvas", r#"{ "nodes": [], "edges": [] }"#);

        let found = list_canvas_files(&root).unwrap();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(found, expected);
    }

    #[test]
    fn list_canvas_files_missing_dir_is_empty_not_error() {
        let bogus = Path::new("/this/path/does/not/exist/anywhere/probably");
        let found = list_canvas_files(bogus).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn export_markdown_includes_all_nodes_and_edges() {
        let parsed = parse_canvas(SAMPLE).unwrap();
        let md = export_canvas_to_markdown(&parsed).unwrap();
        // Heading.
        assert!(md.contains("# Canvas export"));
        // Each node id appears at least once.
        for id in ["n1", "n2", "n3", "g1"] {
            assert!(
                md.contains(&format!("`{id}`")),
                "missing node {id} in:\n{md}"
            );
        }
        // Edge lines include both endpoints.
        assert!(md.contains("`n1`"));
        assert!(md.contains("`n2`"));
        // Text node uses the heading as its title.
        assert!(md.contains("Project plan"));
        // File node references its path.
        assert!(md.contains("notes/design.md"));
    }

    #[test]
    fn export_markdown_handles_empty_canvas() {
        let parsed = parse_canvas(r#"{ "nodes": [], "edges": [] }"#).unwrap();
        let md = export_canvas_to_markdown(&parsed).unwrap();
        assert!(
            md.contains("_(none)_"),
            "empty nodes section should render placeholder, got:\n{md}"
        );
    }

    #[test]
    fn summary_counts_each_kind() {
        let parsed = parse_canvas(SAMPLE).unwrap();
        let s = canvas_summary(&parsed);
        // 1 text, 1 file, 1 link, 1 group, 2 edges.
        assert!(s.contains("4 nodes"), "got {s}");
        assert!(s.contains("1 text"), "got {s}");
        assert!(s.contains("1 file"), "got {s}");
        assert!(s.contains("1 link"), "got {s}");
        assert!(s.contains("1 group"), "got {s}");
        assert!(s.contains("2 edges"), "got {s}");
    }

    #[test]
    fn first_heading_falls_back_to_first_nonempty_line() {
        let parsed = parse_canvas(
            r#"{ "nodes": [
                { "id": "n", "type": "text", "text": "   \nplain text no heading" }
            ] }"#,
        )
        .unwrap();
        let md = export_canvas_to_markdown(&parsed).unwrap();
        assert!(md.contains("plain text no heading"), "got:\n{md}");
    }

    /// Regression for #3181: H2/H3 headings must be recognized — the title
    /// must not include the leading `##`/`###` literal.
    #[test]
    fn first_heading_recognizes_h2_h3_3181() {
        let parsed = parse_canvas(
            r########"{ "nodes": [
                { "id": "h2", "type": "text", "text": "## Section Title\nBody" },
                { "id": "h3", "type": "text", "text": "### Subsection\nBody" },
                { "id": "h6", "type": "text", "text": "###### DeepHeading\nBody" }
            ] }"########,
        )
        .unwrap();
        let md = export_canvas_to_markdown(&parsed).unwrap();
        assert!(
            md.contains("Section Title"),
            "H2 title not extracted — md:\n{md}"
        );
        assert!(
            !md.contains("## Section Title"),
            "H2 literal `##` leaked into title — md:\n{md}"
        );
        assert!(
            md.contains("Subsection"),
            "H3 title not extracted — md:\n{md}"
        );
        assert!(
            !md.contains("### Subsection"),
            "H3 literal `###` leaked into title — md:\n{md}"
        );
        assert!(
            md.contains("DeepHeading"),
            "H6 title not extracted — md:\n{md}"
        );
    }

    /// Regression for #3181: a closing `#` sequence on an ATX heading
    /// (`# Title #`) must be stripped.
    #[test]
    fn first_heading_strips_closing_hashes_3181() {
        let parsed = parse_canvas(
            r########"{ "nodes": [
                { "id": "c", "type": "text", "text": "# Title #\nBody" },
                { "id": "c2", "type": "text", "text": "## Closed   ##\nBody" }
            ] }"########,
        )
        .unwrap();
        let md = export_canvas_to_markdown(&parsed).unwrap();
        // The title should be "Title", not "Title #".
        assert!(
            md.contains("Title"),
            "closing-hash title not extracted — md:\n{md}"
        );
        assert!(
            !md.lines().any(|l| l.contains("Title #")),
            "trailing `#` leaked into title — md:\n{md}"
        );
        assert!(
            md.contains("Closed"),
            "closing-hash H2 title not extracted — md:\n{md}"
        );
        assert!(
            !md.lines().any(|l| l.contains("Closed   ##")),
            "trailing `##` leaked into H2 title — md:\n{md}"
        );
    }

    /// Regression for #3181: `#Foo` (no space after `#`) is NOT a heading
    /// per CommonMark and must fall through to the first-non-empty-line path.
    #[test]
    fn first_heading_rejects_nospace_hash_3181() {
        let parsed = parse_canvas(
            r########"{ "nodes": [
                { "id": "t", "type": "text", "text": "#NotAHeading\nbody" }
            ] }"########,
        )
        .unwrap();
        let md = export_canvas_to_markdown(&parsed).unwrap();
        // Falls back to the whole line (truncated), not stripped to "NotAHeading".
        assert!(
            md.contains("#NotAHeading"),
            "CommonMark-incompatible `#Foo` was wrongly parsed as heading — md:\n{md}"
        );
    }

    /// Regression for #3181: headings of level 7+ (`#######`) are not valid
    /// ATX headings and must not be treated as headings.
    #[test]
    fn first_heading_rejects_seven_hashes_3181() {
        let parsed = parse_canvas(
            r########"{ "nodes": [
                { "id": "t", "type": "text", "text": "####### Not a heading\nbody" }
            ] }"########,
        )
        .unwrap();
        let md = export_canvas_to_markdown(&parsed).unwrap();
        // Falls back to first-non-empty line; title keeps the `#######`.
        assert!(
            md.contains("####### Not a heading"),
            "7-`#` line was wrongly parsed as heading — md:\n{md}"
        );
    }

    #[test]
    fn truncate_respects_utf8_boundary() {
        // "é" is 2 bytes in UTF-8; cutting at byte 1 must not panic.
        let s = "ééééé";
        let t = truncate(s, 3);
        assert!(t.chars().count() <= 3);
        // Ensure no panic.
        let _ = truncate(s, 0);
    }
}
