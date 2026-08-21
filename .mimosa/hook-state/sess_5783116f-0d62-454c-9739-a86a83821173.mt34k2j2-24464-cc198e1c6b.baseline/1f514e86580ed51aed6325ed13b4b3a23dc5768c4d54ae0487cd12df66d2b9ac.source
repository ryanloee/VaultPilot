// Regression test for #3378: --table output missing right border pipe and misaligned columns.
//
// Two defects:
//   1. format_table_row joined cells without a trailing "|"
//   2. Border segments used w+1 dashes while row content used w+2 chars → pipes never lined up

#[cfg(test)]
mod tests {
    use crate::bases::{format_bases_table, BaseColumn, BaseResult, BaseRow, BaseView};

    fn make_result(headers: Vec<(&str, Option<&str>)>, data: Vec<Vec<String>>) -> BaseResult {
        let columns: Vec<BaseColumn> = headers
            .iter()
            .map(|(field, label)| BaseColumn {
                field: (*field).into(),
                label: label.map(|s| s.to_string()),
                width: None,
            })
            .collect();
        let matched = data.len();
        let rows: Vec<BaseRow> = data
            .into_iter()
            .enumerate()
            .map(|(i, values)| BaseRow {
                note_id: format!("n{}", i),
                title: values.first().cloned().unwrap_or_default(),
                values,
            })
            .collect();
        BaseResult {
            view: BaseView::Table,
            columns,
            rows,
            matched,
            scanned: matched * 5,
            kanban_groups: vec![],
            calendar_groups: vec![],
            gallery_groups: vec![],
            summaries: vec![],
            group_summaries: vec![],
        }
    }

    #[test]
    fn regression_3378_rows_end_with_pipe() {
        let result = make_result(
            vec![("title", Some("Title")), ("status", None)],
            vec![
                vec!["Hello".into(), "done".into()],
                vec!["World".into(), "todo".into()],
            ],
        );

        let table = format_bases_table(&result);
        let lines: Vec<&str> = table.lines().collect();

        // Every border line and data line should have pipes.
        // Find all lines that look like table rows (contain '|').
        for line in &lines {
            if line.contains('|') {
                assert!(
                    line.ends_with('|'),
                    "Row should end with '|', got: {:?}",
                    line
                );
            }
        }
    }

    #[test]
    fn regression_3378_border_and_row_alignment() {
        let result = make_result(
            vec![("a", Some("AA")), ("b", Some("BB"))],
            vec![vec!["x".into(), "y".into()]],
        );

        let table = format_bases_table(&result);
        let lines: Vec<&str> = table.lines().collect();

        // The first line is a border, second is header row, third is border, fourth is data.
        let border = lines[0];
        let header = lines[1];

        // Border and header should have the same length for visual alignment.
        assert_eq!(
            border.chars().count(),
            header.chars().count(),
            "Border ({}) and header row ({}) must be the same width",
            border.chars().count(),
            header.chars().count()
        );

        // Every '+' in the border should align with a '|' in the header.
        let border_chars: Vec<char> = border.chars().collect();
        let header_chars: Vec<char> = header.chars().collect();
        for (i, &ch) in border_chars.iter().enumerate() {
            if ch == '+' {
                assert_eq!(
                    header_chars[i], '|',
                    "Border '+' at position {} does not align with '|' in header",
                    i
                );
            }
        }
    }

    #[test]
    fn regression_3378_single_column_alignment() {
        let result = make_result(vec![("name", Some("Name"))], vec![vec!["Alice".into()]]);

        let table = format_bases_table(&result);
        let lines: Vec<&str> = table.lines().collect();

        let border = lines[0];
        let data_row = lines[3]; // border, header, border, data

        assert_eq!(
            border.chars().count(),
            data_row.chars().count(),
            "Single-column border and data row should be the same width"
        );

        // Verify the row has both leading and trailing pipes.
        assert!(data_row.starts_with('|'));
        assert!(data_row.ends_with('|'));
    }
}
