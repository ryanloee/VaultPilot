// Regression test for #3682: compute_summaries group-key filtering always
// produces empty results when all_notes_for_group_filter is empty.
//
// Before the fix, the caller passed `&[]` for all_notes_for_group_filter,
// causing the zip to produce zero elements — so every per-group summary
// returned 0/empty/N/A.

#[cfg(test)]
mod tests {
    use crate::bases::{compute_summaries, ColumnSummary, SummaryFunction};
    use crate::models::NoteMeta;

    fn make_note(id: &str, status: &str) -> NoteMeta {
        NoteMeta {
            id: id.into(),
            title: format!("note-{id}"),
            status: status.into(),
            ..Default::default()
        }
    }

    #[test]
    fn regression_3682_group_summary_counts_matching_rows() {
        // Three notes: 2 with status "done", 1 with status "todo"
        let notes = [
            make_note("1", "done"),
            make_note("2", "todo"),
            make_note("3", "done"),
        ];

        let columns = vec![crate::bases::BaseColumn {
            field: "status".into(),
            label: None,
            width: None,
        }];
        let rows = vec![
            crate::bases::BaseRow {
                note_id: "1".into(),
                title: "a".into(),
                values: vec!["done".into()],
            },
            crate::bases::BaseRow {
                note_id: "2".into(),
                title: "b".into(),
                values: vec!["todo".into()],
            },
            crate::bases::BaseRow {
                note_id: "3".into(),
                title: "c".into(),
                values: vec!["done".into()],
            },
        ];

        let config = crate::bases::BaseConfig {
            group_by: Some("status".into()),
            summaries: vec![ColumnSummary {
                field: "status".into(),
                function: SummaryFunction::Count,
            }],
            ..Default::default()
        };

        let note_refs: Vec<&NoteMeta> = notes.iter().collect();

        // Query the "done" group — should count 2 rows
        let results = compute_summaries(
            &config,
            &rows,
            &columns,
            &crate::property_schema::PropertySchema::empty(),
            Some("done"),
            &note_refs,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "2", "done group should have 2 notes");

        // Query the "todo" group — should count 1 row
        let results_todo = compute_summaries(
            &config,
            &rows,
            &columns,
            &crate::property_schema::PropertySchema::empty(),
            Some("todo"),
            &note_refs,
        );
        assert_eq!(results_todo.len(), 1);
        assert_eq!(results_todo[0].value, "1", "todo group should have 1 note");
    }

    #[test]
    fn regression_3682_empty_filter_produces_empty_group() {
        // Regression: passing &[] for all_notes_for_group_filter should produce
        // empty results (the old broken behavior), confirming the test
        // correctly exercises the filter path.
        let columns = vec![crate::bases::BaseColumn {
            field: "status".into(),
            label: None,
            width: None,
        }];
        let rows = vec![crate::bases::BaseRow {
            note_id: "1".into(),
            title: "a".into(),
            values: vec!["done".into()],
        }];

        let config = crate::bases::BaseConfig {
            group_by: Some("status".into()),
            summaries: vec![ColumnSummary {
                field: "status".into(),
                function: SummaryFunction::Count,
            }],
            ..Default::default()
        };

        // Empty note refs → zip produces nothing → count is 0
        let results = compute_summaries(
            &config,
            &rows,
            &columns,
            &crate::property_schema::PropertySchema::empty(),
            Some("done"),
            &[],
        );
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].value, "0",
            "empty filter slice should yield 0 results"
        );
    }
}
