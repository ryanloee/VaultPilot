//! Unified diff computation for note editing (#1569, #1652).
//!
//! Implements a line-level diff algorithm (Myers' diff variant) to compute
//! the differences between the original and revised versions of a note.
//! The result can be rendered as a unified diff (patch format) or as a
//! structured representation for UI consumption.
//!
//! # Example
//! ```text
//! --- original
//! +++ revised
//! @@ -1,3 +1,4 @@
//!  Line 1
//! -Line 2
//! +Line 2 (edited)
//! +Line 3 (new)
//!  Line 4
//! ```

use serde::{Deserialize, Serialize};

/// A single line in a diff hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLine {
    /// Unchanged context line.
    Context(String),
    /// Line present in original but removed.
    Delete(String),
    /// Line present in revised but newly added.
    Insert(String),
}

/// A hunk of changes: a contiguous block of insertions/deletions with
/// surrounding context lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    /// Starting line number in the original (1-based).
    pub old_start: usize,
    /// Number of lines in the original hunk.
    pub old_count: usize,
    /// Starting line number in the revised (1-based).
    pub new_start: usize,
    /// Number of lines in the revised hunk.
    pub new_count: usize,
    /// Lines in this hunk (context + delete + insert).
    pub lines: Vec<DiffLine>,
}

/// Complete diff result with all hunks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffResult {
    pub hunks: Vec<DiffHunk>,
    /// Total lines added across all hunks.
    pub additions: usize,
    /// Total lines removed across all hunks.
    pub deletions: usize,
}

impl DiffResult {
    /// Returns true if there are no changes (empty diff).
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    /// Summary string: "N additions, M deletions across K hunks".
    pub fn summary(&self) -> String {
        format!(
            "{} addition{}, {} deletion{}, {} hunk{}",
            self.additions,
            if self.additions == 1 { "" } else { "s" },
            self.deletions,
            if self.deletions == 1 { "" } else { "s" },
            self.hunks.len(),
            if self.hunks.len() == 1 { "" } else { "s" },
        )
    }
}

/// Compute a line-level diff between two texts.
///
/// Uses a longest-common-subsequence (LCS) based approach to identify
/// matching lines, then groups changes into hunks with context.
///
/// # Arguments
/// * `old` - The original text.
/// * `new` - The revised text.
/// * `context` - Number of context lines around each change (default: 3).
pub fn compute_diff(old: &str, new: &str, context: usize) -> DiffResult {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Compute LCS table to identify matching lines.
    let matches = lcs_matches(&old_lines, &new_lines);

    // Convert matches to diff operations.
    let ops = matches_to_ops(&matches, old_lines.len(), new_lines.len());

    // Group ops into hunks with context.
    let hunks = group_into_hunks(&ops, &old_lines, &new_lines, context);

    let additions: usize = hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| matches!(l, DiffLine::Insert(_)))
        .count();
    let deletions: usize = hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| matches!(l, DiffLine::Delete(_)))
        .count();

    DiffResult {
        hunks,
        additions,
        deletions,
    }
}

/// LCS match positions: Vec of (old_idx, new_idx) pairs.
type Matches = Vec<(usize, usize)>;

/// Compute the matching line pairs using LCS dynamic programming.
fn lcs_matches(old: &[&str], new: &[&str]) -> Matches {
    let m = old.len();
    let n = new.len();

    // DP table: dp[i][j] = length of LCS of old[i..] and new[j..]
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            if old[i] == new[j] {
                dp[i][j] = dp[i + 1][j + 1] + 1;
            } else {
                dp[i][j] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    // Backtrack to find matched pairs.
    let mut matches = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m && j < n {
        if old[i] == new[j] {
            matches.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    matches
}

/// A diff operation: match, delete, or insert.
#[derive(Debug, Clone)]
enum DiffOp {
    /// Lines match at (old_idx, new_idx).
    Match(usize, usize),
    /// Lines deleted from old at old_idx.
    Delete(usize),
    /// Lines inserted into new at new_idx.
    Insert(usize),
}

/// Convert LCS matches into a sequence of diff operations.
fn matches_to_ops(matches: &Matches, old_len: usize, new_len: usize) -> Vec<DiffOp> {
    let mut ops = Vec::new();
    let mut oi = 0; // old index
    let mut ni = 0; // new index

    for &(mi, mj) in matches {
        // Deletions: old lines before this match that aren't matched.
        while oi < mi {
            ops.push(DiffOp::Delete(oi));
            oi += 1;
        }
        // Insertions: new lines before this match that aren't matched.
        while ni < mj {
            ops.push(DiffOp::Insert(ni));
            ni += 1;
        }
        // The matched line.
        ops.push(DiffOp::Match(mi, mj));
        oi = mi + 1;
        ni = mj + 1;
    }

    // Remaining deletions.
    while oi < old_len {
        ops.push(DiffOp::Delete(oi));
        oi += 1;
    }
    // Remaining insertions.
    while ni < new_len {
        ops.push(DiffOp::Insert(ni));
        ni += 1;
    }

    ops
}

/// Group diff operations into hunks with surrounding context.
fn group_into_hunks(
    ops: &[DiffOp],
    old_lines: &[&str],
    new_lines: &[&str],
    context: usize,
) -> Vec<DiffHunk> {
    // Find indices of change operations (Delete or Insert).
    let change_indices: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, DiffOp::Match(_, _)))
        .map(|(i, _)| i)
        .collect();

    if change_indices.is_empty() {
        return Vec::new();
    }

    // Group changes into hunks: if two changes are within 2*context of each
    // other, they belong to the same hunk.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current_group = vec![change_indices[0]];

    for &ci in &change_indices[1..] {
        if ci - current_group.last().unwrap() <= 2 * context + 1 {
            current_group.push(ci);
        } else {
            groups.push(std::mem::take(&mut current_group));
            current_group = vec![ci];
        }
    }
    groups.push(current_group);

    // Build hunks from groups.
    let mut hunks = Vec::new();
    for group in &groups {
        let first_change = group[0];
        let last_change = *group.last().unwrap();

        // Expand by context.
        let start = first_change.saturating_sub(context);
        let end = (last_change + context + 1).min(ops.len());

        let mut lines = Vec::new();
        let mut old_start = None;
        let mut new_start = None;
        let mut old_count = 0;
        let mut new_count = 0;

        for op in ops.iter().take(end).skip(start) {
            match op {
                DiffOp::Match(oi, ni) => {
                    if old_start.is_none() {
                        old_start = Some(*oi);
                    }
                    if new_start.is_none() {
                        new_start = Some(*ni);
                    }
                    lines.push(DiffLine::Context(old_lines[*oi].to_string()));
                    old_count += 1;
                    new_count += 1;
                }
                DiffOp::Delete(oi) => {
                    if old_start.is_none() {
                        old_start = Some(*oi);
                    }
                    lines.push(DiffLine::Delete(old_lines[*oi].to_string()));
                    old_count += 1;
                }
                DiffOp::Insert(ni) => {
                    if new_start.is_none() {
                        new_start = Some(*ni);
                    }
                    lines.push(DiffLine::Insert(new_lines[*ni].to_string()));
                    new_count += 1;
                }
            }
        }

        hunks.push(DiffHunk {
            old_start: old_start.map(|s| s + 1).unwrap_or(1), // 1-based
            old_count,
            new_start: new_start.map(|s| s + 1).unwrap_or(1),
            new_count,
            lines,
        });
    }

    hunks
}

/// Render a diff result as unified diff (patch) format.
pub fn render_unified_diff(diff: &DiffResult, old_label: &str, new_label: &str) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(&format!("--- {old_label}\n"));
    out.push_str(&format!("+++ {new_label}\n"));

    for hunk in &diff.hunks {
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        ));
        for line in &hunk.lines {
            match line {
                DiffLine::Context(text) => out.push_str(&format!(" {text}\n")),
                DiffLine::Delete(text) => out.push_str(&format!("-{text}\n")),
                DiffLine::Insert(text) => out.push_str(&format!("+{text}\n")),
            }
        }
    }

    out
}

/// Render a diff with ANSI color codes for terminal output.
pub fn render_colored_diff(diff: &DiffResult) -> String {
    let mut out = String::with_capacity(1024);

    for hunk in &diff.hunks {
        out.push_str(&format!(
            "\x1b[36m@@ -{},{} +{},{} @@\x1b[0m\n",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        ));
        for line in &hunk.lines {
            match line {
                DiffLine::Context(text) => out.push_str(&format!(" {text}\n")),
                DiffLine::Delete(text) => out.push_str(&format!("\x1b[31m-{text}\x1b[0m\n")),
                DiffLine::Insert(text) => out.push_str(&format!("\x1b[32m+{text}\x1b[0m\n")),
            }
        }
    }

    out
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_text_no_diff() {
        let result = compute_diff("line1\nline2\nline3", "line1\nline2\nline3", 3);
        assert!(result.is_empty());
        assert_eq!(result.additions, 0);
        assert_eq!(result.deletions, 0);
    }

    #[test]
    fn test_simple_insertion() {
        let old = "line1\nline2\nline3";
        let new = "line1\nline2\ninserted\nline3";
        let result = compute_diff(old, new, 3);
        assert_eq!(result.additions, 1);
        assert_eq!(result.deletions, 0);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_simple_deletion() {
        let old = "line1\ndeleted\nline2\nline3";
        let new = "line1\nline2\nline3";
        let result = compute_diff(old, new, 3);
        assert_eq!(result.additions, 0);
        assert_eq!(result.deletions, 1);
    }

    #[test]
    fn test_modification() {
        let old = "line1\nold\nline3";
        let new = "line1\nnew\nline3";
        let result = compute_diff(old, new, 3);
        assert_eq!(result.additions, 1);
        assert_eq!(result.deletions, 1);
    }

    #[test]
    fn test_empty_old() {
        let result = compute_diff("", "new content", 3);
        assert_eq!(result.additions, 1);
        assert_eq!(result.deletions, 0);
    }

    #[test]
    fn test_empty_new() {
        let result = compute_diff("old content", "", 3);
        assert_eq!(result.additions, 0);
        assert_eq!(result.deletions, 1);
    }

    #[test]
    fn test_both_empty() {
        let result = compute_diff("", "", 3);
        assert!(result.is_empty());
    }

    #[test]
    fn test_summary_format() {
        let result = compute_diff("a\nb\nc", "a\nx\nc", 3);
        let summary = result.summary();
        assert!(summary.contains("1 addition"));
        assert!(summary.contains("1 deletion"));
        assert!(summary.contains("1 hunk"));
    }

    #[test]
    fn test_summary_plural() {
        let result = compute_diff("a\nb", "x\ny\nz", 3);
        let summary = result.summary();
        assert!(summary.contains("additions"));
    }

    #[test]
    fn test_unified_diff_format() {
        let old = "line1\nline2\nline3";
        let new = "line1\nedited\nline3";
        let diff = compute_diff(old, new, 3);
        let output = render_unified_diff(&diff, "original", "revised");
        assert!(output.starts_with("--- original\n"));
        assert!(output.contains("+++ revised\n"));
        assert!(output.contains("@@ "));
        assert!(output.contains("-line2"));
        assert!(output.contains("+edited"));
        assert!(output.contains(" line1"));
        assert!(output.contains(" line3"));
    }

    #[test]
    fn test_colored_diff_has_ansi_codes() {
        let old = "line1\nline2";
        let new = "line1\nedited";
        let diff = compute_diff(old, new, 3);
        let output = render_colored_diff(&diff);
        assert!(output.contains("\x1b[31m")); // red for deletion
        assert!(output.contains("\x1b[32m")); // green for insertion
        assert!(output.contains("\x1b[36m")); // cyan for hunk header
        assert!(output.contains("\x1b[0m")); // reset
    }

    #[test]
    fn test_context_grouping() {
        // Changes far apart should create separate hunks.
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10";
        let new = "1x\n2\n3\n4\n5\n6\n7\n8\n9\n10x";
        let result = compute_diff(old, new, 2);
        // Two changes at line 1 and line 10 with context=2 → should be 2 hunks
        // (gap between them is 8 lines > 2*context+1 = 5)
        assert_eq!(result.hunks.len(), 2);
    }

    #[test]
    fn test_context_merges_close_changes() {
        let old = "1\n2\n3\n4\n5";
        let new = "1x\n2\n3\n4\n5x";
        let result = compute_diff(old, new, 3);
        // Changes at line 1 and 5 with context=3 → should be 1 hunk
        assert_eq!(result.hunks.len(), 1);
    }

    #[test]
    fn test_diff_result_serialization() {
        let result = compute_diff("a\nb\nc", "a\nx\nc", 3);
        let json = serde_json::to_string(&result).unwrap();
        let parsed: DiffResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.additions, 1);
        assert_eq!(parsed.deletions, 1);
    }

    #[test]
    fn test_multiline_insertion() {
        let old = "line1\nline2";
        let new = "line1\nnew1\nnew2\nnew3\nline2";
        let result = compute_diff(old, new, 3);
        assert_eq!(result.additions, 3);
        assert_eq!(result.deletions, 0);
    }

    #[test]
    fn test_reordering_detected_as_delete_insert() {
        // LCS-based diff treats reordering as delete + insert.
        let old = "a\nb\nc";
        let new = "c\nb\na";
        let result = compute_diff(old, new, 3);
        // Some lines match (b), others are delete+insert
        assert!(result.additions > 0);
        assert!(result.deletions > 0);
    }
}
