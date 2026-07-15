//! Regression test for #2832 — Knowledge Graph View must include
//! unlinked-mention (提及) edges, not just `[[wikilink]]` edges.
//!
//! This guards the `detect_unlinked_mention_edges` pure function and the
//! `GraphEdgeKind::Mention` discriminator added to the knowledge graph so
//! that latent plain-text connections between notes are surfaced.

use std::collections::BTreeSet;

use crate::knowledge_graph::{detect_unlinked_mention_edges, GraphEdgeKind};

#[test]
fn regression_2832_mention_edge_detected() {
    let notes = vec![
        (
            "n1".to_string(),
            "Project Alpha".to_string(),
            "See Project Beta for the deployment steps.".to_string(),
        ),
        (
            "n2".to_string(),
            "Project Beta".to_string(),
            "Referenced via [[Project Alpha]] explicitly.".to_string(),
        ),
        (
            "n3".to_string(),
            "Scratch".to_string(),
            "random text".to_string(),
        ),
    ];
    // n1 mentions "Project Beta" in prose but does NOT wikilink it → soft edge n1 → n2.
    // n2 formally wikilinks to n1 ([[Project Alpha]]); that pair must be excluded
    // from mention edges, mirroring build_knowledge_graph_impl's behaviour.
    let mut exclude = BTreeSet::new();
    exclude.insert(("n2".to_string(), "n1".to_string()));
    let edges = detect_unlinked_mention_edges(&notes, &exclude);
    let mention: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == GraphEdgeKind::Mention)
        .collect();
    assert_eq!(
        mention.len(),
        1,
        "expected exactly one unlinked mention edge"
    );
    assert_eq!(mention[0].source, "n1");
    assert_eq!(mention[0].target, "n2");
    assert_eq!(mention[0].label, "Project Beta");
}

#[test]
fn regression_2832_existing_wikilink_not_duplicated() {
    let notes = vec![
        (
            "n1".to_string(),
            "Project Alpha".to_string(),
            "See [[Project Beta]].".to_string(),
        ),
        (
            "n2".to_string(),
            "Project Beta".to_string(),
            "Mentions Project Alpha in prose too.".to_string(),
        ),
    ];
    // The resolved wikilink edge n1 -> n2 must be excluded from mention edges.
    let mut exclude = BTreeSet::new();
    exclude.insert(("n1".to_string(), "n2".to_string()));
    let edges = detect_unlinked_mention_edges(&notes, &exclude);
    // n2 mentions "Project Alpha" in prose → n2 -> n1 mention edge still appears.
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].source, "n2");
    assert_eq!(edges[0].target, "n1");
    assert_eq!(edges[0].kind, GraphEdgeKind::Mention);
}

#[test]
fn regression_2832_short_titles_ignored() {
    let notes = vec![
        ("n1".to_string(), "Go".to_string(), "body a".to_string()),
        (
            "n2".to_string(),
            "Rust".to_string(),
            "this note mentions Go in passing".to_string(),
        ),
    ];
    // "Go" is 2 chars → below the threshold, must not produce an edge.
    let edges = detect_unlinked_mention_edges(&notes, &BTreeSet::new());
    assert!(
        edges.is_empty(),
        "titles shorter than 3 chars must be ignored to avoid false positives"
    );
}
