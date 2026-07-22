//! Multi-format note export (#3276).
//!
//! Provides conversion from Markdown note content to structured file formats
//! such as XLSX (Excel).  Future formats (PDF, PPTX, DOCX) will be added here.
//!
//! The XLSX export uses the `rust_xlsxwriter` crate (already a workspace
//! dependency) to produce native Excel files. Markdown tables (`| ... |`
//! pipe syntax) are parsed into rows and columns.

use std::path::Path;

use anyhow::{Context, Result};
use rust_xlsxwriter::{Format, FormatBorder, Workbook};

/// A single table extracted from Markdown: header row + optional data rows.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Parse all GFM-style pipe tables from a Markdown string.
///
/// A valid table consists of:
/// - A header row: `| Col A | Col B |`
/// - A separator row: `| --- | --- |` (dashes, optional colons for alignment)
/// - Zero or more data rows: `| 1 | 2 |`
///
/// Tables are separated by blank lines or non-table content. Leading/trailing
/// pipes are optional but recommended.
pub fn parse_markdown_tables(markdown: &str) -> Vec<MarkdownTable> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut tables = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if !is_table_row(line) {
            i += 1;
            continue;
        }

        // Found a potential header row. The next line must be a separator.
        if i + 1 < lines.len() && is_separator_row(lines[i + 1].trim()) {
            let headers = parse_row_cells(line);
            let num_cols = headers.len();

            // Skip header + separator
            i += 2;

            let mut rows = Vec::new();
            while i < lines.len() && is_table_row(lines[i].trim()) {
                let cells = parse_row_cells(lines[i].trim());
                // Pad/truncate to header column count for consistency
                let mut cells = cells;
                cells.resize(num_cols, String::new());
                rows.push(cells);
                i += 1;
            }

            tables.push(MarkdownTable { headers, rows });
        } else {
            i += 1;
        }
    }

    tables
}

/// Write a single Markdown table to an XLSX worksheet.
fn write_table_to_worksheet(
    workbook: &mut Workbook,
    worksheet_name: &str,
    table: &MarkdownTable,
) -> Result<()> {
    let worksheet = workbook
        .add_worksheet()
        .set_name(worksheet_name)
        .context("failed to set worksheet name")?;

    let header_format = Format::new()
        .set_bold()
        .set_border_bottom(FormatBorder::Thin)
        .set_background_color(rust_xlsxwriter::Color::RGB(0xD9E1F2));

    // Write header row
    for (col, header) in table.headers.iter().enumerate() {
        worksheet.write_string_with_format(0, col as u16, header, &header_format)?;
    }

    // Write data rows
    for (row_idx, row) in table.rows.iter().enumerate() {
        for (col, cell) in row.iter().enumerate() {
            // Try to write numbers/bools natively for better Excel UX
            if let Some(num) = try_parse_number(cell) {
                worksheet.write_number((row_idx + 1) as u32, col as u16, num)?;
            } else if let Some(b) = try_parse_bool(cell) {
                worksheet.write_boolean((row_idx + 1) as u32, col as u16, b)?;
            } else {
                worksheet.write_string((row_idx + 1) as u32, col as u16, cell)?;
            }
        }
    }

    // Auto-fit column widths for readability
    worksheet.autofit();

    Ok(())
}

/// Export all Markdown tables from a string to a single XLSX file.
///
/// Each table becomes a separate worksheet named "Table 1", "Table 2", etc.
/// If the Markdown contains no tables, an error is returned.
///
/// # Example
/// ```no_run
/// use vaultpilot_lib::export::export_markdown_to_xlsx;
///
/// let md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n";
/// export_markdown_to_xlsx(md, std::path::Path::new("output.xlsx")).unwrap();
/// ```
pub fn export_markdown_to_xlsx(markdown: &str, output_path: &Path) -> Result<()> {
    let tables = parse_markdown_tables(markdown);
    if tables.is_empty() {
        anyhow::bail!("No Markdown tables found in the input to export to XLSX");
    }

    let mut workbook = Workbook::new();
    for (idx, table) in tables.iter().enumerate() {
        let sheet_name = format!("Table {}", idx + 1);
        write_table_to_worksheet(&mut workbook, &sheet_name, table)?;
    }

    workbook
        .save(output_path)
        .with_context(|| format!("failed to save XLSX to {:?}", output_path))?;

    Ok(())
}

/// Export CSV text to an XLSX file (single worksheet).
///
/// Each line is a row; cells are split by the specified delimiter (default `,`).
/// Quoted CSV fields (`"hello, world"`) are handled with basic unquoting.
pub fn export_csv_to_xlsx(csv: &str, output_path: &Path, delimiter: char) -> Result<()> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in csv.lines() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(parse_csv_line(line, delimiter));
    }

    if rows.is_empty() {
        anyhow::bail!("No CSV data found to export to XLSX");
    }

    let _num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let headers = rows[0].clone();
    let data_rows = rows[1..].to_vec();

    let table = MarkdownTable {
        headers,
        rows: data_rows,
    };

    let mut workbook = Workbook::new();
    write_table_to_worksheet(&mut workbook, "Sheet1", &table)?;
    workbook
        .save(output_path)
        .with_context(|| format!("failed to save XLSX to {:?}", output_path))?;

    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Check if a line looks like a Markdown table row (contains pipes).
fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.contains('|')
}

/// Check if a line is a Markdown table separator (| --- | --- |).
fn is_separator_row(line: &str) -> bool {
    let cleaned: String = line.chars().filter(|c| *c != ' ' && *c != '\t').collect();
    if !cleaned.contains('|') {
        return false;
    }
    // After removing spaces, each cell between pipes must be only dashes/colons
    let cells: Vec<&str> = cleaned.split('|').filter(|s| !s.is_empty()).collect();
    if cells.is_empty() {
        return false;
    }
    cells
        .iter()
        .all(|cell| !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':'))
}

/// Parse cells from a Markdown table row line.
/// `| Alice | 30 |` → `["Alice", "30"]`
fn parse_row_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// Parse a CSV line with basic quote handling.
fn parse_csv_line(line: &str, delimiter: char) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                // Check for escaped quote ""
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == delimiter {
            cells.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(ch);
        }
    }
    cells.push(current.trim().to_string());
    cells
}

/// Try to parse a cell as f64 for native Excel number writing.
fn try_parse_number(s: &str) -> Option<f64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Avoid treating pure identifiers as numbers
    trimmed.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Try to parse a cell as boolean.
fn try_parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_table() {
        let md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |";
        let tables = parse_markdown_tables(md);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].headers, vec!["Name", "Age"]);
        assert_eq!(tables[0].rows.len(), 2);
        assert_eq!(tables[0].rows[0], vec!["Alice", "30"]);
        assert_eq!(tables[0].rows[1], vec!["Bob", "25"]);
    }

    #[test]
    fn parse_table_without_leading_pipes() {
        let md = "Name | Age\n--- | ---\nAlice | 30";
        let tables = parse_markdown_tables(md);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].headers, vec!["Name", "Age"]);
        assert_eq!(tables[0].rows[0], vec!["Alice", "30"]);
    }

    #[test]
    fn parse_table_with_alignment_colons() {
        let md = "| Left | Center | Right |\n| :--- | :---: | ---: |\n| a | b | c |";
        let tables = parse_markdown_tables(md);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].headers, vec!["Left", "Center", "Right"]);
        assert_eq!(tables[0].rows[0], vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_multiple_tables() {
        let md = "\
| A | B |
| --- | --- |
| 1 | 2 |

Some text between tables.

| C | D |
| --- | --- |
| 3 | 4 |
";
        let tables = parse_markdown_tables(md);
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].headers, vec!["A", "B"]);
        assert_eq!(tables[1].headers, vec!["C", "D"]);
    }

    #[test]
    fn parse_no_table_in_plain_text() {
        let md = "This is just plain text.\nNo tables here.";
        assert!(parse_markdown_tables(md).is_empty());
    }

    #[test]
    fn parse_pipe_in_text_not_table() {
        // Lines with pipes but no separator row should not be parsed as tables
        let md = "Some text | more text\nNext line";
        assert!(parse_markdown_tables(md).is_empty());
    }

    #[test]
    fn parse_table_with_uneven_columns() {
        let md = "| A | B | C |\n| --- | --- | --- |\n| 1 | 2 |";
        let tables = parse_markdown_tables(md);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].rows[0], vec!["1", "2", ""]); // padded to 3
    }

    #[test]
    fn export_xlsx_with_simple_table() {
        let md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |";
        let tmp =
            std::env::temp_dir().join(format!("vaultpilot-xlsx-test-{}.xlsx", std::process::id()));
        export_markdown_to_xlsx(md, &tmp).expect("export should succeed");
        assert!(tmp.exists(), "XLSX file should exist");
        assert!(
            tmp.metadata().unwrap().len() > 0,
            "file should not be empty"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_xlsx_no_tables_errors() {
        let md = "No tables here.";
        let tmp = std::env::temp_dir().join("vaultpilot-xlsx-test-empty.xlsx");
        let result = export_markdown_to_xlsx(md, &tmp);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_xlsx_multiple_tables_multiple_sheets() {
        let md = "\
| A | B |
| --- | --- |
| 1 | 2 |

| C | D |
| --- | --- |
| 3 | 4 |
";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-xlsx-multi-test-{}.xlsx",
            std::process::id()
        ));
        export_markdown_to_xlsx(md, &tmp).expect("export should succeed");
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_csv_to_xlsx_basic() {
        let csv = "Name,Age\nAlice,30\nBob,25";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-csv-xlsx-test-{}.xlsx",
            std::process::id()
        ));
        export_csv_to_xlsx(csv, &tmp, ',').expect("csv export should succeed");
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn parse_csv_line_with_quotes() {
        let line = r#"hello,"world, with comma",bye"#;
        let cells = parse_csv_line(line, ',');
        assert_eq!(cells, vec!["hello", "world, with comma", "bye"]);
    }

    #[test]
    fn parse_csv_line_with_escaped_quotes() {
        let line = r#""She said ""hi""",next"#;
        let cells = parse_csv_line(line, ',');
        assert_eq!(cells, vec![r#"She said "hi""#, "next"]);
    }

    #[test]
    fn try_parse_number_valid() {
        assert_eq!(try_parse_number("42"), Some(42.0));
        assert_eq!(try_parse_number("2.71"), Some(2.71_f64));
        assert_eq!(try_parse_number("-5"), Some(-5.0));
    }

    #[test]
    fn try_parse_number_invalid() {
        assert_eq!(try_parse_number("hello"), None);
        assert_eq!(try_parse_number(""), None);
        assert_eq!(try_parse_number("NaN"), None);
    }

    #[test]
    fn try_parse_bool_valid() {
        assert_eq!(try_parse_bool("true"), Some(true));
        assert_eq!(try_parse_bool("FALSE"), Some(false));
        assert_eq!(try_parse_bool("True"), Some(true));
    }

    #[test]
    fn try_parse_bool_invalid() {
        assert_eq!(try_parse_bool("yes"), None);
        assert_eq!(try_parse_bool("1"), None);
    }
}
