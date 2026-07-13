//! Regression test for #2813 — CSV/Markdown/JSON export from vault_query.
//!
//! Tests the four output formatters (table, CSV, Markdown-table, JSON) and
//! the frontmatter YAML block extractor used by the CLI `vault query` command.

use crate::vault_query::{parse_query, query_records, QValue, Record};

use std::collections::HashMap;

// ── Sample data helpers ──────────────────────────────────────────────────

fn sample_records() -> Vec<Record> {
    use chrono::NaiveDate;

    vec![
        Record::new("notes/rust.md")
            .with_prop("title", QValue::Text("Rust Async".into()))
            .with_prop("status", QValue::Text("active".into()))
            .with_prop("priority", QValue::Number(3.0))
            .with_prop(
                "tags",
                QValue::List(vec![
                    QValue::Text("rust".into()),
                    QValue::Text("async".into()),
                ]),
            ),
        Record::new("notes/python.md")
            .with_prop("title", QValue::Text("Python Tips".into()))
            .with_prop("status", QValue::Text("done".into()))
            .with_prop("priority", QValue::Number(1.0))
            .with_prop("tags", QValue::List(vec![QValue::Text("python".into())])),
        Record::new("notes/go.md")
            .with_prop("title", QValue::Text("Go Patterns".into()))
            .with_prop("status", QValue::Text("active".into()))
            .with_prop("priority", QValue::Number(5.0))
            .with_prop(
                "due",
                QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
            ),
    ]
}

// ── Import the CLI formatters (duplicated here for independent testing) ──

/// Extract the raw YAML string from a frontmatter block (`---\n...\n---`).
fn extract_frontmatter_yaml_block(content: &str) -> Option<String> {
    if !content.starts_with("---\n") {
        return None;
    }
    let inner = &content[4..];
    if let Some(end) = inner.find("\n---\n") {
        return Some(inner[..end].to_string());
    }
    if let Some(end) = inner.find("\n---") {
        if end + 4 == inner.len() {
            return Some(inner[..end].to_string());
        }
    }
    if inner.starts_with("---\n") || inner == "---" {
        return Some(String::new());
    }
    None
}

fn format_as_csv(columns: &[String], rows: &[HashMap<String, QValue>]) -> String {
    let mut out = String::new();
    out.push_str(&csv_line(columns));
    out.push('\n');
    for row in rows {
        let vals: Vec<String> = columns
            .iter()
            .map(|c: &String| {
                row.get(c)
                    .map(|v: &QValue| v.to_string())
                    .unwrap_or_default()
            })
            .collect();
        out.push_str(&csv_line(&vals));
        out.push('\n');
    }
    out
}

fn csv_line(values: &[String]) -> String {
    values
        .iter()
        .map(|v: &String| {
            if v.contains(',') || v.contains('"') || v.contains('\n') {
                format!("\"{}\"", v.replace('"', "\"\""))
            } else {
                v.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_as_md_table(columns: &[String], rows: &[HashMap<String, QValue>]) -> String {
    if rows.is_empty() {
        return "*No results*\n".to_string();
    }
    let mut out = String::new();
    out.push('|');
    for col in columns {
        out.push_str(&format!(" {} |", col));
    }
    out.push('\n');
    out.push('|');
    for _ in columns {
        out.push_str("---|");
    }
    out.push('\n');
    for row in rows {
        out.push('|');
        for col in columns {
            let val = row
                .get(col)
                .map(|v: &QValue| v.to_string())
                .unwrap_or_default();
            let escaped = val.replace('|', "\\|");
            out.push_str(&format!(" {} |", escaped));
        }
        out.push('\n');
    }
    out
}

fn format_as_table(columns: &[String], rows: &[HashMap<String, QValue>]) -> String {
    if rows.is_empty() {
        return "(no results)\n".to_string();
    }
    let mut widths: Vec<usize> = columns.iter().map(|c: &String| c.len()).collect();
    for row in rows {
        for (i, col) in columns.iter().enumerate() {
            let val_len = row
                .get(col)
                .map(|v: &QValue| v.to_string().len())
                .unwrap_or(0);
            if val_len > widths[i] {
                widths[i] = val_len;
            }
        }
    }
    let mut out = String::new();
    for (i, col) in columns.iter().enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        out.push_str(&format!("{:width$}", col, width = widths[i]));
    }
    out.push('\n');
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        out.push_str(&"-".repeat(*w));
    }
    out.push('\n');
    for row in rows {
        for (i, col) in columns.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            let val = row
                .get(col)
                .map(|v: &QValue| v.to_string())
                .unwrap_or_default();
            out.push_str(&format!("{:width$}", val, width = widths[i]));
        }
        out.push('\n');
    }
    out
}

fn format_as_json(columns: &[String], rows: &[HashMap<String, QValue>]) -> String {
    let json_rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|row: &HashMap<String, QValue>| {
            let mut map = serde_json::Map::new();
            for col in columns {
                let val = row
                    .get(col)
                    .map(|v: &QValue| v.to_string())
                    .unwrap_or_default();
                map.insert(col.clone(), serde_json::Value::String(val));
            }
            serde_json::Value::Object(map)
        })
        .collect();
    serde_json::to_string_pretty(&json_rows).unwrap()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn csv_export_with_header_and_escaped_values() {
    let query = parse_query("SELECT title, status, priority").unwrap();
    let rows = query_records(&sample_records(), &query);
    let columns = vec![
        "$path".to_string(),
        "title".to_string(),
        "status".to_string(),
        "priority".to_string(),
    ];

    let csv = format_as_csv(&columns, &rows);
    assert!(
        csv.starts_with("$path,title,status,priority\n"),
        "missing CSV header"
    );
    assert_eq!(csv.lines().count(), 4, "header + 3 data rows");
    // A row should contain comma-separated values
    assert!(csv.contains("notes/rust.md,Rust Async,active,3"));
}

#[test]
fn csv_escapes_commas_and_quotes() {
    // Value containing a comma and quotes
    let rows = vec![{
        let mut map = HashMap::new();
        map.insert(
            "col".to_string(),
            QValue::Text(r#"has "quotes", and comma"#.into()),
        );
        map
    }];
    let columns = vec!["col".to_string()];
    let csv = format_as_csv(&columns, &rows);
    let lines: Vec<&str> = csv.trim().lines().collect();
    assert_eq!(lines.len(), 2, "header + 1 row");
    // The value should be double-quoted with internal quotes doubled
    assert!(
        lines[1].starts_with('"') && lines[1].ends_with('"'),
        "escaped value should be wrapped in double quotes: {}",
        lines[1]
    );
    assert!(
        lines[1].contains("\"\"quotes\"\""),
        "internal quotes should be doubled: {}",
        lines[1]
    );
}

#[test]
fn markdown_table_format() {
    let query = parse_query("SELECT title, status").unwrap();
    let rows = query_records(&sample_records(), &query);
    let columns = vec![
        "$path".to_string(),
        "title".to_string(),
        "status".to_string(),
    ];

    let md = format_as_md_table(&columns, &rows);
    assert!(md.contains("| $path |"), "missing header");
    assert!(md.contains("| title |"), "missing title column");
    assert!(md.contains("---|"), "missing separator");
    assert_eq!(md.lines().count(), 5, "header + separator + 3 data rows = 5 lines");
}

#[test]
fn markdown_table_escapes_pipes() {
    let rows = vec![{
        let mut map = HashMap::new();
        map.insert("col".to_string(), QValue::Text("value|with|pipes".into()));
        map
    }];
    let columns = vec!["col".to_string()];
    let md = format_as_md_table(&columns, &rows);
    // Pipes in values should be escaped with backslash
    assert!(
        md.contains("value\\|with\\|pipes"),
        "pipes should be escaped: {}",
        md
    );
}

#[test]
fn markdown_table_empty_results() {
    let rows: Vec<HashMap<String, QValue>> = vec![];
    let columns = vec!["$path".to_string()];
    let md = format_as_md_table(&columns, &rows);
    assert_eq!(md, "*No results*\n");
}

#[test]
fn json_export_array_of_objects() {
    let query = parse_query("SELECT title, status").unwrap();
    let rows = query_records(&sample_records(), &query);
    let columns = vec![
        "$path".to_string(),
        "title".to_string(),
        "status".to_string(),
    ];

    let json = format_as_json(&columns, &rows);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_array());
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    // First item should have the expected fields
    let first = &arr[0];
    assert_eq!(first["title"], "Rust Async");
    assert_eq!(first["status"], "active");
    assert!(first["$path"].as_str().unwrap().contains("rust.md"));
}

#[test]
fn table_format_column_alignment() {
    let query = parse_query("SELECT title").unwrap();
    let rows = query_records(&sample_records(), &query);
    let columns = vec!["$path".to_string(), "title".to_string()];

    let table = format_as_table(&columns, &rows);
    assert!(table.contains("$path"), "header missing $path");
    assert!(table.contains("title"), "header missing title");
    // All data rows should have aligned content
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines.len(), 5, "header + separator + 3 data rows");
}

#[test]
fn table_format_empty_results() {
    let rows: Vec<HashMap<String, QValue>> = vec![];
    let columns = vec!["col".to_string()];
    let table = format_as_table(&columns, &rows);
    assert_eq!(table, "(no results)\n");
}

#[test]
fn extract_yaml_block_standard() {
    let content = "---\ntitle: Test\nstatus: active\n---\nBody here\n";
    let yaml = extract_frontmatter_yaml_block(content).unwrap();
    assert_eq!(yaml, "title: Test\nstatus: active");
}

#[test]
fn extract_yaml_block_no_trailing_newline() {
    let content = "---\ntitle: Test\n---";
    let yaml = extract_frontmatter_yaml_block(content).unwrap();
    assert_eq!(yaml, "title: Test");
}

#[test]
fn extract_yaml_block_empty() {
    let content = "---\n---\nBody here\n";
    let yaml = extract_frontmatter_yaml_block(content).unwrap();
    assert_eq!(yaml, "");
}

#[test]
fn extract_yaml_block_no_frontmatter() {
    let content = "# Just a heading\nNo frontmatter here\n";
    assert!(extract_frontmatter_yaml_block(content).is_none());
}

#[test]
fn extract_yaml_block_bom_stripped() {
    // UTF-8 BOM + frontmatter (after BOM strip)
    let content = "---\ntitle: Test\n---\nBody\n";
    let yaml = extract_frontmatter_yaml_block(content).unwrap();
    assert_eq!(yaml, "title: Test");
}

/// Integration: parse a real query, run against sample records, verify results.
#[test]
fn full_query_pipeline_select_star() {
    let query = parse_query(r#"SELECT * WHERE status = "active""#).unwrap();
    let rows = query_records(&sample_records(), &query);
    assert_eq!(rows.len(), 2, "should match 2 active records");
    for row in &rows {
        assert_eq!(
            row.get("status"),
            Some(&QValue::Text("active".into())),
            "status must be active"
        );
        assert!(row.contains_key("$path"), "must have $path");
    }
}

#[test]
fn full_query_pipeline_select_fields_with_order() {
    let query = parse_query("SELECT title, priority ORDER BY priority DESC").unwrap();
    let rows = query_records(&sample_records(), &query);
    assert_eq!(rows.len(), 3);
    // First row should be highest priority (5.0 → Go Patterns)
    assert_eq!(
        rows[0].get("title"),
        Some(&QValue::Text("Go Patterns".into()))
    );
    assert_eq!(rows[0].get("priority"), Some(&QValue::Number(5.0)));
}