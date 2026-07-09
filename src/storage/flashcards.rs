//! Flashcard CRUD + FSRS scheduling persistence (#1912).
//!
//! Flashcards are stored in a dedicated SQLite table with their FSRS
//! scheduling state serialized as JSON. This module provides the storage
//! layer; the scheduling algorithm lives in `crate::fsrs`.

#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use crate::fsrs::{self, CardState, SchedulingState};

use super::pool::open_connection;
use super::StorageContext;

// ────────────────────────────────────────────────────────
// Types
// ────────────────────────────────────────────────────────

/// A flashcard with front/back content and FSRS scheduling state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Flashcard {
    /// Unique identifier (UUID).
    pub id: String,
    /// The question / prompt (front of card).
    pub front: String,
    /// The answer (back of card).
    pub back: String,
    /// Optional source note ID for traceability.
    #[serde(default)]
    pub note_id: String,
    /// Optional tags (comma-separated).
    #[serde(default)]
    pub tags: String,
    /// Serialized FSRS scheduling state (JSON).
    #[serde(default)]
    pub scheduling: String,
    /// Creation timestamp (ISO-8601).
    pub created_at: String,
    /// Last modification timestamp (ISO-8601).
    pub updated_at: String,
}

/// Flashcard with parsed scheduling state for convenience.
#[derive(Debug, Clone)]
pub struct FlashcardWithState {
    pub card: Flashcard,
    pub state: SchedulingState,
}

impl FlashcardWithState {
    /// Whether the card is currently due for review.
    pub fn is_due(&self) -> bool {
        fsrs::is_due(&self.state, Utc::now())
    }

    /// Card state enum.
    pub fn card_state(&self) -> CardState {
        self.state.state
    }

    /// Number of successful reviews.
    pub fn reps(&self) -> u32 {
        self.state.reps
    }

    /// Number of lapses.
    pub fn lapses(&self) -> u32 {
        self.state.lapses
    }
}

/// Statistics about the flashcard collection.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FlashcardStats {
    pub total: usize,
    pub new_cards: usize,
    pub learning: usize,
    pub review: usize,
    pub relearning: usize,
    pub due_now: usize,
    pub total_reps: u32,
    pub total_lapses: u32,
}

// ────────────────────────────────────────────────────────
// CRUD
// ────────────────────────────────────────────────────────

/// Create a new flashcard. Returns the created card.
#[instrument(skip(context))]
pub fn create_flashcard_with_context(
    context: &StorageContext,
    front: &str,
    back: &str,
    note_id: &str,
    tags: &str,
) -> Result<Flashcard> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    let scheduling = serde_json::to_string(&fsrs::new_card_state(Utc::now()))?;

    connection
        .execute(
            r#"
            INSERT INTO flashcards (id, front, back, note_id, tags, scheduling, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![id, front, back, note_id, tags, scheduling, now, now],
        )
        .with_context(|| "failed to create flashcard")?;

    Ok(Flashcard {
        id,
        front: front.to_string(),
        back: back.to_string(),
        note_id: note_id.to_string(),
        tags: tags.to_string(),
        scheduling,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Get a single flashcard by ID.
#[instrument(skip(context))]
pub fn get_flashcard_with_context(context: &StorageContext, id: &str) -> Result<Option<Flashcard>> {
    let (connection, _) = open_connection(context)?;
    connection
        .query_row(
            r#"SELECT id, front, back, note_id, tags, scheduling, created_at, updated_at
               FROM flashcards WHERE id = ?1"#,
            params![id],
            |row| {
                Ok(Flashcard {
                    id: row.get(0)?,
                    front: row.get(1)?,
                    back: row.get(2)?,
                    note_id: row.get(3)?,
                    tags: row.get(4)?,
                    scheduling: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|e| anyhow::anyhow!("failed to query flashcard: {e}"))
}

/// Delete a flashcard by ID. Returns true if deleted.
#[instrument(skip(context))]
pub fn delete_flashcard_with_context(context: &StorageContext, id: &str) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let rows = connection.execute("DELETE FROM flashcards WHERE id = ?1", params![id])?;
    Ok(rows > 0)
}

/// List all flashcards, optionally filtered by tag.
#[instrument(skip(context))]
pub fn list_flashcards_with_context(
    context: &StorageContext,
    tag_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<Flashcard>> {
    let (connection, _) = open_connection(context)?;
    let limit_i64 = limit as i64;

    let cards = if let Some(tag) = tag_filter {
        let pattern = format!("%{tag}%");
        connection
            .prepare(
                r#"SELECT id, front, back, note_id, tags, scheduling, created_at, updated_at
                   FROM flashcards WHERE tags LIKE ?1
                   ORDER BY created_at DESC LIMIT ?2"#,
            )?
            .query_map(params![pattern, limit_i64], |row| {
                Ok(Flashcard {
                    id: row.get(0)?,
                    front: row.get(1)?,
                    back: row.get(2)?,
                    note_id: row.get(3)?,
                    tags: row.get(4)?,
                    scheduling: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        connection
            .prepare(
                r#"SELECT id, front, back, note_id, tags, scheduling, created_at, updated_at
                   FROM flashcards ORDER BY created_at DESC LIMIT ?1"#,
            )?
            .query_map(params![limit_i64], |row| {
                Ok(Flashcard {
                    id: row.get(0)?,
                    front: row.get(1)?,
                    back: row.get(2)?,
                    note_id: row.get(3)?,
                    tags: row.get(4)?,
                    scheduling: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(cards)
}

/// Get all flashcards that are due for review (using FSRS scheduling).
#[instrument(skip(context))]
pub fn get_due_flashcards_with_context(
    context: &StorageContext,
) -> Result<Vec<FlashcardWithState>> {
    let cards = list_flashcards_with_context(context, None, 10000)?;
    let now = Utc::now();

    let due: Vec<FlashcardWithState> = cards
        .into_iter()
        .filter_map(|card| {
            let state = parse_scheduling(&card.scheduling)?;
            if fsrs::is_due(&state, now) {
                Some(FlashcardWithState { card, state })
            } else {
                None
            }
        })
        .collect();

    Ok(due)
}

/// Record a review: apply the FSRS scheduler and persist the updated state.
///
/// Returns the updated flashcard with its new scheduling state.
#[instrument(skip(context))]
pub fn review_flashcard_with_context(
    context: &StorageContext,
    id: &str,
    rating: fsrs::Rating,
) -> Result<Flashcard> {
    let card = get_flashcard_with_context(context, id)?
        .ok_or_else(|| anyhow::anyhow!("flashcard not found: {id}"))?;

    let prev_state = parse_scheduling(&card.scheduling)
        .ok_or_else(|| anyhow::anyhow!("failed to parse scheduling state for card {id}"))?;

    let outcome = fsrs::schedule(&prev_state, rating, Utc::now());
    let new_scheduling = serde_json::to_string(&outcome.new_state)?;

    let now = Utc::now().to_rfc3339();
    let (connection, _) = open_connection(context)?;
    connection.execute(
        r#"UPDATE flashcards SET scheduling = ?1, updated_at = ?2 WHERE id = ?3"#,
        params![new_scheduling, now, id],
    )?;

    let mut updated = card;
    updated.scheduling = new_scheduling;
    updated.updated_at = now;
    Ok(updated)
}

/// Get statistics about the flashcard collection.
#[instrument(skip(context))]
pub fn get_flashcard_stats_with_context(context: &StorageContext) -> Result<FlashcardStats> {
    let cards = list_flashcards_with_context(context, None, 100000)?;
    let now = Utc::now();

    let mut stats = FlashcardStats {
        total: cards.len(),
        ..Default::default()
    };

    for card in &cards {
        if let Some(state) = parse_scheduling(&card.scheduling) {
            stats.total_reps += state.reps;
            stats.total_lapses += state.lapses;
            match state.state {
                CardState::New => stats.new_cards += 1,
                CardState::Learning => stats.learning += 1,
                CardState::Review => stats.review += 1,
                CardState::Relearning => stats.relearning += 1,
            }
            if fsrs::is_due(&state, now) {
                stats.due_now += 1;
            }
        }
    }

    Ok(stats)
}

/// Parse the JSON scheduling state from a flashcard's `scheduling` field.
/// Delegates to `crate::fsrs::parse_scheduling`.
pub fn parse_scheduling(json: &str) -> Option<SchedulingState> {
    fsrs::parse_scheduling(json)
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> StorageContext {
        let dir = std::env::temp_dir().join(format!("vp-fsrs-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let ctx = StorageContext::for_test(&dir);
        super::super::initialize_storage_with_context(&ctx).unwrap();
        ctx
    }

    #[test]
    fn test_create_and_get_flashcard() {
        let ctx = test_context();
        let card = create_flashcard_with_context(
            &ctx,
            "What is Rust?",
            "A systems programming language",
            "",
            "programming",
        )
        .unwrap();

        assert!(!card.id.is_empty());
        assert_eq!(card.front, "What is Rust?");
        assert_eq!(card.back, "A systems programming language");

        let fetched = get_flashcard_with_context(&ctx, &card.id).unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().front, "What is Rust?");
    }

    #[test]
    fn test_list_flashcards() {
        let ctx = test_context();
        for i in 0..5 {
            create_flashcard_with_context(&ctx, &format!("Q{i}"), &format!("A{i}"), "", "test")
                .unwrap();
        }

        let all = list_flashcards_with_context(&ctx, None, 100).unwrap();
        assert_eq!(all.len(), 5);

        let filtered = list_flashcards_with_context(&ctx, Some("test"), 100).unwrap();
        assert_eq!(filtered.len(), 5);
    }

    #[test]
    fn test_delete_flashcard() {
        let ctx = test_context();
        let card = create_flashcard_with_context(&ctx, "Q", "A", "", "").unwrap();
        assert!(delete_flashcard_with_context(&ctx, &card.id).unwrap());
        assert!(get_flashcard_with_context(&ctx, &card.id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_new_card_is_due() {
        let ctx = test_context();
        create_flashcard_with_context(&ctx, "Q", "A", "", "").unwrap();

        let due = get_due_flashcards_with_context(&ctx).unwrap();
        assert_eq!(due.len(), 1);
        assert!(due[0].is_due());
    }

    #[test]
    fn test_review_graduates_new_card() {
        let ctx = test_context();
        let card = create_flashcard_with_context(&ctx, "Q", "A", "", "").unwrap();

        let updated = review_flashcard_with_context(&ctx, &card.id, fsrs::Rating::Good).unwrap();
        let state = parse_scheduling(&updated.scheduling).unwrap();
        assert_eq!(state.state, CardState::Review);
        assert_eq!(state.reps, 1);
    }

    #[test]
    fn test_stats() {
        let ctx = test_context();
        create_flashcard_with_context(&ctx, "Q1", "A1", "", "").unwrap();
        create_flashcard_with_context(&ctx, "Q2", "A2", "", "").unwrap();

        // Review one as Good (graduates)
        let cards = list_flashcards_with_context(&ctx, None, 100).unwrap();
        review_flashcard_with_context(&ctx, &cards[0].id, fsrs::Rating::Good).unwrap();

        let stats = get_flashcard_stats_with_context(&ctx).unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.new_cards, 1);
        assert_eq!(stats.review, 1);
        assert_eq!(stats.total_reps, 1);
    }

    #[test]
    fn test_parse_scheduling_empty() {
        assert!(parse_scheduling("").is_none());
        assert!(parse_scheduling("invalid json").is_none());
    }
}
