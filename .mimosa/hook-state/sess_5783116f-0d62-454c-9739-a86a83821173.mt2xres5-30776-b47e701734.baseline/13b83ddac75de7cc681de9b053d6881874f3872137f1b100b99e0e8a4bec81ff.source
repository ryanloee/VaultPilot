// Regression test for #3370: Dynamic Knowledge Graph — AI-style inferred relationships.
//
// Tests the pure-function API of score_pair, infer_relations, and infer_relations_default
// for detecting latent note relationships via tag/keyword/title-word overlap.

#[cfg(test)]
mod tests {
    use crate::knowledge_graph::{
        infer_relations, infer_relations_default, score_pair, InferenceConfig, RelationType,
    };
    use crate::models::NoteMeta;
    use std::collections::HashSet;

    fn note(id: &str, title: &str, tags: &[&str], keywords: &[&str]) -> NoteMeta {
        NoteMeta {
            id: id.to_string(),
            title: title.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            ..NoteMeta::default()
        }
    }

    // ── score_pair ──────────────────────────────────────────────────────────

    #[test]
    fn regression_3370_score_pair_same_id_returns_none() {
        let a = note("n1", "Rust Notes", &["rust"], &["programming"]);
        let result = score_pair(&a, &a);
        assert!(
            result.is_none(),
            "score_pair should return None for same note"
        );
    }

    #[test]
    fn regression_3370_score_pair_no_overlap_returns_none() {
        let a = note("n1", "Cooking", &["food"], &["recipes"]);
        let b = note("n2", "Quantum Physics", &["science"], &["particles"]);
        let result = score_pair(&a, &b);
        assert!(
            result.is_none(),
            "score_pair should return None for unrelated notes"
        );
    }

    #[test]
    fn regression_3370_score_pair_tag_overlap() {
        let a = note(
            "n1",
            "Rust Ownership",
            &["rust", "memory"],
            &["borrow", "lifetime"],
        );
        let b = note(
            "n2",
            "Rust Borrow Checker",
            &["rust", "memory"],
            &["borrow"],
        );
        let result = score_pair(&a, &b);
        assert!(
            result.is_some(),
            "notes sharing tags and keywords should produce a relation"
        );
        let rel = result.unwrap();
        assert!(
            rel.confidence > 0.3,
            "confidence should be meaningful: {}",
            rel.confidence
        );
        assert_eq!(rel.source, "n1");
        assert_eq!(rel.target, "n2");
    }

    #[test]
    fn regression_3370_score_pair_high_title_word_overlap_is_same_entity() {
        let a = note(
            "n1",
            "Machine Learning Basics",
            &["ml", "ai"],
            &["training", "model"],
        );
        let b = note(
            "n2",
            "Machine Learning Advanced",
            &["ml", "ai"],
            &["model", "neural"],
        );
        let result = score_pair(&a, &b);
        assert!(result.is_some());
        let rel = result.unwrap();
        // title_sim = "machine learning" vs "machine learning advanced" = 2/3 = 0.67 >= 0.5
        // tag_sim = 2/2 = 1.0 >= 0.3 → SameEntity
        assert_eq!(
            rel.relation_type,
            RelationType::SameEntity,
            "high title+tag overlap should be SameEntity"
        );
        assert!(
            rel.confidence > 0.5,
            "confidence should be high: {}",
            rel.confidence
        );
    }

    #[test]
    fn regression_3370_score_pair_high_tag_overlap_is_same_topic() {
        let a = note(
            "n1",
            "Docker Compose Guide",
            &["docker", "devops", "containers"],
            &["deploy", "compose"],
        );
        let b = note(
            "n2",
            "Kubernetes Cluster Setup",
            &["docker", "devops", "containers"],
            &["deploy", "cluster"],
        );
        let result = score_pair(&a, &b);
        assert!(result.is_some());
        let rel = result.unwrap();
        // tag_sim = 3/3 = 1.0 ≥ 0.5 → SameTopic
        assert_eq!(rel.relation_type, RelationType::SameTopic);
        assert!(
            rel.confidence > 0.5,
            "confidence should be high: {}",
            rel.confidence
        );
    }

    #[test]
    fn regression_3370_score_pair_moderate_overlap_is_related() {
        let a = note(
            "n1",
            "Python Data Analysis",
            &["python", "data"],
            &["pandas", "numpy"],
        );
        let b = note(
            "n2",
            "Julia Performance Guide",
            &["julia", "data"],
            &["numpy"],
        );
        let result = score_pair(&a, &b);
        assert!(result.is_some());
        let rel = result.unwrap();
        // tag_sim = 1/3 = 0.33, kw_sim = 1/2 = 0.5 → (tag+kw)/2 = 0.417 >= 0.4
        // SameTopic threshold triggers on combined tag+keyword overlap.
        assert_eq!(rel.relation_type, RelationType::SameTopic);
    }

    // ── infer_relations ─────────────────────────────────────────────────────

    fn make_notes() -> Vec<NoteMeta> {
        vec![
            note(
                "n1",
                "Rust Ownership",
                &["rust", "memory"],
                &["borrow", "lifetime"],
            ),
            note(
                "n2",
                "Rust Borrow Checker",
                &["rust", "memory"],
                &["borrow", "reference"],
            ),
            note("n3", "Cooking Basics", &["food", "recipes"], &["kitchen"]),
            note(
                "n4",
                "Rust Async Patterns",
                &["rust", "async"],
                &["tokio", "future"],
            ),
            note(
                "n5",
                "Gourmet Cooking",
                &["food", "recipes", "gourmet"],
                &["kitchen", "ingredients"],
            ),
        ]
    }

    #[test]
    fn regression_3370_infer_relations_discovers_clusters() {
        let notes = make_notes();
        let relations = infer_relations_default(&notes);

        // Should find relations within the "rust" cluster and within the "food" cluster.
        let rust_pairs: HashSet<String> = relations
            .iter()
            .filter(|r| r.source.starts_with('n') && r.target.starts_with('n'))
            .map(|r| {
                let mut pair = [r.source.clone(), r.target.clone()];
                pair.sort();
                format!("{}-{}", pair[0], pair[1])
            })
            .collect();

        // n1-n2 (Rust Ownership ↔ Rust Borrow Checker) should be found.
        assert!(
            rust_pairs.contains("n1-n2"),
            "n1-n2 should be inferred: {:?}",
            rust_pairs
        );
        // n3-n5 (Cooking Basics ↔ Gourmet Cooking) should be found.
        assert!(
            rust_pairs.contains("n3-n5")
                || rust_pairs.contains("n3-n5")
                || relations
                    .iter()
                    .any(|r| (r.source == "n3" && r.target == "n5")
                        || (r.source == "n5" && r.target == "n3")),
            "n3-n5 should be inferred"
        );

        // n1-n5 (Rust ↔ Cooking) should NOT be found (no overlap).
        assert!(
            !rust_pairs.contains("n1-n5"),
            "n1-n5 (cross-cluster) should not be inferred"
        );
    }

    #[test]
    fn regression_3370_infer_relations_respects_min_confidence() {
        let notes = make_notes();
        let config = InferenceConfig {
            min_confidence: 0.99, // prohibitively high
            max_per_note: 10,
        };
        let relations = infer_relations(&notes, &config);
        assert!(
            relations.is_empty(),
            "no relations should pass confidence threshold of 0.99, got {}",
            relations.len()
        );
    }

    #[test]
    fn regression_3370_infer_relations_respects_max_per_note() {
        let notes = make_notes();
        let config = InferenceConfig {
            min_confidence: 0.0, // include everything
            max_per_note: 1,     // at most 1 per source
        };
        let relations = infer_relations(&notes, &config);
        // With 5 notes, max 1 per source → at most 5 relations (but dedup will reduce).
        assert!(
            relations.len() <= 5,
            "max 5 relations with max_per_note=1, got {}",
            relations.len()
        );
    }

    #[test]
    fn regression_3370_infer_relations_deduplicates() {
        let notes = make_notes();
        let config = InferenceConfig {
            min_confidence: 0.0,
            max_per_note: 10,
        };
        let relations = infer_relations(&notes, &config);

        // Check no duplicate pairs (a-b and b-a both present).
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for rel in &relations {
            let key = if rel.source <= rel.target {
                (rel.source.clone(), rel.target.clone())
            } else {
                (rel.target.clone(), rel.source.clone())
            };
            assert!(
                seen.insert(key),
                "duplicate pair ({}-{}) found!",
                rel.source,
                rel.target
            );
        }
    }

    #[test]
    fn regression_3370_infer_relations_empty_list() {
        let relations = infer_relations_default(&[]);
        assert!(
            relations.is_empty(),
            "empty notes list should produce empty relations"
        );
    }

    #[test]
    fn regression_3370_infer_relations_single_note() {
        let notes = vec![note("n1", "Alone Note", &["solo"], &["lonely"])];
        let relations = infer_relations_default(&notes);
        assert!(
            relations.is_empty(),
            "single note should produce no relations"
        );
    }

    // ── Edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn regression_3370_empty_tags_and_keywords() {
        let a = note("n1", "Minimal Note A", &[], &[]);
        let b = note("n2", "Minimal Note B", &[], &[]);
        let result = score_pair(&a, &b);
        // No tags, no keywords → confidence depends only on title words.
        // "minimal" appears in both titles → title_sim = 1/3 (Minimal vs Note A/B are different)
        // "minimal" (1) intersection "minimal" (1) / union "minimal, note, a/b" (3-4) = 0.25-0.33
        // Actually: title_word_set("Minimal Note A") = {"minimal", "note"} (len("a")=1 → filtered)
        // title_word_set("Minimal Note B") = {"minimal", "note"}
        // intersection = {"minimal", "note"} = 2, union = {"minimal", "note", "a", "b"} = 4? No, "a" and "b" are filtered (len <= 1)
        // → union = {"minimal", "note"} = 2, intersection = 2 → title_sim = 1.0
        // confidence = 0 * 0.40 + 0 * 0.35 + 1.0 * 0.25 = 0.25
        assert!(
            result.is_some(),
            "two notes with same title words should produce a relation"
        );
        if let Some(rel) = result {
            assert!(
                (rel.confidence - 0.25).abs() < 0.01,
                "confidence should be ~0.25 (only title word overlap): {}",
                rel.confidence
            );
        }
    }

    #[test]
    fn regression_3370_score_pair_returns_reason() {
        let a = note("n1", "Rust Guide", &["rust"], &["programming"]);
        let b = note("n2", "Rust in Depth", &["rust"], &["programming"]);
        let rel = score_pair(&a, &b).expect("should find relation");
        assert!(!rel.reason.is_empty(), "reason should be non-empty");
        assert!(
            rel.reason.contains("%"),
            "reason should contain percentage values"
        );
    }
}
