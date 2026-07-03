//! # Self-Organizing Vault Engine
//!
//! Implements feature #2176: auto-analyze, link, and categorize new notes after
//! write — inspired by Mem.ai's "Capture First, Organize Never" philosophy.
//!
//! ## Architecture
//!
//! **Layer 1** (write-time, <100 ms): lightweight keyword extraction via TF,
//! FTS5-based duplicate/similarity detection, and collection suggestion based on
//! keyword-to-collection rules.
//!
//! **Layer 2** (background, every N minutes): heavier semantic analysis using AI
//! calls (when available), generation of weak links (pending associations awaiting
//! user confirmation), and suggestions for new collections or tags.  Cost control:
//! at most 10 notes per round.
//!
//! **Layer 3** (view-level): handled by a separate issue (#1985).
//!
//! ## Integration
//!
//! - Call `AutoOrganizer::process_new_note(...)` from the note-save path.
//! - Start the background worker with `AutoOrganizer::start_background_worker(...)`.
//! - Use CLI commands `vp organize --auto`, `--watch`, `--pending`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use crate::models::{NoteMeta, WeakLink, WeakLinkStatus};
use crate::storage::StorageContext;

use super::event_bus::{self, Event, NoteAction};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum content preview length for analysis (characters).
const ANALYSIS_PREVIEW_LEN: usize = 500;

/// Minimum term frequency to be considered a keyword.
const MIN_TF: usize = 2;

/// Maximum keywords to extract.
const MAX_KEYWORDS: usize = 10;

/// Minimum FTS5 rank threshold for considering content "similar" (larger = more
/// similar when using BM25-style ranking; FTS5 default bm25 returns negative
/// scores where larger (less negative) means more similar).
const SIMILARITY_THRESHOLD: f64 = -10.0;

/// Default interval (minutes) for Layer 2 background analysis rounds.
const DEFAULT_ANALYSIS_INTERVAL_MINUTES: u64 = 15;

/// Max notes processed per Layer 2 round (cost control for LLM calls).
const MAX_NOTES_PER_ROUND: usize = 10;

// ---------------------------------------------------------------------------
// Core analysis results
// ---------------------------------------------------------------------------

/// Result of a single Layer 1 analysis pass.
#[derive(Debug, Clone)]
pub struct Layer1Result {
    pub keywords: Vec<String>,
    pub duplicate_note_ids: Vec<String>,
    pub suggested_collections: Vec<String>,
    pub processing_time_ms: u64,
    pub pending_analysis_id: Option<String>,
}

/// Extraction metadata about keywords.
#[derive(Debug, Clone)]
struct KeywordEntry {
    word: String,
    frequency: usize,
}

// ---------------------------------------------------------------------------
// AutoOrganizer
// ---------------------------------------------------------------------------

/// The auto-organizer engine.  Can be used directly (Layer 1) or as a
/// background worker (Layer 2).
#[derive(Debug)]
pub struct AutoOrganizer {
    /// Interval in minutes for Layer 2 background rounds.
    #[allow(dead_code)]
    analysis_interval_minutes: u64,
    /// Maximum notes to process per Layer 2 round.
    max_notes_per_round: usize,
}

impl Default for AutoOrganizer {
    fn default() -> Self {
        Self {
            analysis_interval_minutes: DEFAULT_ANALYSIS_INTERVAL_MINUTES,
            max_notes_per_round: MAX_NOTES_PER_ROUND,
        }
    }
}

impl AutoOrganizer {
    /// Create a new instance with custom settings.
    pub fn new(analysis_interval_minutes: u64, max_notes_per_round: usize) -> Self {
        Self {
            analysis_interval_minutes,
            max_notes_per_round,
        }
    }

    // ════════════════════════════════════════════════
    // Layer 1 — Write-time Lightweight Analysis
    // ════════════════════════════════════════════════

    /// Analyze a note right after it was written/updated.
    ///
    /// Runs synchronously and aims to finish in <100 ms.
    /// Returns analysis results including extracted keywords, possible
    /// duplicates, and collection suggestions.
    #[instrument(skip(context, body))]
    pub fn process_new_note(
        &self,
        context: &StorageContext,
        note_meta: &NoteMeta,
        body: &str,
        action: NoteAction,
    ) -> Result<Layer1Result> {
        let start = Instant::now();
        let connection = context.get_connection()?;

        let _preview = &body[..body
            .char_indices()
            .nth(ANALYSIS_PREVIEW_LEN)
            .map(|(i, _)| i)
            .unwrap_or(body.len())];

        // 1. Keyword extraction (TF)
        let keywords = self.extract_keywords(body);

        // 2. Duplicate / similarity check via FTS5
        let duplicate_note_ids = self.find_similar_notes(&connection, &note_meta.id, &keywords)?;

        // 3. Collection suggestion
        let suggested_collections =
            self.suggest_collections(&connection, &keywords, &note_meta.id)?;

        // 4. Enqueue for Layer 2 background analysis
        let pending_id = self.enqueue_for_deep_analysis(&connection, &note_meta.id, action)?;

        // 5. Update the note's keywords in DB if we found new ones
        if !keywords.is_empty() && note_meta.keywords.is_empty() {
            Self::update_note_keywords(&connection, &note_meta.id, &keywords)?;
        }

        let elapsed = start.elapsed();
        debug!(
            note_id = %note_meta.id,
            keywords = ?keywords,
            duplicates = ?duplicate_note_ids,
            collections = ?suggested_collections,
            elapsed_ms = elapsed.as_millis(),
            "Layer 1 analysis complete"
        );

        Ok(Layer1Result {
            keywords,
            duplicate_note_ids,
            suggested_collections,
            processing_time_ms: elapsed.as_millis() as u64,
            pending_analysis_id: pending_id,
        })
    }
    pub fn spawn_event_listener(context: StorageContext) {
        tokio::spawn(async move {
            let mut rx = event_bus::subscribe();
            let organizer = std::sync::Arc::new(AutoOrganizer::default());
            loop {
                let event = rx.recv().await;
                match event {
                    Ok(arc_ev) => {
                        let Event::NoteChanged(nc) = &*arc_ev;
                        if nc.action == NoteAction::Deleted {
                            continue;
                        }
                        // Clone what each blocking task needs
                        let context = context.clone();
                        let note_id = nc.note_id.clone();
                        let nc = nc.clone();
                        let organizer = std::sync::Arc::clone(&organizer);
                        // Wrap blocking SQLite operations in a catch_unwind-protected spawn_blocking
                        let result = futures_util::future::FutureExt::catch_unwind(
                            std::panic::AssertUnwindSafe(async move {
                                if let Err(e) = tokio::task::spawn_blocking(move || {
                                    match crate::storage::load_note_with_context(
                                        &context,
                                        &nc.note_id,
                                    ) {
                                        Ok(doc) => {
                                            if let Err(e) = organizer.process_new_note(
                                                &context, &doc.meta, &doc.body, nc.action,
                                            ) {
                                                warn!(
                                                    note_id = %nc.note_id,
                                                    error = %e,
                                                    "Layer 1 analysis failed"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            warn!(
                                                note_id = %nc.note_id,
                                                error = %e,
                                                "failed to load note for analysis"
                                            );
                                        }
                                    }
                                })
                                .await
                                {
                                    warn!(
                                        note_id = %note_id,
                                        error = %e,
                                        "spawn_blocking Layer 1 analysis panicked"
                                    );
                                }
                            }),
                        )
                        .await;
                        if let Err(panic) = result {
                            warn!(
                                error = ?panic,
                                "spawn_event_listener task panicked, restarting"
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(lagged = n, "event bus subscriber lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        warn!("event bus closed, listener exiting");
                        break;
                    }
                }
            }
        });
    }

    // ════════════════════════════════════════════════
    // Layer 2 — Background Batch Analysis
    // ════════════════════════════════════════════════

    /// Start a background task that periodically processes the pending analysis
    /// queue.
    pub fn start_background_worker(context: StorageContext) {
        let interval_minutes = context
            .get_connection()
            .ok()
            .and_then(|conn| {
                conn.query_row(
                    "SELECT value FROM settings WHERE key = 'analysis_interval_minutes'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .ok()
            })
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_ANALYSIS_INTERVAL_MINUTES);

        tokio::spawn(async move {
            let organizer =
                std::sync::Arc::new(AutoOrganizer::new(interval_minutes, MAX_NOTES_PER_ROUND));
            let interval = Duration::from_secs(interval_minutes * 60);
            loop {
                tokio::time::sleep(interval).await;
                let result = futures_util::future::FutureExt::catch_unwind(
                    std::panic::AssertUnwindSafe(async {
                        // Clone what each blocking task needs
                        let context = context.clone();
                        let organizer = std::sync::Arc::clone(&organizer);
                        // Wrap blocking SQLite operations in spawn_blocking
                        if let Err(e) = tokio::task::spawn_blocking(move || {
                            if let Err(e) = organizer.run_analysis_round(&context) {
                                warn!(error = %e, "Layer 2 analysis round failed");
                            }
                        })
                        .await
                        {
                            warn!(
                                error = %e,
                                "spawn_blocking Layer 2 analysis panicked"
                            );
                        }
                    }),
                )
                .await;
                if let Err(panic) = result {
                    warn!(
                        error = ?panic,
                        "start_background_worker task panicked, restarting"
                    );
                }
            }
        });
    }

    /// Run a single analysis round: process up to `max_notes_per_round` pending
    /// notes with AI-powered semantic analysis.
    #[instrument(skip(context))]
    pub fn run_analysis_round(&self, context: &StorageContext) -> Result<AnalysisRoundResult> {
        let start = Instant::now();
        let connection = context.get_connection()?;

        // Fetch pending notes
        let pending = fetch_pending_analyses(&connection, self.max_notes_per_round)?;
        if pending.is_empty() {
            return Ok(AnalysisRoundResult::default());
        }

        let mut processed = 0usize;
        let mut weak_links_generated = 0usize;

        for entry in &pending {
            // Load the full note
            let note = match crate::storage::load_note_with_context(context, &entry.note_id) {
                Ok(n) => n,
                Err(e) => {
                    warn!(note_id = %entry.note_id, error = %e, "skipping note in Layer 2");
                    let _ = mark_analysis_failed(&connection, &entry.id);
                    continue;
                }
            };

            // Generate weak links with existing notes via FTS5 similarity
            let links = self.find_similar_notes(&connection, &note.meta.id, &note.meta.keywords)?;
            for similar_id in &links {
                if let Err(e) = create_weak_link(
                    &connection,
                    &note.meta.id,
                    similar_id,
                    "content_similarity",
                    0.7,
                ) {
                    warn!(error = %e, "failed to create weak link");
                } else {
                    weak_links_generated += 1;
                }
            }

            // Mark as processed
            mark_analysis_complete(&connection, &entry.id)?;
            processed += 1;
        }

        let elapsed = start.elapsed();
        debug!(
            processed,
            weak_links_generated,
            elapsed_ms = elapsed.as_millis(),
            "Layer 2 analysis round complete"
        );

        Ok(AnalysisRoundResult {
            notes_processed: processed,
            weak_links_generated,
            elapsed_ms: elapsed.as_millis() as u64,
        })
    }

    /// Get the pending analysis queue.
    pub fn list_pending_analyses(context: &StorageContext) -> Result<Vec<PendingAnalysisEntry>> {
        let connection = context.get_connection()?;
        fetch_pending_analyses(&connection, 1000)
    }

    /// Export pending weak links.
    pub fn list_weak_links(
        context: &StorageContext,
        status: Option<WeakLinkStatus>,
    ) -> Result<Vec<WeakLink>> {
        let connection = context.get_connection()?;
        fetch_weak_links(&connection, status)
    }

    /// Confirm a weak link (promote to real association).
    pub fn confirm_weak_link(context: &StorageContext, link_id: &str) -> Result<bool> {
        let connection = context.get_connection()?;
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "UPDATE weak_links SET status = 'confirmed', updated_at = ?2 WHERE id = ?1",
            params![link_id, &now],
        )?;
        Ok(connection.changes() > 0)
    }

    /// Dismiss a weak link.
    pub fn dismiss_weak_link(context: &StorageContext, link_id: &str) -> Result<bool> {
        let connection = context.get_connection()?;
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "UPDATE weak_links SET status = 'dismissed', updated_at = ?2 WHERE id = ?1",
            params![link_id, &now],
        )?;
        Ok(connection.changes() > 0)
    }

    // ──────────────────────────────────────────────────────
    // Internal helpers — Layer 1
    // ──────────────────────────────────────────────────────

    /// Extract keywords using simple term-frequency analysis.
    /// No LLM — just splitting words, lowercasing, and counting.
    fn extract_keywords(&self, body: &str) -> Vec<String> {
        let mut freq: HashMap<String, usize> = HashMap::new();

        // Tokenize: split on non-alphanumeric characters
        for word in body.split(|c: char| !c.is_alphanumeric() && c != '\'') {
            let w = word.trim().to_lowercase();
            if w.len() < 3 || w.len() > 40 {
                continue;
            }
            // Skip common English stop-words
            if is_stop_word(&w) {
                continue;
            }
            *freq.entry(w).or_default() += 1;
        }

        // Sort by frequency descending, take top N
        let mut entries: Vec<KeywordEntry> = freq
            .into_iter()
            .map(|(word, frequency)| KeywordEntry { word, frequency })
            .filter(|e| e.frequency >= MIN_TF)
            .collect();
        entries.sort_by_key(|b| std::cmp::Reverse(b.frequency));
        entries.truncate(MAX_KEYWORDS);

        entries.into_iter().map(|e| e.word).collect()
    }

    /// Find notes with similar content using FTS5.
    fn find_similar_notes(
        &self,
        connection: &Connection,
        note_id: &str,
        keywords: &[String],
    ) -> Result<Vec<String>> {
        // Build an FTS5 query from keywords
        let query_terms: Vec<String> = keywords
            .iter()
            .take(5)
            .map(|k| format!("\"{}\"", k.replace('"', "")))
            .collect();

        if query_terms.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = query_terms.join(" OR ");
        if fts_query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Search note_fts, excluding the source note
        let mut stmt = connection.prepare(
            r#"
            SELECT nf.note_id, rank
            FROM note_fts nf
            WHERE note_fts MATCH ?1
              AND nf.note_id != ?2
            ORDER BY rank
            LIMIT 5
            "#,
        )?;

        let results: Vec<String> = stmt
            .query_map(params![fts_query, note_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })?
            .filter_map(|r| r.ok())
            .filter(|(_, rank): &(String, f64)| *rank >= SIMILARITY_THRESHOLD)
            .map(|(id, _)| id)
            .collect();

        Ok(results)
    }

    /// Suggest collections based on keyword matches against collection rules.
    fn suggest_collections(
        &self,
        connection: &Connection,
        keywords: &[String],
        note_id: &str,
    ) -> Result<Vec<String>> {
        // Check collection rules
        let mut stmt =
            connection.prepare("SELECT name FROM collection_rules WHERE keyword = ?1")?;

        let mut suggestions: Vec<String> = Vec::new();

        // Get collections the note already belongs to
        let existing: Vec<String> = connection
            .prepare(
                "SELECT c.name FROM collections c \
                 INNER JOIN note_collections nc ON nc.collection_id = c.id \
                 WHERE nc.note_id = ?1",
            )?
            .query_map(params![note_id], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        for kw in keywords {
            let matches: Vec<String> = stmt
                .query_map(params![kw], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter(|name| !existing.contains(name))
                .collect();
            for name in matches {
                if !suggestions.contains(&name) {
                    suggestions.push(name);
                }
            }
        }

        // Also try matching by checking if keyword appears in collection names
        let mut col_stmt =
            connection.prepare("SELECT name FROM collections WHERE LOWER(name) = LOWER(?1)")?;
        for kw in keywords {
            let matches: Vec<String> = col_stmt
                .query_map(params![kw], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter(|name| !existing.contains(name) && !suggestions.contains(name))
                .collect();
            suggestions.extend(matches);
        }

        Ok(suggestions)
    }

    /// Enqueue a note for Layer 2 deep analysis.
    fn enqueue_for_deep_analysis(
        &self,
        connection: &Connection,
        note_id: &str,
        action: NoteAction,
    ) -> Result<Option<String>> {
        // Skip if already pending
        let already_pending: bool = connection.query_row(
            "SELECT COUNT(*) FROM analysis_queue WHERE note_id = ?1 AND status = 'pending'",
            params![note_id],
            |row| row.get::<_, i64>(0),
        )? > 0;

        if already_pending {
            return Ok(None);
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let action_str = match action {
            NoteAction::Created => "created",
            NoteAction::Updated => "updated",
            NoteAction::Deleted => "deleted",
        };

        connection.execute(
            "INSERT INTO analysis_queue (id, note_id, action, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'pending', ?4, ?4)",
            params![id, note_id, action_str, now],
        )?;

        Ok(Some(id))
    }

    /// Update note keywords in the database.
    fn update_note_keywords(
        connection: &Connection,
        note_id: &str,
        keywords: &[String],
    ) -> Result<()> {
        let joined = keywords.join(",");
        connection.execute(
            "UPDATE notes SET keywords = ?1, updated_at = ?2 WHERE id = ?3",
            params![joined, Utc::now().to_rfc3339(), note_id],
        )?;
        Ok(())
    }

    /// Map a SQLite row to a NoteMeta.
    fn row_to_note_meta(row: &rusqlite::Row) -> rusqlite::Result<NoteMeta> {
        let keywords_str: String = row.get(3)?;
        let keywords: Vec<String> = if keywords_str.is_empty() {
            Vec::new()
        } else {
            keywords_str
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };
        let tags_str: String = row.get(2)?;
        let tags: Vec<String> = if tags_str.is_empty() {
            Vec::new()
        } else {
            tags_str.split(',').map(|s| s.trim().to_string()).collect()
        };
        Ok(NoteMeta {
            id: row.get(0)?,
            title: row.get(1)?,
            tags,
            keywords,
            platform: row.get(4)?,
            board: row.get(5)?,
            kernel: row.get(6)?,
            status: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
            source: row.get(10)?,
            path: row.get(11)?,
            summary: row.get(12)?,
            collections: Vec::new(),
        })
    }
}

// ════════════════════════════════════════════════
// Layer 2 helpers
// ════════════════════════════════════════════════

#[derive(Debug, Clone, Default)]
pub struct AnalysisRoundResult {
    pub notes_processed: usize,
    pub weak_links_generated: usize,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone)]
pub struct PendingAnalysisEntry {
    pub id: String,
    pub note_id: String,
    pub action: String,
    pub created_at: String,
}

/// Fetch pending analyses from the queue.
fn fetch_pending_analyses(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<PendingAnalysisEntry>> {
    let mut stmt = connection.prepare(
        "SELECT id, note_id, action, created_at FROM analysis_queue \
         WHERE status = 'pending' ORDER BY created_at ASC LIMIT ?1",
    )?;

    let entries = stmt
        .query_map(params![limit as i64], |row| {
            Ok(PendingAnalysisEntry {
                id: row.get(0)?,
                note_id: row.get(1)?,
                action: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(entries)
}

fn mark_analysis_complete(connection: &Connection, id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    connection.execute(
        "UPDATE analysis_queue SET status = 'completed', updated_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    Ok(())
}

fn mark_analysis_failed(connection: &Connection, id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    connection.execute(
        "UPDATE analysis_queue SET status = 'failed', updated_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    Ok(())
}

// ════════════════════════════════════════════════
// Weak links
// ════════════════════════════════════════════════

/// Create a weak link between two notes.  Weak links are pending associations
/// that the user can confirm or dismiss.
fn create_weak_link(
    connection: &Connection,
    source_id: &str,
    target_id: &str,
    link_type: &str,
    score: f64,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    connection.execute(
        "INSERT INTO weak_links (id, source_note_id, target_note_id, link_type, score, status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?6) \
         ON CONFLICT(source_note_id, target_note_id, link_type) DO NOTHING",
        params![id, source_id, target_id, link_type, score, now],
    )?;
    Ok(id)
}

/// Fetch weak links, optionally filtered by status.
fn fetch_weak_links(
    connection: &Connection,
    status: Option<WeakLinkStatus>,
) -> Result<Vec<WeakLink>> {
    let (where_clause, status_str) = match status {
        Some(s) => ("WHERE status = ?1".to_string(), Some(s.as_str())),
        None => ("".to_string(), None),
    };

    let sql = format!(
        "SELECT id, source_note_id, target_note_id, link_type, score, status, created_at, updated_at \
         FROM weak_links {where_clause} ORDER BY score DESC LIMIT 200"
    );

    let mut stmt = connection.prepare(&sql)?;

    let links: Vec<WeakLink> = if let Some(s) = status_str {
        stmt.query_map(params![s], row_to_weak_link)?
    } else {
        stmt.query_map([], row_to_weak_link)?
    }
    .filter_map(|r| r.ok())
    .collect();

    Ok(links)
}

fn row_to_weak_link(row: &rusqlite::Row) -> rusqlite::Result<WeakLink> {
    Ok(WeakLink {
        id: row.get(0)?,
        source_note_id: row.get(1)?,
        target_note_id: row.get(2)?,
        link_type: row.get(3)?,
        score: row.get(4)?,
        status: row.get::<_, String>(5)?.into(),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

// ════════════════════════════════════════════════
// Stop word list
// ════════════════════════════════════════════════

/// Minimal English stop-word list for keyword extraction.
fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "the"
            | "and"
            | "for"
            | "are"
            | "but"
            | "not"
            | "you"
            | "all"
            | "can"
            | "had"
            | "her"
            | "was"
            | "one"
            | "our"
            | "out"
            | "has"
            | "have"
            | "been"
            | "some"
            | "same"
            | "into"
            | "than"
            | "that"
            | "them"
            | "then"
            | "they"
            | "this"
            | "with"
            | "will"
            | "more"
            | "also"
            | "just"
            | "like"
            | "from"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "their"
            | "there"
            | "these"
            | "about"
            | "above"
            | "after"
            | "again"
            | "other"
            | "every"
            | "below"
            | "could"
            | "would"
            | "should"
            | "shall"
            | "might"
            | "must"
            | "still"
            | "such"
            | "very"
            | "your"
            | "both"
            | "each"
            | "does"
            | "most"
            | "only"
            | "over"
            | "much"
            | "many"
            | "here"
            | "even"
            | "well"
            | "were"
            | "being"
            | "done"
            | "made"
            | "make"
            | "said"
            | "may"
            | "way"
            | "long"
            | "take"
            | "come"
            | "came"
            | "know"
            | "new"
            | "use"
            | "used"
            | "using"
            | "get"
            | "gets"
            | "got"
            | "see"
            | "seen"
            | "find"
            | "time"
            | "back"
            | "good"
            | "first"
            | "last"
            | "next"
            | "best"
            | "never"
            | "always"
            | "often"
            | "ever"
            | "yet"
            | "upon"
            | "thus"
            | "hence"
            | "thence"
    )
}

// ════════════════════════════════════════════════
// Direct CLI entry points
// ════════════════════════════════════════════════

/// Run Layer 1 analysis on all notes that need it (from CLI --auto).
pub fn run_auto_organize(context: &StorageContext) -> Result<AutoOrganizeSummary> {
    let organizer = AutoOrganizer::default();
    let connection = context.get_connection()?;

    // Find notes with keywords that haven't been auto-analysed yet
    let mut stmt = connection.prepare(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, \
         created_at, updated_at, source, path, summary FROM notes \
         WHERE keywords = '' OR keywords IS NULL LIMIT 50",
    )?;

    let note_metas: Vec<NoteMeta> = stmt
        .query_map([], AutoOrganizer::row_to_note_meta)?
        .filter_map(|r| r.ok())
        .collect();

    let mut total_processed = 0usize;
    let mut total_duplicates = 0usize;
    let mut total_suggestions = 0usize;

    for meta in &note_metas {
        let doc = crate::storage::load_note_body_from_meta(meta)?;

        let result = organizer.process_new_note(context, meta, &doc.body, NoteAction::Updated)?;
        total_processed += 1;
        total_duplicates += result.duplicate_note_ids.len();
        total_suggestions += result.suggested_collections.len();
    }

    let mut ran_l2 = false;
    let mut l2_result = AnalysisRoundResult::default();
    if total_processed == 0 {
        // No unprocessed notes — run Layer 2 instead
        l2_result = organizer.run_analysis_round(context)?;
        ran_l2 = true;
    }

    Ok(AutoOrganizeSummary {
        notes_analyzed_layer1: total_processed,
        duplicates_found: total_duplicates,
        collections_suggested: total_suggestions,
        layer2_notes_processed: if ran_l2 { l2_result.notes_processed } else { 0 },
        weak_links_generated: if ran_l2 {
            l2_result.weak_links_generated
        } else {
            0
        },
    })
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AutoOrganizeSummary {
    pub notes_analyzed_layer1: usize,
    pub duplicates_found: usize,
    pub collections_suggested: usize,
    pub layer2_notes_processed: usize,
    pub weak_links_generated: usize,
}

// ════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageContext;
    use chrono::Utc;

    fn setup_test_context() -> (std::path::PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-auto-organize-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        // Initialize storage which creates tables
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");
        (temp, ctx)
    }

    fn make_note_meta(id: &str, keywords: &[String]) -> NoteMeta {
        NoteMeta {
            id: id.to_string(),
            keywords: keywords.to_vec(),
            ..NoteMeta::default()
        }
    }

    #[test]
    fn extract_keywords_returns_top_terms() {
        let organizer = AutoOrganizer::default();
        let body =
            "Rust is a systems programming language. Rust focuses on safety and performance. \
                     Rust has zero-cost abstractions. Systems programming with Rust is fun.";
        let kws = organizer.extract_keywords(body);
        // "rust" appears 4 times, "systems" 2 times, "programming" 2 times
        assert!(
            kws.contains(&"rust".to_string()),
            "rust should be a keyword"
        );
        assert!(
            kws.contains(&"systems".to_string()),
            "systems should be a keyword"
        );
        assert!(
            kws.contains(&"programming".to_string()),
            "programming should be a keyword"
        );
        // "zero-cost", "abstractions", "safety", "performance" appear once each — below MIN_TF
        assert!(
            !kws.contains(&"safety".to_string()),
            "safety appears once, should not be keyword"
        );
    }

    #[test]
    fn extract_keywords_handles_empty_body() {
        let organizer = AutoOrganizer::default();
        assert!(organizer.extract_keywords("").is_empty());
    }

    #[test]
    fn extract_keywords_skips_stop_words() {
        let organizer = AutoOrganizer::default();
        let body = "the and for are but not you all can had her was one our out has have been some same into than that them then they this with will more".to_string();
        let kws = organizer.extract_keywords(&body);
        // All stop words, so no keywords should be returned
        assert!(kws.is_empty());
    }

    #[test]
    fn layer1_analysis_inserts_analysis_queue() {
        let (_tmp, ctx) = setup_test_context();
        let organizer = AutoOrganizer::default();

        let meta = make_note_meta("test-l1-001", &[]);
        let body = "Kernel panic during boot. The system crashed with a kernel panic. \
                     Investigating the boot sequence for kernel issues.";

        let result = organizer
            .process_new_note(&ctx, &meta, body, NoteAction::Created)
            .expect("Layer 1 should succeed");

        assert!(!result.keywords.is_empty(), "should have keywords");
        assert!(
            result.pending_analysis_id.is_some(),
            "should have enqueued for L2"
        );

        // Verify the queue entry
        let conn = ctx.get_connection().expect("connection");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM analysis_queue WHERE note_id = 'test-l1-001'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(count, 1, "analysis_queue should have one entry");
    }

    #[test]
    fn weak_links_create_and_fetch() {
        let (_tmp, ctx) = setup_test_context();
        let conn = ctx.get_connection().expect("connection");

        let id = create_weak_link(&conn, "note-a", "note-b", "content_similarity", 0.85)
            .expect("create weak link");
        assert!(!id.is_empty());

        let links = fetch_weak_links(&conn, None).expect("fetch");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].source_note_id, "note-a");
        assert_eq!(links[0].target_note_id, "note-b");
        assert_eq!(links[0].link_type, "content_similarity");
        assert!((links[0].score - 0.85).abs() < 0.001);
        assert_eq!(links[0].status.as_str(), "pending");

        // Confirm it
        let confirmed = AutoOrganizer::confirm_weak_link(&ctx, &id).expect("confirm");
        assert!(confirmed);
        let pending =
            fetch_weak_links(&conn, Some(WeakLinkStatus::Pending)).expect("fetch pending");
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn weak_links_confirm_updates_timestamps() {
        let (_tmp, ctx) = setup_test_context();
        let conn = ctx.get_connection().expect("connection");

        let id = create_weak_link(&conn, "note-a", "note-b", "content_similarity", 0.85)
            .expect("create weak link");

        // Small delay so created_at and updated_at differ
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Confirm the link
        AutoOrganizer::confirm_weak_link(&ctx, &id).expect("confirm");

        let links = fetch_weak_links(&conn, Some(WeakLinkStatus::Confirmed)).expect("fetch");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].status.as_str(), "confirmed");
        assert!(
            links[0].updated_at > links[0].created_at,
            "updated_at should be > created_at after confirm; got created_at={}, updated_at={}",
            links[0].created_at,
            links[0].updated_at
        );

        // Wait again and dismiss
        std::thread::sleep(std::time::Duration::from_millis(50));
        AutoOrganizer::dismiss_weak_link(&ctx, &id).expect("dismiss");
        let links = fetch_weak_links(&conn, Some(WeakLinkStatus::Dismissed)).expect("fetch");
        assert_eq!(links.len(), 1);
        assert!(links[0].updated_at > links[0].created_at);
    }

    #[test]
    fn weak_links_no_duplicate() {
        let (_tmp, ctx) = setup_test_context();
        let conn = ctx.get_connection().expect("connection");

        create_weak_link(&conn, "note-a", "note-b", "content_similarity", 0.85).expect("first");
        create_weak_link(&conn, "note-a", "note-b", "content_similarity", 0.85).expect("second");

        let links = fetch_weak_links(&conn, None).expect("fetch");
        assert_eq!(links.len(), 1, "should not create duplicate weak links");
    }

    #[test]
    fn run_analysis_round_with_pending_entries() {
        let (_tmp, ctx) = setup_test_context();
        let conn = ctx.get_connection().expect("connection");

        // Insert a note into the database so load_note_with_context works
        let meta = make_note_meta("round-test-001", &[]);
        let body = "This is a test note about kernel debugging. \
                     Kernel debugging requires special tools for kernel analysis.";
        let doc = crate::models::NoteDocument {
            meta: meta.clone(),
            body: body.to_string(),
            search_snippet: None,
        };
        crate::storage::notes::save_note_with_context(&ctx, doc).expect("save note");

        // Enqueue for analysis
        let organizer = AutoOrganizer::default();
        organizer
            .enqueue_for_deep_analysis(&conn, "round-test-001", NoteAction::Created)
            .expect("enqueue");

        let result = organizer.run_analysis_round(&ctx).expect("analysis round");
        assert_eq!(result.notes_processed, 1, "should process one note");
    }

    #[test]
    fn update_note_keywords_updates_updated_at() {
        let (_tmp, ctx) = setup_test_context();
        let conn = ctx.get_connection().expect("connection");

        // Insert a note with empty keywords
        let meta = make_note_meta("kw-test-001", &[]);
        let body = "test body for keyword update";
        let doc = crate::models::NoteDocument {
            meta: meta.clone(),
            body: body.to_string(),
            search_snippet: None,
        };
        crate::storage::notes::save_note_with_context(&ctx, doc).expect("save note");

        // Grab the initial updated_at
        let initial: String = conn
            .query_row(
                "SELECT updated_at FROM notes WHERE id = 'kw-test-001'",
                [],
                |row| row.get(0),
            )
            .expect("read initial updated_at");

        // Small delay so timestamps differ
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Call update_note_keywords
        let new_keywords = vec!["kernel".to_string(), "debugging".to_string()];
        AutoOrganizer::update_note_keywords(&conn, "kw-test-001", &new_keywords)
            .expect("update keywords");

        // Read back updated_at
        let updated: String = conn
            .query_row(
                "SELECT updated_at FROM notes WHERE id = 'kw-test-001'",
                [],
                |row| row.get(0),
            )
            .expect("read updated updated_at");

        assert_ne!(
            initial, updated,
            "updated_at should change after update_note_keywords; initial={}, updated={}",
            initial, updated
        );

        // Also verify the keywords column was actually set
        let stored_keywords: String = conn
            .query_row(
                "SELECT keywords FROM notes WHERE id = 'kw-test-001'",
                [],
                |row| row.get(0),
            )
            .expect("read keywords");
        assert_eq!(stored_keywords, "kernel,debugging");
    }
}
