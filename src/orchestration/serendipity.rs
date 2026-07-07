//! # Serendipity — Random Knowledge Discovery (#1943)
//!
//! Inspired by Mem.ai's "Serendipity" feature: when the user opens the app,
//! surface 1–3 notes they wrote a long time ago but may have forgotten.
//!
//! ## Algorithm
//!
//! 1. **Candidates**: notes whose `updated_at` is older than 30 days ("stale
//!    notes").
//! 2. **Signal**: notes updated within the last 7 days ("recent notes"). Their
//!    titles, tags, and keywords form the "current interest signal".
//! 3. **Score**: each stale note gets a relevance score based on keyword / tag /
//!    title-word overlap with the recent interest signal.
//! 4. **Random jitter**: a ±50% random multiplier is applied so the same vault
//!    produces different suggestions each time (the "serendipity" effect).
//! 5. **Result**: the top-N scored stale notes are returned with a human-readable
//!    "why this was selected" reason.
//!
//! ## Caching
//!
//! The candidate pool is regenerated on every call because vault sizes under
//! ~10 000 notes make the computation negligible (a few SQL queries and
//! in-memory passes).  If vaults grow significantly, a daily-cached candidate
//! pool can be added later.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use chrono::{Duration, Utc};
use serde::Serialize;
use tracing::instrument;

use crate::models::NoteMeta;
use crate::storage::StorageContext;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Notes not updated in this many days are considered "stale" (candidates for
/// serendipity).
const STALE_DAYS: i64 = 30;

/// Notes updated within this many days form the "recent interest signal".
const RECENT_DAYS: i64 = 7;

/// Maximum number of serendipity items to return.
const DEFAULT_COUNT: usize = 3;

/// Random jitter factor (±50 %): each raw score is multiplied by
/// `1.0 + rng.gen_range(-JITTER..JITTER)`.
const JITTER: f64 = 0.5;

/// Score bonus for each overlapping keyword.
const KEYWORD_BONUS: f64 = 40.0;

/// Score bonus for each overlapping tag.
const TAG_BONUS: f64 = 60.0;

/// Score bonus for each overlapping title word.
const TITLE_WORD_BONUS: f64 = 20.0;

/// Base participation score — every stale note gets this for showing up.
const BASE_PARTICIPATION: f64 = 10.0;

// ---------------------------------------------------------------------------
// Simple deterministic pseudo-random number generator (LCG)
// ---------------------------------------------------------------------------
// We cannot add the `rand` crate due to iron-law restrictions on Cargo.toml,
// so we use a minimal LCG seeded from the system clock.

struct SimpleRng(u64);

impl SimpleRng {
    fn from_clock() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        // Mix the high and low bits for a more uniform seed.
        let seed = (nanos as u64) ^ ((nanos >> 32) as u64);
        SimpleRng(seed)
    }

    /// Generate a uniform f64 in [0.0, 1.0).
    fn gen_f64(&mut self) -> f64 {
        // musl LCG parameters
        const MULT: u64 = 6364136223846793005;
        const INC: u64 = 1442695040888963407;
        self.0 = self.0.wrapping_mul(MULT).wrapping_add(INC);
        // Use top 53 bits for f64 mantissa
        (self.0 >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Generate a uniform f64 in [low, high).
    fn gen_range(&mut self, low: f64, high: f64) -> f64 {
        low + self.gen_f64() * (high - low)
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single serendipity suggestion — an old note the user may want to revisit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerendipityItem {
    /// The note metadata.
    pub note: NoteMeta,
    /// Human-readable reason for the suggestion.
    pub reason: String,
    /// Raw relevance score (before jitter).
    pub raw_score: f64,
    /// Final score after random jitter.
    pub final_score: f64,
}

/// The full serendipity result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerendipityResult {
    /// The serendipity items.
    pub items: Vec<SerendipityItem>,
    /// How many stale notes were considered.
    pub stale_count: usize,
    /// How many recent notes formed the signal.
    pub recent_count: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate serendipity suggestions from the vault.
///
/// Returns up to `count` old notes that may be interesting given the user's
/// recent activity.
#[instrument(skip(context))]
pub fn generate_serendipity(
    context: &StorageContext,
    count: Option<usize>,
) -> Result<SerendipityResult> {
    let count = count.unwrap_or(DEFAULT_COUNT).clamp(1, 10);
    let (connection, _) = crate::storage::pool::open_connection(context)?;

    let now = Utc::now();
    let stale_cutoff = (now - Duration::days(STALE_DAYS)).to_rfc3339();
    let recent_cutoff = (now - Duration::days(RECENT_DAYS)).to_rfc3339();

    // 1. Fetch all note metas (the function already returns them sorted by
    //    updated_at DESC).
    let all_metas = crate::storage::list_all_note_metas(&connection)?;

    // 2. Partition into stale / recent.
    let mut stale: Vec<&NoteMeta> = Vec::new();
    let mut recent: Vec<&NoteMeta> = Vec::new();

    for meta in &all_metas {
        if meta.updated_at.is_empty() || meta.updated_at < stale_cutoff {
            stale.push(meta);
        } else if meta.updated_at >= recent_cutoff {
            recent.push(meta);
        }
    }

    // 3. Build the recent-interest signal: unique keywords, tags, title words.
    let mut recent_keywords: HashSet<String> = HashSet::new();
    let mut recent_tags: HashSet<String> = HashSet::new();
    let mut recent_title_words: HashSet<String> = HashSet::new();

    for meta in &recent {
        for kw in &meta.keywords {
            if !kw.is_empty() {
                recent_keywords.insert(kw.to_lowercase());
            }
        }
        for tag in &meta.tags {
            if !tag.is_empty() {
                recent_tags.insert(tag.to_lowercase());
            }
        }
        for word in meta.title.split_whitespace() {
            let w = word
                .trim_matches(|c: char| c.is_ascii_punctuation())
                .to_lowercase();
            if !w.is_empty() && w.len() > 1 {
                recent_title_words.insert(w);
            }
        }
    }

    // 4. Score each stale note.
    let mut scored: Vec<(&NoteMeta, f64)> = Vec::with_capacity(stale.len());

    for meta in &stale {
        let mut score = BASE_PARTICIPATION;

        // Keyword overlap
        for kw in &meta.keywords {
            if recent_keywords.contains(&kw.to_lowercase()) {
                score += KEYWORD_BONUS;
            }
        }

        // Tag overlap
        for tag in &meta.tags {
            if recent_tags.contains(&tag.to_lowercase()) {
                score += TAG_BONUS;
            }
        }

        // Title word overlap
        for word in meta.title.split_whitespace() {
            let w = word
                .trim_matches(|c: char| c.is_ascii_punctuation())
                .to_lowercase();
            if !w.is_empty() && w.len() > 1 && recent_title_words.contains(&w) {
                score += TITLE_WORD_BONUS;
            }
        }

        scored.push((meta, score));
    }

    // 5. Apply random jitter and sort.
    let mut rng = SimpleRng::from_clock();
    let mut items: Vec<SerendipityItem> = scored
        .into_iter()
        .map(|(meta, raw)| {
            let jitter_factor = 1.0 + rng.gen_range(-JITTER, JITTER);
            let final_score = raw * jitter_factor;
            let reason = build_reason(
                meta,
                raw,
                &recent_keywords,
                &recent_tags,
                &recent_title_words,
            );
            SerendipityItem {
                note: meta.clone(),
                reason,
                raw_score: raw,
                final_score,
            }
        })
        .collect();

    // Sort by final_score descending, take top N.
    items.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items.truncate(count);

    Ok(SerendipityResult {
        items,
        stale_count: stale.len(),
        recent_count: recent.len(),
    })
}

// ---------------------------------------------------------------------------
// Reason builder
// ---------------------------------------------------------------------------

/// Build a human-readable reason explaining why this stale note was selected.
fn build_reason(
    meta: &NoteMeta,
    score: f64,
    recent_keywords: &HashSet<String>,
    recent_tags: &HashSet<String>,
    recent_title_words: &HashSet<String>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Count overlaps for the reason string.
    let kw_overlap: Vec<&str> = meta
        .keywords
        .iter()
        .filter(|kw| recent_keywords.contains(&kw.to_lowercase()))
        .map(|s| s.as_str())
        .collect();
    let tag_overlap: Vec<&str> = meta
        .tags
        .iter()
        .filter(|t| recent_tags.contains(&t.to_lowercase()))
        .map(|s| s.as_str())
        .collect();

    if !kw_overlap.is_empty() {
        parts.push(format!("shared keywords: {}", kw_overlap.join(", ")));
    }
    if !tag_overlap.is_empty() {
        parts.push(format!("shared tags: {}", tag_overlap.join(", ")));
    }

    // Check if any title word overlaps.
    let title_overlap: Vec<String> = meta
        .title
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| c.is_ascii_punctuation())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty() && w.len() > 1 && recent_title_words.contains(w))
        .collect();
    if !title_overlap.is_empty() {
        parts.push(format!(
            "title related to recent interests ({})",
            title_overlap.join(", ")
        ));
    }

    if parts.is_empty() {
        parts.push("not recently viewed".to_string());
    }

    format!("💡 {} · relevance score: {:.1}", parts.join("; "), score)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stale_cutoff_comparison() {
        let now = Utc::now();
        let old = (now - Duration::days(31)).to_rfc3339();
        let borderline = (now - Duration::days(30)).to_rfc3339();
        let recent = (now - Duration::days(1)).to_rfc3339();

        // A note with empty updated_at should be considered stale.
        assert!(String::new() < old);
        assert!(String::new() < borderline);

        // RFC 3339 string comparison works because the format is
        // lexicographically sortable when dates are in the same timezone (UTC).
        assert!(old < recent);
        assert!(borderline < recent);
        assert!(recent > old);
    }

    #[test]
    fn test_build_reason_keywords() {
        let mut recent_kw = HashSet::new();
        recent_kw.insert("rust".to_string());
        recent_kw.insert("async".to_string());

        let meta = NoteMeta {
            title: "Rust Async Patterns".to_string(),
            keywords: vec!["rust".to_string(), "async".to_string(), "tokio".to_string()],
            ..Default::default()
        };

        let reason = build_reason(&meta, 100.0, &recent_kw, &HashSet::new(), &HashSet::new());
        assert!(reason.contains("rust"));
        assert!(reason.contains("async"));
        assert!(reason.contains("100.0"));
    }

    #[test]
    fn test_build_reason_fallback() {
        let meta = NoteMeta {
            title: "Old Note".to_string(),
            ..Default::default()
        };
        let reason = build_reason(
            &meta,
            10.0,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(reason.contains("not recently viewed"));
    }

    #[test]
    fn test_partition_logic() {
        // Verify that the string-comparison logic for splitting stale vs recent
        // works correctly with RFC 3339 timestamps.
        let now = Utc::now();
        let stale_cutoff = (now - Duration::days(STALE_DAYS)).to_rfc3339();
        let recent_cutoff = (now - Duration::days(RECENT_DAYS)).to_rfc3339();

        let very_old = (now - Duration::days(60)).to_rfc3339();
        let slightly_old = (now - Duration::days(20)).to_rfc3339();
        let very_recent = (now - Duration::hours(1)).to_rfc3339();

        // very_old < stale_cutoff → stale
        assert!(very_old < stale_cutoff);
        // slightly_old > stale_cutoff, slightly_old < recent_cutoff → neither
        // stale nor recent
        assert!(slightly_old > stale_cutoff);
        assert!(slightly_old < recent_cutoff);
        // very_recent > recent_cutoff → recent
        assert!(very_recent > recent_cutoff);
    }

    #[test]
    fn test_simple_rng_produces_different_values() {
        let mut rng = SimpleRng::from_clock();
        let a = rng.gen_f64();
        let b = rng.gen_f64();
        // Extremely unlikely to be equal.
        assert_ne!(a, b);
    }

    #[test]
    fn test_simple_rng_range() {
        let mut rng = SimpleRng::from_clock();
        for _ in 0..100 {
            let v = rng.gen_range(-0.5, 0.5);
            assert!(v >= -0.5);
            assert!(v < 0.5);
        }
    }
}
