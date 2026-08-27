//! FSRS (Free Spaced Repetition Scheduler) — spaced repetition engine (#1912).
//!
//! This module implements a simplified FSRS-4.5 inspired scheduler that
//! computes the next review interval, ease factor, and due date based on
//! the learner's rating (Again / Hard / Good / Easy).
//!
//! The algorithm uses the four-card-state memory model:
//!   - Stability (S): how long the memory is retained
//!   - Difficulty (D): intrinsic difficulty of the card (1.0–10.0)
//!   - Repetition count
//!   - State: New / Learning / Review / Relearning
//!
//! See: <https://github.com/open-spaced-repetition/fsrs4anki> for the original
//! FSRS-4.5 parameter definitions and formulas.

#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────────────────
// Types
// ────────────────────────────────────────────────────────

/// Learner's self-assessment of recall quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    /// Complete blackout — forgot the answer entirely.
    #[default]
    Again,
    /// Recalled, but with significant difficulty / hesitation.
    Hard,
    /// Recalled with some effort — the "normal" path.
    Good,
    /// Instant, effortless recall.
    Easy,
}

impl Rating {
    /// Convert to the numeric grade expected by the scheduler (0–3).
    pub fn grade(self) -> u8 {
        match self {
            Rating::Again => 0,
            Rating::Hard => 1,
            Rating::Good => 2,
            Rating::Easy => 3,
        }
    }

    /// Parse from a 1-4 integer or string keyword.
    pub fn from_input(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "1" | "again" | "a" => Some(Rating::Again),
            "2" | "hard" | "h" => Some(Rating::Hard),
            "3" | "good" | "g" | "" => Some(Rating::Good),
            "4" | "easy" | "e" => Some(Rating::Easy),
            _ => None,
        }
    }

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Rating::Again => "Again",
            Rating::Hard => "Hard",
            Rating::Good => "Good",
            Rating::Easy => "Easy",
        }
    }
}

/// Lifecycle state of a flashcard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CardState {
    /// Never reviewed.
    #[default]
    New,
    /// In the initial learning steps (short intervals).
    Learning,
    /// Graduated to longer intervals.
    Review,
    /// Lapsed — was in Review but the learner pressed "Again".
    Relearning,
}

impl CardState {
    pub fn as_str(self) -> &'static str {
        match self {
            CardState::New => "new",
            CardState::Learning => "learning",
            CardState::Review => "review",
            CardState::Relearning => "relearning",
        }
    }
}

/// Persistent scheduling parameters stored per-card.
///
/// All durations are expressed in **days** (float for sub-day precision in
/// learning steps).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SchedulingState {
    /// Current card state.
    #[serde(default)]
    pub state: CardState,
    /// Memory stability in days (how long until ~90% retention).
    #[serde(default)]
    pub stability: f64,
    /// Difficulty, 1.0 (easiest) to 10.0 (hardest).
    #[serde(default = "default_difficulty")]
    pub difficulty: f64,
    /// Number of successful reviews (Good or Easy).
    #[serde(default)]
    pub reps: u32,
    /// Number of times the learner pressed "Again".
    #[serde(default)]
    pub lapses: u32,
    /// Scheduled interval in days for the *current* cycle.
    #[serde(default)]
    pub scheduled_days: f64,
    /// ISO-8601 datetime when the card is next due.
    #[serde(default)]
    pub due: String,
    /// Last review datetime (ISO-8601), empty if never.
    #[serde(default)]
    pub last_review: String,
}

fn default_difficulty() -> f64 {
    INITIAL_DIFFICULTY
}

/// Scheduling outcome returned after processing a rating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOutcome {
    /// Updated state to persist.
    pub new_state: SchedulingState,
    /// Interval in days for this cycle (for display).
    pub interval_days: f64,
    /// Whether the card was marked as "known" (graduated/lapsed to review).
    pub known: bool,
}

impl ReviewOutcome {
    /// Human-friendly interval string e.g. "10m", "4h", "3d", "2w", "1mo".
    pub fn interval_human(&self) -> String {
        humanize_days(self.interval_days)
    }
}

// ────────────────────────────────────────────────────────
// FSRS-4.5 Constants (w = 19 default parameters)
// ────────────────────────────────────────────────────────

/// Target retention level (0.9 = 90%). FSRS optimises for this.
pub const TARGET_RETENTION: f64 = 0.9;

/// Minimum interval clamp (prevents 0-day scheduling).
pub const MIN_INTERVAL: f64 = 0.1; // ~2.4 hours

/// Maximum interval in days (100 years — effectively unbounded).
pub const MAX_INTERVAL: f64 = 36500.0;

/// Maximum fuzz applied to intervals (±5%).
const FUZZ_MAX: f64 = 0.05;

/// Initial stability values for first rating (Again/Hard/Good/Easy).
const INITIAL_STABILITY: [f64; 4] = [0.4, 0.6, 2.4, 5.8];

/// Initial difficulty for a new card.
const INITIAL_DIFFICULTY: f64 = 5.5;

// FSRS-4.5 stability update coefficients (simplified from the 19-parameter model).
// These are the publicly-recommended defaults from the FSRS project.
const W_STABILITY_GOOD_EASY: f64 = 1.39;
const W_STABILITY_HARD: f64 = 1.2;
const W_DIFFICULTY_DECAY: f64 = 0.57;
const W_DIFFICULTY_BASE: f64 = 0.1;

/// Learning step intervals in days (10 min, 1 day).
const LEARNING_STEPS: [f64; 2] = [10.0 / 1440.0, 1.0];

/// Relearning step (10 min).
const RELEARNING_STEP: f64 = 10.0 / 1440.0;

// ────────────────────────────────────────────────────────
// Scheduler
// ────────────────────────────────────────────────────────

/// Compute the retention probability given stability (days) and elapsed time.
///
/// Based on the exponential decay model: R(t) = (1 + t / (9 * S))^(-1)
pub fn retention(stability: f64, elapsed_days: f64) -> f64 {
    if stability <= 0.0 {
        return 0.0;
    }
    (1.0 + elapsed_days / (9.0 * stability)).powf(-1.0)
}

/// Core FSRS scheduling function: given the current state and a rating,
/// compute the next scheduling state.
///
/// `now` is passed in to make the function pure and testable.
pub fn schedule(prev: &SchedulingState, rating: Rating, now: DateTime<Utc>) -> ReviewOutcome {
    let grade = rating.grade();

    // Difficulty update: moves toward easier (lower D) on Good/Easy,
    // toward harder (higher D) on Again/Hard.
    let new_difficulty = update_difficulty(prev.difficulty, grade);

    match prev.state {
        CardState::New => {
            // First review: initial stability from the rating.
            let stability = INITIAL_STABILITY[grade as usize];
            let interval = if grade == 0 {
                // Again → stays in learning step 1 (10 min)
                LEARNING_STEPS[0]
            } else if grade <= 1 {
                // Hard → learning step 2 (1 day)
                LEARNING_STEPS[1]
            } else {
                // Good/Easy → graduate directly
                clamp_interval(stability)
            };
            let state = if grade >= 2 {
                CardState::Review
            } else {
                CardState::Learning
            };
            let reps = if grade >= 2 { 1 } else { 0 };
            let due = now + Duration::seconds((interval * 86400.0) as i64);

            let new_state = SchedulingState {
                state,
                stability,
                difficulty: new_difficulty,
                reps,
                lapses: 0,
                scheduled_days: interval,
                due: due.to_rfc3339(),
                last_review: now.to_rfc3339(),
            };

            ReviewOutcome {
                new_state,
                interval_days: interval,
                known: grade >= 2,
            }
        }
        CardState::Learning => {
            if grade == 0 {
                // Again → restart learning step 1
                let interval = LEARNING_STEPS[0];
                let due = now + Duration::seconds((interval * 86400.0) as i64);
                let new_state = SchedulingState {
                    state: CardState::Learning,
                    stability: prev.stability,
                    difficulty: new_difficulty,
                    reps: prev.reps,
                    lapses: prev.lapses,
                    scheduled_days: interval,
                    due: due.to_rfc3339(),
                    last_review: now.to_rfc3339(),
                };
                ReviewOutcome {
                    new_state,
                    interval_days: interval,
                    known: false,
                }
            } else if grade == 1 {
                // Hard → stay in learning step 2 (or graduate if already past step 1)
                let interval = LEARNING_STEPS[1];
                let due = now + Duration::seconds((interval * 86400.0) as i64);
                let new_state = SchedulingState {
                    state: CardState::Learning,
                    stability: prev.stability.max(interval),
                    difficulty: new_difficulty,
                    reps: prev.reps,
                    lapses: prev.lapses,
                    scheduled_days: interval,
                    due: due.to_rfc3339(),
                    last_review: now.to_rfc3339(),
                };
                ReviewOutcome {
                    new_state,
                    interval_days: interval,
                    known: false,
                }
            } else {
                // Good/Easy → graduate to Review
                let stability = if grade == 2 {
                    INITIAL_STABILITY[2].max(prev.stability * 1.5)
                } else {
                    INITIAL_STABILITY[3].max(prev.stability * 2.0)
                };
                let interval = clamp_interval(stability);
                let due = now + Duration::seconds((interval * 86400.0) as i64);
                let new_state = SchedulingState {
                    state: CardState::Review,
                    stability,
                    difficulty: new_difficulty,
                    reps: prev.reps + 1,
                    lapses: prev.lapses,
                    scheduled_days: interval,
                    due: due.to_rfc3339(),
                    last_review: now.to_rfc3339(),
                };
                ReviewOutcome {
                    new_state,
                    interval_days: interval,
                    known: true,
                }
            }
        }
        CardState::Review => {
            if grade == 0 {
                // Lapse → Relearning
                let new_stability = update_stability_lapse(prev.stability, prev.difficulty);
                let interval = RELEARNING_STEP;
                let due = now + Duration::seconds((interval * 86400.0) as i64);
                let new_state = SchedulingState {
                    state: CardState::Relearning,
                    stability: new_stability,
                    difficulty: new_difficulty,
                    reps: prev.reps,
                    lapses: prev.lapses + 1,
                    scheduled_days: interval,
                    due: due.to_rfc3339(),
                    last_review: now.to_rfc3339(),
                };
                ReviewOutcome {
                    new_state,
                    interval_days: interval,
                    known: false,
                }
            } else {
                // Successful review → increase interval based on rating
                let factor = match rating {
                    Rating::Hard => 1.2,
                    Rating::Good => W_STABILITY_GOOD_EASY,
                    Rating::Easy => 2.5,
                    Rating::Again => unreachable!(),
                };
                let new_stability =
                    update_stability_success(prev.stability, prev.difficulty, factor);
                let interval = clamp_interval(new_stability);
                let due = now + Duration::seconds((interval * 86400.0) as i64);
                let new_state = SchedulingState {
                    state: CardState::Review,
                    stability: new_stability,
                    difficulty: new_difficulty,
                    reps: prev.reps + 1,
                    lapses: prev.lapses,
                    scheduled_days: interval,
                    due: due.to_rfc3339(),
                    last_review: now.to_rfc3339(),
                };
                ReviewOutcome {
                    new_state,
                    interval_days: interval,
                    known: true,
                }
            }
        }
        CardState::Relearning => {
            if grade == 0 {
                // Again → stay in relearning
                let interval = RELEARNING_STEP;
                let due = now + Duration::seconds((interval * 86400.0) as i64);
                let new_state = SchedulingState {
                    state: CardState::Relearning,
                    stability: prev.stability,
                    difficulty: new_difficulty,
                    reps: prev.reps,
                    lapses: prev.lapses,
                    scheduled_days: interval,
                    due: due.to_rfc3339(),
                    last_review: now.to_rfc3339(),
                };
                ReviewOutcome {
                    new_state,
                    interval_days: interval,
                    known: false,
                }
            } else {
                // Good/Easy → back to Review with updated interval
                let new_stability = prev.stability * 0.8; // post-lapse reduced stability
                let interval = clamp_interval(new_stability);
                let due = now + Duration::seconds((interval * 86400.0) as i64);
                let new_state = SchedulingState {
                    state: CardState::Review,
                    stability: new_stability,
                    difficulty: new_difficulty,
                    reps: prev.reps + 1,
                    lapses: prev.lapses,
                    scheduled_days: interval,
                    due: due.to_rfc3339(),
                    last_review: now.to_rfc3339(),
                };
                ReviewOutcome {
                    new_state,
                    interval_days: interval,
                    known: true,
                }
            }
        }
    }
}

/// Compute the ideal interval for a given stability to hit target retention.
pub fn ideal_interval(stability: f64) -> f64 {
    // Solve R(t) = TARGET_RETENTION for t:
    // (1 + t/(9*S))^(-1) = R  →  t = 9*S*(R^(-1) - 1)
    let r = TARGET_RETENTION;
    let t = 9.0 * stability * (r.powf(-1.0) - 1.0);
    clamp_interval(t)
}

// ────────────────────────────────────────────────────────
// Internal helpers
// ────────────────────────────────────────────────────────

fn update_difficulty(prev_d: f64, grade: u8) -> f64 {
    // grade 0=Again → +1, 1=Hard → +0.2, 2=Good → -0.4, 3=Easy → -0.8
    let delta = match grade {
        0 => 1.0,
        1 => 0.4,
        2 => -0.4,
        _ => -0.8,
    };
    let mut d = prev_d + delta * W_DIFFICULTY_BASE;
    // Mean reversion toward INITIAL_DIFFICULTY
    d = d + (INITIAL_DIFFICULTY - d) * W_DIFFICULTY_DECAY;
    d.clamp(1.0, 10.0)
}

fn update_stability_success(prev_s: f64, difficulty: f64, factor: f64) -> f64 {
    // FSRS-4.5 simplified: S' = S * (1 + factor * ease_multiplier)
    // ease_multiplier decreases with difficulty
    let ease_mult = (11.0 - difficulty) / 9.0; // ~0.1 (D=10) to ~1.1 (D=1)
    let new_s = prev_s * (1.0 + factor * ease_mult);
    // Apply small fuzz for natural variation
    let fuzz = 1.0 + (pseudo_random(prev_s) - 0.5) * FUZZ_MAX * 2.0;
    (new_s * fuzz).max(prev_s * 1.01) // always at least slightly increase on success
}

fn update_stability_lapse(prev_s: f64, _difficulty: f64) -> f64 {
    // On lapse, stability drops significantly
    (prev_s * 0.2).max(0.2)
}

fn clamp_interval(days: f64) -> f64 {
    days.clamp(MIN_INTERVAL, MAX_INTERVAL)
}

/// Deterministic pseudo-random from a seed (no global RNG needed).
fn pseudo_random(seed: f64) -> f64 {
    let bits = seed.to_bits();
    let x = bits.wrapping_mul(2654435761);
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Convert a day-based interval to a human-readable string.
pub fn humanize_days(days: f64) -> String {
    let minutes = days * 1440.0;
    if minutes < 1.0 {
        return "<1m".to_string();
    }
    if minutes < 60.0 {
        return format!("{:.0}m", minutes.round());
    }
    let hours = minutes / 60.0;
    if hours < 24.0 {
        return format!("{:.0}h", hours.round());
    }
    let d = days;
    if d < 10.0 {
        return format!("{:.0}d", d.round());
    }
    if d < 70.0 {
        return format!("{:.0}w", (d / 7.0).round());
    }
    format!("{:.1}y", d / 365.0)
}

/// Create a fresh scheduling state for a new card.
pub fn new_card_state(now: DateTime<Utc>) -> SchedulingState {
    SchedulingState {
        state: CardState::New,
        stability: 0.0,
        difficulty: INITIAL_DIFFICULTY,
        reps: 0,
        lapses: 0,
        scheduled_days: 0.0,
        due: now.to_rfc3339(),
        last_review: String::new(),
    }
}

/// Check if a card is due for review at the given time.
pub fn is_due(state: &SchedulingState, now: DateTime<Utc>) -> bool {
    if state.state == CardState::New {
        return true;
    }
    if state.due.is_empty() {
        return true;
    }
    match DateTime::parse_from_rfc3339(&state.due) {
        Ok(due) => now >= due.with_timezone(&Utc),
        Err(_) => true, // unparseable due → treat as due
    }
}

/// Parse a JSON scheduling state string. Returns None on parse failure.
pub fn parse_scheduling(json: &str) -> Option<SchedulingState> {
    if json.is_empty() {
        return None;
    }
    serde_json::from_str(json).ok()
}

/// Parse a JSON scheduling state string, falling back to a fresh New card state.
pub fn parse_scheduling_or_default(json: &str) -> SchedulingState {
    parse_scheduling(json).unwrap_or_else(|| new_card_state(Utc::now()))
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn test_rating_from_input() {
        assert_eq!(Rating::from_input("1"), Some(Rating::Again));
        assert_eq!(Rating::from_input("again"), Some(Rating::Again));
        assert_eq!(Rating::from_input("2"), Some(Rating::Hard));
        assert_eq!(Rating::from_input("hard"), Some(Rating::Hard));
        assert_eq!(Rating::from_input("3"), Some(Rating::Good));
        assert_eq!(Rating::from_input(""), Some(Rating::Good)); // default
        assert_eq!(Rating::from_input("4"), Some(Rating::Easy));
        assert_eq!(Rating::from_input("xyz"), None);
    }

    #[test]
    fn test_new_card_is_due() {
        let state = new_card_state(now());
        assert!(is_due(&state, now()));
    }

    #[test]
    fn test_first_review_again_stays_learning() {
        let state = new_card_state(now());
        let outcome = schedule(&state, Rating::Again, now());
        assert_eq!(outcome.new_state.state, CardState::Learning);
        assert!(!outcome.known);
        assert_eq!(outcome.new_state.reps, 0);
    }

    #[test]
    fn test_first_review_good_graduates() {
        let state = new_card_state(now());
        let outcome = schedule(&state, Rating::Good, now());
        assert_eq!(outcome.new_state.state, CardState::Review);
        assert!(outcome.known);
        assert_eq!(outcome.new_state.reps, 1);
        assert!(outcome.new_state.stability > 0.0);
    }

    #[test]
    fn test_first_review_easy_higher_stability_than_good() {
        let state = new_card_state(now());
        let good = schedule(&state, Rating::Good, now());
        let easy = schedule(&state, Rating::Easy, now());
        assert!(easy.new_state.stability > good.new_state.stability);
    }

    #[test]
    fn test_review_good_increases_interval() {
        let state = new_card_state(now());
        // Graduate first
        let graduated = schedule(&state, Rating::Good, now()).new_state;
        // Then review again
        let next = schedule(&graduated, Rating::Good, now());
        assert!(
            next.interval_days > graduated.scheduled_days,
            "Interval should increase: {} > {}",
            next.interval_days,
            graduated.scheduled_days
        );
        assert_eq!(next.new_state.reps, 2);
    }

    #[test]
    fn test_lapse_goes_to_relearning() {
        let state = new_card_state(now());
        let graduated = schedule(&state, Rating::Good, now()).new_state;
        assert_eq!(graduated.state, CardState::Review);

        let lapsed = schedule(&graduated, Rating::Again, now());
        assert_eq!(lapsed.new_state.state, CardState::Relearning);
        assert!(!lapsed.known);
        assert_eq!(lapsed.new_state.lapses, 1);
    }

    #[test]
    fn test_relearning_good_returns_to_review() {
        let state = new_card_state(now());
        let graduated = schedule(&state, Rating::Good, now()).new_state;
        let lapsed = schedule(&graduated, Rating::Again, now()).new_state;
        assert_eq!(lapsed.state, CardState::Relearning);

        let recovered = schedule(&lapsed, Rating::Good, now());
        assert_eq!(recovered.new_state.state, CardState::Review);
        assert!(recovered.known);
    }

    #[test]
    fn test_difficulty_increases_on_again() {
        let state = new_card_state(now());
        assert_eq!(state.difficulty, INITIAL_DIFFICULTY);

        let after_again = schedule(&state, Rating::Again, now()).new_state;
        assert!(after_again.difficulty > state.difficulty);
    }

    #[test]
    fn test_difficulty_decreases_on_easy() {
        let state = new_card_state(now());
        let after_easy = schedule(&state, Rating::Easy, now()).new_state;
        assert!(
            after_easy.difficulty < state.difficulty,
            "Difficulty should decrease on Easy: {} < {}",
            after_easy.difficulty,
            state.difficulty
        );
    }

    #[test]
    fn test_retention_decreases_with_time() {
        let r0 = retention(1.0, 0.0);
        let r1 = retention(1.0, 1.0);
        let r30 = retention(1.0, 30.0);
        assert!(r0 > r1);
        assert!(r1 > r30);
        assert!(r0 <= 1.0);
    }

    #[test]
    fn test_ideal_interval_hits_target_retention() {
        let s = 5.0;
        let interval = ideal_interval(s);
        let r = retention(s, interval);
        assert!(
            (r - TARGET_RETENTION).abs() < 0.01,
            "Retention {r} should be ~{TARGET_RETENTION} at ideal interval {interval}"
        );
    }

    #[test]
    fn test_humanize_days() {
        assert_eq!(humanize_days(0.0001), "<1m");
        assert_eq!(humanize_days(5.0 / 1440.0), "5m");
        assert_eq!(humanize_days(1.0 / 24.0), "1h");
        assert_eq!(humanize_days(3.0), "3d");
        assert_eq!(humanize_days(14.0), "2w");
        assert_eq!(humanize_days(365.0), "1.0y");
    }

    #[test]
    fn test_is_due_with_future_date() {
        let state = SchedulingState {
            state: CardState::Review,
            stability: 5.0,
            difficulty: 5.0,
            reps: 3,
            lapses: 0,
            scheduled_days: 5.0,
            due: (now() + Duration::days(3)).to_rfc3339(),
            last_review: now().to_rfc3339(),
        };
        assert!(!is_due(&state, now()));
    }

    #[test]
    fn test_is_due_with_past_date() {
        let state = SchedulingState {
            state: CardState::Review,
            stability: 5.0,
            difficulty: 5.0,
            reps: 3,
            lapses: 0,
            scheduled_days: 5.0,
            due: (now() - Duration::days(3)).to_rfc3339(),
            last_review: (now() - Duration::days(8)).to_rfc3339(),
        };
        assert!(is_due(&state, now()));
    }

    #[test]
    fn test_clamp_interval_bounds() {
        assert!(clamp_interval(-1.0) >= MIN_INTERVAL);
        assert!(clamp_interval(f64::MAX) <= MAX_INTERVAL);
    }

    #[test]
    fn test_repeated_reviews_increase_stability() {
        // Simulate 10 consecutive "Good" reviews
        let mut state = new_card_state(now());
        let mut last_stability = state.stability;
        for _ in 0..10 {
            let outcome = schedule(&state, Rating::Good, now());
            state = outcome.new_state;
            if state.state == CardState::Review {
                assert!(
                    state.stability >= last_stability * 0.99,
                    "Stability should generally increase: {} >= {}",
                    state.stability,
                    last_stability
                );
            }
            last_stability = state.stability;
        }
        // First Good on New card → reps=1, then 9 more Good reviews → reps=10
        assert_eq!(state.reps, 10);
        assert!(
            state.stability > 1.0,
            "After 10 reviews stability should be substantial"
        );
    }
}
