//! Regression test for issue #3685: Markdown Table Parser & Editor.
//!
//! Bug:        No structured table manipulation utilities existed. Users had to
//!             manually edit pipe-delimited markdown, which is error-prone for
//!             adding/removing rows/columns and maintaining alignment.
//! Root cause: Missing module — the only table formatting was `format_rows_md_table`
//!             in the Bases module (one-way query output), no parse/edit/serialize
//!             round-trip existed.
//! Fix:        Added `src/markdown_table.rs` with `TableData`, `parse_markdown_table`,
//!             and row/column manipulation methods.
//!
//! These tests verify the public API works correctly with real-world markdown
//! tables, complementing the unit tests in `markdown_table.rs`.

#[cfg(test)]
mod tests {
    use crate::markdown_table::{parse_markdown_table, ColumnAlignment, TableData};

    /// A typical 3-column table with mixed content should parse and round-trip.
    #[test]
    fn regression_3685_parse_and_roundtrip_typical_table() {
        let md = "\
| Task | Priority | Status |
|------|:--------:|-------:|
| Fix bug | High | Done |
| Write docs | Low | Pending |
";
        let table = parse_markdown_table(md).expect("parse");
        assert_eq!(table.headers, vec!["Task", "Priority", "Status"]);
        assert_eq!(table.alignments[0], ColumnAlignment::Left);
        assert_eq!(table.alignments[1], ColumnAlignment::Center);
        assert_eq!(table.alignments[2], ColumnAlignment::Right);
        assert_eq!(table.rows.len(), 2);

        // Round-trip: parse(to_markdown(table)) should preserve data
        let out = table.to_markdown();
        let reparsed = parse_markdown_table(&out).expect("reparse");
        assert_eq!(reparsed.headers, table.headers);
        assert_eq!(reparsed.rows, table.rows);
    }

    /// Adding a row should extend the table correctly.
    #[test]
    fn regression_3685_add_row_to_existing_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let mut table = parse_markdown_table(md).expect("parse");
        table.add_row(vec!["3".to_string(), "4".to_string()]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1], vec!["3", "4"]);

        let out = table.to_markdown();
        let reparsed = parse_markdown_table(&out).expect("reparse");
        assert_eq!(reparsed.rows.len(), 2);
    }

    /// Removing a column should shrink headers and all rows.
    #[test]
    fn regression_3685_remove_column() {
        let md = "| A | B | C |\n|---|---|---|\n| 1 | 2 | 3 |\n";
        let mut table = parse_markdown_table(md).expect("parse");
        assert_eq!(table.headers.len(), 3);
        table.remove_column(1); // remove "B"
        assert_eq!(table.headers, vec!["A", "C"]);
        assert_eq!(table.rows[0], vec!["1", "3"]);
    }

    /// Template generation should produce valid, parseable markdown.
    #[test]
    fn regression_3685_generate_template() {
        let md = TableData::generate_template(3, 4);
        assert!(md.contains("Column 1"));
        assert!(md.contains("Column 4"));
        let table = parse_markdown_table(&md).expect("parse template");
        assert_eq!(table.headers.len(), 4);
        assert_eq!(table.rows.len(), 3);
    }

    /// Non-table input should return None, not panic.
    #[test]
    fn regression_3685_non_table_returns_none() {
        assert!(parse_markdown_table("Just some text").is_none());
        assert!(parse_markdown_table("").is_none());
        assert!(parse_markdown_table("| Header |\nNo separator").is_none());
    }

    /// CJK characters in cells should not cause panics or corruption.
    #[test]
    fn regression_3685_cjk_table() {
        let md = "| 项目 | 状态 |\n|------|------|\n| 修复 | 完成 |\n";
        let table = parse_markdown_table(md).expect("parse");
        assert_eq!(table.headers, vec!["项目", "状态"]);
        assert_eq!(table.rows[0], vec!["修复", "完成"]);

        let out = table.to_markdown();
        let reparsed = parse_markdown_table(&out).expect("reparse");
        assert_eq!(reparsed.headers, vec!["项目", "状态"]);
    }

    /// JSON serialization round-trip should preserve all fields.
    #[test]
    fn regression_3685_json_roundtrip() {
        let table = TableData {
            headers: vec!["X".to_string(), "Y".to_string()],
            alignments: vec![ColumnAlignment::Center, ColumnAlignment::Right],
            rows: vec![vec!["a".to_string(), "b".to_string()]],
        };
        let json = serde_json::to_string(&table).expect("serialize");
        let parsed: TableData = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.headers, table.headers);
        assert_eq!(parsed.alignments, table.alignments);
        assert_eq!(parsed.rows, table.rows);
    }
}
