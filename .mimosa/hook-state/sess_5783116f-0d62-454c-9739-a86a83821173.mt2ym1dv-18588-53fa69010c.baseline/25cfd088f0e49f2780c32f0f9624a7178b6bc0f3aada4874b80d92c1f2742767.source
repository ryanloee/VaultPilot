//! Regression test for #2913 — vault-query CSV/Markdown export column order
//! must be deterministic for `SELECT *` queries.
//!
//! Previously `format_rows_csv` / `format_rows_md_table` pulled columns from
//! `rows[0].keys()`, which iterates a `HashMap` in a non-deterministic order.
//! The same query could therefore emit differently-ordered output between runs,
//! breaking downstream CSV consumers (pandas, Excel imports, etc.).
//!
//! The fix (`collect_ordered_columns`) gathers the union of keys from every row
//! and sorts them: `$path` first, then the rest alphabetically.

use crate::vault_query::{
    format_rows_csv, format_rows_json, format_rows_md_table, parse_query, query_records, QValue,
    Record,
};

use std::collections::HashMap;

// ── Sample data ──────────────────────────────────────────────────────────
//
// Records intentionally have *different* property sets so we also verify the
// union-of-all-keys behaviour (a column present in only some rows still shows
// up, with empty/Null cells for rows that lack it).

fn sample_records() -> Vec<Record> {
    vec![
        Record::new("notes/alpha.md")
            .with_prop("title", QValue::Text("Alpha".into()))
            .with_prop("status", QValue::Text("active".into()))
            .with_prop("priority", QValue::Number(3.0)),
        Record::new("notes/beta.md")
            .with_prop("title", QValue::Text("Beta".into()))
            .with_prop("status", QValue::Text("done".into()))
            // `priority` absent here — must still appear as a column
            .with_prop("zebra", QValue::Text("last-alphabetically".into())),
        Record::new("notes/gamma.md")
            .with_prop("title", QValue::Text("Gamma".into()))
            .with_prop("status", QValue::Text("active".into()))
            .with_prop("priority", QValue::Number(7.0)),
    ]
}

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn csv_select_star_has_deterministic_column_order() {
    // SELECT * → no explicit field list, so order is decided by the formatter.
    let q = parse_query("SELECT *").unwrap();
    let rows = query_records(&sample_records(), &q);

    let csv = format_rows_csv(&rows);
    let header = csv.lines().next().expect("CSV must have a header row");

    // `$path` always first, then the rest alphabetically:
    // priority < status < title < zebra
    assert_eq!(
        header, "$path,priority,status,title,zebra",
        "CSV header columns must be deterministic ($path first, then alphabetical)"
    );
}

#[test]
fn markdown_select_star_has_deterministic_column_order() {
    let q = parse_query("SELECT *").unwrap();
    let rows = query_records(&sample_records(), &q);

    let md = format_rows_md_table(&rows);
    let header = md
        .lines()
        .next()
        .expect("Markdown table must have a header row");

    assert_eq!(
        header, "| $path | priority | status | title | zebra |",
        "Markdown header columns must be deterministic ($path first, then alphabetical)"
    );
}

#[test]
fn csv_output_is_stable_across_repeated_calls() {
    // The whole point of #2913: calling the formatter many times (with
    // independently-built HashMaps) must yield byte-identical output every time.
    let mut outputs: Vec<String> = Vec::new();
    for _ in 0..20 {
        let q = parse_query("SELECT *").unwrap();
        let rows = query_records(&sample_records(), &q);
        outputs.push(format_rows_csv(&rows));
    }
    let first = outputs[0].clone();
    for (i, out) in outputs.iter().enumerate() {
        assert_eq!(
            out, &first,
            "CSV output differed on iteration {i} — column order is non-deterministic (#2913)"
        );
    }
}

#[test]
fn markdown_output_is_stable_across_repeated_calls() {
    let mut outputs: Vec<String> = Vec::new();
    for _ in 0..20 {
        let q = parse_query("SELECT *").unwrap();
        let rows = query_records(&sample_records(), &q);
        outputs.push(format_rows_md_table(&rows));
    }
    let first = outputs[0].clone();
    for (i, out) in outputs.iter().enumerate() {
        assert_eq!(
            out, &first,
            "Markdown output differed on iteration {i} — column order is non-deterministic (#2913)"
        );
    }
}

#[test]
fn csv_union_of_keys_across_heterogeneous_rows() {
    // `zebra` only exists in one row; it must still appear as a column for all
    // rows (empty cell where absent), and it must be last (alphabetical).
    let q = parse_query("SELECT *").unwrap();
    let rows = query_records(&sample_records(), &q);
    let csv = format_rows_csv(&rows);

    let lines: Vec<&str> = csv.trim().lines().collect();
    assert_eq!(lines.len(), 4, "header + 3 data rows");
    // Header ends with the zebra column
    assert!(
        lines[0].ends_with(",zebra"),
        "header should end with the zebra column: {}",
        lines[0]
    );
    // Every data line has 5 comma-separated fields
    for (i, line) in lines.iter().enumerate().skip(1) {
        assert_eq!(
            line.split(',').count(),
            5,
            "row {i} should have 5 columns: {line}"
        );
    }
}

#[test]
fn path_column_always_first_even_when_other_keys_sort_before_it() {
    // `$` (0x24) sorts before letters, so even a plain alphabetical sort would
    // put `$path` first — but we also test a row whose only key is a property
    // name that would sort before `$path` is impossible (no printable ASCII <
    // '$' is a typical prop name). Instead, verify the explicit front-position
    // guarantee by checking `$path` precedes a numerically-prefixed key.
    let mut rows: Vec<HashMap<String, QValue>> = Vec::new();
    let mut row = HashMap::new();
    row.insert("$path".to_string(), QValue::Text("x.md".into()));
    row.insert("001_id".to_string(), QValue::Number(1.0));
    row.insert("aaa".to_string(), QValue::Text("a".into()));
    rows.push(row);

    let csv = format_rows_csv(&rows);
    let header = csv.lines().next().unwrap();
    // `$path` first, then alphabetical: 001_id < aaa
    assert_eq!(header, "$path,001_id,aaa");
}

#[test]
fn json_output_is_deterministic() {
    // serde_json::Map is BTreeMap-backed (no preserve_order feature), so JSON
    // keys are serialized in sorted order. Verify stability across calls.
    let mut outputs: Vec<String> = Vec::new();
    for _ in 0..10 {
        let q = parse_query("SELECT *").unwrap();
        let rows = query_records(&sample_records(), &q);
        let json = format_rows_json(&rows);
        outputs.push(serde_json::to_string(&json).unwrap());
    }
    let first = outputs[0].clone();
    for (i, out) in outputs.iter().enumerate() {
        assert_eq!(
            out, &first,
            "JSON output differed on iteration {i} — non-deterministic (#2913)"
        );
    }
}

#[test]
fn empty_rows_produce_empty_csv_and_markdown() {
    let rows: Vec<HashMap<String, QValue>> = Vec::new();
    assert_eq!(format_rows_csv(&rows), "");
    assert_eq!(format_rows_md_table(&rows), "");
}
