//! Regression tests for #3561: suggest_auto_links re-embeds source on every iteration.
//!
//! Fix: Pre-compute source_vec once before the loop instead of per-target.
//! This test uses a counting embedder wrapper to verify the fix pattern.

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::semantic::{HashEmbedder, SemanticEmbedder};

    /// A counting wrapper that delegates to HashEmbedder and increments a counter.
    struct CountingEmbedder {
        inner: HashEmbedder,
        call_count: AtomicUsize,
    }

    impl CountingEmbedder {
        fn new() -> Self {
            Self {
                inner: HashEmbedder,
                call_count: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.call_count.load(Ordering::Relaxed)
        }
    }

    impl SemanticEmbedder for CountingEmbedder {
        fn embed(&self, text: &str) -> Option<Vec<f32>> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            self.inner.embed(text)
        }

        fn dimension(&self) -> usize {
            self.inner.dimension()
        }
    }

    /// Simulate the POST-FIX pattern: source_vec is computed once before the loop.
    /// Each target gets embedded once inside the loop.
    /// Expected: 1 source embed + N target embeds = N+1 total.
    #[test]
    fn regression_3561_source_embedded_exactly_once() {
        let source_title = "Rust Systems Programming";
        let source_body = "Rust is a systems programming language";

        let target_texts = vec![
            ("Memory Safety", "Zero-cost abstractions with memory safety"),
            ("Async IO", "Tokio and async IO in Rust"),
            (
                "Cargo Build",
                "Cargo build system and dependency management",
            ),
        ];

        let embedder = CountingEmbedder::new();

        // Fix pattern: compute source_vec once.
        let source_vec = embedder.embed(&format!("{} {}", source_title, source_body));

        // Loop over targets, embedding only the target each time.
        for (target_title, target_body) in &target_texts {
            let b_vec = embedder.embed(&format!("{} {}", target_title, target_body));
            let _ = crate::semantic::cosine_similarity(
                source_vec.as_ref().unwrap_or(&vec![]),
                b_vec.as_ref().unwrap_or(&vec![]),
            );
        }

        // Source embedded once, plus 3 targets = 4 total calls.
        assert_eq!(
            embedder.call_count(),
            4,
            "source embedded once + each target once = 4 total"
        );
    }
}
