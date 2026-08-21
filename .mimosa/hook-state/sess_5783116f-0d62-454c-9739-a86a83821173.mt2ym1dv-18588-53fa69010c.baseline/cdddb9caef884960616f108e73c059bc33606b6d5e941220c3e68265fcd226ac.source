//! Markdown Table Parser & Editor — structured manipulation of pipe-delimited
//! tables in note bodies (#3685).
//!
//! Provides:
//! - [`parse_markdown_table`] — parse a markdown table block into [`TableData`]
//! - [`TableData::to_markdown`] — serialize back to aligned markdown
//! - Row/column add/remove operations
//! - Alignment-aware formatting (left / center / right)
//!
//! This module is the backend foundation for the visual table editor.  The
//! WinUI and Mobile front-ends can consume these utilities via the HTTP bridge
//! or call the library directly.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Column alignment in a markdown table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// A parsed markdown table with header, rows, and per-column alignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableData {
    /// Header cell labels.
    pub headers: Vec<String>,
    /// Per-column alignment (same length as `headers`).
    pub alignments: Vec<ColumnAlignment>,
    /// Body rows; each row has the same number of cells as `headers`.
    pub rows: Vec<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a markdown table from a text block.
///
/// Expects at minimum a header row and a separator row (`|---|---|`).
/// Returns `None` if the input does not contain a valid markdown table.
///
/// ```
/// # use vaultpilot_lib::markdown_table::parse_markdown_table;
/// let md = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n";
/// let table = parse_markdown_table(md).unwrap();
/// assert_eq!(table.headers, vec!["Name", "Age"]);
/// assert_eq!(table.rows.len(), 1);
/// ```
pub fn parse_markdown_table(text: &str) -> Option<TableData> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.len() < 2 {
        return None;
    }

    // The first line must look like a table row (contains |).
    if !lines[0].contains('|') {
        return None;
    }

    // The second line must be a separator (---, :--, :-:, --:).
    if !is_separator_line(lines[1]) {
        return None;
    }

    let headers = split_row(lines[0]);
    if headers.is_empty() {
        return None;
    }

    let alignments = parse_alignments(lines[1], headers.len());

    let mut rows = Vec::new();
    for line in &lines[2..] {
        if !line.contains('|') {
            break; // table ended
        }
        let cells = split_row(line);
        // Pad or trim to match header column count.
        let normalized = normalize_row(cells, headers.len());
        rows.push(normalized);
    }

    Some(TableData {
        headers,
        alignments,
        rows,
    })
}

impl TableData {
    /// Serialize the table back to aligned markdown text.
    pub fn to_markdown(&self) -> String {
        if self.headers.is_empty() {
            return String::new();
        }

        let num_cols = self.headers.len();

        // Compute column widths (max cell width per column).
        let mut widths = vec![0usize; num_cols];
        for (i, h) in self.headers.iter().enumerate() {
            widths[i] = display_width(h);
        }
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate().take(num_cols) {
                let w = display_width(cell);
                if w > widths[i] {
                    widths[i] = w;
                }
            }
        }

        // Minimum width 3 (for ---).
        for w in &mut widths {
            *w = (*w).max(3);
        }

        let mut out = String::new();

        // Header row
        out.push('|');
        for (i, h) in self.headers.iter().enumerate() {
            out.push(' ');
            out.push_str(h);
            out.push_str(&" ".repeat(widths[i].saturating_sub(display_width(h))));
            out.push_str(" |");
        }
        out.push('\n');

        // Separator row
        out.push('|');
        for (i, align) in self.alignments.iter().enumerate().take(num_cols) {
            let w = widths[i];
            match align {
                ColumnAlignment::Left => {
                    out.push(':');
                    out.push_str(&"-".repeat(w.saturating_sub(1)));
                }
                ColumnAlignment::Center => {
                    out.push(':');
                    out.push_str(&"-".repeat(w.saturating_sub(2)));
                    out.push(':');
                }
                ColumnAlignment::Right => {
                    out.push_str(&"-".repeat(w.saturating_sub(1)));
                    out.push(':');
                }
            }
            out.push('|');
        }
        out.push('\n');

        // Data rows
        for row in &self.rows {
            out.push('|');
            for (i, cell) in row.iter().enumerate().take(num_cols) {
                out.push(' ');
                out.push_str(cell);
                out.push_str(&" ".repeat(widths[i].saturating_sub(display_width(cell))));
                out.push_str(" |");
            }
            out.push('\n');
        }

        out
    }

    /// Add a new row at the end.  Empty cells are filled with empty strings.
    pub fn add_row(&mut self, cells: Vec<String>) {
        let normalized = normalize_row(cells, self.headers.len());
        self.rows.push(normalized);
    }

    /// Insert a row at the given index.  Panics if `idx > rows.len()`.
    pub fn insert_row(&mut self, idx: usize, cells: Vec<String>) {
        let normalized = normalize_row(cells, self.headers.len());
        self.rows.insert(idx, normalized);
    }

    /// Remove the row at the given index.  Returns the removed row.
    pub fn remove_row(&mut self, idx: usize) -> Option<Vec<String>> {
        if idx < self.rows.len() {
            Some(self.rows.remove(idx))
        } else {
            None
        }
    }

    /// Add a new column with the given header.  Existing rows get an empty cell.
    pub fn add_column(&mut self, header: &str) {
        self.headers.push(header.to_string());
        self.alignments.push(ColumnAlignment::Left);
        for row in &mut self.rows {
            row.push(String::new());
        }
    }

    /// Remove the column at the given index.
    pub fn remove_column(&mut self, idx: usize) {
        if idx >= self.headers.len() {
            return;
        }
        self.headers.remove(idx);
        if idx < self.alignments.len() {
            self.alignments.remove(idx);
        }
        for row in &mut self.rows {
            if idx < row.len() {
                row.remove(idx);
            }
        }
    }

    /// Create a new empty table with the given dimensions.
    pub fn new(num_rows: usize, num_cols: usize) -> Self {
        let headers = (0..num_cols).map(|i| format!("Column {}", i + 1)).collect();
        let alignments = vec![ColumnAlignment::Left; num_cols];
        let rows = (0..num_rows)
            .map(|_| vec![String::new(); num_cols])
            .collect();
        TableData {
            headers,
            alignments,
            rows,
        }
    }

    /// Render as a new markdown table string with `num_rows` and `num_cols`.
    pub fn generate_template(num_rows: usize, num_cols: usize) -> String {
        TableData::new(num_rows, num_cols).to_markdown()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Split a table row into trimmed cells, stripping leading/trailing pipes.
fn split_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed.trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

/// Check if a line is a markdown table separator (e.g., `|---|:--|--:|`).
fn is_separator_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('|') && !trimmed.contains('-') {
        return false;
    }
    let inner = trimmed.trim_start_matches('|').trim_end_matches('|');
    inner.split('|').all(|cell| {
        let c = cell.trim();
        // Valid: "---", ":--", "--:", ":-:", ":", "-"
        !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':') && c.contains('-')
    })
}

/// Parse column alignments from a separator row.
fn parse_alignments(separator: &str, num_cols: usize) -> Vec<ColumnAlignment> {
    let cells = split_row(separator);
    let mut aligns = Vec::with_capacity(num_cols);
    for i in 0..num_cols {
        let cell = cells.get(i).map(|s| s.as_str()).unwrap_or("");
        let trimmed = cell.trim();
        let has_left = trimmed.starts_with(':');
        let has_right = trimmed.ends_with(':');
        match (has_left, has_right) {
            (true, true) => aligns.push(ColumnAlignment::Center),
            (true, false) => aligns.push(ColumnAlignment::Left),
            (false, true) => aligns.push(ColumnAlignment::Right),
            (false, false) => aligns.push(ColumnAlignment::Left),
        }
    }
    aligns
}

/// Pad or trim a row to exactly `n` cells.
fn normalize_row(cells: Vec<String>, n: usize) -> Vec<String> {
    let mut v = cells;
    while v.len() < n {
        v.push(String::new());
    }
    v.truncate(n);
    v
}

/// Get the display width of a string (character count, not byte count).
fn display_width(s: &str) -> usize {
    s.chars().count()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_table() {
        let md = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |\n";
        let table = parse_markdown_table(md).unwrap();
        assert_eq!(table.headers, vec!["Name", "Age"]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0], vec!["Alice", "30"]);
        assert_eq!(table.rows[1], vec!["Bob", "25"]);
    }

    #[test]
    fn test_parse_no_leading_pipe() {
        let md = "Name | Age\n------|-----\nAlice | 30\n";
        let table = parse_markdown_table(md).unwrap();
        assert_eq!(table.headers, vec!["Name", "Age"]);
        assert_eq!(table.rows.len(), 1);
    }

    #[test]
    fn test_parse_alignments() {
        let md = "| L | C | R |\n|:--|:-:|--:|\n| a | b | c |\n";
        let table = parse_markdown_table(md).unwrap();
        assert_eq!(table.alignments[0], ColumnAlignment::Left);
        assert_eq!(table.alignments[1], ColumnAlignment::Center);
        assert_eq!(table.alignments[2], ColumnAlignment::Right);
    }

    #[test]
    fn test_parse_not_a_table() {
        assert!(parse_markdown_table("Hello world").is_none());
        assert!(parse_markdown_table("| Header |\nNot a separator").is_none());
        assert!(parse_markdown_table("").is_none());
        assert!(parse_markdown_table("single line").is_none());
    }

    #[test]
    fn test_to_markdown_roundtrip() {
        let md = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n";
        let table = parse_markdown_table(md).unwrap();
        let out = table.to_markdown();
        let reparsed = parse_markdown_table(&out).unwrap();
        assert_eq!(reparsed.headers, table.headers);
        assert_eq!(reparsed.rows, table.rows);
    }

    #[test]
    fn test_to_markdown_alignment() {
        let table = TableData {
            headers: vec!["L".to_string(), "C".to_string(), "R".to_string()],
            alignments: vec![
                ColumnAlignment::Left,
                ColumnAlignment::Center,
                ColumnAlignment::Right,
            ],
            rows: vec![vec!["a".to_string(), "b".to_string(), "c".to_string()]],
        };
        let md = table.to_markdown();
        assert!(md.contains(":--"));
        assert!(md.contains(":-:"));
        assert!(md.contains("--:"));
    }

    #[test]
    fn test_add_row() {
        let mut table = TableData {
            headers: vec!["A".to_string(), "B".to_string()],
            alignments: vec![ColumnAlignment::Left, ColumnAlignment::Left],
            rows: vec![],
        };
        table.add_row(vec!["1".to_string(), "2".to_string()]);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0], vec!["1", "2"]);
    }

    #[test]
    fn test_add_row_pads() {
        let mut table = TableData {
            headers: vec!["A".to_string(), "B".to_string(), "C".to_string()],
            alignments: vec![ColumnAlignment::Left; 3],
            rows: vec![],
        };
        table.add_row(vec!["only one".to_string()]);
        assert_eq!(table.rows[0].len(), 3);
        assert_eq!(table.rows[0][2], "");
    }

    #[test]
    fn test_remove_row() {
        let mut table = TableData {
            headers: vec!["A".to_string()],
            alignments: vec![ColumnAlignment::Left],
            rows: vec![vec!["1".to_string()], vec!["2".to_string()]],
        };
        let removed = table.remove_row(0);
        assert_eq!(removed, Some(vec!["1".to_string()]));
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0], vec!["2"]);
    }

    #[test]
    fn test_add_column() {
        let mut table = TableData {
            headers: vec!["A".to_string()],
            alignments: vec![ColumnAlignment::Left],
            rows: vec![vec!["1".to_string()]],
        };
        table.add_column("B");
        assert_eq!(table.headers, vec!["A", "B"]);
        assert_eq!(table.rows[0], vec!["1", ""]);
    }

    #[test]
    fn test_remove_column() {
        let mut table = TableData {
            headers: vec!["A".to_string(), "B".to_string()],
            alignments: vec![ColumnAlignment::Left, ColumnAlignment::Left],
            rows: vec![vec!["1".to_string(), "2".to_string()]],
        };
        table.remove_column(0);
        assert_eq!(table.headers, vec!["B"]);
        assert_eq!(table.rows[0], vec!["2"]);
    }

    #[test]
    fn test_generate_template() {
        let md = TableData::generate_template(2, 3);
        let table = parse_markdown_table(&md).unwrap();
        assert_eq!(table.headers.len(), 3);
        assert_eq!(table.rows.len(), 2);
        assert!(table.headers[0].contains("Column 1"));
    }

    #[test]
    fn test_new_table() {
        let table = TableData::new(3, 2);
        assert_eq!(table.headers.len(), 2);
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0].len(), 2);
    }

    #[test]
    fn test_table_with_cjk_chars() {
        // CJK characters have display width ~2 but char count ~1.
        // Ensure parsing handles them without panic.
        let md = "| 名前 | 年齢 |\n|------|------|\n| アリス | 30 |\n";
        let table = parse_markdown_table(md).unwrap();
        assert_eq!(table.headers, vec!["名前", "年齢"]);
        assert_eq!(table.rows[0], vec!["アリス", "30"]);
        // Round-trip
        let out = table.to_markdown();
        assert!(parse_markdown_table(&out).is_some());
    }

    #[test]
    fn test_insert_row() {
        let mut table = TableData {
            headers: vec!["A".to_string()],
            alignments: vec![ColumnAlignment::Left],
            rows: vec![vec!["1".to_string()], vec!["3".to_string()]],
        };
        table.insert_row(1, vec!["2".to_string()]);
        assert_eq!(table.rows[1], vec!["2"]);
        assert_eq!(table.rows[2], vec!["3"]);
    }

    #[test]
    fn test_parse_uneven_columns() {
        let md = "| A | B | C |\n|---|---|---|\n| 1 | 2 |\n";
        let table = parse_markdown_table(md).unwrap();
        assert_eq!(table.headers.len(), 3);
        assert_eq!(table.rows[0].len(), 3);
        assert_eq!(table.rows[0][2], ""); // padded
    }

    #[test]
    fn test_serialization_roundtrip() {
        let table = TableData {
            headers: vec!["A".to_string(), "B".to_string()],
            alignments: vec![ColumnAlignment::Center, ColumnAlignment::Right],
            rows: vec![vec!["1".to_string(), "2".to_string()]],
        };
        let json = serde_json::to_string(&table).unwrap();
        let parsed: TableData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.headers, table.headers);
        assert_eq!(parsed.alignments, table.alignments);
        assert_eq!(parsed.rows, table.rows);
    }

    #[test]
    fn test_to_markdown_empty_headers() {
        let table = TableData {
            headers: vec![],
            alignments: vec![],
            rows: vec![],
        };
        assert_eq!(table.to_markdown(), "");
    }
}
