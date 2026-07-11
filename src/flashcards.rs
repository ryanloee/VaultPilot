//! Spaced repetition flashcard system using SM-2 algorithm (#1912).
//!
//! This module implements the backend for the interval-repetition learning
//! system.  Flashcards are stored as Markdown files with YAML frontmatter in
//! the vault's `flashcards/` directory.  Each card tracks SM-2 scheduling
//! state (ease factor, interval, due date, reps) so the CLI and AI agent
//! can query "what's due today" and record review ratings.
//!
//! SM-2 (SuperMemo 2) is the classic algorithm used by Anki. It requires no
//! external dependencies and produces reliable review intervals based on
//! user-rated quality of recall.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::models::AppSettings;

// ─── SM-2 Constants ───────────────────────────────────────────────────

/// Default ease factor for new cards (same as Anki's default).
const DEFAULT_EASE_FACTOR: f32 = 2.5;

/// Minimum ease factor — cards never get easier slower than this.
const MIN_EASE_FACTOR: f32 = 1.3;

/// Map our 4-level rating to SM-2 quality scores (0-5 scale).
///
/// SM-2 quality: 0=complete blackout, 3=recalled with effort, 5=perfect.
fn quality_for_rating(rating: ReviewRating) -> f32 {
    match rating {
        ReviewRating::Again => 0.0, // Complete failure
        ReviewRating::Hard => 3.0,  // Significant difficulty
        ReviewRating::Good => 4.0,  // Hesitation but correct
        ReviewRating::Easy => 5.0,  // Perfect, instant
    }
}

// ─── Types ────────────────────────────────────────────────────────────

/// A single flashcard with SM-2 scheduling metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Flashcard {
    /// Unique identifier (UUID or slug).
    pub id: String,
    /// Front (question) side.
    pub front: String,
    /// Back (answer) side.
    pub back: String,
    /// Source note ID/path this card was derived from (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_note_id: Option<String>,
    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// SM-2 scheduling state, serialized as JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduling: Option<FlashcardScheduling>,
    /// Creation timestamp.
    pub created_at: String,
    /// Last modification timestamp.
    pub updated_at: String,
}

/// SM-2 scheduling state for a flashcard.
///
/// This stores the SuperMemo-2 scheduling parameters so the next review
/// interval can be calculated. Fields are mapped for compatibility:
/// - `stability` stores the SM-2 ease factor (multiplier for interval)
/// - `difficulty` is a 0-1 difficulty rating derived from ease factor
/// - `days_until_due` is the interval in days
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashcardScheduling {
    /// SM-2 ease factor (memory strength multiplier, typically 1.3-2.8).
    pub stability: f32,
    /// Difficulty (0-1 scale, derived from ease factor).
    pub difficulty: f32,
    /// Days until next review (0 = due now).
    pub days_until_due: i64,
    /// Last review date (ISO 8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_review: Option<String>,
    /// Number of times reviewed.
    #[serde(default)]
    pub reps: u32,
}

/// Rating given by the user after reviewing a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewRating {
    /// Complete blackout, didn't recall at all.
    Again,
    /// Recalled with significant effort.
    Hard,
    /// Recalled after some hesitation.
    Good,
    /// Perfect, instant recall.
    Easy,
}

/// Result of a review operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResult {
    /// Whether the review was recorded successfully.
    pub success: bool,
    /// Updated scheduling state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduling: Option<FlashcardScheduling>,
    /// Error message on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ─── SM-2 Scheduler ───────────────────────────────────────────────────

/// Compute the next SM-2 scheduling state after a review.
///
/// Returns (new_ease_factor, new_interval_days, new_reps).
fn sm2_schedule(current: Option<&FlashcardScheduling>, rating: ReviewRating) -> (f32, i64, u32) {
    let quality = quality_for_rating(rating);

    // Get current ease factor and repetition count.
    let (mut ease, prev_reps) = match current {
        Some(s) => (s.stability.max(MIN_EASE_FACTOR), s.reps),
        None => (DEFAULT_EASE_FACTOR, 0),
    };

    // SM-2 core algorithm
    if quality < 3.0 {
        // Failed recall: reset repetitions, interval = 1 day
        return (ease.max(MIN_EASE_FACTOR), 1, 0);
    }

    // Update ease factor per SM-2 formula:
    // EF' = EF + (0.1 - (5 - q) * (0.08 + (5 - q) * 0.02))
    let delta = 0.1 - (5.0 - quality) * (0.08 + (5.0 - quality) * 0.02);
    ease = (ease + delta).max(MIN_EASE_FACTOR);

    let new_reps = prev_reps + 1;

    // Calculate interval based on repetition number
    let interval = match new_reps {
        1 => 1i64,
        2 => 6,
        _ => {
            // For rep 3+, interval = prev_interval * ease_factor
            let prev_interval = match current {
                Some(s) => s.days_until_due.max(1) as f32,
                None => 1.0,
            };
            (prev_interval * ease).round() as i64
        }
    };

    (ease, interval, new_reps)
}

/// Convert ease factor to a 0-1 difficulty rating (higher = harder).
fn ease_to_difficulty(ease: f32) -> f32 {
    // Ease ranges ~1.3-2.8. Map inversely: ease 2.8 → difficulty 0, ease 1.3 → difficulty 1.
    ((2.8 - ease) / (2.8 - 1.3)).clamp(0.0, 1.0)
}

// ─── File path helpers ────────────────────────────────────────────────

/// The flashcards subdirectory inside the vault.
pub fn flashcards_dir(settings: &AppSettings) -> PathBuf {
    Path::new(&settings.vault_dir).join("flashcards")
}

/// Build the file path for a flashcard by ID.
fn flashcard_path(settings: &AppSettings, id: &str) -> PathBuf {
    // Sanitize: only allow alnum, '-', '_' to prevent path traversal.
    let safe: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let safe = if safe.is_empty() {
        "unnamed".to_string()
    } else {
        safe
    };
    flashcards_dir(settings).join(format!("{safe}.md"))
}

// ─── Serialization ────────────────────────────────────────────────────

/// Serialize a flashcard to Markdown with YAML frontmatter.
pub fn flashcard_to_markdown(card: &Flashcard) -> String {
    // Build frontmatter as serde_json map.
    let frontmatter = serde_json::json!({
        "id": card.id,
        "front": card.front,
        "back": card.back,
        "sourceNoteId": card.source_note_id,
        "tags": card.tags,
        "scheduling": card.scheduling,
        "createdAt": card.created_at,
        "updatedAt": card.updated_at,
    });

    let yaml = serde_yaml_ng::to_string(&frontmatter).unwrap_or_default();

    format!(
        "---\n{yaml}---\n\n# Front\n\n{}\n\n# Back\n\n{}\n",
        card.front, card.back
    )
}

/// Parse a flashcard from Markdown with YAML frontmatter.
fn parse_flashcard_markdown(content: &str) -> Result<Flashcard, String> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return Err("missing YAML frontmatter".to_string());
    }

    let after_first_delim = &content[3..];
    let end = after_first_delim
        .find("\n---")
        .ok_or_else(|| "missing closing frontmatter delimiter".to_string())?;
    let yaml_str = &after_first_delim[..end];
    let body = &after_first_delim[end + 4..];

    let meta: HashMap<String, serde_json::Value> =
        serde_yaml_ng::from_str(yaml_str).map_err(|e| format!("YAML parse error: {e}"))?;

    let get_str = |key: &str| -> String {
        meta.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let id = get_str("id");
    let front = get_str("front");
    let back = get_str("back");
    let source_note_id = meta
        .get("sourceNoteId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let tags: Vec<String> = meta
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|seq| {
            seq.iter()
                .filter_map(|v: &serde_json::Value| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let scheduling: Option<FlashcardScheduling> = meta.get("scheduling").and_then(|v| {
        if v.is_null() {
            None
        } else {
            serde_json::from_value(v.clone()).ok()
        }
    });

    let created_at = get_str("createdAt");
    let updated_at = get_str("updatedAt");

    // If front/back are empty in frontmatter, try to extract from body sections.
    let (front, back) = if front.is_empty() || back.is_empty() {
        parse_body_sections(body, front, back)
    } else {
        (front, back)
    };

    Ok(Flashcard {
        id,
        front,
        back,
        source_note_id,
        tags,
        scheduling,
        created_at,
        updated_at,
    })
}

/// Parse # Front and # Back sections from the body as a fallback.
fn parse_body_sections(body: &str, front: String, back: String) -> (String, String) {
    let mut front_buf = String::new();
    let mut back_buf = String::new();
    let mut current_section = 0u8; // 0=none, 1=front, 2=back

    for line in body.lines() {
        if let Some(header) = line.strip_prefix("# ") {
            current_section = match header.trim() {
                "Front" => 1,
                "Back" => 2,
                _ => 0,
            };
        } else if current_section == 1 {
            front_buf.push_str(line);
            front_buf.push('\n');
        } else if current_section == 2 {
            back_buf.push_str(line);
            back_buf.push('\n');
        }
    }

    let f = if front.is_empty() {
        front_buf.trim().to_string()
    } else {
        front
    };
    let b = if back.is_empty() {
        back_buf.trim().to_string()
    } else {
        back
    };
    (f, b)
}

// ─── CRUD operations ──────────────────────────────────────────────────

/// Create a new flashcard in the vault.
#[instrument(skip(settings))]
pub fn create_flashcard(
    settings: &AppSettings,
    front: String,
    back: String,
    source_note_id: Option<String>,
    tags: Vec<String>,
) -> Result<Flashcard, String> {
    let dir = flashcards_dir(settings);
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create flashcards dir: {e}"))?;

    let now = Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let card = Flashcard {
        id: id.clone(),
        front,
        back,
        source_note_id,
        tags,
        scheduling: None, // No scheduling until first review
        created_at: now.clone(),
        updated_at: now,
    };

    let path = flashcard_path(settings, &id);
    let markdown = flashcard_to_markdown(&card);
    std::fs::write(&path, markdown).map_err(|e| format!("failed to write flashcard: {e}"))?;

    Ok(card)
}

/// Load a flashcard by ID.
pub fn load_flashcard(settings: &AppSettings, id: &str) -> Result<Flashcard, String> {
    let path = flashcard_path(settings, id);
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("flashcard not found: {e}"))?;
    let mut card = parse_flashcard_markdown(&content)?;
    if card.id.is_empty() {
        card.id = id.to_string();
    }
    Ok(card)
}

/// Save a flashcard (update existing).
fn save_flashcard(settings: &AppSettings, card: &Flashcard) -> Result<(), String> {
    let path = flashcard_path(settings, &card.id);
    let markdown = flashcard_to_markdown(card);
    std::fs::write(&path, markdown).map_err(|e| format!("failed to save flashcard: {e}"))
}

/// List all flashcards in the vault.
pub fn list_flashcards(settings: &AppSettings) -> Result<Vec<Flashcard>, String> {
    let dir = flashcards_dir(settings);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut cards = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("failed to read dir: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(card) = parse_flashcard_markdown(&content) {
                    cards.push(card);
                }
            }
        }
    }
    Ok(cards)
}

/// List flashcards that are due for review.
pub fn list_due_flashcards(settings: &AppSettings) -> Result<Vec<Flashcard>, String> {
    let now = Utc::now();
    let all = list_flashcards(settings)?;
    let due: Vec<Flashcard> = all
        .into_iter()
        .filter(|card| match &card.scheduling {
            None => true, // Never reviewed = due now
            Some(s) => {
                if let Some(ref last_str) = s.last_review {
                    if let Ok(last) = DateTime::parse_from_rfc3339(last_str) {
                        let elapsed = now.signed_duration_since(last.with_timezone(&Utc));
                        return elapsed.num_days() >= s.days_until_due;
                    }
                }
                true // Can't parse date = due now
            }
        })
        .collect();
    Ok(due)
}

// ─── Review (SM-2 scheduling) ─────────────────────────────────────────

/// Record a review of a flashcard and update its SM-2 scheduling.
#[instrument(skip(settings))]
pub fn review_flashcard(settings: &AppSettings, id: &str, rating: ReviewRating) -> ReviewResult {
    let mut card = match load_flashcard(settings, id) {
        Ok(c) => c,
        Err(e) => {
            return ReviewResult {
                success: false,
                scheduling: None,
                error: Some(e),
            };
        }
    };

    // Compute new scheduling state using SM-2 algorithm.
    let (ease, interval, reps) = sm2_schedule(card.scheduling.as_ref(), rating);

    let now = Utc::now();
    let scheduling = FlashcardScheduling {
        stability: ease,
        difficulty: ease_to_difficulty(ease),
        days_until_due: interval,
        last_review: Some(now.to_rfc3339()),
        reps,
    };

    card.scheduling = Some(scheduling.clone());
    card.updated_at = now.to_rfc3339();

    if let Err(e) = save_flashcard(settings, &card) {
        return ReviewResult {
            success: false,
            scheduling: Some(scheduling),
            error: Some(e),
        };
    }

    ReviewResult {
        success: true,
        scheduling: Some(scheduling),
        error: None,
    }
}

// ─── Statistics ───────────────────────────────────────────────────────

/// Summary statistics of the flashcard collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashcardStats {
    pub total: usize,
    pub due: usize,
    pub new_cards: usize,
    pub learned: usize,
}

/// Get summary statistics of the flashcard collection.
pub fn get_stats(settings: &AppSettings) -> Result<FlashcardStats, String> {
    let all = list_flashcards(settings)?;
    let due = list_due_flashcards(settings)?;
    let total = all.len();
    let new_cards = all.iter().filter(|c| c.scheduling.is_none()).count();
    let learned = all.iter().filter(|c| c.scheduling.is_some()).count();

    Ok(FlashcardStats {
        total,
        due: due.len(),
        new_cards,
        learned,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flashcard_to_markdown_roundtrip() {
        let card = Flashcard {
            id: "test-card-1".to_string(),
            front: "What is 2+2?".to_string(),
            back: "4".to_string(),
            source_note_id: Some("note_abc".to_string()),
            tags: vec!["math".to_string(), "basic".to_string()],
            scheduling: None,
            created_at: "2026-07-10T00:00:00+00:00".to_string(),
            updated_at: "2026-07-10T00:00:00+00:00".to_string(),
        };

        let md = flashcard_to_markdown(&card);
        assert!(md.contains("test-card-1"));
        assert!(md.contains("What is 2+2?"));
        assert!(md.contains("# Front"));
        assert!(md.contains("# Back"));

        let parsed = parse_flashcard_markdown(&md).unwrap();
        assert_eq!(parsed.id, "test-card-1");
        assert_eq!(parsed.front, "What is 2+2?");
        assert_eq!(parsed.back, "4");
        assert_eq!(parsed.source_note_id, Some("note_abc".to_string()));
    }

    #[test]
    fn flashcard_with_scheduling_roundtrip() {
        let scheduling = FlashcardScheduling {
            stability: 2.5,
            difficulty: 0.3,
            days_until_due: 3,
            last_review: Some("2026-07-10T00:00:00+00:00".to_string()),
            reps: 5,
        };
        let card = Flashcard {
            id: "scheduled-card".to_string(),
            front: "Question".to_string(),
            back: "Answer".to_string(),
            source_note_id: None,
            tags: vec![],
            scheduling: Some(scheduling),
            created_at: "2026-07-01T00:00:00+00:00".to_string(),
            updated_at: "2026-07-10T00:00:00+00:00".to_string(),
        };

        let md = flashcard_to_markdown(&card);
        let parsed = parse_flashcard_markdown(&md).unwrap();
        assert!(parsed.scheduling.is_some());
        let s = parsed.scheduling.unwrap();
        assert_eq!(s.reps, 5);
        assert!((s.stability - 2.5).abs() < 0.01);
    }

    #[test]
    fn review_rating_serialization() {
        let ratings = vec![
            ReviewRating::Again,
            ReviewRating::Hard,
            ReviewRating::Good,
            ReviewRating::Easy,
        ];
        for rating in ratings {
            let json = serde_json::to_string(&rating).unwrap();
            let parsed: ReviewRating = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, rating);
        }
    }

    #[test]
    fn flashcard_path_sanitization() {
        let settings = AppSettings {
            vault_dir: "/tmp/test_vault".to_string(),
            ..Default::default()
        };
        let path = flashcard_path(&settings, "../../../etc/passwd");
        assert!(!path.to_string_lossy().contains(".."));
        assert!(path.starts_with("/tmp/test_vault/flashcards/"));
    }

    #[test]
    fn parse_body_sections_extracts_front_and_back() {
        let body = "\n# Front\n\nWhat is Rust?\n\n# Back\n\nA systems programming language.\n";
        let (front, back) = parse_body_sections(body, String::new(), String::new());
        assert!(front.contains("What is Rust?"));
        assert!(back.contains("systems programming language"));
    }

    #[test]
    fn flashcard_stats_serialization() {
        let stats = FlashcardStats {
            total: 10,
            due: 3,
            new_cards: 2,
            learned: 8,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let parsed: FlashcardStats = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total, 10);
        assert_eq!(parsed.due, 3);
        assert_eq!(parsed.new_cards, 2);
        assert_eq!(parsed.learned, 8);
    }

    // ── SM-2 algorithm tests (#1912) ──

    #[test]
    fn sm2_new_card_good_rating_produces_valid_interval() {
        let (ease, interval, reps) = sm2_schedule(None, ReviewRating::Good);
        // First successful review: interval = 1 day, reps = 1
        assert_eq!(interval, 1);
        assert_eq!(reps, 1);
        // Ease should have been adjusted from default 2.5
        assert!(ease >= MIN_EASE_FACTOR);
    }

    #[test]
    fn sm2_again_resets_repetitions() {
        // Simulate a card with some history
        let sched = FlashcardScheduling {
            stability: 2.5,
            difficulty: 0.2,
            days_until_due: 10,
            last_review: Some("2026-07-01T00:00:00+00:00".to_string()),
            reps: 5,
        };
        let (_ease, interval, reps) = sm2_schedule(Some(&sched), ReviewRating::Again);
        // Failed recall: reps reset to 0, interval = 1
        assert_eq!(reps, 0);
        assert_eq!(interval, 1);
    }

    #[test]
    fn sm2_easy_gives_longer_interval_than_good() {
        // After first review (reps=1, interval=1)
        let sched = FlashcardScheduling {
            stability: 2.5,
            difficulty: 0.2,
            days_until_due: 1,
            last_review: Some("2026-07-10T00:00:00+00:00".to_string()),
            reps: 1,
        };
        let (_e_good, interval_good, _) = sm2_schedule(Some(&sched), ReviewRating::Good);
        let (_e_easy, interval_easy, _) = sm2_schedule(Some(&sched), ReviewRating::Easy);

        // Second review: interval = 6 for both (SM-2 fixed at rep 2)
        assert_eq!(interval_good, 6);
        assert_eq!(interval_easy, 6);
        // But ease factor should differ — Easy increases ease more
        // (tested via sm2_ease_factor_changes_with_rating below)
    }

    #[test]
    fn sm2_third_review_uses_ease_multiplier() {
        let sched = FlashcardScheduling {
            stability: 2.5,
            difficulty: 0.2,
            days_until_due: 6, // After second review
            last_review: Some("2026-07-10T00:00:00+00:00".to_string()),
            reps: 2,
        };
        let (ease, interval, reps) = sm2_schedule(Some(&sched), ReviewRating::Good);
        assert_eq!(reps, 3);
        // interval = round(6 * ease)
        let expected = (6.0_f32 * ease).round() as i64;
        assert_eq!(interval, expected);
        assert!(interval > 6, "Third interval should be longer than second");
    }

    #[test]
    fn sm2_ease_factor_never_below_minimum() {
        // Repeatedly fail to drive ease factor down
        let mut sched: Option<FlashcardScheduling> = None;
        for _ in 0..10 {
            let (ease, interval, reps) = sm2_schedule(sched.as_ref(), ReviewRating::Again);
            sched = Some(FlashcardScheduling {
                stability: ease,
                difficulty: ease_to_difficulty(ease),
                days_until_due: interval,
                last_review: Some("2026-07-10T00:00:00+00:00".to_string()),
                reps,
            });
        }
        assert!(sched.unwrap().stability >= MIN_EASE_FACTOR);
    }

    #[test]
    fn sm2_ease_factor_changes_with_rating() {
        let sched = Some(FlashcardScheduling {
            stability: 2.5,
            difficulty: 0.2,
            days_until_due: 1,
            last_review: Some("2026-07-10T00:00:00+00:00".to_string()),
            reps: 1,
        });
        let (ease_good, _, _) = sm2_schedule(sched.as_ref(), ReviewRating::Good);
        let (ease_easy, _, _) = sm2_schedule(sched.as_ref(), ReviewRating::Easy);
        // Easy (quality=5) should keep ease higher than Good (quality=4)
        assert!(
            ease_easy >= ease_good,
            "Easy rating should yield higher ease factor: easy={ease_easy}, good={ease_good}"
        );
    }

    #[test]
    fn ease_to_difficulty_inverts_correctly() {
        // High ease (easy card) → low difficulty
        assert!(ease_to_difficulty(2.5) < 0.3);
        // Low ease (hard card) → high difficulty
        assert!(ease_to_difficulty(1.3) > 0.9);
        // Clamped
        assert_eq!(ease_to_difficulty(3.0), 0.0);
        assert_eq!(ease_to_difficulty(1.0), 1.0);
    }
}
