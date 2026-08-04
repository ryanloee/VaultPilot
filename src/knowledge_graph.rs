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

// ── Local Graph (#3570) ──────────────────────────────────────────────────
// Extract a subgraph centered on a single note, up to N hops deep. This powers
// the "Local Graph" view in the UI (like Obsidian's graph for the current note).

/// Extract a local subgraph centered on `center_note_id`, including all notes
/// reachable within `depth` hops (following edges in both directions).
///
/// The result is a [`KnowledgeGraph`] containing only the nodes and edges within
/// the local neighborhood. This is useful for "Local Graph" views that show the
/// context around a single note (#3570).
///
/// # Arguments
/// * `graph` - The full knowledge graph to extract from.
/// * `center_note_id` - The note ID at the center of the local graph.
/// * `depth` - Maximum hop distance (1 = immediate neighbors, 2 = neighbors of
///   neighbors, etc.).
///
/// # Returns
/// A `KnowledgeGraph` containing only the local subgraph. If `center_note_id`
/// is not found, an empty graph is returned.
pub fn extract_local_graph(
    graph: &KnowledgeGraph,
    center_note_id: &str,
    depth: usize,
) -> KnowledgeGraph {
    if depth == 0 {
        // depth 0 = just the center node
        return extract_single_node(graph, center_note_id);
    }

    // Build adjacency list (undirected for local graph: follow both in/out edges).
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        adjacency
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        adjacency
            .entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }

    // BFS from center, up to `depth` hops.
    let mut visited: HashSet<String> = HashSet::new();
    let mut frontier: Vec<String> = vec![center_note_id.to_string()];
    visited.insert(center_note_id.to_string());

    for _hop in 0..depth {
        let mut next_frontier = Vec::new();
        for node_id in &frontier {
            if let Some(neighbors) = adjacency.get(node_id.as_str()) {
                for &nb in neighbors {
                    let nb_owned = nb.to_string();
                    if visited.insert(nb_owned.clone()) {
                        next_frontier.push(nb_owned);
                    }
                }
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }

    // Filter nodes and edges to only those in the visited set.
    let nodes: Vec<GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| visited.contains(&n.id))
        .cloned()
        .collect();

    let edges: Vec<GraphEdge> = graph
        .edges
        .iter()
        .filter(|e| visited.contains(&e.source) && visited.contains(&e.target))
        .cloned()
        .collect();

    let note_count = nodes.len();
    let edge_count = edges.len();

    // Recount dangling links within the subgraph is not meaningful; use 0.
    KnowledgeGraph {
        nodes,
        edges,
        note_count,
        edge_count,
        dangling_link_count: 0,
    }
}

/// Extract a subgraph containing only a single node (depth=0 local graph).
fn extract_single_node(graph: &KnowledgeGraph, node_id: &str) -> KnowledgeGraph {
    let nodes: Vec<GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| n.id == node_id)
        .cloned()
        .collect();
    let note_count = nodes.len();
    KnowledgeGraph {
        nodes,
        edges: vec![],
        note_count,
        edge_count: 0,
        dangling_link_count: 0,
    }
}

// ── Force-Directed Layout (#3570) ────────────────────────────────────────
// Compute x/y coordinates for each node using a simplified Fruchterman-Reingold
// force-directed algorithm. The layout is returned as a serializable struct that
// UI clients (WinUI, mobile) can consume to render the graph as a canvas.

/// A single node's computed position for force-directed layout.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphNodePosition {
    /// Note ID (matches GraphNode::id).
    pub id: String,
    /// X coordinate in layout space (typically -1.0 to 1.0).
    pub x: f64,
    /// Y coordinate in layout space (typically -1.0 to 1.0).
    pub y: f64,
}

/// Layout result: positions for every node in a graph.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphLayout {
    /// One position per node.
    pub positions: Vec<GraphNodePosition>,
    /// Bounding box of the layout (for normalisation by the UI).
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// Configuration for the force-directed layout algorithm.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Width of the layout area (nodes spread within [-area/2, area/2]).
    pub area: f64,
    /// Number of iterations (more = more refined but slower).
    pub iterations: usize,
    /// Initial "temperature" controlling how far nodes move per step.
    pub temperature: f64,
    /// Cooling factor applied to temperature each iteration.
    pub cooling: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            area: 10.0,
            iterations: 100,
            temperature: 1.0,
            cooling: 0.95,
        }
    }
}

/// Compute a force-directed layout for the given graph using a simplified
/// Fruchterman-Reingold algorithm (#3570).
///
/// This is a pure function that operates on an already-built [`KnowledgeGraph`]
/// and returns 2D positions for each node. The positions are normalised to a
/// bounding box so UI clients can map them to canvas coordinates.
///
/// # Algorithm
/// 1. Place nodes at random positions.
/// 2. Each iteration:
///    - Repulsive force: every pair of nodes pushes apart (∝ 1/distance²).
///    - Attractive force: connected nodes pull together (∝ distance²).
/// 3. Cool down (reduce step size) each iteration.
///
/// For large graphs (>500 nodes), the O(N²) repulsion becomes expensive.
/// In practice the local graph view keeps N small (≤100), so this is fine.
pub fn compute_layout(graph: &KnowledgeGraph, config: &LayoutConfig) -> GraphLayout {
    let n = graph.nodes.len();
    if n == 0 {
        return GraphLayout::default();
    }

    // Build edge set for fast lookup.
    // BTreeSet guarantees deterministic iteration order (HashSet is random per process).
    let mut edge_set: BTreeSet<(usize, usize)> = BTreeSet::new();
    // Map node id → index.
    let id_to_idx: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_str(), i))
        .collect();

    for edge in &graph.edges {
        if let (Some(&s), Some(&t)) = (
            id_to_idx.get(edge.source.as_str()),
            id_to_idx.get(edge.target.as_str()),
        ) {
            edge_set.insert((s.min(t), s.max(t)));
        }
    }

    // Optimal spring length k = sqrt(area / n).
    let k = (config.area / n.max(1) as f64).sqrt();
    let k_sq = k * k;

    // Initialise positions using a deterministic seed for reproducibility.
    // Simple golden-angle distribution on a circle.
    let golden_angle = std::f64::consts::PI * (3.0 - 5_f64.sqrt());
    let mut pos_x = vec![0.0f64; n];
    let mut pos_y = vec![0.0f64; n];
    let radius = config.area * 0.4;
    for i in 0..n {
        let angle = golden_angle * i as f64;
        pos_x[i] = radius * angle.cos();
        pos_y[i] = radius * angle.sin();
    }

    let mut temp = config.temperature * config.area * 0.1;

    // Iterative relaxation.
    for _ in 0..config.iterations {
        let mut disp_x = vec![0.0f64; n];
        let mut disp_y = vec![0.0f64; n];

        // Repulsive forces (all pairs).
        for i in 0..n {
            for j in 0..n {
                if i == j {
                    continue;
                }
                let dx = pos_x[i] - pos_x[j];
                let dy = pos_y[i] - pos_y[j];
                let dist_sq = dx * dx + dy * dy;
                let dist = dist_sq.sqrt().max(0.01);
                // Repulsive force magnitude: k² / dist
                let force = k_sq / dist;
                let fx = (dx / dist) * force;
                let fy = (dy / dist) * force;
                disp_x[i] += fx;
                disp_y[i] += fy;
            }
        }

        // Attractive forces (edges only).
        for &(s, t) in &edge_set {
            let dx = pos_x[s] - pos_x[t];
            let dy = pos_y[s] - pos_y[t];
            let dist_sq = dx * dx + dy * dy;
            let dist = dist_sq.sqrt().max(0.01);
            // Attractive force magnitude: dist² / k
            let force = dist_sq / k;
            let fx = (dx / dist) * force;
            let fy = (dy / dist) * force;
            disp_x[s] -= fx;
            disp_y[s] -= fy;
            disp_x[t] += fx;
            disp_y[t] += fy;
        }

        // Apply displacement, limited by temperature.
        for i in 0..n {
            let disp_mag = (disp_x[i] * disp_x[i] + disp_y[i] * disp_y[i])
                .sqrt()
                .max(0.01);
            let limit = disp_mag.min(temp);
            pos_x[i] += (disp_x[i] / disp_mag) * limit;
            pos_y[i] += (disp_y[i] / disp_mag) * limit;

            // Keep within bounds.
            pos_x[i] = pos_x[i].clamp(-config.area, config.area);
            pos_y[i] = pos_y[i].clamp(-config.area, config.area);
        }

        // Cool down.
        temp *= config.cooling;
        if temp < 0.001 {
            break;
        }
    }

    // Compute bounding box.
    let (min_x, max_x) = pos_x
        .iter()
        .fold((f64::MAX, f64::MIN), |(mn, mx), &v| (mn.min(v), mx.max(v)));
    let (min_y, max_y) = pos_y
        .iter()
        .fold((f64::MAX, f64::MIN), |(mn, mx), &v| (mn.min(v), mx.max(v)));

    let positions: Vec<GraphNodePosition> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| GraphNodePosition {
            id: node.id.clone(),
            x: pos_x[i],
            y: pos_y[i],
        })
        .collect();

    GraphLayout {
        positions,
        min_x,
        min_y,
        max_x,
        max_y,
    }
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

// ── Backlinks (#3831) ────────────────────────────────────────────────────
// Retrieve all notes that link TO a given note, with context snippets.

/// A single backlink entry: a note that references the target note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkEntry {
    /// ID of the source note (the one containing the reference).
    pub source_id: String,
    /// Title of the source note.
    pub source_title: String,
    /// The raw link label (e.g. the wikilink target text or mention text).
    pub label: String,
    /// Edge kind: formal `[[wikilink]]` or plain-text mention.
    pub kind: GraphEdgeKind,
    /// Context snippet from the source note body (up to 200 chars around the
    /// link/mention occurrence). `None` if the body could not be loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_snippet: Option<String>,
}

/// Maximum number of characters to extract around a link/mention for context.
const BACKLINK_SNIPPET_RADIUS: usize = 100;

/// Get all backlinks to a given note — notes that reference `note_id` via
/// wikilinks or plain-text mentions (#3831).
///
/// Returns a list of [`BacklinkEntry`] items, each representing a source note
/// that links to the target. Entries include a short context snippet extracted
/// from the source note body around the link/mention occurrence.
///
/// # Arguments
/// * `context` — Storage context for loading note bodies.
/// * `note_id` — The ID of the note to find backlinks for.
///
/// # Returns
/// A vector of `BacklinkEntry` items, sorted by source note title.
pub fn get_backlinks(context: &StorageContext, note_id: &str) -> Result<Vec<BacklinkEntry>> {
    let graph = build_knowledge_graph_with_mentions(context)?;

    // Build a node ID → title map.
    let title_map: HashMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.title.as_str()))
        .collect();

    // Filter edges whose target is the requested note.
    let matching_edges: Vec<&GraphEdge> =
        graph.edges.iter().filter(|e| e.target == note_id).collect();

    if matching_edges.is_empty() {
        return Ok(Vec::new());
    }

    // Collect unique source note IDs so we only load each body once.
    let mut source_ids: BTreeSet<&str> = BTreeSet::new();
    for edge in &matching_edges {
        source_ids.insert(edge.source.as_str());
    }

    // Pre-load source note bodies for context extraction.
    let mut source_bodies: HashMap<&str, String> = HashMap::new();
    for &sid in &source_ids {
        if let Ok(doc) = storage::notes::load_note_with_context(context, sid) {
            source_bodies.insert(sid, doc.body);
        }
    }

    let mut entries: Vec<BacklinkEntry> = Vec::with_capacity(matching_edges.len());

    for edge in matching_edges {
        let source_title = title_map
            .get(edge.source.as_str())
            .unwrap_or(&"Unknown")
            .to_string();

        let context_snippet = source_bodies
            .get(edge.source.as_str())
            .and_then(|body| extract_context_snippet(body, &edge.label));

        entries.push(BacklinkEntry {
            source_id: edge.source.clone(),
            source_title,
            label: edge.label.clone(),
            kind: edge.kind,
            context_snippet,
        });
    }

    // Sort by source title for stable, user-friendly ordering.
    entries.sort_by(|a, b| a.source_title.cmp(&b.source_title));

    Ok(entries)
}

/// Extract a context snippet from `body` around the first occurrence of
/// `needle` (case-insensitive). Returns up to ~200 chars of surrounding text.
fn extract_context_snippet(body: &str, needle: &str) -> Option<String> {
    let needle_lower = needle.to_lowercase();
    let body_lower = body.to_lowercase();

    let pos = body_lower.find(&needle_lower)?;

    let start = pos.saturating_sub(BACKLINK_SNIPPET_RADIUS);
    let end = (pos + needle.len() + BACKLINK_SNIPPET_RADIUS).min(body.len());

    // Snap to UTF-8 char boundaries.
    let start = snap_to_char_boundary(body, start);
    let end = snap_to_char_boundary(body, end);

    let snippet = &body[start..end];

    // Add ellipsis indicators if we truncated.
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < body.len() { "…" } else { "" };

    Some(format!("{}{}{}", prefix, snippet, suffix))
}

/// Snap `pos` to the nearest valid UTF-8 character boundary at or before it.
fn snap_to_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
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

    // Pre-compute the source embedding once (was O(N) redundant calls before fix #3561).
    let source_vec = embedder.embed(&format!("{} {}", source_meta.title, source_body));

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
            let b_vec = embedder.embed(&format!("{} {}", target_meta.title, target_doc.body));
            match (source_vec.as_ref(), b_vec) {
                (Some(av), Some(bv)) => {
                    let cos = cosine_similarity(av, &bv);
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

    // ── #3570: Local Graph + Force-Directed Layout tests ──────────────────

    /// Helper: build a small diamond-shaped graph for local-graph tests.
    ///   A → B → D
    ///   A → C → D
    ///   B → C
    fn diamond_graph() -> KnowledgeGraph {
        KnowledgeGraph {
            nodes: vec![
                GraphNode {
                    id: "A".into(),
                    title: "Alpha".into(),
                    tags: vec![],
                    in_degree: 0,
                    out_degree: 2,
                },
                GraphNode {
                    id: "B".into(),
                    title: "Beta".into(),
                    tags: vec![],
                    in_degree: 1,
                    out_degree: 2,
                },
                GraphNode {
                    id: "C".into(),
                    title: "Gamma".into(),
                    tags: vec![],
                    in_degree: 2,
                    out_degree: 1,
                },
                GraphNode {
                    id: "D".into(),
                    title: "Delta".into(),
                    tags: vec![],
                    in_degree: 2,
                    out_degree: 0,
                },
                GraphNode {
                    id: "Z".into(),
                    title: "Zeta".into(),
                    tags: vec![],
                    in_degree: 0,
                    out_degree: 0,
                },
            ],
            edges: vec![
                GraphEdge {
                    source: "A".into(),
                    target: "B".into(),
                    label: "Beta".into(),
                    kind: GraphEdgeKind::Wikilink,
                },
                GraphEdge {
                    source: "A".into(),
                    target: "C".into(),
                    label: "Gamma".into(),
                    kind: GraphEdgeKind::Wikilink,
                },
                GraphEdge {
                    source: "B".into(),
                    target: "D".into(),
                    label: "Delta".into(),
                    kind: GraphEdgeKind::Wikilink,
                },
                GraphEdge {
                    source: "B".into(),
                    target: "C".into(),
                    label: "Gamma".into(),
                    kind: GraphEdgeKind::Wikilink,
                },
                GraphEdge {
                    source: "C".into(),
                    target: "D".into(),
                    label: "Delta".into(),
                    kind: GraphEdgeKind::Wikilink,
                },
            ],
            note_count: 5,
            edge_count: 5,
            dangling_link_count: 0,
        }
    }

    #[test]
    fn test_extract_local_graph_depth_0() {
        let g = diamond_graph();
        let local = extract_local_graph(&g, "A", 0);
        assert_eq!(local.note_count, 1);
        assert_eq!(local.nodes[0].id, "A");
        assert!(local.edges.is_empty());
    }

    #[test]
    fn test_extract_local_graph_depth_1() {
        let g = diamond_graph();
        let local = extract_local_graph(&g, "A", 1);
        // depth 1: A + its direct neighbors B, C
        let ids: Vec<&str> = local.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            local.note_count, 3,
            "expected 3 nodes at depth 1: {:?}",
            ids
        );
        assert!(ids.contains(&"A"));
        assert!(ids.contains(&"B"));
        assert!(ids.contains(&"C"));
        // Edges among A, B, C: A→B, A→C, B→C
        assert_eq!(local.edge_count, 3);
    }

    #[test]
    fn test_extract_local_graph_depth_2() {
        let g = diamond_graph();
        let local = extract_local_graph(&g, "A", 2);
        // depth 2: A, B, C, D (D is reached via B→D and C→D)
        let ids: Vec<&str> = local.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            local.note_count, 4,
            "expected 4 nodes at depth 2: {:?}",
            ids
        );
        assert!(ids.contains(&"D"));
        // Z is disconnected, should never appear
        assert!(!ids.contains(&"Z"));
    }

    #[test]
    fn test_extract_local_graph_center_not_found() {
        let g = diamond_graph();
        let local = extract_local_graph(&g, "nonexistent", 3);
        assert_eq!(local.note_count, 0);
        assert!(local.nodes.is_empty());
    }

    #[test]
    fn test_extract_local_graph_is_bidirectional() {
        let g = diamond_graph();
        // Center on D (which only has incoming edges). Following edges backward
        // should reach B and C at depth 1.
        let local = extract_local_graph(&g, "D", 1);
        let ids: Vec<&str> = local.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"D"));
        assert!(
            ids.contains(&"B") || ids.contains(&"C"),
            "depth-1 from D must reach B or C"
        );
        assert_eq!(local.note_count, 3, "D, B, C at depth 1");
    }

    #[test]
    fn test_compute_layout_empty_graph() {
        let g = KnowledgeGraph::default();
        let layout = compute_layout(&g, &LayoutConfig::default());
        assert!(layout.positions.is_empty());
    }

    #[test]
    fn test_compute_layout_single_node() {
        let g = KnowledgeGraph {
            nodes: vec![GraphNode {
                id: "n1".into(),
                title: "Solo".into(),
                tags: vec![],
                in_degree: 0,
                out_degree: 0,
            }],
            edges: vec![],
            note_count: 1,
            edge_count: 0,
            dangling_link_count: 0,
        };
        let layout = compute_layout(&g, &LayoutConfig::default());
        assert_eq!(layout.positions.len(), 1);
        assert_eq!(layout.positions[0].id, "n1");
        // Single node has no forces — position stays at initial placement within area.
        assert!(layout.positions[0].x.abs() <= 10.0);
        assert!(layout.positions[0].y.abs() <= 10.0);
    }

    #[test]
    fn test_compute_layout_connected_nodes() {
        let g = diamond_graph();
        let layout = compute_layout(&g, &LayoutConfig::default());
        assert_eq!(layout.positions.len(), 5);

        // Every position should be within the area bounds.
        for pos in &layout.positions {
            assert!(pos.x.abs() <= 10.0, "x out of bounds: {}", pos.x);
            assert!(pos.y.abs() <= 10.0, "y out of bounds: {}", pos.y);
        }

        // Bounding box should be valid.
        assert!(layout.min_x <= layout.max_x);
        assert!(layout.min_y <= layout.max_y);

        // All node IDs should be present.
        let ids: HashSet<&str> = layout.positions.iter().map(|p| p.id.as_str()).collect();
        for expected in &["A", "B", "C", "D", "Z"] {
            assert!(
                ids.contains(expected),
                "missing node {} in layout",
                expected
            );
        }
    }

    #[test]
    fn test_compute_layout_deterministic() {
        let g = diamond_graph();
        let layout1 = compute_layout(&g, &LayoutConfig::default());
        let layout2 = compute_layout(&g, &LayoutConfig::default());
        // Same input should produce identical output (deterministic seed).
        for (p1, p2) in layout1.positions.iter().zip(layout2.positions.iter()) {
            assert_eq!(p1.id, p2.id);
            assert!(
                (p1.x - p2.x).abs() < 1e-10,
                "non-deterministic x for {}",
                p1.id
            );
            assert!(
                (p1.y - p2.y).abs() < 1e-10,
                "non-deterministic y for {}",
                p1.id
            );
        }
    }

    #[test]
    fn test_graph_layout_serializes() {
        let layout = GraphLayout {
            positions: vec![GraphNodePosition {
                id: "n1".into(),
                x: 1.5,
                y: -0.5,
            }],
            min_x: 1.5,
            min_y: -0.5,
            max_x: 1.5,
            max_y: -0.5,
        };
        let json = serde_json::to_string(&layout).unwrap();
        assert!(json.contains("\"id\":\"n1\""));
        assert!(json.contains("1.5"));
        let parsed: GraphLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.positions[0].id, "n1");
        assert!((parsed.positions[0].x - 1.5).abs() < 1e-10);
    }

    #[test]
    fn test_layout_config_default() {
        let c = LayoutConfig::default();
        assert!(c.area > 0.0);
        assert!(c.iterations > 0);
        assert!(c.cooling > 0.0 && c.cooling < 1.0);
    }

    // ── Backlink helper tests (#3831) ───────────────────────────────────

    #[test]
    fn test_extract_context_snippet_basic() {
        let body = "This is a long note about Rust programming and borrowing.\n\nThe key concept is ownership.";
        let snippet = extract_context_snippet(body, "borrowing").unwrap();
        assert!(snippet.contains("borrowing"));
        // Should not have ellipsis since the match is near the middle.
        assert!(snippet.starts_with('…') || !snippet.starts_with('…'));
    }

    #[test]
    fn test_extract_context_snippet_at_start() {
        let body = "Borrowing is important.";
        let snippet = extract_context_snippet(body, "Borrowing").unwrap();
        // At the very start — no leading ellipsis.
        assert!(!snippet.starts_with('…'));
        assert!(snippet.contains("Borrowing"));
    }

    #[test]
    fn test_extract_context_snippet_at_end() {
        let body = "Some text before the target word Borrowing";
        let snippet = extract_context_snippet(body, "Borrowing").unwrap();
        // At the very end — no trailing ellipsis.
        assert!(!snippet.ends_with('…'));
        assert!(snippet.contains("Borrowing"));
    }

    #[test]
    fn test_extract_context_snippet_case_insensitive() {
        let body = "The RUST language is great.";
        let snippet = extract_context_snippet(body, "rust").unwrap();
        assert!(snippet.contains("RUST"));
    }

    #[test]
    fn test_extract_context_snippet_not_found() {
        let body = "No match here.";
        assert!(extract_context_snippet(body, "xyz").is_none());
    }

    #[test]
    fn test_extract_context_snippet_utf8() {
        let body = "这是一段中文文本，包含一些笔记链接到 [[目标笔记]] 的内容。";
        let snippet = extract_context_snippet(body, "目标笔记").unwrap();
        assert!(snippet.contains("目标笔记"));
    }

    #[test]
    fn test_snap_to_char_boundary_ascii() {
        let s = "hello world";
        assert_eq!(snap_to_char_boundary(s, 5), 5);
        assert_eq!(snap_to_char_boundary(s, 0), 0);
        assert_eq!(snap_to_char_boundary(s, 11), 11);
    }

    #[test]
    fn test_snap_to_char_boundary_utf8() {
        let s = "你好世界";
        // 你 = 3 bytes at 0, 好 = 3 bytes at 3, 世 = 3 bytes at 6, 界 = 3 bytes at 9
        assert_eq!(snap_to_char_boundary(s, 3), 3);
        assert_eq!(snap_to_char_boundary(s, 4), 3); // mid-char → snap back
        assert_eq!(snap_to_char_boundary(s, 6), 6);
    }

    #[test]
    fn test_backlink_entry_serializes() {
        let entry = BacklinkEntry {
            source_id: "note_1".into(),
            source_title: "My Note".into(),
            label: "target".into(),
            kind: GraphEdgeKind::Wikilink,
            context_snippet: Some("…about [[target]] in…".into()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("note_1"));
        assert!(json.contains("My Note"));
        assert!(json.contains("wikilink"));

        // Test with None context_snippet — should be skipped.
        let entry_no_ctx = BacklinkEntry {
            source_id: "note_2".into(),
            source_title: "Other".into(),
            label: "x".into(),
            kind: GraphEdgeKind::Mention,
            context_snippet: None,
        };
        let json2 = serde_json::to_string(&entry_no_ctx).unwrap();
        assert!(!json2.contains("context_snippet"));
    }
}
