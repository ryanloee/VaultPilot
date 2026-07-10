//! Knowledge graph — auto-generate node-edge relationships from wikilinks (#1913).
//!
//! This module builds a graph representation of the vault by extracting
//! `[[wikilink]]` references from every note and resolving them to note
//! titles. The result can be serialized to DOT (Graphviz) or JSON for
//! consumption by the CLI `graph` command, WinUI graph view, or Android.
//!
//! MVP scope (#1913 Phase 1):
//! - Nodes: every note in the vault.
//! - Edges: resolved wikilink from note A → note B.
//! - DOT output for Graphviz rendering.
//! - JSON output for programmatic consumption (future UI clients).

use crate::models::{NoteMeta, WikilinkRef};
use crate::storage::{self, StorageContext};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A node in the knowledge graph representing a single note.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphNode {
    /// Stable identifier (same as NoteMeta::id).
    pub id: String,
    /// Human-readable note title.
    pub title: String,
    /// Tags attached to the note (for UI styling/filtering).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Number of incoming links (backlinks). Populated during graph build.
    #[serde(default)]
    pub in_degree: usize,
    /// Number of outgoing links (wikilinks). Populated during graph build.
    #[serde(default)]
    pub out_degree: usize,
}

/// A directed edge from source note → target note (via wikilink).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphEdge {
    /// Source note ID.
    pub source: String,
    /// Target note ID (resolved wikilink target).
    pub target: String,
    /// Raw wikilink text (may differ from target title if aliased).
    pub label: String,
}

/// Complete knowledge graph: all nodes + edges.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnowledgeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Total number of notes scanned (including notes with no links).
    pub note_count: usize,
    /// Number of edges (resolved links between notes).
    pub edge_count: usize,
    /// Number of unresolved (dangling) wikilinks.
    pub dangling_link_count: usize,
}

/// Output format for the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphOutputFormat {
    /// DOT language for Graphviz rendering.
    #[default]
    Dot,
    /// JSON for programmatic consumption.
    Json,
}

/// Build a complete knowledge graph from the vault.
///
/// Iterates over every note, extracts `[[wikilink]]` targets, and resolves
/// them against the note title index. Both resolved and unresolved links are
/// tracked; only resolved links become edges.
///
/// # Arguments
/// * `context` - Storage context for database access.
///
/// # Errors
/// Returns an error if the database cannot be opened or a note body cannot
/// be loaded.
pub fn build_knowledge_graph(context: &StorageContext) -> Result<KnowledgeGraph> {
    let (connection, _) = storage::pool::open_connection(context)?;
    let all_metas = storage::list_all_note_metas(&connection)?;

    // Build case-insensitive title → NoteMeta lookup.
    let mut title_index: HashMap<String, &NoteMeta> = HashMap::with_capacity(all_metas.len());
    for meta in &all_metas {
        title_index.insert(meta.title.to_lowercase(), meta);
    }

    // Track in-degrees and out-degrees.
    let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
    let mut out_degree: BTreeMap<String, usize> = BTreeMap::new();

    // Deduplicate edges: (source_id, target_id) → label
    let mut edge_set: BTreeSet<(String, String)> = BTreeSet::new();
    let mut edges: Vec<GraphEdge> = Vec::new();

    let mut dangling_link_count = 0usize;

    // Map note ID → its resolved wikilinks (for edge building).
    let mut note_wikilinks: BTreeMap<String, Vec<WikilinkRef>> = BTreeMap::new();

    for meta in &all_metas {
        // Load note body to extract wikilinks.
        let doc = match storage::notes::load_note_with_context(context, &meta.id) {
            Ok(doc) => doc,
            Err(_) => continue, // skip notes that can't be loaded
        };

        let raw_links = storage::notes::extract_wikilinks(&doc.body);
        if raw_links.is_empty() {
            continue;
        }

        // Resolve each wikilink target against the title index.
        let mut resolved: Vec<WikilinkRef> = Vec::new();
        for (target, alias) in &raw_links {
            // Try exact case-insensitive match first.
            if let Some(target_meta) = title_index.get(&target.to_lowercase()) {
                resolved.push(WikilinkRef {
                    target: target.clone(),
                    alias: alias.clone(),
                    note: Some((*target_meta).clone()),
                });
            } else {
                // Dangling link — not resolved to any note.
                dangling_link_count += 1;
                resolved.push(WikilinkRef {
                    target: target.clone(),
                    alias: alias.clone(),
                    note: None,
                });
            }
        }

        note_wikilinks.insert(meta.id.clone(), resolved);
    }

    // Build edges from resolved wikilinks.
    for (source_id, wikilinks) in &note_wikilinks {
        for wl in wikilinks {
            if let Some(target_meta) = &wl.note {
                // Skip self-loops (a note linking to itself).
                if target_meta.id == *source_id {
                    continue;
                }

                let edge_key = (source_id.clone(), target_meta.id.clone());
                if edge_set.insert(edge_key.clone()) {
                    edges.push(GraphEdge {
                        source: source_id.clone(),
                        target: target_meta.id.clone(),
                        label: wl.target.clone(),
                    });
                    *in_degree.entry(target_meta.id.clone()).or_default() += 1;
                    *out_degree.entry(source_id.clone()).or_default() += 1;
                }
            }
        }
    }

    // Build node list with degree information.
    let nodes: Vec<GraphNode> = all_metas
        .iter()
        .map(|meta| GraphNode {
            id: meta.id.clone(),
            title: meta.title.clone(),
            tags: meta.tags.clone(),
            in_degree: *in_degree.get(&meta.id).unwrap_or(&0),
            out_degree: *out_degree.get(&meta.id).unwrap_or(&0),
        })
        .collect();

    let edge_count = edges.len();

    Ok(KnowledgeGraph {
        nodes,
        edges,
        note_count: all_metas.len(),
        edge_count,
        dangling_link_count,
    })
}

/// Render the knowledge graph as DOT language for Graphviz.
///
/// Produces a directed graph (`digraph`) where each note is a node labeled
/// with its title, and each resolved wikilink is a directed edge.
///
/// # Example output
/// ```text
/// digraph vault {
///     rankdir=LR;
///     "note_001" [label="Introduction"];
///     "note_002" [label="Advanced Topics"];
///     "note_001" -> "note_002" [label="[[Advanced Topics]]"];
/// }
/// ```
///
/// Render with: `vp graph --dot | dot -Tsvg -o graph.svg`
pub fn render_dot(graph: &KnowledgeGraph) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("digraph vault {\n");
    out.push_str("    rankdir=LR;\n");
    out.push_str("    node [shape=box, style=rounded, fontname=\"Helvetica\"];\n");
    out.push_str("    edge [fontname=\"Helvetica\", fontsize=10];\n");
    out.push('\n');

    // Nodes
    for node in &graph.nodes {
        // Escape special DOT characters in label
        let label = dot_escape(&node.title);
        let tags_attr = if node.tags.is_empty() {
            String::new()
        } else {
            format!(", tags=\"{}\"", dot_escape(&node.tags.join(", ")))
        };
        // Color orphan nodes (no links) differently for visual identification
        let color = if node.in_degree == 0 && node.out_degree == 0 {
            "color=gray, fontcolor=gray"
        } else {
            "color=steelblue"
        };
        out.push_str(&format!(
            "    \"{}\" [label=\"{}\"{tags_attr}, {color}];\n",
            dot_escape(&node.id),
            label,
        ));
    }

    out.push('\n');

    // Edges
    for edge in &graph.edges {
        out.push_str(&format!(
            "    \"{}\" -> \"{}\" [label=\"{}\"];\n",
            dot_escape(&edge.source),
            dot_escape(&edge.target),
            dot_escape(&edge.label),
        ));
    }

    out.push_str("}\n");
    out
}

/// Escape special characters for DOT string literals.
fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Render the knowledge graph as JSON.
pub fn render_json(graph: &KnowledgeGraph) -> Result<String> {
    Ok(serde_json::to_string_pretty(graph)?)
}

/// Render the graph in the specified format.
pub fn render(graph: &KnowledgeGraph, format: GraphOutputFormat) -> Result<String> {
    match format {
        GraphOutputFormat::Dot => Ok(render_dot(graph)),
        GraphOutputFormat::Json => render_json(graph),
    }
}

/// Get a summary of graph statistics (for CLI status display).
pub fn graph_summary(graph: &KnowledgeGraph) -> String {
    let orphans = graph
        .nodes
        .iter()
        .filter(|n| n.in_degree == 0 && n.out_degree == 0)
        .count();
    let hub_nodes = graph.nodes.iter().filter(|n| n.in_degree >= 3).count();

    format!(
        "Knowledge Graph: {} notes, {} links, {} unresolved links\n\
         {} orphan notes (no links), {} hub notes (3+ backlinks)\n\
         Average links per note: {:.1}",
        graph.note_count,
        graph.edge_count,
        graph.dangling_link_count,
        orphans,
        hub_nodes,
        if graph.note_count > 0 {
            graph.edge_count as f64 / graph.note_count as f64
        } else {
            0.0
        }
    )
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dot_escape() {
        assert_eq!(dot_escape("hello"), "hello");
        assert_eq!(dot_escape("a\\b"), "a\\\\b");
        assert_eq!(dot_escape(r#""quoted""#), "\\\"quoted\\\"");
        assert_eq!(dot_escape("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn test_render_dot_empty_graph() {
        let graph = KnowledgeGraph::default();
        let dot = render_dot(&graph);
        assert!(dot.starts_with("digraph vault {"));
        assert!(dot.ends_with("}\n"));
        assert!(dot.contains("rankdir=LR"));
    }

    #[test]
    fn test_render_dot_with_nodes() {
        let graph = KnowledgeGraph {
            nodes: vec![
                GraphNode {
                    id: "n1".into(),
                    title: "Introduction".into(),
                    tags: vec!["basics".into()],
                    in_degree: 0,
                    out_degree: 1,
                },
                GraphNode {
                    id: "n2".into(),
                    title: "Advanced".into(),
                    tags: vec![],
                    in_degree: 1,
                    out_degree: 0,
                },
            ],
            edges: vec![GraphEdge {
                source: "n1".into(),
                target: "n2".into(),
                label: "Advanced".into(),
            }],
            note_count: 2,
            edge_count: 1,
            dangling_link_count: 0,
        };
        let dot = render_dot(&graph);
        assert!(dot.contains(r#""n1" [label="Introduction""#));
        assert!(dot.contains(r#""n2" [label="Advanced""#));
        assert!(dot.contains(r#""n1" -> "n2" [label="Advanced"];"#));
    }

    #[test]
    fn test_render_dot_orphan_color() {
        let graph = KnowledgeGraph {
            nodes: vec![GraphNode {
                id: "orphan".into(),
                title: "Lonely Note".into(),
                tags: vec![],
                in_degree: 0,
                out_degree: 0,
            }],
            edges: vec![],
            note_count: 1,
            edge_count: 0,
            dangling_link_count: 0,
        };
        let dot = render_dot(&graph);
        assert!(dot.contains("color=gray"));
        assert!(!dot.contains("color=steelblue"));
    }

    #[test]
    fn test_render_dot_connected_color() {
        let graph = KnowledgeGraph {
            nodes: vec![GraphNode {
                id: "hub".into(),
                title: "Hub".into(),
                tags: vec![],
                in_degree: 5,
                out_degree: 2,
            }],
            edges: vec![],
            note_count: 1,
            edge_count: 0,
            dangling_link_count: 0,
        };
        let dot = render_dot(&graph);
        assert!(dot.contains("color=steelblue"));
        assert!(!dot.contains("color=gray"));
    }

    #[test]
    fn test_render_json() {
        let graph = KnowledgeGraph {
            nodes: vec![GraphNode {
                id: "n1".into(),
                title: "Test".into(),
                tags: vec!["tag1".into()],
                in_degree: 0,
                out_degree: 0,
            }],
            edges: vec![],
            note_count: 1,
            edge_count: 0,
            dangling_link_count: 0,
        };
        let json = render_json(&graph).unwrap();
        let parsed: KnowledgeGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.note_count, 1);
        assert_eq!(parsed.nodes[0].title, "Test");
    }

    #[test]
    fn test_graph_summary() {
        let graph = KnowledgeGraph {
            nodes: vec![
                GraphNode {
                    id: "n1".into(),
                    title: "A".into(),
                    tags: vec![],
                    in_degree: 0,
                    out_degree: 1,
                },
                GraphNode {
                    id: "n2".into(),
                    title: "B".into(),
                    tags: vec![],
                    in_degree: 1,
                    out_degree: 0,
                },
                GraphNode {
                    id: "n3".into(),
                    title: "C".into(),
                    tags: vec![],
                    in_degree: 0,
                    out_degree: 0,
                },
            ],
            edges: vec![GraphEdge {
                source: "n1".into(),
                target: "n2".into(),
                label: "B".into(),
            }],
            note_count: 3,
            edge_count: 1,
            dangling_link_count: 0,
        };
        let summary = graph_summary(&graph);
        assert!(summary.contains("3 notes"));
        assert!(summary.contains("1 links"));
        assert!(summary.contains("1 orphan"));
    }

    #[test]
    fn test_render_dot_with_special_chars() {
        let graph = KnowledgeGraph {
            nodes: vec![GraphNode {
                id: "n1".into(),
                title: "Quote \" Test".into(),
                tags: vec![],
                in_degree: 0,
                out_degree: 0,
            }],
            edges: vec![],
            note_count: 1,
            edge_count: 0,
            dangling_link_count: 0,
        };
        let dot = render_dot(&graph);
        // The label should have escaped quotes
        assert!(dot.contains(r#"label="Quote \" Test""#));
    }

    #[test]
    fn test_render_dot_with_tags() {
        let graph = KnowledgeGraph {
            nodes: vec![GraphNode {
                id: "n1".into(),
                title: "Tagged".into(),
                tags: vec!["rust".into(), "async".into()],
                in_degree: 0,
                out_degree: 0,
            }],
            edges: vec![],
            note_count: 1,
            edge_count: 0,
            dangling_link_count: 0,
        };
        let dot = render_dot(&graph);
        assert!(dot.contains(r#"tags="rust, async""#));
    }

    #[test]
    fn test_graph_output_format_default() {
        assert_eq!(GraphOutputFormat::default(), GraphOutputFormat::Dot);
    }

    #[test]
    fn test_render_dot_format() {
        let graph = KnowledgeGraph::default();
        let result = render(&graph, GraphOutputFormat::Dot).unwrap();
        assert!(result.starts_with("digraph"));
    }

    #[test]
    fn test_render_json_format() {
        let graph = KnowledgeGraph::default();
        let result = render(&graph, GraphOutputFormat::Json).unwrap();
        assert!(result.starts_with('{'));
    }
}
