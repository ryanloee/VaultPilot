// Regression test for #3343: Bases `--table` CLI flag produces terminal-width-aware
// text table output instead of JSON.

#[cfg(test)]
mod tests {
    use crate::bases::{format_bases_table, BaseColumn, BaseResult, BaseRow, BaseView};

    #[test]
    fn regression_3343_format_bases_table_basic() {
        let result = BaseResult {
            view: BaseView::Table,
            columns: vec![
                BaseColumn {
                    field: "title".into(),
                    label: Some("Title".into()),
                    width: None,
                },
                BaseColumn {
                    field: "status".into(),
                    label: None,
                    width: None,
                },
            ],
            rows: vec![
                BaseRow {
                    note_id: "n1".into(),
                    title: "Fix bug".into(),
                    values: vec!["Fix bug".into(), "done".into()],
                },
                BaseRow {
                    note_id: "n2".into(),
                    title: "Write docs".into(),
                    values: vec!["Write docs".into(), "in-progress".into()],
                },
            ],
            matched: 2,
            scanned: 10,
            kanban_groups: vec![],
        };

        let table = format_bases_table(&result);
        // Should contain headers.
        assert!(table.contains("Title"), "missing Title header");
        assert!(table.contains("status"), "missing status header");
        // Should contain row data.
        assert!(table.contains("Fix bug"), "missing Fix bug row");
        assert!(table.contains("done"), "missing done value");
        assert!(table.contains("Write docs"), "missing Write docs row");
        assert!(table.contains("in-progress"), "missing in-progress value");
        // Should contain summary line.
        assert!(table.contains("2 rows (10 scanned)"), "missing summary");
        // Should have borders (ASCII art).
        assert!(table.contains("+"), "missing border characters");
        assert!(table.contains("|"), "missing column separators");
    }

    #[test]
    fn regression_3343_empty_columns_returns_no_columns() {
        let result = BaseResult {
            view: BaseView::Table,
            columns: vec![],
            rows: vec![],
            matched: 0,
            scanned: 0,
            kanban_groups: vec![],
        };
        let table = format_bases_table(&result);
        assert_eq!(table, "(no columns)");
    }

    #[test]
    fn regression_3343_truncates_long_cells() {
        let result = BaseResult {
            view: BaseView::Table,
            columns: vec![BaseColumn {
                field: "content".into(),
                label: None,
                width: None,
            }],
            rows: vec![BaseRow {
                note_id: "n1".into(),
                title: "Long".into(),
                values: vec![
                    "This is a very long content that should be truncated with ellipsis in the table output".into(),
                ],
            }],
            matched: 1,
            scanned: 5,
            kanban_groups: vec![],
        };

        let table = format_bases_table(&result);
        // With terminal width 80 and 1 column, even the max width is capped at 60,
        // so the 96-char string should be truncated with …
        assert!(
            table.contains("…"),
            "long content should be truncated with ellipsis"
        );
    }
}
