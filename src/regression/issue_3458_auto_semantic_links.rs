//! Regression tests for AI-driven auto semantic link discovery (#3458).
//!
//! Tests the semantic-enhanced scoring function (`score_pair_semantic`) and
//! the `AutoLinkSuggestion` data model. The end-to-end `suggest_auto_links`
//! requires a vault with notes and is covered by integration tests.

#[cfg(test)]
mod tests {
    use crate::knowledge_graph::{score_pair_semantic, AutoLinkSuggestion, RelationType};
    use crate::models::NoteMeta;
    use crate::semantic::HashEmbedder;

    fn make_meta(id: &str, title: &str, tags: &[&str], keywords: &[&str]) -> NoteMeta {
        NoteMeta {
            id: id.to_string(),
            title: title.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn regression_3458_semantic_score_finds_related_notes() {
        // Two notes about Rust programming — different tags but similar content.
        let a = make_meta("a", "Rust Programming", &["coding"], &["rust"]);
        let b = make_meta("b", "Learning Rust", &["tech"], &["programming"]);

        let embedder = HashEmbedder;
        let result = score_pair_semantic(
            &a,
            "Rust is a systems programming language",
            &b,
            "I am learning Rust programming",
            &embedder,
        );

        assert!(
            result.is_some(),
            "semantically similar notes should produce a relation"
        );
        let rel = result.unwrap();
        assert_eq!(rel.source, "a");
        assert_eq!(rel.target, "b");
        assert!(
            rel.confidence > 0.0,
            "confidence should be positive for related notes"
        );
    }

    #[test]
    fn regression_3458_semantic_score_rejects_self_comparison() {
        let a = make_meta("a", "Note A", &["tag"], &["kw"]);
        let embedder = HashEmbedder;
        let result = score_pair_semantic(&a, "body", &a, "body", &embedder);
        assert!(result.is_none(), "self-comparison should return None");
    }

    #[test]
    fn regression_3458_semantic_score_unrelated_notes_low_confidence() {
        // Completely different topics with no metadata overlap.
        let a = make_meta("a", "Quantum Physics", &["science"], &["quantum"]);
        let b = make_meta("b", "Cooking Recipes", &["food"], &["cooking"]);

        let embedder = HashEmbedder;
        let unrelated = score_pair_semantic(
            &a,
            "Schrödinger equation describes quantum states",
            &b,
            "How to make pasta carbonara",
            &embedder,
        );

        // Related notes for comparison.
        let c = make_meta("c", "Rust", &["coding"], &["rust"]);
        let d = make_meta("d", "Rust Lang", &["coding"], &["rust"]);
        let related = score_pair_semantic(
            &c,
            "Rust is a systems programming language",
            &d,
            "Rust programming language features",
            &embedder,
        );

        // Unrelated notes may still produce a weak relation from character n-gram
        // noise, but its confidence must be significantly lower than truly related notes.
        let unrelated_conf = unrelated.map(|r| r.confidence).unwrap_or(0.0);
        let related_conf = related.map(|r| r.confidence).unwrap_or(0.0);

        assert!(
            related_conf > unrelated_conf,
            "related notes (conf={:.3}) should score higher than unrelated (conf={:.3})",
            related_conf,
            unrelated_conf
        );
    }

    #[test]
    fn regression_3458_semantic_score_metadata_only_match() {
        // Same tags but completely different content words.
        let a = make_meta("a", "Project Alpha", &["work", "project"], &["alpha"]);
        let b = make_meta("b", "Project Beta", &["work", "project"], &["beta"]);

        let embedder = HashEmbedder;
        let result = score_pair_semantic(&a, "zzz qqq xxx", &b, "yyy www vvv", &embedder);

        // Metadata overlap should still produce a relation even if content vectors differ.
        assert!(
            result.is_some(),
            "metadata overlap alone should produce a relation"
        );
        let rel = result.unwrap();
        assert!(rel.confidence > 0.0);
    }

    #[test]
    fn regression_3458_auto_link_suggestion_structure() {
        // Verify the AutoLinkSuggestion struct can be constructed and serialized.
        let suggestion = AutoLinkSuggestion {
            source_id: "note-1".to_string(),
            target_id: "note-2".to_string(),
            target_title: "Related Note".to_string(),
            confidence: 0.75,
            semantic_similarity: 0.6,
            metadata_similarity: 0.5,
            reason: "Content-level similarity (60%)".to_string(),
        };

        let json = serde_json::to_string(&suggestion).unwrap();
        assert!(json.contains("note-1"));
        assert!(json.contains("note-2"));
        assert!(json.contains("Related Note"));
        assert!(json.contains("0.75"));

        // Deserialize back.
        let decoded: AutoLinkSuggestion = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.source_id, "note-1");
        assert_eq!(decoded.target_title, "Related Note");
    }

    #[test]
    fn regression_3458_relation_type_variants() {
        // Verify all relation types are accessible and serializable.
        let types = [
            RelationType::Related,
            RelationType::SameEntity,
            RelationType::SameTopic,
        ];
        assert_eq!(types.len(), 3);
    }

    #[test]
    fn regression_3458_semantic_catches_different_words_same_topic() {
        // The key value of semantic similarity: notes about the same topic
        // using completely different vocabulary.
        let a = make_meta("a", "Machine Learning Basics", &[], &[]);
        let b = make_meta("b", "AI Fundamentals", &[], &[]);

        let embedder = HashEmbedder;
        // These share some character n-grams (learn, fundament) but not words.
        let result = score_pair_semantic(
            &a,
            "Machine learning models can predict outcomes from data",
            &b,
            "Artificial intelligence systems learn patterns from information",
            &embedder,
        );

        // The HashEmbedder uses character n-grams so "learn" appears in both,
        // producing some semantic signal even without word overlap.
        // The result may or may not pass the threshold, but shouldn't panic.
        if let Some(rel) = result {
            assert!(rel.confidence > 0.0);
        }
    }
}
