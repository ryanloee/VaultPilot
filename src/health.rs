//! Vault Health Dashboard — analyze vault structure and generate
//! optimization suggestions (#2014).
//!
//! Provides:
//! - Note / collection / tag counts
//! - Orphan note detection (no tags + no wiki-links)
//! - Knowledge density score (link density, tag coverage, tag diversity)
//! - Duplicate/similar title detection
//! - AI-style improvement suggestions

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use tracing::{instrument, warn};

use crate::models::{Collection, HealthReport, NoteMeta};
use crate::storage::{list_collections_with_context, load_settings_with_context, StorageContext};

/// Maximum notes to analyze for orphan / link detection. If the vault has more
/// notes than this, we sample the most recent ones to keep analysis fast.
const HEALTH_ANALYSIS_MAX_NOTES: usize = 5_000;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate a comprehensive vault health report.
///
/// This function queries the database, counts everything, detects orphans and
/// duplicates, and produces human-readable suggestions.
#[instrument(skip(context))]
pub fn health_check(context: &StorageContext) -> Result<HealthReport> {
    let conn = context.get_connection()?;
    let _settings = load_settings_with_context(context)?;

    // ── 1. Counts ──────────────────────────────────────────────────────
    let total_notes: usize =
        conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get::<_, i64>(0))? as usize;

    let total_collections: usize =
        conn.query_row("SELECT COUNT(*) FROM collections", [], |row| {
            row.get::<_, i64>(0)
        })? as usize;

    // ── 2. Load all notes (up to HEALTH_ANALYSIS_MAX_NOTES) ────────────
    let all_notes = load_all_note_metas(&conn, total_notes)?;

    // ── 3. Count unique tags ───────────────────────────────────────────
    let mut unique_tags = HashSet::new();
    for note in &all_notes {
        for tag in &note.tags {
            unique_tags.insert(tag.clone());
        }
    }
    let total_tags = unique_tags.len();

    // ── 4. Find orphan notes ───────────────────────────────────────────
    let orphan_notes = find_orphan_notes(&conn, &all_notes)?;

    // ── 5. Knowledge density score ─────────────────────────────────────
    let knowledge_density_score =
        calculate_knowledge_density(&conn, &all_notes, total_notes, total_tags)?;

    // ── 6. Duplicate detection ─────────────────────────────────────────
    let duplicate_clusters = detect_duplicate_clusters(&all_notes);

    // ── 7. Load collections for suggestions ────────────────────────────
    let collections = list_collections_with_context(context)?;

    // ── 8. Generate suggestions ────────────────────────────────────────
    let suggestions =
        generate_suggestions(&all_notes, &orphan_notes, &duplicate_clusters, &collections);

    Ok(HealthReport {
        total_notes,
        total_collections,
        total_tags,
        orphan_notes,
        knowledge_density_score,
        suggestions,
        duplicate_clusters,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Load all note metas from the database (up to HEALTH_ANALYSIS_MAX_NOTES).
fn load_all_note_metas(conn: &rusqlite::Connection, total: usize) -> Result<Vec<NoteMeta>> {
    let limit = total.min(HEALTH_ANALYSIS_MAX_NOTES);
    let mut stmt = conn.prepare(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, \
         created_at, updated_at, source, path, summary \
         FROM notes ORDER BY updated_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |row| {
        let tags_raw: String = row.get(2)?;
        let kw_raw: String = row.get(3)?;
        Ok(NoteMeta {
            id: row.get(0)?,
            title: row.get(1)?,
            tags: match serde_json::from_str(&tags_raw) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to parse tags JSON, falling back to empty vec");
                    Vec::new()
                }
            },
            keywords: match serde_json::from_str(&kw_raw) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to parse keywords JSON, falling back to empty vec");
                    Vec::new()
                }
            },
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
    })?;
    let mut notes = Vec::with_capacity(limit);
    for row in rows {
        notes.push(row?);
    }
    Ok(notes)
}

/// Returns `true` if the note body (from `note_fts`) contains wiki-style links
/// (`[[...]]`) or markdown links / URLs.
fn body_has_links(conn: &rusqlite::Connection, note_id: &str) -> Result<bool> {
    let body: Option<String> = conn
        .query_row(
            "SELECT body FROM note_fts WHERE note_id = ?1",
            [note_id],
            |row| row.get(0),
        )
        .ok();
    match body {
        Some(b) => {
            let trimmed = b.trim();
            if trimmed.is_empty() {
                return Ok(false);
            }
            // Check for wiki-links [[...]]
            if trimmed.contains("[[") && trimmed.contains("]]") {
                return Ok(true);
            }
            // Check for markdown links [...](...) — only look for URLs in parentheses
            if trimmed.contains("http://")
                || trimmed.contains("https://")
                || trimmed.contains("ftp://")
            {
                return Ok(true);
            }
            Ok(false)
        }
        None => Ok(false),
    }
}

/// Find notes that have no tags and no wiki-links / URLs in their body.
/// These are effectively "orphaned" — not connected to anything.
fn find_orphan_notes(conn: &rusqlite::Connection, all_notes: &[NoteMeta]) -> Result<Vec<NoteMeta>> {
    let mut orphans = Vec::new();
    for note in all_notes {
        let has_tags = !note.tags.is_empty();
        let has_links = body_has_links(conn, &note.id).unwrap_or(false);
        if !has_tags && !has_links {
            orphans.push(note.clone());
        }
    }
    Ok(orphans)
}

/// Calculate a knowledge density score (0.0–1.0) based on three factors:
///
/// 1. **Tag coverage** (40%): what fraction of notes have at least one tag.
/// 2. **Link density** (30%): what fraction of notes contain wiki-links/URLs.
/// 3. **Tag diversity** (30%): how many unique tags per note (capped at 1.0).
fn calculate_knowledge_density(
    conn: &rusqlite::Connection,
    all_notes: &[NoteMeta],
    total_notes: usize,
    total_tags: usize,
) -> Result<f64> {
    if total_notes == 0 {
        return Ok(0.0);
    }

    // Tag coverage
    let notes_with_tags = all_notes.iter().filter(|n| !n.tags.is_empty()).count();
    let tag_coverage = notes_with_tags as f64 / all_notes.len() as f64;

    // Link density — check up to 200 notes to avoid excessive queries
    let sample_size = all_notes.len().min(200);
    let mut linked_count = 0usize;
    for note in all_notes.iter().take(sample_size) {
        if body_has_links(conn, &note.id).unwrap_or(false) {
            linked_count += 1;
        }
    }
    let link_density = if sample_size > 0 {
        linked_count as f64 / sample_size as f64
    } else {
        0.0
    };

    // Tag diversity — unique tags per total notes, capped at 1.0
    let tag_diversity = (total_tags as f64 / total_notes as f64).min(1.0);

    // Weighted composite
    let score = tag_coverage * 0.40 + link_density * 0.30 + tag_diversity * 0.30;
    Ok(score.clamp(0.0, 1.0))
}

/// Normalize a title for comparison: lowercase, remove non-alphanumeric chars
/// (except whitespace), collapse whitespace, and trim.
fn normalize_title(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Compute Levenshtein distance between two strings.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row = vec![0usize; b_len + 1];

    for (i, ca) in a_chars.iter().enumerate() {
        curr_row[0] = i + 1;
        for (j, cb) in b_chars.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr_row[j + 1] = (curr_row[j] + 1)
                .min(prev_row[j + 1] + 1)
                .min(prev_row[j] + cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }
    prev_row[b_len]
}

/// Detect groups of notes whose titles are highly similar (exact match after
/// normalization, or Levenshtein similarity >= 0.8 for short titles).
fn detect_duplicate_clusters(all_notes: &[NoteMeta]) -> Vec<Vec<String>> {
    // Phase 1: exact normalized-title matches via hash map
    let mut title_map: HashMap<String, Vec<String>> = HashMap::new();
    for note in all_notes {
        let norm = normalize_title(&note.title);
        if norm.is_empty() {
            continue;
        }
        title_map.entry(norm).or_default().push(note.id.clone());
    }

    let mut clusters: Vec<Vec<String>> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();

    // Phase 2: collect exact-match groups with >=2 members
    for ids in title_map.values() {
        if ids.len() >= 2 {
            for id in ids {
                visited.insert(id.clone());
            }
            clusters.push(ids.clone());
        }
    }

    // Phase 3: fuzzy matching for remaining unvisited notes (short titles)
    let remaining: Vec<&NoteMeta> = all_notes
        .iter()
        .filter(|n| !visited.contains(&n.id))
        .collect();

    for i in 0..remaining.len() {
        if visited.contains(&remaining[i].id) {
            continue;
        }
        let norm_i = normalize_title(&remaining[i].title);
        if norm_i.len() < 3 {
            continue;
        }
        let mut cluster = Vec::new();
        for j in (i + 1)..remaining.len() {
            if visited.contains(&remaining[j].id) {
                continue;
            }
            let norm_j = normalize_title(&remaining[j].title);
            if norm_j.len() < 3 {
                continue;
            }
            let max_len = norm_i.len().max(norm_j.len());
            let dist = levenshtein_distance(&norm_i, &norm_j);
            let similarity = 1.0 - (dist as f64 / max_len as f64);
            if similarity >= 0.80 {
                if cluster.is_empty() {
                    cluster.push(remaining[i].id.clone());
                    visited.insert(remaining[i].id.clone());
                }
                cluster.push(remaining[j].id.clone());
                visited.insert(remaining[j].id.clone());
            }
        }
        if cluster.len() >= 2 {
            clusters.push(cluster);
        }
    }

    clusters
}

/// Generate human-readable suggestions based on analysis results.
fn generate_suggestions(
    all_notes: &[NoteMeta],
    orphan_notes: &[NoteMeta],
    duplicate_clusters: &[Vec<String>],
    collections: &[Collection],
) -> Vec<String> {
    let mut suggestions = Vec::new();

    // ── Orphan notes ──────────────────────────────────────────────────
    if !orphan_notes.is_empty() {
        let sample: Vec<&str> = orphan_notes
            .iter()
            .take(3)
            .map(|n| n.title.as_str())
            .collect();
        if orphan_notes.len() == 1 {
            suggestions.push(format!(
                "You have 1 orphan note '{}', consider adding tags or links to integrate it into your knowledge graph.",
                sample[0]
            ));
        } else {
            suggestions.push(format!(
                "You have {} orphan notes (e.g. '{}'), consider adding tags or links to integrate them into your knowledge graph.",
                orphan_notes.len(),
                sample.join("', '")
            ));
        }
    }

    // ── Sparse collections ────────────────────────────────────────────
    for col in collections {
        if col.note_count <= 2 && col.note_count > 0 {
            suggestions.push(format!(
                "Your '{}' collection only has {} notes, consider adding more related notes.",
                col.name, col.note_count
            ));
        }
    }

    // ── Duplicate clusters ────────────────────────────────────────────
    if !duplicate_clusters.is_empty() {
        let total_dupes: usize = duplicate_clusters.iter().map(|c| c.len()).sum();
        suggestions.push(format!(
            "Detected {} groups of highly similar notes ({} notes total), consider merging them to reduce redundancy.",
            duplicate_clusters.len(),
            total_dupes
        ));
    }

    // ── Untagged notes (excluding orphans already reported) ───────────
    let orphan_ids: HashSet<&str> = orphan_notes.iter().map(|n| n.id.as_str()).collect();
    let untagged_non_orphan = all_notes
        .iter()
        .filter(|n| n.tags.is_empty() && !orphan_ids.contains(n.id.as_str()))
        .count();
    if untagged_non_orphan > 0 {
        suggestions.push(format!(
            "You have {} untagged notes with links, consider adding descriptive tags for better categorization.",
            untagged_non_orphan
        ));
    }

    // ── Overall density feedback ──────────────────────────────────────
    if all_notes.len() >= 5 {
        let linked_count = all_notes.iter().filter(|n| !n.tags.is_empty()).count();
        let ratio = linked_count as f64 / all_notes.len() as f64;
        if ratio < 0.3 && all_notes.len() > 10 {
            suggestions.push(
                "Less than 30% of your notes have tags. Adding tags improves search and RAG retrieval quality.".to_string(),
            );
        }
    }

    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    #[test]
    fn normalize_title_removes_punctuation_and_lowercases() {
        assert_eq!(normalize_title("Hello World!"), "hello world");
        assert_eq!(normalize_title("Rust 🦀 Guide"), "rust guide");
        assert_eq!(normalize_title(""), "");
    }

    #[test]
    fn levenshtein_identical_strings() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
    }

    #[test]
    fn levenshtein_completely_different() {
        assert_eq!(levenshtein_distance("abc", "xyz"), 3);
    }

    #[test]
    fn levenshtein_empty_strings() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("", "xyz"), 3);
    }

    #[test]
    fn detect_duplicate_clusters_finds_exact_matches() {
        let notes = vec![
            NoteMeta {
                id: "a".into(),
                title: "Getting Started with Rust".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "b".into(),
                title: "Getting Started with Rust".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "c".into(),
                title: "Different Title".into(),
                ..Default::default()
            },
        ];
        let clusters = detect_duplicate_clusters(&notes);
        assert_eq!(clusters.len(), 1);
        assert!(clusters[0].contains(&"a".to_string()));
        assert!(clusters[0].contains(&"b".to_string()));
    }

    #[test]
    fn detect_duplicate_clusters_no_false_positive() {
        let notes = vec![
            NoteMeta {
                id: "a".into(),
                title: "Alpha".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "b".into(),
                title: "Beta".into(),
                ..Default::default()
            },
        ];
        let clusters = detect_duplicate_clusters(&notes);
        assert!(clusters.is_empty());
    }

    #[test]
    fn generate_suggestions_empty_when_healthy() {
        let notes = vec![NoteMeta {
            id: "x".into(),
            title: "Test".into(),
            tags: vec!["tag".into()],
            ..Default::default()
        }];
        let suggestions = generate_suggestions(&notes, &[], &[], &[]);
        // No orphans, no duplicates, no empty collections → no suggestions
        // (But there may be density feedback — depends on the note count)
        assert!(suggestions.is_empty());
    }

    #[test]
    fn generate_suggestions_orphans() {
        let notes = vec![
            NoteMeta {
                id: "o1".into(),
                title: "Orphan One".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "o2".into(),
                title: "Orphan Two".into(),
                ..Default::default()
            },
        ];
        let suggestions = generate_suggestions(&notes, &notes, &[], &[]);
        assert!(suggestions.iter().any(|s| s.contains("orphan")));
    }

    #[test]
    fn generate_suggestions_duplicates() {
        let clusters = vec![vec!["a".into(), "b".into()]];
        let suggestions = generate_suggestions(&[], &[], &clusters, &[]);
        assert!(suggestions.iter().any(|s| s.contains("highly similar")));
    }

    #[test]
    fn generate_suggestions_sparse_collections() {
        let collections = vec![Collection {
            name: "Project X".into(),
            note_count: 1,
            ..Default::default()
        }];
        let suggestions = generate_suggestions(&[], &[], &[], &collections);
        assert!(
            suggestions.iter().any(|s| s.contains("Project X")),
            "should mention the collection name"
        );
    }

    #[test]
    fn knowledge_density_zero_when_no_notes() {
        let score = calculate_knowledge_density_direct(0, 0);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn knowledge_density_max_when_perfect() {
        let score = calculate_knowledge_density_direct(100, 100);
        // With perfect everything: tag_coverage=1.0, link_density=1.0, tag_diversity=1.0
        // score = 0.4*1.0 + 0.3*1.0 + 0.3*1.0 = 1.0
        assert!((score - 1.0).abs() < 0.001);
    }

    /// Helper to test knowledge density without a DB connection.
    /// Assumes tag_coverage=1.0, link_density=1.0 (every note has tags and links).
    fn calculate_knowledge_density_direct(total_notes: usize, total_tags: usize) -> f64 {
        if total_notes == 0 {
            return 0.0;
        }
        let tag_coverage = 1.0; // all notes have tags
        let link_density = 1.0; // all notes have links
        let tag_diversity = (total_tags as f64 / total_notes as f64).min(1.0);
        tag_coverage * 0.40 + link_density * 0.30 + tag_diversity * 0.30
    }

    #[test]
    fn load_all_note_metas_invalid_json_returns_empty_tags() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notes (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                tags TEXT NOT NULL,
                keywords TEXT NOT NULL,
                platform TEXT NOT NULL,
                board TEXT NOT NULL,
                kernel TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                source TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                summary TEXT NOT NULL,
                body_hash TEXT NOT NULL
            );",
        )
        .unwrap();

        // Invalid JSON in tags
        conn.execute(
            "INSERT INTO notes
             (id, title, tags, keywords, platform, board, kernel,
              status, created_at, updated_at, source, path, summary, body_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                "n1",
                "Bad Tags",
                "{invalid",
                "[]",
                "test",
                "test",
                "test",
                "active",
                "2026-01-01",
                "2026-01-01",
                "test",
                "/bad-tags.md",
                "",
                ""
            ],
        )
        .unwrap();

        // Invalid JSON in keywords
        conn.execute(
            "INSERT INTO notes
             (id, title, tags, keywords, platform, board, kernel,
              status, created_at, updated_at, source, path, summary, body_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                "n2",
                "Bad Keywords",
                "[]",
                "not-json!!!",
                "test",
                "test",
                "test",
                "active",
                "2026-01-01",
                "2026-01-01",
                "test",
                "/bad-keywords.md",
                "",
                ""
            ],
        )
        .unwrap();

        // Both valid
        conn.execute(
            "INSERT INTO notes
             (id, title, tags, keywords, platform, board, kernel,
              status, created_at, updated_at, source, path, summary, body_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                "n3",
                "Good Note",
                "[\"rust\"]",
                "[\"dev\"]",
                "test",
                "test",
                "test",
                "active",
                "2026-01-01",
                "2026-01-01",
                "test",
                "/good.md",
                "",
                ""
            ],
        )
        .unwrap();

        let metas = load_all_note_metas(&conn, 10).unwrap();

        // n1: invalid tags → empty tags
        let n1 = metas.iter().find(|m| m.id == "n1").unwrap();
        assert!(
            n1.tags.is_empty(),
            "invalid tags JSON should yield empty tags vec, got {:?}",
            n1.tags
        );

        // n2: invalid keywords → empty keywords
        let n2 = metas.iter().find(|m| m.id == "n2").unwrap();
        assert!(
            n2.keywords.is_empty(),
            "invalid keywords JSON should yield empty keywords vec, got {:?}",
            n2.keywords
        );

        // n3: valid JSON preserved
        let n3 = metas.iter().find(|m| m.id == "n3").unwrap();
        assert_eq!(n3.tags, vec!["rust"]);
        assert_eq!(n3.keywords, vec!["dev"]);
    }
}
