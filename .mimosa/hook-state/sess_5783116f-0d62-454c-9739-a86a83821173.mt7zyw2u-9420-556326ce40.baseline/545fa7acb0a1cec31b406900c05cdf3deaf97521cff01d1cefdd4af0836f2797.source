//! Zero-index instant search (#1903).
//!
//! Searches the vault's raw `.md` files directly, with no FTS5 index required.
//! This module is the fallback for [`super::search::search_notes_with_context`]:
//! when the FTS5 index has no matches (e.g. it has not been built yet, or the
//! query terms simply are absent from the indexed corpus), the vault directory
//! is walked and every markdown file is parsed and scored in memory.
//!
//! Ranking reuses the exact same [`document_relevance_score`] used by the
//! indexed path, so title/tag/body weighting is identical between the two.
//! Tag, keyword, and date-range filters are likewise shared, so the only
//! observable difference between an indexed and an instant result set is the
//! (higher) latency of the full scan.

use std::path::{Path, PathBuf};

use anyhow::Result;
use tracing::{debug, instrument, warn};
use walkdir::WalkDir;

use crate::models::{NoteDocument, SearchQuery, SearchResult};

use super::notes::parse_markdown_note;
use super::search::{
    document_relevance_score, extract_search_terms, filter_by_date_range, has_all_terms,
    is_markdown_file,
};
use super::settings::load_settings_with_context;
use super::StorageContext;

/// Maximum directory depth to descend while scanning the vault. Matches the
/// indexer in [`super::notes::rebuild_index_with_context`] so the instant
/// fallback covers exactly the same set of files.
const MAX_SCAN_DEPTH: usize = 20;

/// Half-width (in chars) of context shown on each side of a matched term in a
/// generated snippet. Kept compact to mirror FTS5's `snippet(..., 64)` window.
const SNIPPET_CONTEXT_CHARS: usize = 48;
/// Total target length (in chars) of a generated snippet.
const SNIPPET_TOTAL_CHARS: usize = 180;

/// Zero-index instant search returning a [`SearchResult`] of [`NoteMeta`].
///
/// This is the interface-compatible entry point used as the FTS5 fallback. It
/// applies tag/keyword/date filters and pagination internally and returns a
/// complete result, so callers can treat it as a drop-in replacement for the
/// indexed search path.
#[instrument(skip(context, query))]
pub fn instant_search_notes_with_context(
    context: &StorageContext,
    query: SearchQuery,
) -> Result<SearchResult> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);

    let mut scored = scan_and_score(context, &query)?;

    // total reflects the fully-filtered set BEFORE pagination.
    let total = scored.len();
    let effective_offset = offset.min(scored.len());
    scored.drain(..effective_offset);
    scored.truncate(limit);

    let notes = scored.into_iter().map(|(doc, _)| doc.meta).collect();
    Ok(SearchResult { notes, total })
}

/// Zero-index instant search returning full [`NoteDocument`]s with generated
/// highlight snippets (using the same `==term==` markers as FTS5).
///
/// Use this richer variant when the caller wants the matched body excerpt, not
/// just metadata. Pagination (offset/limit) is applied.
#[instrument(skip(context, query))]
pub fn instant_search_documents_with_context(
    context: &StorageContext,
    query: SearchQuery,
) -> Result<Vec<NoteDocument>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);

    let terms = extract_search_terms(&query.text);
    let mut scored = scan_and_score(context, &query)?;

    let effective_offset = offset.min(scored.len());
    scored.drain(..effective_offset);
    scored.truncate(limit);

    Ok(scored
        .into_iter()
        .map(|(mut doc, _)| {
            if let Some(snippet) = build_snippet(&doc.body, &terms) {
                doc.search_snippet = Some(snippet);
            }
            doc
        })
        .collect())
}

/// Walk the vault, parse every note, score it against the query text, apply
/// tag/keyword/date filters, and return the matches sorted by relevance
/// (descending), breaking ties by `updated_at` descending.
///
/// Returns an empty vector when the query text is blank, the vault directory
/// is unset/missing, or no note matches.
fn scan_and_score(
    context: &StorageContext,
    query: &SearchQuery,
) -> Result<Vec<(NoteDocument, i64)>> {
    if query.text.trim().is_empty() {
        return Ok(Vec::new());
    }

    let settings = load_settings_with_context(context)?;
    let vault_dir = PathBuf::from(&settings.vault_dir);
    if vault_dir.as_os_str().is_empty() || !vault_dir.is_dir() {
        debug!(
            vault_dir = %vault_dir.display(),
            "instant search: vault dir unset or missing, nothing to scan"
        );
        return Ok(Vec::new());
    }

    let markdown_files = collect_markdown_files(&vault_dir);
    debug!(
        files = markdown_files.len(),
        query = %query.text,
        "instant search: scanning vault files"
    );

    let mut scored: Vec<(NoteDocument, i64)> = Vec::new();
    for file in &markdown_files {
        let doc = match parse_markdown_note(file, "instant") {
            Ok(doc) => doc,
            Err(e) => {
                // Skip files that cannot be parsed (oversized, invalid
                // frontmatter, non-UTF8, ...) rather than aborting the scan.
                warn!(
                    path = %file.display(),
                    error = %e,
                    "instant search: skipping unparseable note file"
                );
                continue;
            }
        };
        let score = document_relevance_score(&query.text, &doc);
        if score > 0 {
            scored.push((doc, score));
        }
    }

    // Tag filter: note must contain ALL requested tags (case-insensitive),
    // matching the indexed path's `has_all_terms` semantics.
    if !query.tags.is_empty() {
        scored.retain(|(doc, _)| has_all_terms(&doc.meta.tags, &query.tags));
    }
    // Keyword filter: same all-terms requirement.
    if !query.keywords.is_empty() {
        scored.retain(|(doc, _)| has_all_terms(&doc.meta.keywords, &query.keywords));
    }

    // Date-range filter: reuse the indexed-path helper so timezone-aware
    // comparison rules are identical. It operates on NoteMeta, so project out
    // and rebuild — metas are cheap to clone for the filtered remainder.
    if query.created_after.is_some()
        || query.created_before.is_some()
        || query.modified_after.is_some()
        || query.modified_before.is_some()
    {
        let metas = scored
            .iter()
            .map(|(doc, _)| doc.meta.clone())
            .collect::<Vec<_>>();
        let filtered = filter_by_date_range(
            metas,
            query.created_after.as_deref(),
            query.created_before.as_deref(),
            query.modified_after.as_deref(),
            query.modified_before.as_deref(),
        );
        let keep: std::collections::HashSet<String> = filtered.into_iter().map(|m| m.id).collect();
        scored.retain(|(doc, _)| keep.contains(&doc.meta.id));
    }

    // Sort by relevance descending; break ties by recency (updated_at desc).
    scored.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.0.meta.updated_at.cmp(&left.0.meta.updated_at))
    });

    Ok(scored)
}

/// Recursively collect every `.md` file under `root`, mirroring the indexer's
/// coverage (same depth bound and [`is_markdown_file`] predicate).
fn collect_markdown_files(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .max_depth(MAX_SCAN_DEPTH)
        .into_iter()
        .filter_entry(|entry| {
            // Prune hidden directories (`.obsidian`, `.trash`, `.git`, ...) so
            // deleted/config files never surface as search hits. Hidden *files*
            // are still allowed through and filtered by the markdown predicate.
            entry.file_type().is_file() || !is_hidden_name(entry.file_name())
        })
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file() && is_markdown_file(entry.path()))
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

/// `true` if the entry's file name begins with `.` (a Unix-style hidden name).
fn is_hidden_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().map(|s| s.starts_with('.')).unwrap_or(false)
}

/// Build a short highlight snippet around the first occurrence of any query
/// term in `text`, using FTS5-compatible `==term==` markers.
///
/// Returns `None` when the text is empty, no term matches, or there are no
/// terms. The search is case-insensitive; the emitted snippet preserves the
/// original casing of the matched text.
fn build_snippet(text: &str, terms: &[String]) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || terms.is_empty() {
        return None;
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let lower: Vec<char> = trimmed.to_lowercase().chars().collect();

    // Pre-compute each term as a lowercase char slice we can compare against.
    let term_chars: Vec<Vec<char>> = terms
        .iter()
        .filter(|t| t.chars().count() > 1)
        .map(|t| t.chars().collect::<Vec<_>>())
        .filter(|tc| !tc.is_empty())
        .collect();
    if term_chars.is_empty() {
        return None;
    }

    // Earliest (char) index at which any term begins.
    let mut earliest: Option<usize> = None;
    for tc in &term_chars {
        if let Some(pos) = find_subslice(&lower, tc) {
            earliest = Some(match earliest {
                None => pos,
                Some(existing) => pos.min(existing),
            });
        }
    }
    let center = earliest?;

    let start = center.saturating_sub(SNIPPET_CONTEXT_CHARS);
    let end = (start + SNIPPET_TOTAL_CHARS).min(chars.len());

    let mut out = String::with_capacity(end - start + 4);
    if start > 0 {
        out.push('…');
    }

    // Single left-to-right pass: at each position, wrap the longest matching
    // term in `==...==`. Comparisons use the lowercased window so casing is
    // ignored while the emitted text keeps the original case.
    let mut i = start;
    while i < end {
        let mut matched_len = 0usize;
        for tc in &term_chars {
            let len = tc.len();
            if len > matched_len && i + len <= end && lower[i..i + len] == tc[..] {
                matched_len = len;
            }
        }
        if matched_len > 0 {
            out.push_str("==");
            for ch in &chars[i..i + matched_len] {
                out.push(*ch);
            }
            out.push_str("==");
            i += matched_len;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }

    if end < chars.len() {
        out.push('…');
    }
    Some(out)
}

/// Index of the first occurrence of `needle` in `haystack` (char-indexed),
/// or `None`. Linear scan — haystack slices are short (snippets only).
fn find_subslice(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    (0..=last).find(|&start| haystack[start..start + needle.len()] == needle[..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageContext;
    use chrono::Utc;
    use std::fs;

    fn setup_temp_vault() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-instant-search-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        // for_test sets default_vault_dir to temp/vault — create it and return
        // that path so notes written by tests land inside the scanned area.
        let vault = temp.join("vault");
        fs::create_dir_all(&vault).expect("vault dir");
        let ctx = StorageContext::for_test(&temp);
        (vault, ctx)
    }

    fn write_note(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).expect("write note");
    }

    // ── build_snippet unit tests ─────────────────────────────────────────

    #[test]
    fn snippet_wraps_first_match_with_markers() {
        let snippet = build_snippet("hello world kernel panic", &["kernel".into()]).unwrap();
        assert!(
            snippet.contains("==kernel=="),
            "expected highlighted term, got: {snippet}"
        );
    }

    #[test]
    fn snippet_is_case_insensitive() {
        let snippet = build_snippet("The MMC module is great", &["mmc".into()]).unwrap();
        assert!(
            snippet.contains("==MMC=="),
            "original casing preserved inside markers, got: {snippet}"
        );
    }

    #[test]
    fn snippet_returns_none_when_no_match() {
        assert!(build_snippet("nothing relevant here", &["kernel".into()]).is_none());
    }

    #[test]
    fn snippet_returns_none_for_empty_text() {
        assert!(build_snippet("", &["kernel".into()]).is_none());
    }

    #[test]
    fn snippet_returns_none_for_empty_terms() {
        assert!(build_snippet("some body text", &[]).is_none());
    }

    #[test]
    fn snippet_highlights_cjk_term() {
        let snippet = build_snippet("这是一段关于内核配置的笔记", &["内核".into()]).unwrap();
        assert!(
            snippet.contains("==内核=="),
            "CJK term should be highlighted, got: {snippet}"
        );
    }

    #[test]
    fn snippet_elides_with_ellipsis_when_windowed() {
        // A long body where the match is deep inside; both ends should be cut.
        let body = "a".repeat(300) + "kernel" + &"b".repeat(300);
        let snippet = build_snippet(&body, &["kernel".into()]).unwrap();
        assert!(
            snippet.starts_with('…'),
            "leading ellipsis expected, got: {snippet}"
        );
        assert!(
            snippet.ends_with('…'),
            "trailing ellipsis expected, got: {snippet}"
        );
        assert!(snippet.contains("==kernel=="));
    }

    // ── end-to-end scan tests ────────────────────────────────────────────

    #[test]
    fn instant_search_finds_body_match_without_index() {
        let (vault, ctx) = setup_temp_vault();
        write_note(
            &vault,
            "a.md",
            "# Boot Timeout\nThe kernel fails to boot after 30s.",
        );
        write_note(&vault, "b.md", "# Unrelated\nNothing about the topic.");

        let result = instant_search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "kernel".into(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("instant search");

        assert_eq!(result.total, 1, "exactly one note should match");
        assert_eq!(result.notes[0].title, "Boot Timeout");
        assert!(
            result.notes[0].path.ends_with("a.md"),
            "matched note path should point at a.md: {}",
            result.notes[0].path
        );

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn instant_search_ranks_title_above_body() {
        let (vault, ctx) = setup_temp_vault();
        // title-only match vs body-only match for the same term
        write_note(
            &vault,
            "title.md",
            "# Kernel Config\nrandom unrelated words",
        );
        write_note(
            &vault,
            "body.md",
            "# Some Title\nhere we discuss the kernel internals",
        );

        let result = instant_search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "kernel".into(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("instant search");

        assert_eq!(result.notes.len(), 2);
        assert_eq!(
            result.notes[0].title, "Kernel Config",
            "title match should rank first"
        );

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn instant_search_filters_by_tag() {
        let (vault, ctx) = setup_temp_vault();
        write_note(
            &vault,
            "tagged.md",
            "---\ntitle: Tagged\ntags: [hardware]\n---\nkernel details",
        );
        write_note(
            &vault,
            "untagged.md",
            "---\ntitle: Untagged\ntags: [software]\n---\nkernel details",
        );

        let result = instant_search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "kernel".into(),
                tags: vec!["hardware".into()],
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("instant search");

        assert_eq!(result.total, 1);
        assert_eq!(result.notes[0].title, "Tagged");

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn instant_search_returns_empty_for_blank_query() {
        let (vault, ctx) = setup_temp_vault();
        write_note(&vault, "x.md", "# Anything\nbody");

        let result = instant_search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "   ".into(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("instant search");

        assert_eq!(result.total, 0);
        assert!(result.notes.is_empty());

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn instant_search_supports_cjk_query() {
        let (vault, ctx) = setup_temp_vault();
        write_note(&vault, "cn.md", "# 笔记标题\n这里讨论内核配置的相关内容");

        let result = instant_search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "内核".into(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("instant search");

        assert_eq!(result.total, 1, "CJK query should match");
        assert_eq!(result.notes[0].title, "笔记标题");

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn instant_search_documents_attaches_snippet() {
        let (vault, ctx) = setup_temp_vault();
        write_note(
            &vault,
            "d.md",
            "# Title\nsome preamble text here kernel panic details follow",
        );

        let docs = instant_search_documents_with_context(
            &ctx,
            SearchQuery {
                text: "kernel".into(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("instant search documents");

        assert_eq!(docs.len(), 1);
        let snippet = docs[0].search_snippet.as_deref().expect("snippet present");
        assert!(
            snippet.contains("==kernel=="),
            "snippet should highlight term, got: {snippet}"
        );

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn instant_search_skips_hidden_directories() {
        let (vault, ctx) = setup_temp_vault();
        // A real note at the vault root.
        write_note(&vault, "real.md", "# Real\nkernel content");
        // A "deleted" note lurking in a hidden trash folder.
        let trash = vault.join(".trash");
        fs::create_dir_all(&trash).expect("trash dir");
        write_note(&trash, "deleted.md", "# Deleted\nkernel content");

        let result = instant_search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "kernel".into(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("instant search");

        assert_eq!(
            result.total, 1,
            "hidden-directory note must not appear in results"
        );
        assert!(result.notes[0].path.ends_with("real.md"));

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn instant_search_respects_pagination() {
        let (vault, ctx) = setup_temp_vault();
        for i in 0..5 {
            write_note(
                &vault,
                &format!("n{i}.md"),
                &format!("# Note {i}\nkernel entry number {i}"),
            );
        }

        let page = instant_search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "kernel".into(),
                limit: Some(2),
                offset: Some(1),
                ..Default::default()
            },
        )
        .expect("instant search");

        assert_eq!(page.notes.len(), 2, "limit applied");
        assert_eq!(page.total, 5, "total reflects full filtered set");

        let _ = fs::remove_dir_all(&vault);
    }
}
