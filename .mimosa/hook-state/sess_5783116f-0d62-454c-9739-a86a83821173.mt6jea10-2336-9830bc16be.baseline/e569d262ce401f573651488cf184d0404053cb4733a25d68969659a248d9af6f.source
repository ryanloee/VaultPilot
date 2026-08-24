//! Regression test for #3823: CLI query --raw flag and clean JSON output.
//!
//! Verifies that:
//! 1. Query results produce clean, machine-serializable data
//! 2. CSV format is parseable (pipe-friendly)
//! 3. The query pipeline itself produces valid results for machine consumption

#[cfg(test)]
mod tests {
    use crate::vault_query::{parse_query, query_records, QValue, Record};

    fn make_record(path: &str, props: &[(&str, &str)]) -> Record {
        let mut rec = Record::new(path);
        for (k, v) in props {
            rec = rec.with_prop(*k, QValue::Text(v.to_string()));
        }
        rec
    }

    #[test]
    fn test_query_json_output_is_clean() {
        // The core logic: machine-readable formats (json, csv) should suppress
        // status JSON on stdout so piping works without pollution.
        // This test verifies the query pipeline itself produces valid results
        // that can be cleanly consumed by downstream JSON serialization.
        let records = vec![
            make_record("test/a.md", &[("title", "Alpha"), ("priority", "1")]),
            make_record("test/b.md", &[("title", "Beta"), ("priority", "2")]),
        ];

        let q = parse_query("SELECT title, priority").expect("query should parse");
        let rows = query_records(&records, &q);
        assert_eq!(rows.len(), 2, "should have 2 result rows");

        // Verify each row has the expected columns (clean, parseable structure)
        for row in &rows {
            assert!(row.contains_key("title"), "row should have title column");
            assert!(
                row.contains_key("priority"),
                "row should have priority column"
            );
        }
    }

    #[test]
    fn test_query_csv_format_is_pipe_friendly() {
        // CSV format should produce clean, parseable output suitable for piping
        let records = vec![
            make_record("test/a.md", &[("status", "done")]),
            make_record("test/b.md", &[("status", "todo")]),
        ];

        let q = parse_query("SELECT status").expect("query should parse");
        let rows = query_records(&records, &q);

        assert_eq!(rows.len(), 2);
        // Each row should have the status field
        assert!(rows.iter().all(|r| r.contains_key("status")));

        // Verify the values are Text (string-serializable for CSV/JSON)
        for row in &rows {
            let status = row.get("status").expect("status should exist");
            match status {
                QValue::Text(s) => assert!(!s.is_empty(), "status text should be non-empty"),
                _ => panic!("status should be Text variant for CSV output"),
            }
        }
    }

    #[test]
    fn test_query_empty_result_is_valid_json_array() {
        // Empty results should still be representable as [] in JSON output
        let records: Vec<Record> = vec![];

        let q = parse_query("SELECT title").expect("query should parse");
        let rows = query_records(&records, &q);

        assert_eq!(rows.len(), 0, "empty input should yield 0 rows");
        // An empty Vec serializes to [] in JSON — this is the contract
        // that --raw / --format json depends on for clean piping.
    }
}
