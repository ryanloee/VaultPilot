//! Regression tests for FSRS spaced repetition feature (#1912).
//!
//! Verifies the full lifecycle: create → review → lapse → recover → stats.

#[cfg(test)]
mod tests {
    use crate::fsrs::{self, CardState, Rating};
    use crate::storage::flashcards::{
        self, create_flashcard_with_context, get_due_flashcards_with_context,
        get_flashcard_stats_with_context, review_flashcard_with_context,
    };
    use crate::storage::StorageContext;
    use chrono::Utc;

    fn test_context() -> StorageContext {
        let dir = std::env::temp_dir().join(format!("vp-fsrs-regression-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let ctx = StorageContext::for_test(&dir);
        crate::storage::initialize_storage_with_context(&ctx).unwrap();
        ctx
    }

    /// Full lifecycle: new → learn → review → lapse → recover → stats.
    #[test]
    fn test_issue_1912_full_flashcard_lifecycle() {
        let ctx = test_context();

        // 1. Create a new flashcard
        let card = create_flashcard_with_context(
            &ctx,
            "What is FSRS?",
            "Free Spaced Repetition Scheduler",
            "",
            "learning",
        )
        .unwrap();

        // 2. Verify it's due immediately (new card)
        let due = get_due_flashcards_with_context(&ctx).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].card.id, card.id);
        assert_eq!(due[0].card_state(), CardState::New);

        // 3. Review with "Good" → should graduate to Review
        let updated = review_flashcard_with_context(&ctx, &card.id, Rating::Good).unwrap();
        let state = flashcards::parse_scheduling(&updated.scheduling).unwrap();
        assert_eq!(state.state, CardState::Review);
        assert_eq!(state.reps, 1);
        assert_eq!(state.lapses, 0);

        // 4. Review again with "Good" → interval should increase
        let updated2 = review_flashcard_with_context(&ctx, &card.id, Rating::Good).unwrap();
        let state2 = flashcards::parse_scheduling(&updated2.scheduling).unwrap();
        assert_eq!(state2.reps, 2);
        assert!(
            state2.stability > state.stability,
            "Stability should increase on Good review"
        );

        // 5. Review with "Again" → should lapse to Relearning
        let updated3 = review_flashcard_with_context(&ctx, &card.id, Rating::Again).unwrap();
        let state3 = flashcards::parse_scheduling(&updated3.scheduling).unwrap();
        assert_eq!(state3.state, CardState::Relearning);
        assert_eq!(state3.lapses, 1);

        // 6. Review with "Good" → should recover back to Review
        let updated4 = review_flashcard_with_context(&ctx, &card.id, Rating::Good).unwrap();
        let state4 = flashcards::parse_scheduling(&updated4.scheduling).unwrap();
        assert_eq!(state4.state, CardState::Review);
        assert_eq!(state4.reps, 3);
        assert_eq!(state4.lapses, 1); // lapses don't decrease

        // 7. Verify stats
        let stats = get_flashcard_stats_with_context(&ctx).unwrap();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.review, 1);
        assert_eq!(stats.total_reps, 3);
        assert_eq!(stats.total_lapses, 1);
    }

    /// Test that FSRS scheduler produces monotonically increasing intervals
    /// on repeated "Good" reviews (core FSRS guarantee).
    #[test]
    fn test_issue_1912_monotonic_interval_growth() {
        let now = Utc::now();
        let mut state = fsrs::new_card_state(now);
        let mut intervals: Vec<f64> = Vec::new();

        for _ in 0..15 {
            let outcome = fsrs::schedule(&state, Rating::Good, now);
            intervals.push(outcome.interval_days);
            state = outcome.new_state;
        }

        // After graduation (first Good), intervals should generally increase
        let review_intervals = &intervals[1..];
        for i in 1..review_intervals.len() {
            assert!(
                review_intervals[i] >= review_intervals[i - 1] * 0.95,
                "Interval should not decrease significantly: [{:?}]",
                review_intervals
            );
        }
    }

    /// Test that "Easy" produces longer intervals than "Good" on the same card.
    #[test]
    fn test_issue_1912_easy_longer_than_good() {
        let now = Utc::now();

        // Graduate first
        let state = fsrs::new_card_state(now);
        let graduated = fsrs::schedule(&state, Rating::Good, now).new_state;

        let good_outcome = fsrs::schedule(&graduated, Rating::Good, now);
        let easy_outcome = fsrs::schedule(&graduated, Rating::Easy, now);

        assert!(
            easy_outcome.interval_days > good_outcome.interval_days,
            "Easy ({}) should give longer interval than Good ({})",
            easy_outcome.interval_days,
            good_outcome.interval_days
        );
    }
}
