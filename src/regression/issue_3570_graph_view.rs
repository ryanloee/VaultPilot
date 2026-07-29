//! Regression test for #3570: Graph View — local graph + force-directed layout.
//!
//! Verifies that:
//! 1. `extract_local_graph` correctly limits the subgraph to `depth` hops.
//! 2. `compute_layout` produces deterministic, in-bounds coordinates.
//! 3. The layout result serializes to JSON for UI consumption.

use crate::knowledge_graph::{
    self, GraphEdge, GraphEdgeKind, GraphLayout, GraphNode, KnowledgeGraph, LayoutConfig,
};

/// Build a small chain graph: A → B → C → D → E
fn chain_graph() -> KnowledgeGraph {
    KnowledgeGraph {
        nodes: vec![
            GraphNode {
                id: "A".into(),
                title: "Alpha".into(),
                tags: vec![],
                in_degree: 0,
                out_degree: 1,
            },
            GraphNode {
                id: "B".into(),
                title: "Beta".into(),
                tags: vec![],
                in_degree: 1,
                out_degree: 1,
            },
            GraphNode {
                id: "C".into(),
                title: "Gamma".into(),
                tags: vec![],
                in_degree: 1,
                out_degree: 1,
            },
            GraphNode {
                id: "D".into(),
                title: "Delta".into(),
                tags: vec![],
                in_degree: 1,
                out_degree: 1,
            },
            GraphNode {
                id: "E".into(),
                title: "Epsilon".into(),
                tags: vec![],
                in_degree: 1,
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
            GraphEdge {
                source: "D".into(),
                target: "E".into(),
                label: "Epsilon".into(),
                kind: GraphEdgeKind::Wikilink,
            },
        ],
        note_count: 5,
        edge_count: 4,
        dangling_link_count: 0,
    }
}

#[test]
fn regression_3570_local_graph_depth_limits_nodes() {
    let g = chain_graph();

    // depth 1 from C: should include C, B (backlink), D (forward link) = 3 nodes
    let local1 = knowledge_graph::extract_local_graph(&g, "C", 1);
    assert_eq!(
        local1.note_count, 3,
        "depth 1 from C should include C, B, D"
    );

    // depth 2 from C: should include C, B, D, A, E = 5 nodes (entire chain)
    let local2 = knowledge_graph::extract_local_graph(&g, "C", 2);
    assert_eq!(
        local2.note_count, 5,
        "depth 2 from C should include all 5 nodes"
    );

    // depth 1 from A (end of chain): A, B = 2 nodes
    let local_a = knowledge_graph::extract_local_graph(&g, "A", 1);
    assert_eq!(local_a.note_count, 2, "depth 1 from A should include A, B");
}

#[test]
fn regression_3570_local_graph_edges_are_subset() {
    let g = chain_graph();
    let local = knowledge_graph::extract_local_graph(&g, "B", 1);

    // depth 1 from B: nodes A, B, C → edges A→B, B→C (2 edges)
    assert_eq!(local.edge_count, 2);
    let sources: Vec<&str> = local.edges.iter().map(|e| e.source.as_str()).collect();
    let targets: Vec<&str> = local.edges.iter().map(|e| e.target.as_str()).collect();
    assert!(sources.contains(&"A") && targets.contains(&"B"));
    assert!(sources.contains(&"B") && targets.contains(&"C"));
}

#[test]
fn regression_3570_layout_produces_valid_positions() {
    let g = chain_graph();
    let layout = knowledge_graph::compute_layout(&g, &LayoutConfig::default());

    // Every node gets a position.
    assert_eq!(layout.positions.len(), g.nodes.len());

    // All positions within area bounds.
    for pos in &layout.positions {
        assert!(pos.x.abs() <= LayoutConfig::default().area);
        assert!(pos.y.abs() <= LayoutConfig::default().area);
    }

    // Bounding box is consistent.
    assert!(layout.min_x <= layout.max_x);
    assert!(layout.min_y <= layout.max_y);
}

#[test]
fn regression_3570_layout_is_deterministic() {
    let g = chain_graph();
    let l1 = knowledge_graph::compute_layout(&g, &LayoutConfig::default());
    let l2 = knowledge_graph::compute_layout(&g, &LayoutConfig::default());

    assert_eq!(l1.positions.len(), l2.positions.len());
    for (p1, p2) in l1.positions.iter().zip(l2.positions.iter()) {
        assert_eq!(p1.id, p2.id);
        let dx = (p1.x - p2.x).abs();
        let dy = (p1.y - p2.y).abs();
        // f64 computations should be deterministic, but allow tiny floating error.
        assert!(
            dx < 1e-12 && dy < 1e-12,
            "non-deterministic layout for {}: ({:.15}, {:.15}) vs ({:.15}, {:.15}), dx={}, dy={}",
            p1.id,
            p1.x,
            p1.y,
            p2.x,
            p2.y,
            dx,
            dy
        );
    }
}

#[test]
fn regression_3570_layout_json_serialization() {
    let layout = GraphLayout::default();
    let json = serde_json::to_string(&layout).unwrap();
    let parsed: GraphLayout = serde_json::from_str(&json).unwrap();
    assert!(parsed.positions.is_empty());
}

#[test]
fn regression_3570_local_graph_with_orphan() {
    // Add an orphan node Z that has no edges.
    let mut g = chain_graph();
    g.nodes.push(GraphNode {
        id: "Z".into(),
        title: "Zeta".into(),
        tags: vec![],
        in_degree: 0,
        out_degree: 0,
    });
    g.note_count = 6;

    // Local graph from A should never include the orphan Z.
    let local = knowledge_graph::extract_local_graph(&g, "A", 5);
    let ids: Vec<&str> = local.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(
        !ids.contains(&"Z"),
        "orphan Z must not appear in local graph"
    );
}
