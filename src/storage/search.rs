use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use tracing::{debug, instrument, warn};

use crate::models::{NoteDocument, NoteMeta, SearchQuery, SearchResult};
use chrono::{DateTime, NaiveDate, Utc};

use super::pool::open_connection;
use super::StorageContext;

use super::notes::extract_image_text;
use super::{compute_image_perceptual_hash, load_note_body_from_meta};

#[derive(Debug, Clone)]
struct AttachmentEntry {
    note_id: String,
    path: String,
    file_name: String,
    stem: String,
    ocr_text: String,
    #[allow(dead_code)] // Used in row_to_attachment_entry but not read after construction
    semantic_vector: Option<Vec<f32>>,
    #[allow(dead_code)] // Used in row_to_attachment_entry but not read after construction
    perceptual_hash: Option<u64>,
}

const ATTACHMENT_VECTOR_DIM: usize = 192;
#[instrument(skip(context))]
pub fn search_notes_with_context(
    context: &StorageContext,
    query: SearchQuery,
) -> Result<SearchResult> {
    let (connection, _) = open_connection(context)?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);
    let has_filters = !query.tags.is_empty()
        || !query.keywords.is_empty()
        || query.created_after.is_some()
        || query.created_before.is_some()
        || query.modified_after.is_some()
        || query.modified_before.is_some();
    debug!(text = %query.text, limit = limit, offset = offset, has_filters = has_filters, "searching notes");

    let (notes, total) = if query.text.trim().is_empty() {
        if has_filters {
            // SQL-level filtering when text is empty but filters are specified.
            // This avoids the bug where query_recent_note_metas returns only
            // the N most recent notes and post-filtering silently drops results.
            let filtered = query_filtered_note_metas(&connection, &query, limit, offset)?;
            let count = count_filtered_notes(&connection, &query)?;
            (filtered, count)
        } else {
            let recent = query_recent_note_metas(&connection, limit, offset)?;
            let count = count_all_notes(&connection)?;
            (recent, count)
        }
    } else {
        // When filters are active, over-fetch to compensate for post-filtering attrition.
        let fetch_limit = if has_filters {
            // Fetch more candidates so that after in-memory filtering we still
            // have enough results to fill the requested page.
            (limit + offset).saturating_mul(4).max(50)
        } else {
            limit + offset
        };
        let fts_results = rank_note_metas(context, &connection, &query.text, &[], fetch_limit)?;
        let (notes, used_like_fallback) = if fts_results.is_empty() {
            // Fuzzy/approximate fallback: split query into words and use LIKE
            let like_results = query_like_note_metas(&connection, &query.text, fetch_limit)?;
            if like_results.is_empty() {
                // #1903: zero-index instant search fallback. When both FTS5 and
                // the LIKE fallback return nothing (e.g. the index has not been
                // built yet, or the query terms simply are not in the indexed
                // corpus), scan the vault directory directly and match against
                // the raw note files. The instant search owns filtering,
                // ranking, and pagination, so it returns a complete result and
                // we short-circuit the rest of this function.
                return super::instant_search::instant_search_notes_with_context(context, query);
            }
            (like_results, true)
        } else {
            (fts_results, false)
        };
        // NOTE: offset is NOT applied here — it must be applied AFTER in-memory
        // filtering so that page boundaries are correct for filtered results.
        //
        // Use FTS COUNT(*) for accurate pagination total when no in-memory filters
        // are active. With filters, use the post-filtering count as an upper bound
        // (some matches may be filtered out, making this an overcount).
        let fts_total = if !notes.is_empty() {
            if used_like_fallback {
                // FTS COUNT(*) would return 0 for LIKE-only matches;
                // use a dedicated LIKE COUNT for accurate pagination.
                count_like_matches(&connection, &query.text).unwrap_or(notes.len())
            } else {
                count_fts_matches(&connection, &query.text).unwrap_or(notes.len())
            }
        } else {
            0
        };
        let initial_total = fts_total;
        (notes, initial_total)
    };

    // In-memory filtering (for FTS path where SQL filtering isn't applied)
    let mut notes = notes;
    if !query.tags.is_empty() && !query.text.trim().is_empty() {
        notes.retain(|note| has_all_terms(&note.tags, &query.tags));
    }
    if !query.keywords.is_empty() && !query.text.trim().is_empty() {
        notes.retain(|note| has_all_terms(&note.keywords, &query.keywords));
    }

    // Date range filtering (for FTS path where SQL filtering isn't applied)
    if !query.text.trim().is_empty() {
        notes = filter_by_date_range(
            notes,
            query.created_after.as_deref(),
            query.created_before.as_deref(),
            query.modified_after.as_deref(),
            query.modified_before.as_deref(),
        );
    }

    // For SQL paths, total was computed via COUNT(*) above.
    // For FTS path with filters: use a combined FTS+filter COUNT(*) for accuracy.
    // For FTS path without filters: total = FTS COUNT(*) (accurate).
    let total = if query.text.trim().is_empty() {
        total
    } else if has_filters {
        // Combined FTS + filter count gives the exact total (#2089).
        match count_fts_matches_with_filters(&connection, &query) {
            Ok(accurate_total) => accurate_total.max(notes.len()),
            Err(_) => notes.len(), // fallback to lower bound on error
        }
    } else {
        // total was set to fts_total (FTS COUNT(*)) above; use it directly.
        // If the FTS count underestimates due to LIKE fallback, use max.
        total.max(notes.len())
    };

    // For FTS path: apply offset AFTER in-memory filtering so page boundaries
    // are correct. For non-FTS paths, offset was already applied in SQL.
    if !query.text.trim().is_empty() {
        let effective_offset = offset.min(notes.len());
        notes = notes
            .into_iter()
            .skip(effective_offset)
            .take(limit)
            .collect();
    } else {
        notes.truncate(limit);
    }

    Ok(SearchResult { notes, total })
}

/// Instant title-only search for typeahead (<100ms).
/// Uses SQLite LIKE on the title column — no FTS overhead.
pub fn typeahead_search(
    context: &StorageContext,
    query: &str,
    limit: usize,
) -> Result<Vec<NoteMeta>> {
    let (connection, _) = open_connection(context)?;
    let limit = limit.clamp(1, 50);
    let escaped = escape_like_pattern(&query.to_lowercase());
    let pattern = format!("%{}%", escaped);

    let mut statement = connection.prepare(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary
         FROM notes
         WHERE LOWER(title) LIKE ?1 ESCAPE '\\'
         ORDER BY updated_at DESC
         LIMIT ?2",
    )?;
    let rows = statement
        .query_map(params![pattern, limit as i64], row_to_meta)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Full deep search that includes semantic/vector scoring.
/// Meant to be called after the initial FTS5 keyword search to find
/// additional semantically-related results.
pub fn deep_search_notes(context: &StorageContext, query: SearchQuery) -> Result<SearchResult> {
    let (connection, _) = open_connection(context)?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);

    if query.text.trim().is_empty() {
        let notes = query_recent_note_metas(&connection, limit, offset)?;
        let total = count_all_notes(&connection)?;
        return Ok(SearchResult { notes, total });
    }

    // Step 1: FTS5 keyword search (same as fast path)
    let fetch_limit = limit + offset;
    let fts_results = rank_note_metas(context, &connection, &query.text, &[], fetch_limit)?;

    // Step 2: Semantic/vector search — compute semantic vectors and rank
    let semantic_scores = query_attachment_semantic_scores(&connection, &query.text)?;
    let mut scored_ids: Vec<(String, i64)> = semantic_scores.into_iter().collect();
    scored_ids.sort_by_key(|b| std::cmp::Reverse(b.1)); // highest score first

    // Step 3: Build combined result set from FTS results + semantically scored notes
    let mut seen_ids: HashSet<String> = fts_results.iter().map(|n| n.id.clone()).collect();
    let mut combined = fts_results;

    for (note_id, _score) in scored_ids {
        if seen_ids.contains(&note_id) {
            continue;
        }
        if combined.len() >= fetch_limit {
            break;
        }
        // Load meta for semantically-matched note
        if let Some(meta) = load_note_meta_by_id(&connection, &note_id)? {
            combined.push(meta);
            seen_ids.insert(note_id);
        }
    }

    // Step 4: Apply offset/limit
    let combined_len = combined.len();
    let effective_offset = offset.min(combined_len);
    let notes = combined
        .into_iter()
        .skip(effective_offset)
        .take(limit)
        .collect::<Vec<_>>();
    let total = combined_len;

    Ok(SearchResult { notes, total })
}

pub(super) fn build_attachment_semantic_text(
    file_name: &str,
    stem: &str,
    ocr_text: &str,
) -> String {
    [file_name.trim(), stem.trim(), ocr_text.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn build_text_semantic_vector(text: &str) -> Option<Vec<f32>> {
    let mut vector = vec![0.0_f32; ATTACHMENT_VECTOR_DIM];
    let terms = extract_search_terms(text);
    if terms.is_empty() {
        return None;
    }

    for term in terms {
        let hash = stable_term_hash(&term);
        let index = (hash as usize) % ATTACHMENT_VECTOR_DIM;
        let sign = if (hash >> 63) == 0 { 1.0_f32 } else { -1.0_f32 };
        vector[index] += sign;

        if term.chars().count() > 3 {
            for gram in sliding_char_grams(&term, 3) {
                let gram_hash = stable_term_hash(&gram);
                let gram_index = (gram_hash as usize) % ATTACHMENT_VECTOR_DIM;
                let gram_sign = if (gram_hash >> 63) == 0 {
                    0.5_f32
                } else {
                    -0.5_f32
                };
                vector[gram_index] += gram_sign;
            }
        }
    }

    normalize_vector(&mut vector);
    Some(vector)
}

pub(super) fn serialize_semantic_vector(vector: &[f32]) -> String {
    serde_json::to_string(vector).unwrap_or_default()
}

fn deserialize_semantic_vector(raw: &str) -> Option<Vec<f32>> {
    let vector = serde_json::from_str::<Vec<f32>>(raw).ok()?;
    if vector.len() == ATTACHMENT_VECTOR_DIM {
        Some(vector)
    } else {
        None
    }
}

fn normalize_vector(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
}

fn similarity_to_rank_score(similarity: f32) -> i64 {
    if similarity >= 0.85 {
        220
    } else if similarity >= 0.7 {
        170
    } else if similarity >= 0.55 {
        120
    } else if similarity >= 0.4 {
        80
    } else if similarity >= 0.25 {
        40
    } else {
        0
    }
}

fn stable_term_hash(text: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(bytes)
}

fn sliding_char_grams(text: &str, gram_size: usize) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() < gram_size {
        return Vec::new();
    }

    chars
        .windows(gram_size)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

pub(super) fn image_similarity_score(query_hash: u64, candidate_hash: u64) -> i64 {
    let distance = (query_hash ^ candidate_hash).count_ones() as i64;
    match distance {
        0..=2 => 240,
        3..=6 => 180,
        7..=10 => 120,
        11..=14 => 70,
        15..=18 => 30,
        _ => 0,
    }
}
fn row_to_meta(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteMeta> {
    let tags: String = row.get(2)?;
    let keywords: String = row.get(3)?;
    Ok(NoteMeta {
        id: row.get(0)?,
        title: row.get(1)?,
        tags: match serde_json::from_str(&tags) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    field = "tags",
                    error = %e,
                    "failed to parse tags JSON: {}", e,
                );
                Vec::new()
            }
        },
        keywords: match serde_json::from_str(&keywords) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    field = "keywords",
                    error = %e,
                    "failed to parse keywords JSON: {}", e,
                );
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
}

fn query_recent_note_metas(
    connection: &Connection,
    limit: usize,
    offset: usize,
) -> Result<Vec<NoteMeta>> {
    let mut statement = connection.prepare(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary
         FROM notes
         ORDER BY updated_at DESC
         LIMIT ?1 OFFSET ?2",
    )?;
    let rows = statement
        .query_map([limit as i64, offset as i64], row_to_meta)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// SQL-level filtered note query for empty-text searches with active filters.
/// Pushes tag, keyword, and date-range filters into the SQL WHERE clause so
/// that pagination operates on the correctly filtered result set.
/// Build the shared WHERE clause and params for note filtering (tags, keywords, date ranges).
/// Returns (where_clause, params) — where_clause is empty string when no filters apply.
fn build_note_filter_clause(
    query: &SearchQuery,
    start_index: usize,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut param_idx = start_index;

    for tag in &query.tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            conditions.push(format!(
                "EXISTS (SELECT 1 FROM json_each(CASE WHEN json_valid(tags) THEN tags ELSE '[]' END) WHERE LOWER(json_each.value) = LOWER(?{param_idx}))"
            ));
            params.push(Box::new(trimmed.to_string()));
            param_idx += 1;
        }
    }

    for kw in &query.keywords {
        let trimmed = kw.trim();
        if !trimmed.is_empty() {
            conditions.push(format!(
                "EXISTS (SELECT 1 FROM json_each(CASE WHEN json_valid(keywords) THEN keywords ELSE '[]' END) WHERE LOWER(json_each.value) = LOWER(?{param_idx}))"
            ));
            params.push(Box::new(trimmed.to_string()));
            param_idx += 1;
        }
    }

    let date_filters: [(&str, &str, Option<&str>); 4] = [
        ("created_at", ">=", query.created_after.as_deref()),
        ("created_at", "<=", query.created_before.as_deref()),
        ("updated_at", ">=", query.modified_after.as_deref()),
        ("updated_at", "<=", query.modified_before.as_deref()),
    ];
    for (col, op, val) in date_filters {
        if let Some(v) = val {
            if !v.is_empty() {
                conditions.push(format!("({col} = '' OR {col} {op} ?{param_idx})"));
                params.push(Box::new(v.to_string()));
                param_idx += 1;
            }
        }
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    (where_clause, params)
}

fn query_filtered_note_metas(
    connection: &Connection,
    query: &SearchQuery,
    limit: usize,
    offset: usize,
) -> Result<Vec<NoteMeta>> {
    let (where_clause, mut params) = build_note_filter_clause(query, 1);
    let mut param_idx = params.len() + 1;

    params.push(Box::new(limit as i64));
    let limit_idx = param_idx;
    param_idx += 1;
    params.push(Box::new(offset as i64));
    let offset_idx = param_idx;

    let sql = format!(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary
         FROM notes
         {where_clause}
         ORDER BY updated_at DESC
         LIMIT ?{limit_idx} OFFSET ?{offset_idx}"
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map(param_refs.as_slice(), row_to_meta)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Count all notes matching the SQL-level filters (tags, keywords, date ranges)
/// without LIMIT/OFFSET. Used alongside `query_filtered_note_metas` to report
/// the true total for pagination.
fn count_filtered_notes(connection: &Connection, query: &SearchQuery) -> Result<usize> {
    let (where_clause, params) = build_note_filter_clause(query, 1);

    let sql = format!("SELECT COUNT(*) FROM notes {where_clause}");
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut statement = connection.prepare(&sql)?;
    let count: i64 = statement.query_row(param_refs.as_slice(), |row| row.get(0))?;
    Ok(count as usize)
}

/// Count all notes in the database (no filters).
fn count_all_notes(connection: &Connection) -> Result<usize> {
    let mut statement = connection.prepare("SELECT COUNT(*) FROM notes")?;
    let count: i64 = statement.query_row([], |row| row.get(0))?;
    Ok(count as usize)
}

/// List all note metas without any LIMIT clause. Used by export functions
/// that need to process every note in the vault.
pub(crate) fn list_all_note_metas(connection: &Connection) -> Result<Vec<NoteMeta>> {
    let mut statement = connection.prepare(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary
         FROM notes
         ORDER BY updated_at DESC",
    )?;
    let rows = statement
        .query_map([], row_to_meta)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Escape SQL LIKE wildcard characters (`%`, `_`, `\`) in user input
/// so they are treated as literal characters in a LIKE pattern.
fn escape_like_pattern(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '%' | '_' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Fuzzy LIKE-based fallback when FTS5 returns no results.
/// Splits the query into words and matches any note whose title or summary
/// contains at least one of the words (case-insensitive).
fn query_like_note_metas(
    connection: &Connection,
    query_text: &str,
    limit: usize,
) -> Result<Vec<NoteMeta>> {
    let words: Vec<&str> = query_text
        .split_whitespace()
        .filter(|w| w.len() >= 2)
        .take(20) // Limit to 20 search terms to avoid SQLITE_MAX_VARIABLE_NUMBER overflow (#436)
        .collect();
    if words.is_empty() {
        return Ok(Vec::new());
    }

    // Build a WHERE clause: (title LIKE '%word1%' OR body LIKE '%word1%' OR ...)
    let mut conditions = Vec::new();
    let mut param_values: Vec<String> = Vec::new();
    for word in &words {
        let escaped = escape_like_pattern(&word.to_lowercase());
        let pattern = format!("%{}%", escaped);
        conditions.push("(LOWER(title) LIKE ? ESCAPE '\\' OR LOWER(summary) LIKE ? ESCAPE '\\')");
        param_values.push(pattern.clone());
        param_values.push(pattern);
    }
    let where_clause = conditions.join(" OR ");

    let sql = format!(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, \
         created_at, updated_at, source, path, summary \
         FROM notes WHERE {} ORDER BY updated_at DESC LIMIT ?",
        where_clause
    );

    let mut statement = connection.prepare(&sql)?;
    // Append the limit parameter
    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = param_values
        .into_iter()
        .map(|v| Box::new(v) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    all_params.push(Box::new(limit as i64));
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(|p| p.as_ref()).collect();

    let rows = statement
        .query_map(param_refs.as_slice(), row_to_meta)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Attempt to parse a date/time string into a UTC DateTime.
/// Supports full RFC 3339 (e.g. `2024-01-15T10:00:00Z`, `2024-01-15T10:00:00+08:00`)
/// and plain date-only strings (`YYYY-MM-DD`, assumed midnight UTC).
/// Returns `None` for empty or unparseable strings (filter is conservatively skipped).
fn parse_dt(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    // Full RFC 3339 with timezone
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Date-only: YYYY-MM-DD (assume midnight UTC)
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d.and_hms_opt(0, 0, 0)?.and_utc());
    }
    // Fallback: chrono's flexible parser, tries many ISO 8601 / RFC 3339 variants
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt);
    }
    None
}

/// Filter a list of NoteMetas by optional date range bounds.
/// Parses RFC 3339 timestamps into `DateTime<FixedOffset>` for proper
/// timezone-aware comparison, avoiding incorrect string-comparison results
/// when the inputs mix different timezone offset formats (e.g. `Z` vs `+00:00`).
pub(super) fn filter_by_date_range(
    mut notes: Vec<NoteMeta>,
    created_after: Option<&str>,
    created_before: Option<&str>,
    modified_after: Option<&str>,
    modified_before: Option<&str>,
) -> Vec<NoteMeta> {
    // Pre-parse filter boundaries once (cached for all notes).
    let created_after_dt = created_after.and_then(parse_dt);
    let created_before_dt = created_before.and_then(parse_dt);
    let modified_after_dt = modified_after.and_then(parse_dt);
    let modified_before_dt = modified_before.and_then(parse_dt);

    notes.retain(|note| {
        // Parse each note's timestamps; if unparseable (e.g. empty/invalid),
        // skip the corresponding filter checks (conservative: keep the note).
        let created_dt = parse_dt(&note.created_at);
        let updated_dt = parse_dt(&note.updated_at);

        if let Some(after_dt) = created_after_dt {
            if let Some(ref dt) = created_dt {
                if *dt < after_dt {
                    return false;
                }
            }
        }
        if let Some(before_dt) = created_before_dt {
            if let Some(ref dt) = created_dt {
                if *dt > before_dt {
                    return false;
                }
            }
        }
        if let Some(after_dt) = modified_after_dt {
            if let Some(ref dt) = updated_dt {
                if *dt < after_dt {
                    return false;
                }
            }
        }
        if let Some(before_dt) = modified_before_dt {
            if let Some(ref dt) = updated_dt {
                if *dt > before_dt {
                    return false;
                }
            }
        }
        true
    });
    notes
}

/// Count total FTS5 matches for a query without fetching rows.
/// Count total matches for a LIKE-based search (used when FTS5 has no results).
fn count_like_matches(connection: &Connection, query_text: &str) -> Result<usize> {
    let words: Vec<&str> = query_text
        .split_whitespace()
        .filter(|w| w.len() >= 2)
        .take(20)
        .collect();
    if words.is_empty() {
        return Ok(0);
    }

    let mut conditions = Vec::new();
    let mut param_values: Vec<String> = Vec::new();
    for word in &words {
        let escaped = escape_like_pattern(&word.to_lowercase());
        let pattern = format!("%{}%", escaped);
        conditions.push("(LOWER(title) LIKE ? ESCAPE '\\' OR LOWER(summary) LIKE ? ESCAPE '\\')");
        param_values.push(pattern.clone());
        param_values.push(pattern);
    }
    let where_clause = conditions.join(" OR ");
    let sql = format!("SELECT COUNT(*) FROM notes WHERE {}", where_clause);

    let mut statement = connection.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
        .iter()
        .map(|v| v as &dyn rusqlite::types::ToSql)
        .collect();

    let count: i64 = statement.query_row(param_refs.as_slice(), |row| row.get(0))?;
    Ok(count as usize)
}

fn count_fts_matches(connection: &Connection, text: &str) -> Result<usize> {
    let fts_query = make_fts_query(text);
    if fts_query.trim().is_empty() {
        return Ok(0);
    }
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM note_fts WHERE note_fts MATCH ?1",
        params![fts_query],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

/// Count FTS matches with additional tag/keyword/date-range filters applied at
/// the SQL level. This gives an **accurate** total for FTS + filter hybrid
/// queries, avoiding the lower-bound problem of post-filtering only the
/// over-fetched candidate set.
fn count_fts_matches_with_filters(connection: &Connection, query: &SearchQuery) -> Result<usize> {
    let fts_query = make_fts_query(&query.text);
    if fts_query.trim().is_empty() {
        return Ok(0);
    }

    // Build the filter clause with placeholders starting at ?2 — ?1 is
    // reserved for the FTS MATCH param. Generating the correct indices
    // directly avoids the fragile (and buggy for >=9 filters) String::replace
    // shifting that used to live here (#2130).
    let (filter_where, filter_params) = build_note_filter_clause(query, 2);

    if filter_where.is_empty() {
        // No filters — plain FTS count is sufficient.
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM note_fts WHERE note_fts MATCH ?1",
            params![fts_query],
            |row| row.get(0),
        )?;
        return Ok(count as usize);
    }

    // Strip the leading "WHERE " from the filter clause — we'll wrap it.
    let filter_conditions = filter_where
        .strip_prefix("WHERE ")
        .unwrap_or(&filter_where)
        .to_string();

    let sql = format!(
        "SELECT COUNT(*) FROM notes \
         WHERE id IN (SELECT note_id FROM note_fts WHERE note_fts MATCH ?1) \
         AND ({filter_conditions})"
    );

    // Prepend the FTS query param so ?1 → fts_query, ?2..?N+1 → filter params.
    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    all_params.push(Box::new(fts_query));
    all_params.extend(filter_params);

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(|p| p.as_ref()).collect();
    let mut statement = connection.prepare(&sql)?;
    let count: i64 = statement.query_row(param_refs.as_slice(), |row| row.get(0))?;
    Ok(count as usize)
}

///// Combined FTS5 query that returns both ordered note IDs and body snippets
/// in a single query, halving the FTS5 query cost.
fn query_fts_ids_and_snippets(
    connection: &Connection,
    text: &str,
    limit: usize,
) -> Result<(Vec<String>, HashMap<String, String>)> {
    let fts_query = make_fts_query(text);
    if fts_query.trim().is_empty() {
        return Ok((Vec::new(), HashMap::new()));
    }

    let mut statement = connection.prepare(
        "SELECT note_id, snippet(note_fts, 3, '==', '==', '…', 64)
         FROM note_fts
         WHERE note_fts MATCH ?1
         ORDER BY bm25(note_fts)
         LIMIT ?2",
    )?;

    let rows = match statement.query_map(params![fts_query, limit as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "FTS5 combined query failed, returning empty results");
            return Ok((Vec::new(), HashMap::new()));
        }
    };

    let mut ids = Vec::new();
    let mut snippets = HashMap::new();
    for row in rows {
        let (note_id, snippet) = row?;
        ids.push(note_id.clone());
        if !snippet.trim().is_empty() && snippet.contains("==") {
            snippets.insert(note_id, snippet);
        }
    }
    Ok((ids, snippets))
}

fn query_attachment_fts_note_ids(
    connection: &Connection,
    text: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let fts_query = make_fts_query(text);
    if fts_query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(
        "SELECT note_id
         FROM attachment_fts
         WHERE attachment_fts MATCH ?1
         ORDER BY bm25(attachment_fts)
         LIMIT ?2",
    )?;
    let rows = match statement.query_map(
        params![fts_query, (limit.saturating_mul(3)) as i64],
        |row| row.get::<_, String>(0),
    ) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "Attachment FTS5 query failed, returning empty results");
            return Ok(Vec::new());
        }
    };

    let mut note_ids = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let note_id = row?;
        if seen.insert(note_id.clone()) {
            note_ids.push(note_id);
        }
        if note_ids.len() >= limit {
            break;
        }
    }

    Ok(note_ids)
}

pub(super) fn load_note_meta_by_id(
    connection: &Connection,
    note_id: &str,
) -> Result<Option<NoteMeta>> {
    connection
        .query_row(
            "SELECT id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary
             FROM notes
             WHERE id = ?1
             LIMIT 1",
            [note_id],
            row_to_meta,
        )
        .optional()
        .map_err(Into::into)
}

fn query_recent_note_ids(connection: &Connection, limit: usize) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT id
         FROM notes
         ORDER BY updated_at DESC
         LIMIT ?1",
    )?;
    let rows = statement
        .query_map([limit as i64], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn row_to_attachment_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AttachmentEntry> {
    let semantic_vector: String = row.get(5)?;
    let perceptual_hash: String = row.get(6)?;
    Ok(AttachmentEntry {
        note_id: row.get(0)?,
        path: row.get(2)?,
        file_name: row.get(3)?,
        stem: row.get(4)?,
        ocr_text: row.get(7).unwrap_or_default(),
        semantic_vector: deserialize_semantic_vector(&semantic_vector),
        perceptual_hash: if perceptual_hash.trim().is_empty() {
            None
        } else {
            u64::from_str_radix(perceptual_hash.trim(), 16).ok()
        },
    })
}

fn load_attachment_entries_by_note_ids(
    connection: &Connection,
    note_ids: &[String],
) -> Result<HashMap<String, Vec<AttachmentEntry>>> {
    let mut attachments = HashMap::<String, Vec<AttachmentEntry>>::new();
    let batch_size = 50usize;

    for chunk in note_ids.chunks(batch_size) {
        let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT note_id, id, path, file_name, stem, semantic_vector, perceptual_hash, ocr_text
             FROM attachments
             WHERE note_id IN ({})",
            placeholders.join(", ")
        );
        let mut statement = connection.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = chunk
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = statement.query_map(params.as_slice(), row_to_attachment_entry)?;
        for row in rows {
            let entry = row?;
            attachments
                .entry(entry.note_id.clone())
                .or_default()
                .push(entry);
        }
    }

    Ok(attachments)
}

fn query_visual_candidate_scores(
    connection: &Connection,
    image_paths: &[String],
) -> Result<HashMap<String, i64>> {
    let query_hashes = image_paths
        .iter()
        .filter_map(|path| compute_image_perceptual_hash(Path::new(path)))
        .collect::<Vec<_>>();
    if query_hashes.is_empty() {
        return Ok(HashMap::new());
    }

    let mut scores = HashMap::new();
    let batch_size: i64 = 500;

    // Keyset pagination keyed on the attachment id keeps the scanned row set
    // stable across pages. A bare LIMIT/OFFSET without ORDER BY has an
    // undefined row order, and even with ORDER BY the OFFSET window drifts
    // when rows are inserted/deleted concurrently, which can cause
    // attachments to be skipped or double-counted and drop their visual
    // similarity scores from the ranking (#2147). Anchoring on `id > last_id`
    // is robust to both problems.
    let mut last_id = String::new();

    // Only SELECT the columns we need (id, note_id, perceptual_hash) to
    // reduce I/O and memory allocation for large vaults (#504).
    loop {
        let mut count: usize = 0;
        {
            let mut statement = connection.prepare(
                "SELECT id, note_id, perceptual_hash
                 FROM attachments
                 WHERE perceptual_hash <> '' AND id > ?1
                 ORDER BY id
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(params![last_id, batch_size], |row| {
                let id: String = row.get(0)?;
                let hash_str: String = row.get(2)?;
                let hash = u64::from_str_radix(hash_str.trim(), 16).ok();
                Ok((id, row.get::<_, String>(1)?, hash))
            })?;
            let mut batch_last_id = String::new();
            for row in rows {
                let (id, note_id, hash_opt) = row?;
                count += 1;
                // Track the cursor for *every* row (even ones whose hash fails
                // to parse) so the next page never re-scans them.
                batch_last_id = id;
                let Some(attachment_hash) = hash_opt else {
                    continue;
                };

                let best = query_hashes
                    .iter()
                    .map(|query_hash| image_similarity_score(*query_hash, attachment_hash))
                    .max()
                    .unwrap_or_default();
                if best <= 0 {
                    continue;
                }

                scores
                    .entry(note_id)
                    .and_modify(|current: &mut i64| *current = (*current).max(best))
                    .or_insert(best);
            }
            // Advance the cursor once the whole batch is consumed, so a
            // partially-failing iteration restarts at the same position.
            last_id = batch_last_id;
        }

        if count < batch_size as usize {
            break;
        }
    }

    Ok(scores)
}

fn query_attachment_semantic_scores(
    connection: &Connection,
    query_text: &str,
) -> Result<HashMap<String, i64>> {
    let Some(query_vector) = build_text_semantic_vector(query_text) else {
        return Ok(HashMap::new());
    };

    let mut scores = HashMap::new();
    let batch_size: i64 = 500;

    // Keyset pagination keyed on the attachment id keeps the scanned row set
    // stable across pages; see query_visual_candidate_scores for the full
    // rationale (#2147).
    let mut last_id = String::new();

    // Only SELECT the columns we need (id, note_id, semantic_vector) to
    // reduce I/O and memory allocation for large vaults (#504).
    loop {
        let mut count: usize = 0;
        {
            let mut statement = connection.prepare(
                "SELECT id, note_id, semantic_vector
                 FROM attachments
                 WHERE semantic_vector <> '' AND id > ?1
                 ORDER BY id
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(params![last_id, batch_size], |row| {
                let id: String = row.get(0)?;
                let sv: String = row.get(2)?;
                let vector = deserialize_semantic_vector(&sv);
                Ok((id, row.get::<_, String>(1)?, vector))
            })?;
            let mut batch_last_id = String::new();
            for row in rows {
                let (id, note_id, candidate_vector_opt) = row?;
                count += 1;
                // Track the cursor for *every* row so the next page never
                // re-scans it.
                batch_last_id = id;
                let Some(candidate_vector) = candidate_vector_opt else {
                    continue;
                };
                let similarity = cosine_similarity(&query_vector, &candidate_vector);
                let score = similarity_to_rank_score(similarity);
                if score <= 0 {
                    continue;
                }

                scores
                    .entry(note_id)
                    .and_modify(|current: &mut i64| *current = (*current).max(score))
                    .or_insert(score);
            }
            // Advance the cursor once the whole batch is consumed, so a
            // partially-failing iteration restarts at the same position.
            last_id = batch_last_id;
        }

        if count < batch_size as usize {
            break;
        }
    }

    Ok(scores)
}

pub(super) fn rank_note_metas(
    context: &StorageContext,
    connection: &Connection,
    query: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteMeta>> {
    let docs = rank_documents(context, connection, query, image_paths, limit)?;
    Ok(docs.into_iter().map(|doc| doc.meta).collect())
}

pub(super) fn rank_documents(
    _context: &StorageContext,
    connection: &Connection,
    query: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteDocument>> {
    // Single FTS5 query returns both ordered IDs and body snippets.
    let (note_fts_ids, fts_snippets) =
        query_fts_ids_and_snippets(connection, query, limit.saturating_mul(6).max(18))?;
    let attachment_query = attachment_query_text(query, image_paths);
    let attachment_fts_ids = query_attachment_fts_note_ids(
        connection,
        &attachment_query,
        limit.saturating_mul(4).max(12),
    )?;
    let visual_scores = query_visual_candidate_scores(connection, image_paths)?;
    let semantic_scores = query_attachment_semantic_scores(connection, &attachment_query)?;
    let recent_ids = query_recent_note_ids(
        connection,
        limit
            .saturating_mul(6)
            .max(if image_paths.is_empty() { 24 } else { 12 }),
    )?;
    let candidate_ids = build_candidate_note_ids(
        &note_fts_ids,
        &attachment_fts_ids,
        &semantic_scores,
        &visual_scores,
        &recent_ids,
        limit,
    );
    let attachment_entries = load_attachment_entries_by_note_ids(connection, &candidate_ids)?;
    let mut ranked = Vec::new();

    for note_id in candidate_ids {
        let Some(meta) = load_note_meta_by_id(connection, &note_id)? else {
            continue;
        };
        let Ok(mut doc) = load_note_body_from_meta(&meta) else {
            continue;
        };
        // Attach FTS5 snippet with highlight markers when available.
        doc.search_snippet = fts_snippets.get(&note_id).cloned();
        let attachments = attachment_entries
            .get(&note_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        let mut score = document_relevance_score(query, &doc);
        score += attachment_text_relevance_score(&attachment_query, attachments);

        if let Some(index) = note_fts_ids.iter().position(|id| id == &note_id) {
            score += 200_i64.saturating_sub(index as i64 * 10);
        }
        if let Some(index) = attachment_fts_ids.iter().position(|id| id == &note_id) {
            score += 150_i64.saturating_sub(index as i64 * 8);
        }
        if let Some(semantic_score) = semantic_scores.get(&note_id) {
            score += *semantic_score;
        }
        if let Some(visual_score) = visual_scores.get(&note_id) {
            score += *visual_score;
        }

        if score > 0 {
            ranked.push((score, doc));
        }
    }

    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.meta.updated_at.cmp(&left.1.meta.updated_at))
    });

    Ok(ranked.into_iter().take(limit).map(|(_, doc)| doc).collect())
}

fn build_candidate_note_ids(
    note_fts_ids: &[String],
    attachment_fts_ids: &[String],
    semantic_scores: &HashMap<String, i64>,
    visual_scores: &HashMap<String, i64>,
    recent_ids: &[String],
    limit: usize,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut semantic_ranked = semantic_scores.iter().collect::<Vec<_>>();
    let mut visual_ranked = visual_scores.iter().collect::<Vec<_>>();
    semantic_ranked.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    visual_ranked.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));

    for note_id in note_fts_ids {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }
    for note_id in attachment_fts_ids {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }
    for (note_id, _) in semantic_ranked {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }
    for (note_id, _) in visual_ranked {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }
    for note_id in recent_ids {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }

    ids.truncate(limit.saturating_mul(8).max(24));
    ids
}

fn push_candidate_note_id(note_id: &str, seen: &mut HashSet<String>, ids: &mut Vec<String>) {
    if seen.insert(note_id.to_string()) {
        ids.push(note_id.to_string());
    }
}

fn attachment_query_text(query: &str, image_paths: &[String]) -> String {
    let mut parts = Vec::new();
    if !query.trim().is_empty() {
        parts.push(query.trim().to_string());
    }

    for path in image_paths {
        let candidate = Path::new(path)
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(candidate) = candidate {
            parts.push(candidate.to_string());
        }
        if let Ok(ocr_text) = extract_image_text(Path::new(path)) {
            if !ocr_text.trim().is_empty() {
                parts.push(ocr_text.trim().to_string());
            }
        }
    }

    parts.join(" ")
}

fn attachment_text_relevance_score(query_text: &str, attachments: &[AttachmentEntry]) -> i64 {
    if attachments.is_empty() {
        return 0;
    }

    let normalized_query = normalize_query_for_search(query_text);
    let terms = extract_search_terms(query_text);
    if normalized_query.is_empty() && terms.is_empty() {
        return 0;
    }

    let mut score = 0_i64;
    let mut matched_terms = 0_i64;

    for term in &terms {
        if attachments.iter().any(|attachment| {
            let file_name = normalize_search_text(&attachment.file_name);
            let stem = normalize_search_text(&attachment.stem);
            let path = normalize_search_text(&attachment.path);
            let ocr_text = normalize_search_text(&attachment.ocr_text);
            file_name.contains(term.as_str())
                || stem.contains(term.as_str())
                || path.contains(term.as_str())
                || ocr_text.contains(term.as_str())
        }) {
            matched_terms += 1;
        }
    }

    for attachment in attachments {
        let file_name = normalize_search_text(&attachment.file_name);
        let stem = normalize_search_text(&attachment.stem);
        let path = normalize_search_text(&attachment.path);
        let ocr_text = normalize_search_text(&attachment.ocr_text);

        if !normalized_query.is_empty() {
            if stem.contains(&normalized_query) {
                score += 90;
            }
            if file_name.contains(&normalized_query) {
                score += 70;
            }
            if path.contains(&normalized_query) {
                score += 30;
            }
            if ocr_text.contains(&normalized_query) {
                score += 120;
            }
        }

        for term in &terms {
            if stem.contains(term) {
                score += 34;
            }
            if file_name.contains(term) {
                score += 24;
            }
            if path.contains(term) {
                score += 12;
            }
            if ocr_text.contains(term) {
                score += 40;
            }
        }
    }

    if matched_terms >= 2 {
        score += 35;
    }
    if matched_terms >= 4 {
        score += 60;
    }

    score
}

pub(super) fn document_relevance_score(query: &str, doc: &NoteDocument) -> i64 {
    let normalized_query = normalize_query_for_search(query);
    if normalized_query.is_empty() {
        return 0;
    }

    let terms = extract_search_terms(query);
    let title = normalize_search_text(&doc.meta.title);
    let summary = normalize_search_text(&doc.meta.summary);
    let keywords = normalize_search_text(&doc.meta.keywords.join(" "));
    let tags = normalize_search_text(&doc.meta.tags.join(" "));
    let path = normalize_search_text(&doc.meta.path);
    let body = normalize_search_text(&doc.body);

    let mut score = 0_i64;

    if title.contains(&normalized_query) {
        score += 160;
    }
    if summary.contains(&normalized_query) {
        score += 120;
    }
    if keywords.contains(&normalized_query) {
        score += 140;
    }
    if path.contains(&normalized_query) {
        score += 80;
    }
    if body.contains(&normalized_query) {
        score += 90;
    }

    for term in &terms {
        if title.contains(term) {
            score += 70;
        }
        if summary.contains(term) {
            score += 50;
        }
        if keywords.contains(term) {
            score += 60;
        }
        if tags.contains(term) {
            score += 40;
        }
        if path.contains(term) {
            score += 16;
        }
        if body.contains(term) {
            score += 24;
        }
    }

    let matched_terms = terms
        .iter()
        .filter(|term| {
            title.contains(term.as_str())
                || summary.contains(term.as_str())
                || keywords.contains(term.as_str())
                || tags.contains(term.as_str())
                || path.contains(term.as_str())
                || body.contains(term.as_str())
        })
        .count() as i64;

    if matched_terms > 0 {
        score += matched_terms * 12;
    }

    if matched_terms >= 2 {
        score += 40;
    }

    if matched_terms >= 4 {
        score += 80;
    }

    score += crate::search_rules::SearchRules::global()
        .domain_relevance_bonus(&terms, &collect_document_terms(doc));

    score
}

pub(super) fn normalize_search_text(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || is_cjk(ch) || ch == '_' || ch == '-' || ch == '.' {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
}

pub(super) fn normalize_query_for_search(text: &str) -> String {
    let mut normalized = normalize_search_text(text);
    for noise in [
        "告诉我",
        "帮我",
        "请问",
        "麻烦",
        "一下",
        "一下子",
        "这个",
        "那个",
        "怎么做",
        "怎么办",
        "怎么刷",
        "怎么",
        "如何",
        "是什么",
        "是什么样",
        "有没有",
        "之前",
        "以前",
        "一下呢",
        "一下啊",
        "一下呀",
    ] {
        normalized = normalized.replace(noise, " ");
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn extract_search_terms(text: &str) -> Vec<String> {
    let normalized = normalize_query_for_search(text);
    let mut terms = Vec::new();
    let mut seen = HashSet::new();

    for token in normalized.split_whitespace() {
        push_search_term(token, &mut seen, &mut terms);

        let segments = split_search_token(token);
        if segments.len() > 1 {
            for segment in &segments {
                push_search_term(segment, &mut seen, &mut terms);
            }
            for window in segments.windows(2) {
                let merged = window.concat();
                push_search_term(&merged, &mut seen, &mut terms);
            }
        }

        push_cjk_ngrams(token, 2, &mut seen, &mut terms);
        push_cjk_ngrams(token, 3, &mut seen, &mut terms);
    }

    let expanded = terms.clone();
    for term in expanded {
        for alias in expand_term_aliases(&term) {
            push_search_term(&alias, &mut seen, &mut terms);
        }
    }

    terms
}

fn push_search_term(term: &str, seen: &mut HashSet<String>, terms: &mut Vec<String>) {
    let cleaned = term.trim();
    if cleaned.len() <= 1 || is_noise_term(cleaned) {
        return;
    }
    if seen.insert(cleaned.to_string()) {
        terms.push(cleaned.to_string());
    }
}

fn push_cjk_ngrams(
    token: &str,
    gram_size: usize,
    seen: &mut HashSet<String>,
    terms: &mut Vec<String>,
) {
    let cjk_chars: Vec<char> = token
        .chars()
        .filter(|ch| is_cjk(*ch) && !is_cjk_stop_char(*ch))
        .collect();
    if cjk_chars.len() < gram_size {
        return;
    }

    for window in cjk_chars.windows(gram_size) {
        let gram = window.iter().collect::<String>();
        push_search_term(&gram, seen, terms);
    }
}

fn split_search_token(token: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_kind = None::<u8>;

    for ch in token.chars() {
        let kind = if is_cjk(ch) {
            1
        } else if ch.is_ascii_alphanumeric() {
            2
        } else {
            0
        };

        if kind == 0 {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            current_kind = None;
            continue;
        }

        if current_kind.is_some() && current_kind != Some(kind) && !current.is_empty() {
            parts.push(current.clone());
            current.clear();
        }

        current.push(ch);
        current_kind = Some(kind);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

fn expand_term_aliases(term: &str) -> Vec<String> {
    crate::search_rules::SearchRules::global().expand_term_aliases(term)
}

pub(super) fn collect_document_terms(doc: &NoteDocument) -> Vec<String> {
    extract_search_terms(&format!(
        "{}\n{}\n{}\n{}\n{}",
        doc.meta.title,
        doc.meta.summary,
        doc.meta.tags.join(" "),
        doc.meta.keywords.join(" "),
        doc.body
    ))
}

fn is_noise_term(term: &str) -> bool {
    matches!(
        term,
        "什么"
            | "事情"
            | "问题"
            | "一下"
            | "一下子"
            | "怎么"
            | "如何"
            | "那个"
            | "这个"
            | "告诉"
            | "帮我"
            | "请问"
            | "之前"
            | "以前"
            | "还有"
            | "一下呢"
            | "一下啊"
            | "一下呀"
            | "资料库"
    )
}

fn is_cjk_stop_char(ch: char) -> bool {
    matches!(
        ch,
        '的' | '了' | '呢' | '吗' | '啊' | '呀' | '吧' | '么' | '我' | '你'
    )
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{3000}'..='\u{303F}'   // CJK Symbols and Punctuation
        | '\u{3040}'..='\u{309F}'   // Japanese Hiragana
        | '\u{30A0}'..='\u{30FF}'   // Japanese Katakana
        | '\u{3400}'..='\u{4DBF}'   // CJK Unified Ideographs Extension A
        | '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{AC00}'..='\u{D7AF}'   // Korean Hangul Syllables
        | '\u{F900}'..='\u{FAFF}'   // CJK Compatibility Ideographs
        | '\u{20000}'..='\u{2A6DF}' // CJK Extension B
        | '\u{2A700}'..='\u{2B73F}' // CJK Extension C
        | '\u{2B740}'..='\u{2B81F}' // CJK Extension D
        | '\u{2B820}'..='\u{2CEAF}' // CJK Extension E
        | '\u{2CEB0}'..='\u{2EBEF}' // CJK Extension F
        | '\u{30000}'..='\u{3134F}' // CJK Extension G
    )
}

pub(super) fn has_all_terms(source: &[String], expected: &[String]) -> bool {
    expected.iter().all(|needle| {
        source
            .iter()
            .any(|item| item.eq_ignore_ascii_case(needle.trim()))
    })
}

pub(super) fn sanitize_terms(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(|value| {
            let normalized = value.to_string();
            if seen.insert(normalized.to_lowercase()) {
                Some(normalized)
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn fallback_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        "Untitled Note".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn fallback_source(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        "manual".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn slugify(value: &str) -> String {
    crate::utils::slugify(value)
}

pub(super) fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(super) fn derived_note_id(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!(
        "note-{}",
        &hash_content(&canonical.to_string_lossy())[0..24]
    )
}

pub(super) fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

/// Escape a single term for safe use in an FTS5 query by wrapping it in
/// double quotes and doubling any embedded double-quote characters. This
/// prevents FTS5 special characters (`*`, `"`, `(`, `)`, `+`, `-`, `:`, `^`,
/// `NEAR`, `AND`, `OR`, `NOT`) from being interpreted as query operators.
fn escape_fts5_term(term: &str) -> String {
    let cleaned: String = term
        .chars()
        .filter(|ch| ch.is_alphanumeric() || is_cjk(*ch) || *ch == '_' || *ch == '-')
        .collect();
    if cleaned.is_empty() {
        return String::new();
    }
    // Double any embedded double quotes, then wrap the whole term in double
    // quotes so FTS5 treats it as a literal token.
    format!("\"{}\"", cleaned.replace('"', "\"\""))
}

fn make_fts_query(text: &str) -> String {
    let terms: Vec<String> = extract_search_terms(text)
        .into_iter()
        .map(|term| escape_fts5_term(&term))
        .filter(|term| !term.is_empty())
        .take(8)
        .collect();
    terms.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    // AttachmentEntry is defined in this module
    use crate::storage::{initialize_storage_with_context, save_note_with_context};

    fn setup_temp_context() -> (std::path::PathBuf, crate::storage::StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-search-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = crate::storage::StorageContext::for_test(&temp);
        (temp, ctx)
    }

    /// Insert `total` note + attachment pairs where every attachment shares the
    /// supplied perceptual hash and semantic vector, forcing multi-page
    /// pagination (batch_size is 500). Each pair gets a distinct note_id so the
    /// caller can count scored notes. #2147.
    fn insert_many_scored_attachments(
        conn: &Connection,
        total: usize,
        perceptual_hash: &str,
        semantic_vector: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        for i in 0..total {
            let note_id = format!("note-{i}");
            conn.execute(
                "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary, body_hash)
                 VALUES (?1, ?2, '', '', '', '', '', '', ?3, ?3, '', ?4, '', '')",
                params![&note_id, format!("Note {i}"), &now, format!("/p/{i}.md")],
            )
            .expect("insert note");
            conn.execute(
                "INSERT INTO attachments (id, note_id, path, file_name, stem, ocr_text, semantic_vector, perceptual_hash, created_at)
                 VALUES (?1, ?2, ?3, '', '', '', ?4, ?5, ?6)",
                params![
                    format!("att-{i}"),
                    &note_id,
                    format!("/a/{i}.png"),
                    semantic_vector,
                    perceptual_hash,
                    &now
                ],
            )
            .expect("insert attachment");
        }
    }

    /// #2147: keyset pagination must visit every matching attachment even when
    /// the result set spans more than one batch_size page (500). All 550
    /// attachments share the query image's perceptual hash, so every one must
    /// receive a score — none may be dropped by unstable LIMIT/OFFSET ordering.
    #[test]
    fn visual_candidate_scores_visit_all_attachments_across_pages() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        super::super::pool::ensure_schema(&conn).expect("schema");

        // A small image whose perceptual hash every attachment will share.
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-visual-keyset-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&tmp).expect("temp dir");
        let query_image = tmp.join("query.png");
        let mut img = image::GrayImage::new(9, 8);
        for y in 0..8 {
            for x in 0..9 {
                img.put_pixel(x, y, image::Luma([if x < 4 { 255 } else { 0 }]));
            }
        }
        img.save(&query_image).expect("save query image");
        let query_hash =
            compute_image_perceptual_hash(&query_image).expect("query perceptual hash");
        let hash_hex = format!("{query_hash:016x}");

        let total = 550usize;
        insert_many_scored_attachments(&conn, total, &hash_hex, "");

        let scores =
            query_visual_candidate_scores(&conn, &[query_image.to_string_lossy().into_owned()])
                .expect("scores");

        assert_eq!(
            scores.len(),
            total,
            "all {} matching attachments must be scored across pagination pages; got {}",
            total,
            scores.len()
        );
    }

    /// #2147: the semantic-score query shares the same pagination mechanism and
    /// must likewise visit every matching attachment across pages. All 550
    /// attachments store the query's own semantic vector (cosine similarity 1.0),
    /// so every one must be scored.
    #[test]
    fn attachment_semantic_scores_visit_all_attachments_across_pages() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        super::super::pool::ensure_schema(&conn).expect("schema");

        let query_text = "vaultpilot attachment semantic regression";
        let query_vector = build_text_semantic_vector(query_text).expect("query semantic vector");
        let semantic_vector = serialize_semantic_vector(&query_vector);

        let total = 550usize;
        insert_many_scored_attachments(&conn, total, "", &semantic_vector);

        let scores = query_attachment_semantic_scores(&conn, query_text).expect("scores");

        assert_eq!(
            scores.len(),
            total,
            "all {} matching attachments must be scored across pagination pages; got {}",
            total,
            scores.len()
        );
    }

    #[test]
    fn escape_like_pattern_escapes_wildcards() {
        // Percent sign
        assert_eq!(escape_like_pattern("100%"), "100\\%");
        // Underscore
        assert_eq!(escape_like_pattern("a_b"), "a\\_b");
        // Backslash
        assert_eq!(escape_like_pattern("a\\b"), "a\\\\b");
        // Combined
        assert_eq!(escape_like_pattern("%_\\test"), "\\%\\_\\\\test");
        // No wildcards
        assert_eq!(escape_like_pattern("hello"), "hello");
        // Empty string
        assert_eq!(escape_like_pattern(""), "");
    }
    #[test]
    fn extract_search_terms_understands_mixed_cn_and_domain_terms() {
        let terms = extract_search_terms("告诉我 sd卡的引脚复用怎么做的");
        assert!(terms.iter().any(|term| term == "sd"));
        assert!(terms.iter().any(|term| term == "sd卡"));
        assert!(terms.iter().any(|term| term == "引脚"));
        assert!(terms.iter().any(|term| term == "复用"));
        assert!(terms.iter().any(|term| term == "引脚复用"));
        assert!(terms.iter().any(|term| term == "mmc"));
    }
    #[test]
    fn fts_query_preserves_pure_cjk_text() {
        let query = make_fts_query("你好");
        assert!(
            !query.is_empty(),
            "CJK text should not be stripped from FTS query"
        );
        // Terms are now wrapped in double quotes for FTS5 safety
        assert!(query.contains("\"你好\""));
    }

    #[test]
    fn fts_query_handles_cjk_search_terms() {
        let query = make_fts_query("引脚配置");
        assert!(
            !query.is_empty(),
            "CJK search terms should produce non-empty FTS query"
        );
        // extract_search_terms generates bigrams like "引脚" and "配置"
        // Terms are now wrapped in double quotes for FTS5 safety
        assert!(query.contains("\"引脚\""));
        assert!(query.contains("\"配置\""));
    }

    #[test]
    fn escape_fts5_term_wraps_in_quotes() {
        assert_eq!(escape_fts5_term("hello"), "\"hello\"");
        assert_eq!(escape_fts5_term("test_123"), "\"test_123\"");
        assert_eq!(escape_fts5_term("non-volatile"), "\"non-volatile\"");
    }

    #[test]
    fn escape_fts5_term_strips_special_chars() {
        // FTS5 operators like *, +, :, ^, (, ) are stripped by the filter
        assert_eq!(escape_fts5_term("hello*world"), "\"helloworld\"");
        assert_eq!(escape_fts5_term("foo+bar"), "\"foobar\"");
        assert_eq!(escape_fts5_term("a:b"), "\"ab\"");
        assert_eq!(escape_fts5_term("test^case"), "\"testcase\"");
        assert_eq!(escape_fts5_term("(parens)"), "\"parens\"");
    }

    #[test]
    fn escape_fts5_term_empty_after_filtering() {
        assert_eq!(escape_fts5_term(""), String::new());
        assert_eq!(escape_fts5_term("***"), String::new());
        // Only chars that are NOT alphanumeric, CJK, '_', or '-' produce empty
        assert_eq!(escape_fts5_term("+^:"), String::new());
    }

    #[test]
    fn escape_fts5_term_preserves_unicode_letters() {
        // French accented characters
        assert_eq!(escape_fts5_term("café"), "\"café\"");
        // German umlauts
        assert_eq!(escape_fts5_term("über"), "\"über\"");
        assert_eq!(escape_fts5_term("schön"), "\"schön\"");
        // Russian Cyrillic
        assert_eq!(escape_fts5_term("Москва"), "\"Москва\"");
        // Mixed ASCII + Unicode
        assert_eq!(escape_fts5_term("résumé"), "\"résumé\"");
    }

    #[test]
    fn make_fts_query_escapes_special_characters() {
        // extract_search_terms splits "hello*world" into "hello" and "world"
        let query = make_fts_query("hello*world");
        assert!(query.contains("\"hello\""));
        assert!(query.contains("\"world\""));
    }

    #[test]
    fn make_fts_query_does_not_produce_fts5_operators() {
        // A query like "test -not" should not produce a bare minus sign
        let query = make_fts_query("test -not");
        // All terms should be quoted, no bare operators
        for part in query.split_whitespace() {
            assert!(
                part.starts_with('"') && part.ends_with('"'),
                "each term should be quoted: {part}"
            );
        }
    }
    #[test]
    fn semantic_vectors_rank_related_text_higher() {
        let query = build_text_semantic_vector("github release workflow tag publish")
            .expect("query vector");
        let related =
            build_text_semantic_vector("release tag publish github").expect("related vector");
        let unrelated =
            build_text_semantic_vector("pinmux mmc gpio kernel").expect("unrelated vector");

        assert!(cosine_similarity(&query, &related) > cosine_similarity(&query, &unrelated));
        assert!(similarity_to_rank_score(cosine_similarity(&query, &related)) > 0);
    }

    #[test]
    fn attachment_text_score_uses_ocr_text() {
        let attachments = vec![AttachmentEntry {
            note_id: "n1".to_string(),
            path: "D:/vault/2026/04/release.md".to_string(),
            file_name: "screenshot.png".to_string(),
            stem: "screenshot".to_string(),
            ocr_text: "GitHub Release v0.1.1 publish workflow".to_string(),
            semantic_vector: None,
            perceptual_hash: None,
        }];

        assert!(attachment_text_relevance_score("release workflow", &attachments) > 0);
    }

    #[test]
    fn relevance_score_hits_sd_pinmux_note_from_natural_query() {
        let doc = NoteDocument {
            meta: NoteMeta {
                id: "1".to_string(),
                title: "RK3566 SD卡复用引脚电路示意图".to_string(),
                tags: vec![
                    "RK3566".to_string(),
                    "SD卡".to_string(),
                    "引脚复用".to_string(),
                ],
                keywords: vec![
                    "sd card".to_string(),
                    "pin multiplexing".to_string(),
                    "mmc".to_string(),
                ],
                platform: "RK3566".to_string(),
                board: String::new(),
                kernel: String::new(),
                status: "待确认".to_string(),
                created_at: String::new(),
                updated_at: "2026-04-10T00:00:00Z".to_string(),
                source: "manual".to_string(),
                path: "vault/2026/04/rk3566-sd.md".to_string(),
                summary: "记录 RK3566 平台下 SD 卡引脚复用的电路与对照信息".to_string(),
                collections: Vec::new(),
            },
            body:
                "## 概述\nSD 卡接口引脚连接定义。\n## 备注\n软件层可参考 Device Tree pinctrl 配置。"
                    .to_string(),
            search_snippet: None,
        };

        assert!(document_relevance_score("sd卡的引脚复用怎么做的", &doc) > 200);
    }
    #[test]
    fn relevance_score_hits_flash_command_note_from_broad_query() {
        let doc = NoteDocument {
            meta: NoteMeta {
                id: "2".to_string(),
                title: "刷机命令记录".to_string(),
                tags: vec!["刷机".to_string()],
                keywords: vec![
                    "wboot".to_string(),
                    "update".to_string(),
                    "zboot".to_string(),
                ],
                platform: String::new(),
                board: String::new(),
                kernel: String::new(),
                status: "已解决".to_string(),
                created_at: String::new(),
                updated_at: "2026-04-10T00:00:00Z".to_string(),
                source: "manual".to_string(),
                path: "vault/2026/04/flash.md".to_string(),
                summary: "之前刷机时使用过的命令记录".to_string(),
                collections: Vec::new(),
            },
            body: "相关命令: wboot -w update zboot.img".to_string(),
            search_snippet: None,
        };

        assert!(document_relevance_score("刷机怎么刷啊", &doc) > 180);
    }
    // ── 1.12 normalize_search_text / normalize_query_for_search ──

    #[test]
    fn normalize_search_text_lowercases() {
        assert!(normalize_search_text("Hello WORLD").contains("hello world"));
    }

    #[test]
    fn normalize_search_text_preserves_cjk() {
        let result = normalize_search_text("测试MMC模块");
        assert!(result.contains("测试"));
        assert!(result.contains("mmc"));
    }

    #[test]
    fn normalize_query_removes_noise_phrases() {
        let result = normalize_query_for_search("告诉我sd卡怎么做");
        assert!(!result.contains("告诉我"));
        assert!(result.contains("sd"));
    }
    // ── 1.13 is_noise_term ──

    #[test]
    fn noise_terms_detected() {
        assert!(is_noise_term("什么"));
        assert!(is_noise_term("怎么"));
        assert!(is_noise_term("如何"));
        assert!(is_noise_term("这个"));
        assert!(is_noise_term("那个"));
    }

    #[test]
    fn real_terms_not_noise() {
        assert!(!is_noise_term("sd卡"));
        assert!(!is_noise_term("mmc"));
        assert!(!is_noise_term("flash"));
        assert!(!is_noise_term("刷机"));
    }
    // ── 1.14 expand_term_aliases ──

    #[test]
    fn expand_sd_aliases() {
        let aliases = expand_term_aliases("sd");
        assert!(aliases.contains(&"sd卡".to_string()));
        assert!(aliases.contains(&"sdio".to_string()));
        assert!(aliases.contains(&"mmc".to_string()));
        assert!(aliases.contains(&"tf".to_string()));
    }

    #[test]
    fn expand_flash_aliases() {
        let aliases = expand_term_aliases("刷机");
        assert!(aliases.contains(&"烧录".to_string()));
        assert!(aliases.contains(&"flash".to_string()));
        assert!(aliases.contains(&"wboot".to_string()));
    }

    #[test]
    fn expand_gpio_aliases() {
        let aliases = expand_term_aliases("gpio");
        assert!(aliases.contains(&"管脚".to_string()));
        assert!(aliases.contains(&"引脚".to_string()));
    }

    #[test]
    fn expand_pinmux_aliases() {
        let aliases = expand_term_aliases("pinmux");
        assert!(aliases.contains(&"引脚复用".to_string()));
        assert!(aliases.contains(&"iomux".to_string()));
    }

    #[test]
    fn expand_random_term_returns_empty() {
        assert!(expand_term_aliases("something_unrelated_xyz").is_empty());
    }
    // ── 1.15 is_cjk / is_cjk_stop_char ──

    #[test]
    fn cjk_chars_identified() {
        assert!(is_cjk('电'));
        assert!(is_cjk('的'));
        assert!(!is_cjk('A'));
        assert!(!is_cjk('1'));
        // CJK Extension A (U+3400–U+4DBF)
        assert!(is_cjk('\u{3400}'));
        // CJK Compatibility Ideographs (U+F900–U+FAFF)
        assert!(is_cjk('\u{F900}'));
    }

    #[test]
    fn cjk_stop_chars_detected() {
        assert!(is_cjk_stop_char('的'));
        assert!(is_cjk_stop_char('了'));
        assert!(!is_cjk_stop_char('电'));
    }
    // ── 1.16 sliding_char_grams ──

    #[test]
    fn sliding_grams_normal() {
        let result = sliding_char_grams("abcd", 3);
        assert_eq!(result, vec!["abc", "bcd"]);
    }

    #[test]
    fn sliding_grams_too_short() {
        assert!(sliding_char_grams("ab", 3).is_empty());
    }

    #[test]
    fn sliding_grams_exact_length() {
        assert_eq!(sliding_char_grams("abc", 3), vec!["abc"]);
    }
    // ── 1.17 document_relevance_score edge cases ──

    #[test]
    fn relevance_empty_query_returns_zero() {
        let doc = NoteDocument::default();
        assert_eq!(document_relevance_score("", &doc), 0);
    }

    #[test]
    fn relevance_no_match_returns_zero() {
        let doc = NoteDocument {
            meta: NoteMeta {
                title: "Completely Unrelated".to_string(),
                ..Default::default()
            },
            body: "Nothing relevant here".to_string(),
            search_snippet: None,
        };
        assert_eq!(document_relevance_score("mmc sd卡 pinmux", &doc), 0);
    }

    #[test]
    fn relevance_body_only_match() {
        let doc = NoteDocument {
            body: "mmc timeout after 30 seconds".to_string(),
            ..Default::default()
        };
        assert!(document_relevance_score("mmc timeout", &doc) > 0);
    }
    // ── 1.18 attachment_text_relevance_score edge cases ──

    #[test]
    fn attachment_score_empty_attachments_zero() {
        assert_eq!(attachment_text_relevance_score("mmc", &[]), 0);
    }

    #[test]
    fn attachment_score_empty_query_zero() {
        let attachments = vec![AttachmentEntry {
            note_id: "n".to_string(),
            path: "p".to_string(),
            file_name: "f.png".to_string(),
            stem: "f".to_string(),
            ocr_text: "text".to_string(),
            semantic_vector: None,
            perceptual_hash: None,
        }];
        assert_eq!(attachment_text_relevance_score("", &attachments), 0);
    }

    #[test]
    fn attachment_score_ocr_match_higher_than_filename() {
        let attachments = vec![AttachmentEntry {
            note_id: "n".to_string(),
            path: "p".to_string(),
            file_name: "img.png".to_string(),
            stem: "img".to_string(),
            ocr_text: "mmc timeout register dump".to_string(),
            semantic_vector: None,
            perceptual_hash: None,
        }];
        let score_ocr = attachment_text_relevance_score("mmc timeout register", &attachments);
        let score_fname = attachment_text_relevance_score("img", &attachments);
        assert!(score_ocr > score_fname);
    }
    // ── 1.19 build_candidate_note_ids ──

    #[test]
    fn build_candidates_deduplicates() {
        let ids = build_candidate_note_ids(
            &["a".to_string(), "b".to_string()],
            &["b".to_string(), "c".to_string()],
            &HashMap::new(),
            &HashMap::new(),
            &["c".to_string(), "d".to_string()],
            10,
        );
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"b".to_string()));
        assert!(ids.contains(&"c".to_string()));
        assert!(ids.contains(&"d".to_string()));
    }

    #[test]
    fn build_candidates_truncates_to_limit() {
        let many: Vec<String> = (0..100).map(|i| format!("id{i}")).collect();
        let result = build_candidate_note_ids(&many, &[], &HashMap::new(), &HashMap::new(), &[], 2);
        assert!(result.len() <= 24); // limit*8.max(24) with limit=2 → 24
    }
    // ── 1.20 cosine_similarity / normalize_vector ──

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0_f32, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.001);
    }

    #[test]
    fn normalize_vector_zero_no_panic() {
        let mut v = vec![0.0_f32; 3];
        normalize_vector(&mut v); // should not divide by zero
        assert_eq!(v, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn normalize_vector_produces_unit() {
        let mut v = vec![3.0_f32, 4.0];
        normalize_vector(&mut v);
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }
    // ── 1.21 similarity_to_rank_score ──

    #[test]
    fn similarity_to_rank_boundary_values() {
        assert_eq!(similarity_to_rank_score(0.85), 220);
        assert_eq!(similarity_to_rank_score(0.70), 170);
        assert_eq!(similarity_to_rank_score(0.55), 120);
        assert_eq!(similarity_to_rank_score(0.40), 80);
        assert_eq!(similarity_to_rank_score(0.25), 40);
        assert_eq!(similarity_to_rank_score(0.10), 0);
        assert_eq!(similarity_to_rank_score(1.0), 220); // top bucket
    }
    // ── 1.22 image_similarity_score ──

    #[test]
    fn image_similarity_identical() {
        assert_eq!(image_similarity_score(0xABCD, 0xABCD), 240);
    }

    #[test]
    fn image_similarity_boundary_distances() {
        let base: u64 = 0;
        let d2: u64 = (1u64 << 2) - 1; // 2 bits differ
        assert_eq!(image_similarity_score(base, d2), 240);
    }

    #[test]
    fn image_similarity_max_distance() {
        assert_eq!(image_similarity_score(0, u64::MAX), 0);
    }
    // ── 1.23 serialize/deserialize semantic vector ──

    #[test]
    fn semantic_vector_round_trip() {
        let v: Vec<f32> = (0..ATTACHMENT_VECTOR_DIM)
            .map(|i| i as f32 * 0.01)
            .collect();
        let serialized = serialize_semantic_vector(&v);
        let deserialized = deserialize_semantic_vector(&serialized).expect("deserialize");
        assert_eq!(deserialized.len(), ATTACHMENT_VECTOR_DIM);
        for (a, b) in v.iter().zip(deserialized.iter()) {
            assert!((a - b).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn deserialize_wrong_dimension_returns_none() {
        let v = vec![1.0_f32; 10];
        let serialized = serde_json::to_string(&v).unwrap();
        assert!(deserialize_semantic_vector(&serialized).is_none());
    }

    #[test]
    fn deserialize_garbage_returns_none() {
        assert!(deserialize_semantic_vector("not json").is_none());
    }

    // ── 1.24 build_text_semantic_vector ──

    #[test]
    fn semantic_vector_empty_text_returns_none() {
        assert!(build_text_semantic_vector("").is_none());
    }

    #[test]
    fn semantic_vector_produces_normalized() {
        let v = build_text_semantic_vector("github release workflow").expect("vector");
        assert_eq!(v.len(), ATTACHMENT_VECTOR_DIM);
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    // ── 1.25 stable_term_hash ──

    #[test]
    fn stable_term_hash_consistent() {
        assert_eq!(stable_term_hash("mmc"), stable_term_hash("mmc"));
    }

    #[test]
    fn stable_term_hash_different_inputs() {
        assert_ne!(stable_term_hash("mmc"), stable_term_hash("sdio"));
    }
    #[test]
    fn search_notes_filters_by_text() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        for (i, (title, tags)) in [
            ("MMC timeout fix", vec!["kernel".to_string()]),
            ("SD卡引脚配置", vec!["hardware".to_string()]),
            ("刷机命令记录", vec!["tool".to_string()]),
        ]
        .into_iter()
        .enumerate()
        {
            save_note_with_context(
                &ctx,
                NoteDocument {
                    meta: NoteMeta {
                        title: title.to_string(),
                        tags,
                        ..Default::default()
                    },
                    body: format!("Content for note {}", i),
                    search_snippet: None,
                },
            )
            .expect("save");
        }

        let results = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "MMC".to_string(),
                tags: vec![],
                keywords: vec![],
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("search");
        assert!(results.notes.iter().any(|n| n.title.contains("MMC")));
    }

    #[test]
    fn search_notes_filters_by_tags() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        save_note_with_context(
            &ctx,
            NoteDocument {
                meta: NoteMeta {
                    title: "Tagged Note".to_string(),
                    tags: vec!["kernel".to_string()],
                    ..Default::default()
                },
                body: "Tagged content".to_string(),
                search_snippet: None,
            },
        )
        .expect("save");

        let results = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: String::new(),
                tags: vec!["kernel".to_string()],
                keywords: vec![],
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("search by tag");
        assert!(results
            .notes
            .iter()
            .any(|n| n.tags.contains(&"kernel".to_string())));
    }

    #[test]
    fn search_notes_tag_filter_exact_match() {
        // Regression: tag filter must use exact match, not substring.
        // Searching for tag "sd" must NOT match a note tagged "sdcard".
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        save_note_with_context(
            &ctx,
            NoteDocument {
                meta: NoteMeta {
                    title: "SDCard Note".to_string(),
                    tags: vec!["sdcard".to_string()],
                    ..Default::default()
                },
                body: "content".to_string(),
                search_snippet: None,
            },
        )
        .expect("save");

        let results = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: String::new(),
                tags: vec!["sd".to_string()],
                keywords: vec![],
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("search by tag");
        assert!(
            results.notes.is_empty(),
            "tag 'sd' should not match tag 'sdcard', got {} results",
            results.notes.len()
        );
    }

    #[test]
    fn search_total_count_not_capped_by_limit() {
        // Regression: total must reflect the full matching set, not just
        // the LIMIT-clamped page.  See issue #769.
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        for i in 0..5 {
            save_note_with_context(
                &ctx,
                NoteDocument {
                    meta: NoteMeta {
                        title: format!("Note {i}"),
                        ..Default::default()
                    },
                    body: format!("Body {i}"),
                    search_snippet: None,
                },
            )
            .expect("save");
        }

        // No text, no filters → query_recent_note_metas path.
        let results = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: String::new(),
                limit: Some(2),
                ..Default::default()
            },
        )
        .expect("search");
        assert_eq!(results.notes.len(), 2, "page should contain 2 notes");
        assert_eq!(
            results.total, 5,
            "total should reflect all 5 notes, not the LIMIT"
        );

        // With tag filter → query_filtered_note_metas path.
        save_note_with_context(
            &ctx,
            NoteDocument {
                meta: NoteMeta {
                    title: "Tagged".to_string(),
                    tags: vec!["important".to_string()],
                    ..Default::default()
                },
                body: "tagged body".to_string(),
                search_snippet: None,
            },
        )
        .expect("save tagged");

        let results = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: String::new(),
                tags: vec!["important".to_string()],
                limit: Some(1),
                ..Default::default()
            },
        )
        .expect("search filtered");
        assert_eq!(results.notes.len(), 1, "page should contain 1 note");
        assert_eq!(results.total, 1, "total should reflect the 1 matching note");
    }

    // ── #2130: placeholder shift corrupts ?10+ ─────────────────────

    #[test]
    fn count_fts_matches_with_nine_filters_no_placeholder_corruption() {
        // Regression (#2130): count_fts_matches_with_filters used String::replace
        // to shift placeholders ?N → ?{N+1}. With >=9 filters, replace("?1","?2")
        // corrupted ?10 into ?20, producing invalid SQL and making the count
        // fall back to a (wrong) lower bound. With the start_index fix, the
        // placeholders are generated correctly and the count succeeds.
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        // 3 notes whose body FTS-matches "alpha", each carrying 5 tags + 4 keywords
        // so the filter clause has exactly 9 params (placeholders ?2..?10).
        for i in 0..3 {
            save_note_with_context(
                &ctx,
                NoteDocument {
                    meta: NoteMeta {
                        title: format!("Alpha {i}"),
                        tags: vec![
                            "t1".into(),
                            "t2".into(),
                            "t3".into(),
                            "t4".into(),
                            "t5".into(),
                        ],
                        keywords: vec!["k1".into(), "k2".into(), "k3".into(), "k4".into()],
                        ..Default::default()
                    },
                    body: "alpha content".into(),
                    search_snippet: None,
                },
            )
            .expect("save");
        }

        let (connection, _) = open_connection(&ctx).expect("connect");
        let query = SearchQuery {
            text: "alpha".into(),
            tags: vec![
                "t1".into(),
                "t2".into(),
                "t3".into(),
                "t4".into(),
                "t5".into(),
            ],
            keywords: vec!["k1".into(), "k2".into(), "k3".into(), "k4".into()],
            limit: Some(50),
            ..Default::default()
        };

        let count = count_fts_matches_with_filters(&connection, &query)
            .expect("count must not error with 9 filters");
        assert_eq!(count, 3, "all 3 notes match the FTS term + 9 filters");
    }

    #[test]
    fn search_total_accurate_with_many_filters_and_many_notes() {
        // Regression (#2130): with >50 matching notes and >=9 filters, the
        // over-fetched candidate set (fetch_limit capped) is smaller than the
        // true total. A corrupted count would fall back to notes.len() (the
        // candidate page size), under-reporting the total.
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        let total_notes = 60;
        for i in 0..total_notes {
            save_note_with_context(
                &ctx,
                NoteDocument {
                    meta: NoteMeta {
                        title: format!("Beta {i}"),
                        tags: vec![
                            "t1".into(),
                            "t2".into(),
                            "t3".into(),
                            "t4".into(),
                            "t5".into(),
                        ],
                        keywords: vec!["k1".into(), "k2".into(), "k3".into(), "k4".into()],
                        ..Default::default()
                    },
                    body: "beta keyword".into(),
                    search_snippet: None,
                },
            )
            .expect("save");
        }

        let results = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "beta".into(),
                tags: vec![
                    "t1".into(),
                    "t2".into(),
                    "t3".into(),
                    "t4".into(),
                    "t5".into(),
                ],
                keywords: vec!["k1".into(), "k2".into(), "k3".into(), "k4".into()],
                limit: Some(2),
                ..Default::default()
            },
        )
        .expect("search");
        assert_eq!(results.notes.len(), 2, "page should contain 2 notes");
        assert_eq!(
            results.total, total_notes,
            "total must reflect all {total_notes} matching notes, not the over-fetched page bound"
        );
    }

    // ── stable_term_hash ──────────────────────────────────────────

    #[test]
    fn stable_term_hash_deterministic() {
        let h1 = stable_term_hash("hello world");
        let h2 = stable_term_hash("hello world");
        assert_eq!(h1, h2, "same input must produce same hash");
    }

    #[test]
    fn stable_term_hash_different_inputs_differ() {
        let h1 = stable_term_hash("hello");
        let h2 = stable_term_hash("world");
        assert_ne!(h1, h2, "different inputs should produce different hashes");
    }

    #[test]
    fn stable_term_hash_empty_string() {
        // Should not panic
        let _ = stable_term_hash("");
    }

    #[test]
    fn stable_term_hash_cjk() {
        // Should not panic on CJK characters
        let h = stable_term_hash("你好世界");
        assert_eq!(h, stable_term_hash("你好世界"));
    }

    #[test]
    fn stable_term_hash_long_input() {
        let long_text = "a".repeat(10_000);
        let h = stable_term_hash(&long_text);
        assert_ne!(h, 0); // very unlikely to be zero
    }

    // ── 1.26 normalize_search_text boundary ──

    #[test]
    fn normalize_search_text_empty_string() {
        assert_eq!(normalize_search_text(""), "");
    }

    #[test]
    fn normalize_search_text_special_chars_become_spaces() {
        // Punctuation, brackets, etc. should be replaced with spaces
        let result = normalize_search_text("hello!@#world");
        assert_eq!(result, "hello   world");
    }

    #[test]
    fn normalize_search_text_preserves_underscores_and_dashes() {
        let result = normalize_search_text("note-id_v2.md");
        assert_eq!(result, "note-id_v2.md");
    }

    // ── 1.27 normalize_query_for_search boundary ──

    #[test]
    fn normalize_query_empty_string() {
        assert_eq!(normalize_query_for_search(""), "");
    }

    #[test]
    fn normalize_query_all_noise_returns_empty() {
        // All tokens are noise phrases
        let result = normalize_query_for_search("告诉我帮我请问");
        assert!(
            result.trim().is_empty(),
            "all-noise query should collapse, got: '{result}'"
        );
    }

    #[test]
    fn normalize_query_preserves_real_content_among_noise() {
        let result = normalize_query_for_search("请问mmc模块怎么配置");
        assert!(result.contains("mmc"), "should keep 'mmc', got: '{result}'");
        assert!(
            result.contains("模块"),
            "should keep '模块', got: '{result}'"
        );
        assert!(
            result.contains("配置"),
            "should keep '配置', got: '{result}'"
        );
    }

    // ── 1.28 extract_search_terms boundary ──

    #[test]
    fn extract_terms_empty_string() {
        let terms = extract_search_terms("");
        assert!(terms.is_empty(), "empty input should yield no terms");
    }

    #[test]
    fn extract_terms_single_char_filtered_out() {
        // Single-char tokens are filtered by push_search_term (len <= 1)
        let terms = extract_search_terms("a b c");
        assert!(terms.is_empty(), "single-char tokens should be filtered");
    }

    #[test]
    fn extract_terms_deduplicates() {
        let terms = extract_search_terms("mmc mmc mmc");
        let mmc_count = terms.iter().filter(|t| *t == "mmc").count();
        assert_eq!(
            mmc_count, 1,
            "duplicate 'mmc' should be deduped, got {mmc_count}"
        );
    }

    // ── 1.29 filter_by_date_range ──

    #[test]
    fn filter_date_range_no_filters_returns_all() {
        let notes = vec![
            NoteMeta {
                id: "1".into(),
                created_at: "2026-01-01".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "2".into(),
                created_at: "2026-06-01".into(),
                ..Default::default()
            },
        ];
        let result = filter_by_date_range(notes, None, None, None, None);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_date_range_created_after() {
        let notes = vec![
            NoteMeta {
                id: "1".into(),
                created_at: "2026-01-01".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "2".into(),
                created_at: "2026-06-01".into(),
                ..Default::default()
            },
        ];
        let result = filter_by_date_range(notes, Some("2026-03-01"), None, None, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "2");
    }

    #[test]
    fn filter_date_range_created_before() {
        let notes = vec![
            NoteMeta {
                id: "1".into(),
                created_at: "2026-01-01".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "2".into(),
                created_at: "2026-06-01".into(),
                ..Default::default()
            },
        ];
        let result = filter_by_date_range(notes, None, Some("2026-03-01"), None, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "1");
    }

    #[test]
    fn filter_date_range_empty_date_skips_filter() {
        // Notes with empty created_at should pass through
        let notes = vec![NoteMeta {
            id: "1".into(),
            created_at: String::new(),
            ..Default::default()
        }];
        let result = filter_by_date_range(notes, Some("2026-01-01"), None, None, None);
        assert_eq!(result.len(), 1, "empty date should not be filtered");
    }

    #[test]
    fn filter_date_range_modified_filters() {
        let notes = vec![
            NoteMeta {
                id: "1".into(),
                updated_at: "2026-01-15".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "2".into(),
                updated_at: "2026-06-15".into(),
                ..Default::default()
            },
        ];
        let result =
            filter_by_date_range(notes, None, None, Some("2026-03-01"), Some("2026-12-01"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "2");
    }

    // ── 1.30 document_relevance_score boundary ──

    #[test]
    fn relevance_score_empty_body_zero() {
        let doc = NoteDocument {
            meta: NoteMeta {
                title: "Title".into(),
                ..Default::default()
            },
            body: String::new(),
            search_snippet: None,
        };
        // No matching terms in title either
        assert_eq!(document_relevance_score("mmc", &doc), 0);
    }

    #[test]
    fn relevance_score_title_match_higher_than_body_only() {
        let title_doc = NoteDocument {
            meta: NoteMeta {
                title: "mmc configuration guide".into(),
                ..Default::default()
            },
            body: "unrelated content".into(),
            search_snippet: None,
        };
        let body_doc = NoteDocument {
            meta: NoteMeta {
                title: "unrelated".into(),
                ..Default::default()
            },
            body: "mmc configuration guide".into(),
            search_snippet: None,
        };
        let title_score = document_relevance_score("mmc", &title_doc);
        let body_score = document_relevance_score("mmc", &body_doc);
        assert!(
            title_score > body_score,
            "title match ({title_score}) should beat body-only ({body_score})"
        );
    }

    #[test]
    fn relevance_score_tag_match_contributes() {
        let doc = NoteDocument {
            meta: NoteMeta {
                title: "Note".into(),
                tags: vec!["hardware".into(), "mmc".into()],
                ..Default::default()
            },
            body: "unrelated".into(),
            search_snippet: None,
        };
        assert!(
            document_relevance_score("mmc", &doc) > 0,
            "tag match should yield positive score"
        );
    }
}
