// Regression tests for #3666: Query Blocks column summaries (count/empty/unique/sum/average/max/min).

#[cfg(test)]
mod tests {
    use crate::bases::{compute_summaries, ColumnSummary, SummaryFunction};

    #[test]
    fn regression_3666_count_summary_counts_non_empty() {
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
                values: vec!["".into()],
            },
            crate::bases::BaseRow {
                note_id: "3".into(),
                title: "c".into(),
                values: vec!["todo".into()],
            },
        ];

        let config = crate::bases::BaseConfig {
            summaries: vec![ColumnSummary {
                field: "status".into(),
                function: SummaryFunction::Count,
            }],
            ..Default::default()
        };

        let results = compute_summaries(
            &config,
            &rows,
            &columns,
            &crate::property_schema::PropertySchema::empty(),
            None,
            &[],
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "2"); // 2 non-empty: "done" and "todo"
    }

    #[test]
    fn regression_3666_empty_returns_empty_when_no_summaries() {
        let config = crate::bases::BaseConfig::default();
        let results = compute_summaries(
            &config,
            &[],
            &[],
            &crate::property_schema::PropertySchema::empty(),
            None,
            &[],
        );
        assert!(results.is_empty());
    }
}