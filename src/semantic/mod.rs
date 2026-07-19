//! Semantic embedding infrastructure for VaultPilot.
//!
//! This module defines the [`SemanticEmbedder`] trait — a pluggable interface
//! for generating semantic vector embeddings from text.  The trait is used by
//! the search pipeline to power "search by meaning, not just by keyword".
//!
//! ## Architecture
//!
//! The module was extracted from the existing keyword‑n‑gram based vectorizer
//! that lived in `src/storage/search.rs`.  That implementation is now
//! [`HashEmbedder`] (the default).  Future ML‑based embedders (ONNX / llama.cpp
//! / local HTTP endpoints) can be added by implementing [`SemanticEmbedder`]
//! and wiring the chosen provider through [`AppSettings`].
//!
//! ```text
//! SemanticEmbedder  (trait, this module)
//!  ├── HashEmbedder (default, keyword‑n‑gram)
//!  └── … future ML embedders …
//! ```

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A pluggable embedder that converts text into a fixed‑dimension float vector.
///
/// The resulting vector should be normalised (unit length) so that cosine
/// similarity is a meaningful distance metric.
pub trait SemanticEmbedder: Send + Sync {
    /// Embed `text` into a float vector.
    ///
    /// Returns `None` when the input is empty or contains no embeddable
    /// content (the caller should skip scoring for that document).
    fn embed(&self, text: &str) -> Option<Vec<f32>>;

    /// Dimensionality of the vectors returned by [`Self::embed`].
    fn dimension(&self) -> usize;
}

// ---------------------------------------------------------------------------
// Embedder selection — stored in AppSettings
// ---------------------------------------------------------------------------

/// Which embedding provider to use for semantic search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EmbeddingProvider {
    /// Keyword n‑gram hash‑based embedder (built‑in, no external deps).
    ///
    /// This is the historic behaviour: a 192‑dim vector built from character
    /// n‑gram hashes.  Fast, deterministic, zero‑dependency.
    #[default]
    HashNGram,
}

// ---------------------------------------------------------------------------
// Hash‑based embedder (the classic implementation)
// ---------------------------------------------------------------------------

const HASH_EMBEDDER_DIM: usize = 192;

/// Keyword n‑gram hash‑based [`SemanticEmbedder`].
///
/// Builds a 192‑dimensional vector by:
/// 1. Extracting search terms (lowercased alphabetic tokens).
/// 2. Hashing each term and its character 3‑grams into bucket indices.
/// 3. Normalising the resulting vector to unit length.
///
/// This is the **default** embedder and matches the behaviour previously
/// hard‑coded inside `src/storage/search.rs`.
pub struct HashEmbedder;

impl HashEmbedder {
    pub const DIM: usize = HASH_EMBEDDER_DIM;
}

impl SemanticEmbedder for HashEmbedder {
    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let terms = extract_embedding_terms(text);
        if terms.is_empty() {
            return None;
        }

        let mut vector = vec![0.0_f32; Self::DIM];

        for term in terms {
            let hash = stable_hash(&term);
            let index = (hash as usize) % Self::DIM;
            let sign = if (hash >> 63) == 0 { 1.0 } else { -1.0 };
            vector[index] += sign;

            // Character 3‑grams for sub‑word signal
            if term.chars().count() > 3 {
                let grams: Vec<String> = sliding_char_grams(&term, 3);
                for gram in grams {
                    let gh = stable_hash(&gram);
                    let gi = (gh as usize) % Self::DIM;
                    let gs = if (gh >> 63) == 0 { 0.5 } else { -0.5 };
                    vector[gi] += gs;
                }
            }
        }

        normalize_vector(&mut vector);
        Some(vector)
    }

    fn dimension(&self) -> usize {
        Self::DIM
    }
}

// ---------------------------------------------------------------------------
// Helpers (ported from src/storage/search.rs)
// ---------------------------------------------------------------------------

/// Extract lowercased alphabetic tokens from `text`.
fn extract_embedding_terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphabetic())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

/// Stable hash for a term (deterministic across runs).
fn stable_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Build character n‑grams of width `n` from `s`.
fn sliding_char_grams(s: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < n {
        return vec![s.to_string()];
    }
    chars
        .windows(n)
        .map(|w| w.iter().collect::<String>())
        .collect()
}

/// Normalise `vector` to unit length (L2 norm).
fn normalize_vector(vector: &mut [f32]) {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for v in vector.iter_mut() {
            *v /= norm;
        }
    }
}

/// Cosine similarity between two normalised vectors.
///
/// Both slices must have the same length.  The caller should ensure they have
/// been normalised to unit length so that the dot product equals cosine.
pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| a * b)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

/// Convenience: create the default embedder.
pub fn default_embedder() -> Box<dyn SemanticEmbedder> {
    Box::new(HashEmbedder)
}

/// Create an embedder from a provider variant.
pub fn embedder_from_provider(provider: EmbeddingProvider) -> Box<dyn SemanticEmbedder> {
    match provider {
        EmbeddingProvider::HashNGram => Box::new(HashEmbedder),
    }
}

// ---------------------------------------------------------------------------
// Serialisation helpers (for storing vectors in the DB)
// ---------------------------------------------------------------------------

/// Serialise a float vector to a JSON string for database storage.
pub fn serialize_vector(vector: &[f32]) -> String {
    serde_json::to_string(vector).unwrap_or_default()
}

/// Deserialise a float vector from a JSON string stored in the database.
pub fn deserialize_vector(raw: &str) -> Option<Vec<f32>> {
    let vector = serde_json::from_str::<Vec<f32>>(raw).ok()?;
    if vector.len() == HASH_EMBEDDER_DIM {
        Some(vector)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_embedder_returns_some_for_nonempty_text() {
        let embedder = HashEmbedder;
        let vec = embedder.embed("hello world");
        assert!(vec.is_some());
        let v = vec.unwrap();
        assert_eq!(v.len(), HashEmbedder::DIM);
    }

    #[test]
    fn hash_embedder_returns_none_for_empty_text() {
        let embedder = HashEmbedder;
        assert!(embedder.embed("").is_none());
    }

    #[test]
    fn hash_embedder_similar_texts_have_positive_similarity() {
        let embedder = HashEmbedder;
        let a = embedder.embed("apple banana cherry").unwrap();
        let b = embedder.embed("apple banana date").unwrap();
        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.0, "similarity should be positive, got {sim}");
    }

    #[test]
    fn hash_embedder_different_texts_have_lower_similarity() {
        let embedder = HashEmbedder;
        let a = embedder.embed("apple banana cherry").unwrap();
        let b = embedder.embed("xylophone zebra quantum").unwrap();
        let sim_ab = cosine_similarity(&a, &b);

        // Self-similarity should be higher
        let sim_aa = cosine_similarity(&a, &a);
        assert!(
            sim_aa > sim_ab,
            "self-similarity ({sim_aa:.4}) should be > cross-similarity ({sim_ab:.4})"
        );
    }

    #[test]
    fn vector_normalisation_unit_length() {
        let embedder = HashEmbedder;
        let v = embedder.embed("test vector normalisation").unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm should be ~1.0, got {norm}");
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![0.6, 0.8];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6, "expected 1.0, got {sim}");
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6, "expected 0.0, got {sim}");
    }

    #[test]
    fn serde_round_trip() {
        let embedder = HashEmbedder;
        let v = embedder.embed("round trip test").unwrap();
        let json = serialize_vector(&v);
        let back = deserialize_vector(&json).expect("deserialise failed");
        assert_eq!(v.len(), back.len());
        for (a, b) in v.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn default_embedder_is_hash() {
        let e = default_embedder();
        assert_eq!(e.dimension(), HashEmbedder::DIM);
        assert!(e.embed("test").is_some());
    }

    #[test]
    fn embedder_from_provider_hash() {
        let e = embedder_from_provider(EmbeddingProvider::HashNGram);
        assert_eq!(e.dimension(), HashEmbedder::DIM);
        assert!(e.embed("test").is_some());
    }

    #[test]
    fn sliding_char_grams_basic() {
        let grams = sliding_char_grams("hello", 3);
        assert_eq!(grams, vec!["hel", "ell", "llo"]);
    }

    #[test]
    fn sliding_char_grams_shorter_than_n() {
        let grams = sliding_char_grams("ab", 3);
        assert_eq!(grams, vec!["ab"]);
    }

    #[test]
    fn extract_embedding_terms_basic() {
        let terms = extract_embedding_terms("Hello World! Test123");
        assert_eq!(terms, vec!["hello", "world", "test"]);
    }

    #[test]
    fn extract_embedding_terms_empty() {
        let terms: Vec<String> = extract_embedding_terms("  123!@#  ");
        assert!(terms.is_empty());
    }

    #[test]
    fn serialize_vector_empty_returns_array() {
        let v: Vec<f32> = vec![];
        let json = serialize_vector(&v);
        assert_eq!(json, "[]");
    }
}
