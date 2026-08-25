//! Quick Switcher fuzzy matching & ranking engine — Issue #3086.
//!
//! Provides the backend scoring/ranking layer for a VS Code / Obsidian-style
//! Quick Switcher (Ctrl+K) and Command Palette (Ctrl+Shift+K). The engine is
//! pure and fully unit-testable; the WinUI / mobile UI layers wrap it to render
//! ranked results.
//!
//! - [`fuzzy_score`] — scores how well `query` matches `target` as a
//!   subsequence (fzf-style: bonuses for word boundaries, camelCase edges,
//!   consecutive chars, and prefix matches; penalties for gaps). Returns `None`
//!   when `query` is not a subsequence of `target` (case-insensitively).
//! - [`SwitcherCandidate`] — a single selectable item (note, tag, command,
//!   recent file).
//! - [`rank_candidates`] — scores and sorts a batch of candidates, applying
//!   type-priority and recency tie-breakers so that recent files float to the
//!   top for an empty query.
//!
//! ## Scoring intuition
//!
//! Higher scores are better. An exact prefix match (`abc` in `abcdef`) scores
//! far higher than a scattered subsequence (`abc` in `xaybzc`). Matches at word
//! boundaries (`/`, `-`, `_`, space, camelCase edges) are boosted, so
//! `set` matches `settings.rs` strongly. The algorithm is a dynamic-programming
//! Smith-Waterman variant (O(m·n²) per candidate, negligible for typical
//! quick-switcher inputs where both strings are short).

use std::collections::HashMap;

/// Characters that delimit "words" for boundary-bonus purposes.
const SEPARATORS: &[char] = &[' ', '-', '_', '/', '.', '\\', '(', ')', '[', ']', '\t'];

// --- Tunable scoring constants (fzf-flavoured) -----------------------------
/// Base score contributed by every matched character.
const SCORE_MATCH: i64 = 16;
/// Bonus when the match lands on the very first character of the target.
const BONUS_FIRST_CHAR: i64 = 100;
/// Bonus when the matched character follows a separator (word boundary).
const BONUS_BOUNDARY: i64 = 90;
/// Bonus at a camelCase edge (lowercase → uppercase transition).
const BONUS_CAMEL: i64 = 60;
/// Bonus added when a match is consecutive with the previous matched char.
const BONUS_CONSECUTIVE: i64 = 30;
/// Penalty per skipped character *between* two matches.
const PENALTY_GAP: i64 = -5;
/// Penalty per skipped character *before* the first match.
const PENALTY_LEADING_GAP: i64 = -3;

// --- Candidate-ranking constants -------------------------------------------
/// Recency boost applied per rank (most-recent file gets the largest boost).
const RECENCY_BOOST: i64 = 50;
/// Type-priority bonuses applied on top of the fuzzy score / recency.
const TYPE_RECENT: i64 = 400;
const TYPE_NOTE: i64 = 80;
const TYPE_TAG: i64 = 60;
const TYPE_COMMAND: i64 = 40;

/// Sentinel used for "no possible score" inside the DP. Kept well away from
/// `i64::MIN` so saturating additions never overflow.
const NEG_INF: i64 = i64::MIN / 4;

/// Score how well `query` matches `target` as a case-insensitive subsequence.
///
/// Returns `Some(score)` when every character of `query` appears in `target`
/// in order (not necessarily contiguously), or `None` otherwise. An empty
/// query matches every target with a neutral score of `0`.
///
/// Higher scores indicate better (denser, earlier, boundary-aligned) matches.
pub fn fuzzy_score(query: &str, target: &str) -> Option<i64> {
    let q: Vec<char> = query.chars().map(|c| c.to_ascii_lowercase()).collect();
    let t: Vec<char> = target.chars().map(|c| c.to_ascii_lowercase()).collect();
    let (m, n) = (q.len(), t.len());

    if m == 0 {
        return Some(0);
    }
    if m > n {
        return None;
    }

    // Position bonuses are computed from the *original* target so that
    // camelCase edges survive lowercasing.
    let orig: Vec<char> = target.chars().collect();
    let bonus: Vec<i64> = (0..n).map(|j| position_bonus(&orig, j)).collect();

    // DP row for query character 0: the last (and only) match is at target[j].
    let mut prev: Vec<i64> = vec![NEG_INF; n];
    for j in 0..n {
        if q[0] == t[j] {
            prev[j] = SCORE_MATCH + bonus[j] + (j as i64) * PENALTY_LEADING_GAP;
        }
    }

    // Rows for query characters 1..m.
    for (i, qi) in q.iter().enumerate().skip(1) {
        let mut cur = vec![NEG_INF; n];
        // Position j must leave room for the i earlier query characters.
        for j in i..n {
            if *qi != t[j] {
                continue;
            }
            let char_score = SCORE_MATCH + bonus[j];
            let mut best_prev = NEG_INF;
            // Previous match at some position k < j.
            for (k, prev_val) in prev.iter().enumerate().take(j).skip(i - 1) {
                if *prev_val == NEG_INF {
                    continue;
                }
                let gap = (j - k - 1) as i64;
                let mut s = prev_val.saturating_add(char_score);
                if gap == 0 {
                    s = s.saturating_add(BONUS_CONSECUTIVE);
                } else {
                    s = s.saturating_add(ggap_penalty(gap));
                }
                if s > best_prev {
                    best_prev = s;
                }
            }
            cur[j] = best_prev;
        }
        prev = cur;
    }

    let result = prev.iter().copied().max().unwrap_or(NEG_INF);
    if result <= NEG_INF {
        None
    } else {
        Some(result)
    }
}

/// Per-position bonus for matching `target[j]`, based on surrounding context.
fn position_bonus(target: &[char], j: usize) -> i64 {
    if j == 0 {
        return BONUS_FIRST_CHAR;
    }
    let prev = target[j - 1];
    let cur = target[j];
    if is_separator(prev) {
        return BONUS_BOUNDARY;
    }
    if prev.is_ascii_lowercase() && cur.is_ascii_uppercase() {
        return BONUS_CAMEL;
    }
    // digit ↔ letter transitions are also natural word boundaries
    if prev.is_ascii_digit() && cur.is_ascii_alphabetic()
        || prev.is_ascii_alphabetic() && cur.is_ascii_digit()
    {
        return BONUS_BOUNDARY;
    }
    0
}

/// Whether `c` is a word separator for boundary-bonus purposes.
fn is_separator(c: char) -> bool {
    SEPARATORS.contains(&c)
}

/// Interior-gap penalty for `gap` skipped characters between two matches.
fn ggap_penalty(gap: i64) -> i64 {
    gap.saturating_mul(PENALTY_GAP)
}

/// The kind of a [`SwitcherCandidate`], used for type-priority tie-breaking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateKind {
    /// A vault note (matched by title / filename / path).
    Note,
    /// A tag found across the vault.
    Tag,
    /// A palette command (e.g. "New note", "Toggle theme").
    Command,
    /// A recently-opened file (always floats near the top).
    RecentFile,
}

/// A single selectable Quick Switcher item.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitcherCandidate {
    /// Discriminator used for type-priority tie-breaking.
    pub kind: CandidateKind,
    /// Primary display string and fuzzy-match key (note title, tag, label…).
    pub title: String,
    /// Optional secondary match key / underlying vault path for notes.
    pub path: Option<String>,
}

impl SwitcherCandidate {
    /// Build a note candidate from a title and vault path.
    pub fn note(title: impl Into<String>, path: impl Into<String>) -> Self {
        SwitcherCandidate {
            kind: CandidateKind::Note,
            title: title.into(),
            path: Some(path.into()),
        }
    }

    /// Build a tag candidate.
    pub fn tag(name: impl Into<String>) -> Self {
        SwitcherCandidate {
            kind: CandidateKind::Tag,
            title: name.into(),
            path: None,
        }
    }

    /// Build a command-palette candidate.
    pub fn command(label: impl Into<String>) -> Self {
        SwitcherCandidate {
            kind: CandidateKind::Command,
            title: label.into(),
            path: None,
        }
    }

    /// Build a recent-file candidate.
    pub fn recent_file(title: impl Into<String>, path: impl Into<String>) -> Self {
        SwitcherCandidate {
            kind: CandidateKind::RecentFile,
            title: title.into(),
            path: Some(path.into()),
        }
    }
}

/// A candidate paired with its computed score, returned by [`rank_candidates`].
#[derive(Debug, Clone)]
pub struct RankedCandidate {
    /// The original candidate (owned copy).
    pub candidate: SwitcherCandidate,
    /// Higher is better.
    pub score: i64,
}

/// Score, boost, and sort `candidates` for `query`.
///
/// For a non-empty query each candidate is fuzzy-matched against its title
/// (falling back to its path); non-matches are dropped. For an empty query every
/// candidate is retained and ranked purely by recency and type priority, so
/// recently-opened files appear first — matching the behaviour of VS Code's and
/// Obsidian's switchers.
///
/// `recent_paths` is ordered most-recent-first; candidates whose `path` appears
/// earlier receive a larger recency boost.
pub fn rank_candidates(
    query: &str,
    candidates: &[SwitcherCandidate],
    recent_paths: &[String],
) -> Vec<RankedCandidate> {
    let q = query.trim();
    let empty_query = q.is_empty();

    // Precompute recency rank for O(1) lookup.
    let recency_rank: HashMap<&str, usize> = recent_paths
        .iter()
        .enumerate()
        .map(|(idx, p)| (p.as_str(), idx))
        .collect();

    let mut ranked: Vec<RankedCandidate> = candidates
        .iter()
        .filter_map(|c| {
            let fuzzy = if empty_query {
                0
            } else {
                let primary = fuzzy_score(q, &c.title).unwrap_or(NEG_INF);
                let secondary = c
                    .path
                    .as_ref()
                    .and_then(|p| fuzzy_score(q, p))
                    .unwrap_or(NEG_INF);
                primary.max(secondary)
            };

            if !empty_query && fuzzy <= NEG_INF {
                return None;
            }

            let recency = c
                .path
                .as_ref()
                .and_then(|p| recency_rank.get(p.as_str()).copied())
                .map(|idx| (recent_paths.len().saturating_sub(idx) as i64) * RECENCY_BOOST)
                .unwrap_or(0);

            let type_bonus = match c.kind {
                CandidateKind::RecentFile => TYPE_RECENT,
                CandidateKind::Note => TYPE_NOTE,
                CandidateKind::Tag => TYPE_TAG,
                CandidateKind::Command => TYPE_COMMAND,
            };

            Some(RankedCandidate {
                candidate: c.clone(),
                score: fuzzy + recency + type_bonus,
            })
        })
        .collect();

    // Sort descending by score; ties broken alphabetically by title for stability.
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.candidate.title.cmp(&b.candidate.title))
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- fuzzy_score basics ----------------

    #[test]
    fn empty_query_matches_everything_neutrally() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert_eq!(fuzzy_score("", ""), Some(0));
    }

    #[test]
    fn non_subsequence_returns_none() {
        assert_eq!(fuzzy_score("xyz", "abc"), None);
        assert_eq!(fuzzy_score("ab", "ba"), None); // wrong order
        assert_eq!(fuzzy_score("longerquery", "short"), None); // query longer
    }

    #[test]
    fn case_insensitive_match() {
        assert!(fuzzy_score("ABC", "abcdef").is_some());
        assert!(fuzzy_score("abc", "ABCDEF").is_some());
        assert!(fuzzy_score("Set", "settings.rs").is_some());
    }

    // ---------------- scoring ordering properties ----------------

    #[test]
    fn exact_prefix_beats_scattered_match() {
        let prefix = fuzzy_score("abc", "abcdef").unwrap();
        let scattered = fuzzy_score("abc", "xaybzc").unwrap();
        assert!(
            prefix > scattered,
            "prefix {prefix} should beat scattered {scattered}"
        );
    }

    #[test]
    fn contiguous_match_beats_gappy_match() {
        // Both are subsequences; the contiguous one should score higher.
        // (Targets without separators isolate the gap penalty from boundary
        // bonuses, so the comparison is clean.)
        let tight = fuzzy_score("set", "settings").unwrap();
        let gappy = fuzzy_score("set", "sxextxt").unwrap();
        assert!(tight > gappy, "tight {tight} > gappy {gappy}");
    }

    #[test]
    fn word_boundary_beats_mid_word() {
        // "set" aligns with a boundary in "offset_value" (after "of") and in
        // "set_value" (at the start). The start-of-string one should win.
        let start = fuzzy_score("set", "set_value").unwrap();
        let mid = fuzzy_score("set", "offset_value").unwrap();
        assert!(start > mid, "start {start} > mid {mid}");
    }

    #[test]
    fn camelcase_boundary_is_boosted() {
        // "VP" aligns with the camelCase edge in "VaultPilot" (V at 0, P at the
        // lower→upper boundary) but in the flattened "vaultpilot" there is no
        // camelCase edge, so the boundary bonus is absent.
        let camel = fuzzy_score("vp", "VaultPilot").unwrap();
        let flat = fuzzy_score("vp", "vaultpilot").unwrap();
        assert!(camel > flat, "camel {camel} > flat {flat}");
    }

    #[test]
    fn shorter_target_scores_higher_for_same_match() {
        // Denser match in a shorter string beats a sparse match in a long one.
        let short = fuzzy_score("ab", "ab").unwrap();
        let long = fuzzy_score("ab", "axxxxxxxxxxxxb").unwrap();
        assert!(short > long, "short {short} > long {long}");
    }

    #[test]
    fn more_consecutive_chars_score_higher() {
        let full = fuzzy_score("vault", "vaultpilot").unwrap();
        let partial = fuzzy_score("vlt", "vaultpilot").unwrap();
        // Matching more of the prefix should produce a higher per-signal score;
        // both are positive, full prefix is strictly better.
        assert!(full > partial, "full {full} > partial {partial}");
    }

    // ---------------- rank_candidates ----------------

    #[test]
    fn empty_query_ranks_by_recency_then_type() {
        let cands = vec![
            SwitcherCandidate::note("Zebra", "z.md"),
            SwitcherCandidate::recent_file("Alpha", "a.md"),
            SwitcherCandidate::note("Mango", "m.md"),
        ];
        let recent = vec!["a.md".to_string(), "m.md".to_string()];
        let ranked = rank_candidates("", &cands, &recent);
        // Alpha is a RecentFile AND most recent → must come first.
        assert_eq!(ranked[0].candidate.title, "Alpha");
        // Mango (recent note) before Zebra (non-recent note).
        assert_eq!(ranked[1].candidate.title, "Mango");
        assert_eq!(ranked[2].candidate.title, "Zebra");
    }

    #[test]
    fn non_matching_candidates_dropped_for_query() {
        let cands = vec![
            SwitcherCandidate::note("Settings", "settings.md"),
            SwitcherCandidate::note("Zebra", "z.md"),
            SwitcherCandidate::note("Server Config", "server.md"),
        ];
        let ranked = rank_candidates("set", &cands, &[]);
        let titles: Vec<_> = ranked.iter().map(|r| r.candidate.title.as_str()).collect();
        assert!(titles.contains(&"Settings"));
        // "Server Config" also contains s-e-t as subsequence but much worse.
        // "Zebra" does not match at all → dropped.
        assert!(!titles.contains(&"Zebra"));
        assert_eq!(ranked[0].candidate.title, "Settings");
    }

    #[test]
    fn path_fallback_matches_when_title_misses() {
        let cands = vec![SwitcherCandidate::note(
            "My Daily Log",
            "journals/2026/sep.md",
        )];
        // Query matches the path but not the title strongly.
        let ranked = rank_candidates("sep", &cands, &[]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].candidate.title, "My Daily Log");
    }

    #[test]
    fn recency_breaks_tie_among_equal_fuzzy_scores() {
        // Two notes with identical fuzzy profiles; the recent one floats up.
        let cands = vec![
            SwitcherCandidate::note("alpha", "a1.md"),
            SwitcherCandidate::note("alpha", "a2.md"),
        ];
        let recent = vec!["a2.md".to_string()];
        let ranked = rank_candidates("alpha", &cands, &recent);
        assert_eq!(ranked[0].candidate.path.as_deref(), Some("a2.md"));
    }

    #[test]
    fn command_candidates_ranked_by_label() {
        let cands = vec![
            SwitcherCandidate::command("Toggle Theme"),
            SwitcherCandidate::command("New Note"),
            SwitcherCandidate::command("New Folder"),
        ];
        let ranked = rank_candidates("new", &cands, &[]);
        let titles: Vec<_> = ranked.iter().map(|r| r.candidate.title.as_str()).collect();
        // Both "New Note" and "New Folder" match with equal fuzzy scores; the
        // alphabetical tie-break puts "New Folder" before "New Note".
        assert!(titles.starts_with(&["New Folder", "New Note"]));
        // "Toggle Theme" is dropped (no "new" subsequence).
        assert!(!titles.contains(&"Toggle Theme"));
    }

    #[test]
    fn tag_candidates_match_by_name() {
        let cands = vec![
            SwitcherCandidate::tag("rust"),
            SwitcherCandidate::tag("ai"),
            SwitcherCandidate::tag("research"),
        ];
        let ranked = rank_candidates("rus", &cands, &[]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].candidate.title, "rust");
    }

    #[test]
    fn unicode_titles_match() {
        // Regression: must operate on chars, not bytes (UTF-8 safety).
        let cands = vec![SwitcherCandidate::note("笔记/项目", "notes/proj.md")];
        let ranked = rank_candidates("项", &cands, &[]);
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn mixed_kinds_ranked_together() {
        let cands = vec![
            SwitcherCandidate::note("settings.md", "settings.md"),
            SwitcherCandidate::command("Open Settings"),
            SwitcherCandidate::tag("settings"),
        ];
        let ranked = rank_candidates("settings", &cands, &[]);
        assert_eq!(ranked.len(), 3);
        // All three match; ordering by score then title is deterministic.
        let titles: Vec<_> = ranked.iter().map(|r| r.candidate.title.as_str()).collect();
        assert!(titles.contains(&"settings.md"));
        assert!(titles.contains(&"Open Settings"));
        assert!(titles.contains(&"settings"));
    }

    #[test]
    fn more_interior_gaps_strictly_lower_score() {
        // Adding interior gaps between matches strictly decreases the score.
        let zero_gap = fuzzy_score("ab", "ab").unwrap();
        let one_gap = fuzzy_score("ab", "axb").unwrap();
        let two_gap = fuzzy_score("ab", "axxb").unwrap();
        assert!(zero_gap > one_gap, "{zero_gap} > {one_gap}");
        assert!(one_gap > two_gap, "{one_gap} > {two_gap}");
    }
}
