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
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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

/// The provenance of a graph edge, so UI clients can distinguish formal
/// `[[wikilink]]` connections from latent plain-text mentions (#2832).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GraphEdgeKind {
    /// Resolved `[[wikilink]]` between two notes.
    #[default]
    Wikilink,
    /// Plain-text mention of a note's title without a `[[…]]` wrapper
    /// (an "unlinked mention" / 未链接提及). Surfaces latent connections
    /// the user hasn't formalised into wikilinks yet (#2832).
    Mention,
}

/// A directed edge from source note → target note.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphEdge {
    /// Source note ID.
    pub source: String,
    /// Target note ID (resolved wikilink target).
    pub target: String,
    /// Raw edge text — the wikilink target or the mentioned title.
    pub label: String,
    /// Provenance of the edge: a formal wikilink or a plain-text mention.
    #[serde(default)]
    pub kind: GraphEdgeKind,
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

// ── Inferred Relations (#3370) ───────────────────────────────────────────
// AI-style latent relationship detection: discovers hidden connections between
// notes that don't have explicit [[wikilinks]] by analysing tag, keyword, and
// title-word overlap. Produces typed, confidence-scored relationships that can
// be visualised in the graph alongside wikilink and mention edges.

/// Type of inferred relationship between two notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RelationType {
    /// Notes share significant topical overlap (tags + keywords).
    #[default]
    Related,
    /// Notes mention the same named entity (high title-word overlap).
    SameEntity,
    /// Notes belong to the same conceptual cluster (very high overall similarity).
    SameTopic,
}

/// An inferred relationship between two notes — discovered through content
/// similarity rather than explicit `[[wikilinks]]` or plain-text mentions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferredRelation {
    /// Source note ID.
    pub source: String,
    /// Target note ID.
    pub target: String,
    /// Type of relationship inferred.
    #[serde(default)]
    pub relation_type: RelationType,
    /// Confidence score in [0.0, 1.0] — higher means more certain.
    pub confidence: f64,
    /// Human-readable explanation of why this relation was inferred.
    pub reason: String,
}

/// Configuration for relation inference.
#[derive(Debug, Clone)]
pub struct InferenceConfig {
    /// Minimum confidence threshold to include a relation.
    pub min_confidence: f64,
    /// Maximum relations to emit per source note (top-N by confidence).
    pub max_per_note: usize,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.3,
            max_per_note: 5,
        }
    }
}

/// Score the similarity between two notes and infer a typed relationship.
///
/// This is a **pure function** — it takes two `NoteMeta` references and returns
/// an `InferredRelation` if the confidence exceeds zero, or `None` if the notes
/// are unrelated.
///
/// Scoring weights (mirrors `serendipity.rs` weighting but normalised to [0, 1]):
/// - Tag overlap: 40 % of confidence
/// - Keyword overlap: 35 % of confidence
/// - Title-word overlap: 25 % of confidence
pub fn score_pair(a: &NoteMeta, b: &NoteMeta) -> Option<InferredRelation> {
    // Skip self-comparison.
    if a.id == b.id {
        return None;
    }

    // Compute overlap sets.
    let a_tags: HashSet<&str> = a.tags.iter().map(|s| s.as_str()).collect();
    let b_tags: HashSet<&str> = b.tags.iter().map(|s| s.as_str()).collect();
    let a_kws: HashSet<&str> = a.keywords.iter().map(|s| s.as_str()).collect();
    let b_kws: HashSet<&str> = b.keywords.iter().map(|s| s.as_str()).collect();

    let a_title_words = title_word_set(&a.title);
    let b_title_words = title_word_set(&b.title);

    // Jaccard similarity for each dimension.
    let tag_sim = jaccard(&a_tags, &b_tags);
    let kw_sim = jaccard(&a_kws, &b_kws);
    let title_sim = jaccard(&a_title_words, &b_title_words);

    // Weighted confidence.
    let confidence = tag_sim * 0.40 + kw_sim * 0.35 + title_sim * 0.25;

    if confidence < f64::EPSILON {
        return None;
    }

    // Determine relation type based on dominant signal.
    let (relation_type, reason) = if title_sim >= 0.5 && tag_sim >= 0.3 {
        (
            RelationType::SameEntity,
            format!(
                "Strong title-word and tag overlap (title: {:.0}%, tags: {:.0}%)",
                title_sim * 100.0,
                tag_sim * 100.0
            ),
        )
    } else if tag_sim >= 0.5 || (tag_sim + kw_sim) / 2.0 >= 0.4 {
        (
            RelationType::SameTopic,
            format!(
                "High topical overlap (tags: {:.0}%, keywords: {:.0}%)",
                tag_sim * 100.0,
                kw_sim * 100.0
            ),
        )
    } else {
        (
            RelationType::Related,
            format!(
                "Moderate overlap (tags: {:.0}%, keywords: {:.0}%, title: {:.0}%)",
                tag_sim * 100.0,
                kw_sim * 100.0,
                title_sim * 100.0
            ),
        )
    };

    Some(InferredRelation {
        source: a.id.clone(),
        target: b.id.clone(),
        relation_type,
        confidence,
        reason,
    })
}

/// Infer latent relationships among a set of notes.
///
/// Compares every unordered pair of notes, computes similarity, and returns
/// relationships that exceed the confidence threshold. Results are sorted by
/// confidence (descending). Each source note contributes at most
/// `config.max_per_note` relations.
///
/// This is a pure, database-free function suitable for unit testing.
pub fn infer_relations(notes: &[NoteMeta], config: &InferenceConfig) -> Vec<InferredRelation> {
    let mut all: Vec<InferredRelation> = Vec::new();
    // Dedup before per-source truncation: track seen unordered pairs globally.
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

    for i in 0..notes.len() {
        let mut per_source: Vec<InferredRelation> = Vec::new();
        for j in 0..notes.len() {
            if i == j {
                continue;
            }
            // score_pair checks a→b; we want unordered pairs so we take
            // the max-direction score (symmetric enough for similarity).
            if let Some(rel) = score_pair(&notes[i], &notes[j]) {
                if rel.confidence >= config.min_confidence {
                    // Dedup at insertion time: if this unordered pair is already
                    // tracked by another source, keep the higher-confidence version.
                    let key = if rel.source <= rel.target {
                        (rel.source.clone(), rel.target.clone())
                    } else {
                        (rel.target.clone(), rel.source.clone())
                    };
                    if seen.insert(key) {
                        per_source.push(rel);
                    }
                }
            }
        }
        // Keep only top-N unique pairs per source.
        per_source.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        per_source.truncate(config.max_per_note);
        all.extend(per_source);
    }

    // Final sort by confidence descending.
    all.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    all
}

/// Convenience wrapper with default config.
pub fn infer_relations_default(notes: &[NoteMeta]) -> Vec<InferredRelation> {
    infer_relations(notes, &InferenceConfig::default())
}

/// Compute the Jaccard similarity between two sets: |A ∩ B| / |A ∪ B|.
fn jaccard<T: std::hash::Hash + Eq>(a: &HashSet<T>, b: &HashSet<T>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Extract significant words from a title (lowercased, punctuation-trimmed, len > 1).
fn title_word_set(title: &str) -> HashSet<String> {
    title
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| c.is_ascii_punctuation())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty() && w.len() > 1)
        .collect()
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
/// Build a knowledge graph from the vault using only resolved `[[wikilink]]`
/// edges. This preserves the original (#1913) behaviour.
///
/// See [`build_knowledge_graph_with_mentions`] for a variant that also
/// includes latent plain-text "unlinked mention" edges (#2832).
pub fn build_knowledge_graph(context: &StorageContext) -> Result<KnowledgeGraph> {
    build_knowledge_graph_impl(context, false)
}

/// Build a knowledge graph that **additionally** includes unlinked-mention
/// edges (#2832): a note whose body mentions another note's title as plain
/// text (case-insensitive whole-word, outside code blocks/frontmatter) but
/// does not `[[wikilink]]` to it. These surface latent connections the user
/// hasn't formalised yet and are rendered as dashed edges in DOT output.
pub fn build_knowledge_graph_with_mentions(context: &StorageContext) -> Result<KnowledgeGraph> {
    build_knowledge_graph_impl(context, true)
}

/// Core graph builder. When `include_mentions` is true, plain-text mention
/// edges are added on top of the resolved wikilink edges.
fn build_knowledge_graph_impl(
    context: &StorageContext,
    include_mentions: bool,
) -> Result<KnowledgeGraph> {
    let (connection, _) = storage::pool::open_connection(context)?;
    let all_metas = storage::list_all_note_metas(&connection)?;

    // Collected (id, title, body) for unlinked-mention detection.
    let mut loaded_notes: Vec<(String, String, String)> = Vec::new();

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

        // Keep body for unlinked-mention detection.
        loaded_notes.push((meta.id.clone(), meta.title.clone(), doc.body.clone()));

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
                        kind: GraphEdgeKind::Wikilink,
                    });
                    *in_degree.entry(target_meta.id.clone()).or_default() += 1;
                    *out_degree.entry(source_id.clone()).or_default() += 1;
                }
            }
        }
    }

    // Optionally add unlinked-mention (soft) edges on top of wikilinks.
    if include_mentions {
        let mention_edges = detect_unlinked_mention_edges(&loaded_notes, &edge_set);
        for me in mention_edges {
            *in_degree.entry(me.target.clone()).or_default() += 1;
            *out_degree.entry(me.source.clone()).or_default() += 1;
            edges.push(me);
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

/// Detect latent "unlinked mention" edges among a set of notes.
///
/// For every ordered pair of distinct notes `(source, target)`, if `source`'s
/// body mentions `target`'s title as plain text (case-insensitive whole-word,
/// outside code blocks and frontmatter) **and** the pair is not already a
/// resolved wikilink (i.e. present in `exclude`), a soft `Mention` edge is
/// produced.
///
/// This is a pure, database-free function so it can be unit-tested directly
/// and reused by the graph builder and the `notes.unlinked_mentions` tool
/// (#2832).
pub fn detect_unlinked_mention_edges(
    notes: &[(String, String, String)],
    exclude: &BTreeSet<(String, String)>,
) -> Vec<GraphEdge> {
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

    for (src_id, _src_title, src_body) in notes {
        for (tgt_id, tgt_title, _tgt_body) in notes {
            if src_id == tgt_id {
                continue;
            }
            let tgt_title_trimmed = tgt_title.trim();
            if tgt_title_trimmed.len() < 3 {
                // Titles shorter than 3 chars produce too many false positives.
                continue;
            }
            let tgt_lower = tgt_title_trimmed.to_lowercase();
            if crate::storage::notes::body_mentions_title(src_body, &tgt_lower) {
                let key = (src_id.clone(), tgt_id.clone());
                if exclude.contains(&key) {
                    continue;
                }
                if seen.insert(key) {
                    edges.push(GraphEdge {
                        source: src_id.clone(),
                        target: tgt_id.clone(),
                        label: tgt_title_trimmed.to_string(),
                        kind: GraphEdgeKind::Mention,
                    });
                }
            }
        }
    }

    edges
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
        // Unlinked mentions are rendered dashed so the UI can distinguish
        // formal `[[wikilinks]]` from latent plain-text mentions (#2832).
        let style = if edge.kind == GraphEdgeKind::Mention {
            ", style=dashed, color=gray"
        } else {
            ""
        };
        out.push_str(&format!(
            "    \"{}\" -> \"{}\" [label=\"{}\"{}];\n",
            dot_escape(&edge.source),
            dot_escape(&edge.target),
            dot_escape(&edge.label),
            style,
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

// ── Semantic-enhanced relation inference (#3458 Phase 1) ─────────────────
// Adds semantic vector similarity to the existing metadata-overlap scoring.
// Uses the HashEmbedder from src/semantic/mod.rs to compute cosine similarity
// between note content vectors, then combines it with tag/keyword/title
// overlap for a richer confidence score.

use crate::semantic::{cosine_similarity, default_embedder, SemanticEmbedder};

/// Weight of semantic vector similarity in the combined confidence score.
const SEMANTIC_WEIGHT: f64 = 0.45;
/// Weight of metadata overlap (tags + keywords + title) in the combined score.
const METADATA_WEIGHT: f64 = 0.55;

/// A link suggestion produced by the auto-discovery engine — a note that is
/// semantically related to the source note but not yet linked via `[[wikilink]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoLinkSuggestion {
    /// The note that should be linked FROM (source).
    pub source_id: String,
    /// The note that should be linked TO (suggested target).
    pub target_id: String,
    /// Title of the suggested target note (for `[[Title]]` insertion).
    pub target_title: String,
    /// Combined confidence score [0.0, 1.0].
    pub confidence: f64,
    /// Semantic vector similarity component [0.0, 1.0].
    pub semantic_similarity: f64,
    /// Metadata overlap component [0.0, 1.0].
    pub metadata_similarity: f64,
    /// Human-readable reason for the suggestion.
    pub reason: String,
}

/// Score the similarity between two notes using **both** metadata overlap and
/// semantic vector similarity of their content.
///
/// This enhances [`score_pair`] by adding content-level semantic similarity,
/// catching related notes that share no tags/keywords but discuss the same
/// topic in different words (#3458).
pub fn score_pair_semantic(
    a: &NoteMeta,
    a_body: &str,
    b: &NoteMeta,
    b_body: &str,
    embedder: &dyn SemanticEmbedder,
) -> Option<InferredRelation> {
    if a.id == b.id {
        return None;
    }

    // Metadata overlap score (same as score_pair).
    let a_tags: HashSet<&str> = a.tags.iter().map(|s| s.as_str()).collect();
    let b_tags: HashSet<&str> = b.tags.iter().map(|s| s.as_str()).collect();
    let a_kws: HashSet<&str> = a.keywords.iter().map(|s| s.as_str()).collect();
    let b_kws: HashSet<&str> = b.keywords.iter().map(|s| s.as_str()).collect();
    let a_title_words = title_word_set(&a.title);
    let b_title_words = title_word_set(&b.title);

    let tag_sim = jaccard(&a_tags, &b_tags);
    let kw_sim = jaccard(&a_kws, &b_kws);
    let title_sim = jaccard(&a_title_words, &b_title_words);
    let metadata_score = tag_sim * 0.40 + kw_sim * 0.35 + title_sim * 0.25;

    // Semantic vector similarity from content bodies.
    let semantic_score: f64 = {
        let a_vec = embedder.embed(&format!("{} {}", a.title, a_body));
        let b_vec = embedder.embed(&format!("{} {}", b.title, b_body));
        match (a_vec, b_vec) {
            (Some(av), Some(bv)) => {
                let cos = cosine_similarity(&av, &bv);
                // cosine_similarity returns f32; normalize to [0, 1] and cast to f64.
                (((cos as f64) + 1.0) / 2.0).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    };

    // Combined confidence.
    let confidence = semantic_score * SEMANTIC_WEIGHT + metadata_score * METADATA_WEIGHT;

    if confidence < f64::EPSILON {
        return None;
    }

    let (relation_type, reason) = if semantic_score >= 0.6 && metadata_score >= 0.3 {
        (
            RelationType::SameTopic,
            format!(
                "Strong semantic + metadata match (semantic: {:.0}%, metadata: {:.0}%)",
                semantic_score * 100.0,
                metadata_score * 100.0
            ),
        )
    } else if semantic_score >= 0.5 {
        (
            RelationType::Related,
            format!(
                "Content-level semantic similarity ({:.0}%) with some metadata overlap ({:.0}%)",
                semantic_score * 100.0,
                metadata_score * 100.0
            ),
        )
    } else if metadata_score >= 0.3 {
        (
            RelationType::Related,
            format!(
                "Metadata overlap (tags: {:.0}%, keywords: {:.0}%) reinforced by semantic signal ({:.0}%)",
                tag_sim * 100.0,
                kw_sim * 100.0,
                semantic_score * 100.0
            ),
        )
    } else {
        return None;
    };

    Some(InferredRelation {
        source: a.id.clone(),
        target: b.id.clone(),
        relation_type,
        confidence,
        reason,
    })
}

/// Suggest auto-links for a single note: find the most semantically similar
/// notes in the vault that are NOT already linked via `[[wikilink]]`.
///
/// This is the core backend function for the "Heads Up" / auto-link feature
/// (#3458). It:
/// 1. Loads all note metas and bodies.
/// 2. Computes semantic + metadata similarity for each pair.
/// 3. Excludes notes already linked from the source note's body.
/// 4. Returns top-N suggestions sorted by confidence.
///
/// Returns suggestions sorted by confidence (descending).
pub fn suggest_auto_links(
    context: &StorageContext,
    source_note_id: &str,
    max_suggestions: usize,
) -> Result<Vec<AutoLinkSuggestion>> {
    let (connection, _) = storage::pool::open_connection(context)?;
    let all_metas = storage::list_all_note_metas(&connection)?;
    let embedder = default_embedder();

    // Find the source note.
    let source_meta = all_metas
        .iter()
        .find(|m| m.id == source_note_id)
        .ok_or_else(|| anyhow::anyhow!("source note not found: {}", source_note_id))?;

    // Load source note body and extract existing wikilink targets.
    let source_doc = storage::notes::load_note_with_context(context, source_note_id)?;
    let existing_links: HashSet<String> = storage::notes::extract_wikilinks(&source_doc.body)
        .into_iter()
        .map(|(target, _)| target.to_lowercase())
        .collect();

    let source_body = &source_doc.body;

    // Score against every other note.
    let mut suggestions: Vec<AutoLinkSuggestion> = Vec::new();

    for target_meta in &all_metas {
        if target_meta.id == source_note_id {
            continue;
        }

        // Skip if already linked.
        if existing_links.contains(&target_meta.title.to_lowercase()) {
            continue;
        }

        let target_doc = match storage::notes::load_note_with_context(context, &target_meta.id) {
            Ok(doc) => doc,
            Err(_) => continue,
        };

        // Compute combined score.
        let a_tags: HashSet<&str> = source_meta.tags.iter().map(|s| s.as_str()).collect();
        let b_tags: HashSet<&str> = target_meta.tags.iter().map(|s| s.as_str()).collect();
        let a_kws: HashSet<&str> = source_meta.keywords.iter().map(|s| s.as_str()).collect();
        let b_kws: HashSet<&str> = target_meta.keywords.iter().map(|s| s.as_str()).collect();
        let a_title = title_word_set(&source_meta.title);
        let b_title = title_word_set(&target_meta.title);

        let tag_sim = jaccard(&a_tags, &b_tags);
        let kw_sim = jaccard(&a_kws, &b_kws);
        let title_sim = jaccard(&a_title, &b_title);
        let metadata_score = tag_sim * 0.40 + kw_sim * 0.35 + title_sim * 0.25;

        let semantic_score: f64 = {
            let a_vec = embedder.embed(&format!("{} {}", source_meta.title, source_body));
            let b_vec = embedder.embed(&format!("{} {}", target_meta.title, target_doc.body));
            match (a_vec, b_vec) {
                (Some(av), Some(bv)) => {
                    let cos = cosine_similarity(&av, &bv);
                    (((cos as f64) + 1.0) / 2.0).clamp(0.0, 1.0)
                }
                _ => 0.0,
            }
        };

        let confidence = semantic_score * SEMANTIC_WEIGHT + metadata_score * METADATA_WEIGHT;

        // Only suggest notes with meaningful similarity.
        if confidence < 0.15 {
            continue;
        }

        let reason = if semantic_score >= 0.5 {
            format!(
                "Content-level similarity ({:.0}%) with metadata overlap ({:.0}%)",
                semantic_score * 100.0,
                metadata_score * 100.0
            )
        } else {
            format!(
                "Shared metadata (tags: {:.0}%, keywords: {:.0}%)",
                tag_sim * 100.0,
                kw_sim * 100.0
            )
        };

        suggestions.push(AutoLinkSuggestion {
            source_id: source_note_id.to_string(),
            target_id: target_meta.id.clone(),
            target_title: target_meta.title.clone(),
            confidence,
            semantic_similarity: semantic_score,
            metadata_similarity: metadata_score,
            reason,
        });
    }

    // Sort by confidence descending and truncate.
    suggestions.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    suggestions.truncate(max_suggestions);

    Ok(suggestions)
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
                kind: GraphEdgeKind::Wikilink,
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
                kind: GraphEdgeKind::Wikilink,
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
    fn test_detect_unlinked_mention_edges() {
        // (id, title, body)
        let notes: Vec<(String, String, String)> = vec![
            (
                "a".into(),
                "Rust".into(),
                "I love Rust and systems programming.".into(),
            ),
            (
                "b".into(),
                "Python".into(),
                "Rust is faster than Python for this task.".into(),
            ),
            ("c".into(), "Go".into(), "Nothing interesting here.".into()),
        ];
        // "b" mentions "Rust" in prose → one soft edge b -> a.
        let edges = detect_unlinked_mention_edges(&notes, &BTreeSet::new());
        assert_eq!(edges.len(), 1, "expected exactly one mention edge");
        assert_eq!(edges[0].source, "b");
        assert_eq!(edges[0].target, "a");
        assert_eq!(edges[0].kind, GraphEdgeKind::Mention);
    }

    #[test]
    fn test_detect_unlinked_mention_excludes_wikilinks() {
        let notes: Vec<(String, String, String)> = vec![
            ("a".into(), "Rust".into(), "body a".into()),
            (
                "b".into(),
                "Python".into(),
                "See [[Rust]] and also mention Rust again.".into(),
            ),
        ];
        // b already wikilinks to a → excluded from mention edges.
        let mut exclude = BTreeSet::new();
        exclude.insert(("b".into(), "a".into()));
        let edges = detect_unlinked_mention_edges(&notes, &exclude);
        assert!(edges.is_empty(), "wikilinked pair must be excluded");
    }

    #[test]
    fn test_detect_unlinked_mention_skips_short_titles() {
        let notes: Vec<(String, String, String)> = vec![
            ("a".into(), "Go".into(), "body a".into()),
            ("b".into(), "Rust".into(), "mentions Go here".into()),
        ];
        // "Go" is 2 chars → ignored, no edge.
        let edges = detect_unlinked_mention_edges(&notes, &BTreeSet::new());
        assert!(
            edges.is_empty(),
            "titles shorter than 3 chars must be ignored"
        );
    }

    #[test]
    fn test_render_dot_mention_dashed() {
        let graph = KnowledgeGraph {
            nodes: vec![
                GraphNode {
                    id: "a".into(),
                    title: "Rust".into(),
                    tags: vec![],
                    in_degree: 1,
                    out_degree: 0,
                },
                GraphNode {
                    id: "b".into(),
                    title: "Python".into(),
                    tags: vec![],
                    in_degree: 0,
                    out_degree: 1,
                },
            ],
            edges: vec![GraphEdge {
                source: "b".into(),
                target: "a".into(),
                label: "Rust".into(),
                kind: GraphEdgeKind::Mention,
            }],
            note_count: 2,
            edge_count: 1,
            dangling_link_count: 0,
        };
        let dot = render_dot(&graph);
        assert!(
            dot.contains("style=dashed"),
            "mention edge must render dashed"
        );
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

    /// Regression: infer_relations must dedup BEFORE per-source truncation.
    /// When max_per_note=1 and B→A is the top score for source B but (A,B) was
    /// already claimed by source A, the next unique pair B→C must survive.
    /// (Issue #3387)
    #[test]
    fn test_infer_relations_dedup_before_truncation() {
        // Create 3 notes where A→B has the best score, A→C and B→C less so.
        // Tags and keywords are tuned to produce: A→B > A→C = B→C.
        let notes = vec![
            NoteMeta {
                id: "A".into(),
                title: "Alpha".into(),
                tags: vec!["common".into()],
                keywords: vec!["common_kw".into()],
                ..Default::default()
            },
            NoteMeta {
                id: "B".into(),
                title: "Beta".into(),
                tags: vec!["common".into()],
                keywords: vec!["common_kw".into(), "extra_b".into()],
                ..Default::default()
            },
            NoteMeta {
                id: "C".into(),
                title: "Gamma".into(),
                tags: vec!["common".into()],
                keywords: vec!["extra_c".into()],
                ..Default::default()
            },
        ];

        let config = InferenceConfig {
            min_confidence: 0.0, // accept everything
            max_per_note: 1,     // aggressive truncation exposes the bug
        };

        let relations = infer_relations(&notes, &config);

        // Must have 2 unique relations, not 1:
        //   A→B (or B→A) from source A
        //   B→C (the unique pair that would have been lost under the old per-source-then-dedup ordering)
        assert_eq!(
            relations.len(),
            2,
            "expected 2 unique relations (A↔B and B↔C), got {}: {:?}",
            relations.len(),
            relations
        );

        // Verify the unordered pairs are exactly {A,B} and {B,C}.
        let pairs: Vec<(&str, &str)> = relations
            .iter()
            .map(|r| {
                if r.source <= r.target {
                    (r.source.as_str(), r.target.as_str())
                } else {
                    (r.target.as_str(), r.source.as_str())
                }
            })
            .collect();
        assert!(pairs.contains(&("A", "B")), "missing pair A-B");
        assert!(pairs.contains(&("B", "C")), "missing pair B-C");
        assert!(
            !pairs.contains(&("A", "C")),
            "unexpected pair A-C (max_per_note=1 should drop it)"
        );
    }
}
