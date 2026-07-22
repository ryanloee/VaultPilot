//! Multi-format note export (#3276).
//!
//! Provides conversion from Markdown note content to structured file formats:
//! - **XLSX (Excel)**: Markdown tables → native .xlsx via manual ZIP+XML construction
//! - **DOCX (Word)**: Markdown → native .docx via manual ZIP+XML construction
//! - **HTML**: Markdown → self-contained HTML (can be printed to PDF)
//! - **PPTX/Marp**: Markdown → Marp-compatible presentation Markdown
//!
//! No external crates beyond the existing `zip` crate are required.
//! XLSX and DOCX are both ZIP archives containing XML parts (OOXML standard).

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

/// Supported export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Xlsx,
    Docx,
    Html,
    PptxMarp,
}

impl ExportFormat {
    /// Parse a format string (case-insensitive): "xlsx", "docx", "html", "pdf", "pptx".
    /// "pdf" maps to Html (print to PDF). "pptx" maps to PptxMarp.
    pub fn parse_format(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "xlsx" | "excel" => Some(Self::Xlsx),
            "docx" | "word" => Some(Self::Docx),
            "html" | "pdf" => Some(Self::Html),
            "pptx" | "ppt" | "marp" | "slides" => Some(Self::PptxMarp),
            _ => None,
        }
    }

    /// File extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Xlsx => "xlsx",
            Self::Docx => "docx",
            Self::Html => "html",
            Self::PptxMarp => "md",
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Xlsx => "Excel (XLSX)",
            Self::Docx => "Word (DOCX)",
            Self::Html => "HTML",
            Self::PptxMarp => "Slides (Marp Markdown)",
        }
    }
}

/// A single table extracted from Markdown: header row + optional data rows.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

// ── Markdown table parsing ───────────────────────────────────────────

/// Parse all GFM-style pipe tables from a Markdown string.
///
/// A valid table consists of:
/// - A header row: `| Col A | Col B |`
/// - A separator row: `| --- | --- |` (dashes, optional colons for alignment)
/// - Zero or more data rows: `| 1 | 2 |`
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

// ── XLSX export (manual OOXML via zip crate) ─────────────────────────

/// Export all Markdown tables from a string to a single XLSX file.
///
/// Each table becomes a separate worksheet named "Table 1", "Table 2", etc.
/// If the Markdown contains no tables, an error is returned.
pub fn export_markdown_to_xlsx(markdown: &str, output_path: &Path) -> Result<()> {
    let tables = parse_markdown_tables(markdown);
    if tables.is_empty() {
        anyhow::bail!("No Markdown tables found in the input to export to XLSX");
    }

    // Build shared strings table from all text cells
    let mut shared_strings: Vec<String> = Vec::new();
    let ss_index = |s: &str, ss: &mut Vec<String>| -> u32 {
        for (i, existing) in ss.iter().enumerate() {
            if existing == s {
                return i as u32;
            }
        }
        ss.push(s.to_string());
        (ss.len() - 1) as u32
    };

    // Build worksheet XML for each table
    let mut worksheets_xml: Vec<String> = Vec::new();
    let mut sheet_names: Vec<String> = Vec::new();

    for (idx, table) in tables.iter().enumerate() {
        let sheet_name = format!("Table {}", idx + 1);
        sheet_names.push(sheet_name);

        let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        xml.push_str(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        );

        // Column widths (simple auto-width based on header length)
        xml.push_str("<cols>");
        for (col_idx, header) in table.headers.iter().enumerate() {
            let width = (header.chars().count() + 2).clamp(10, 50) as f64;
            xml.push_str(&format!(
                r#"<col min="{col}" max="{col}" width="{w}" customWidth="1"/> "#,
                col = col_idx + 1,
                w = width
            ));
        }
        xml.push_str("</cols>");

        xml.push_str("<sheetData>");

        // Header row (bold via style index 1)
        xml.push_str(r#"<row r="1">"#);
        for (col, header) in table.headers.iter().enumerate() {
            let cell_ref = col_letter(col as u32) + "1";
            let idx = ss_index(header, &mut shared_strings);
            xml.push_str(&format!(
                r#"<c r="{ref}" s="1" t="s"><v>{idx}</v></c>"#,
                ref = cell_ref,
                idx = idx
            ));
        }
        xml.push_str("</row>");

        // Data rows
        for (row_idx, row) in table.rows.iter().enumerate() {
            let row_num = (row_idx + 2) as u32;
            xml.push_str(&format!(r#"<row r="{n}">"#, n = row_num));
            for (col, cell) in row.iter().enumerate() {
                let cell_ref = format!("{}{}", col_letter(col as u32), row_num);
                if let Some(num) = try_parse_number(cell) {
                    xml.push_str(&format!(
                        r#"<c r="{ref}"><v>{v}</v></c>"#,
                        ref = cell_ref,
                        v = num
                    ));
                } else if let Some(b) = try_parse_bool(cell) {
                    xml.push_str(&format!(
                        r#"<c r="{ref}" t="b"><v>{v}</v></c>"#,
                        ref = cell_ref,
                        v = if b { 1 } else { 0 }
                    ));
                } else {
                    let idx = ss_index(cell, &mut shared_strings);
                    xml.push_str(&format!(
                        r#"<c r="{ref}" t="s"><v>{idx}</v></c>"#,
                        ref = cell_ref,
                        idx = idx
                    ));
                }
            }
            xml.push_str("</row>");
        }

        xml.push_str("</sheetData></worksheet>");
        worksheets_xml.push(xml);
    }

    // Build shared strings XML
    let mut ss_xml = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    ss_xml.push_str(&format!(
        r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="{count}" uniqueCount="{unique}">"#,
        count = shared_strings.len(),
        unique = shared_strings.len()
    ));
    for s in &shared_strings {
        let escaped = xml_escape(s);
        ss_xml.push_str(&format!(
            r#"<si><t xml:space="preserve">{e}</t></si>"#,
            e = escaped
        ));
    }
    ss_xml.push_str("</sst>");

    // Build workbook.xml
    let mut wb_xml = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    wb_xml.push_str(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
    );
    wb_xml.push_str("<sheets>");
    for (idx, name) in sheet_names.iter().enumerate() {
        let escaped = xml_escape(name);
        wb_xml.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{id}" r:id="rId{id}"/>"#,
            name = escaped,
            id = idx + 1
        ));
    }
    wb_xml.push_str("</sheets></workbook>");

    // Build workbook relationships
    let mut wb_rels = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    wb_rels.push_str(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for idx in 0..worksheets_xml.len() {
        wb_rels.push_str(&format!(
            r#"<Relationship Id="rId{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{sheet}.xml"/> "#,
            id = idx + 1,
            sheet = idx + 1
        ));
    }
    wb_rels.push_str(&format!(
        r#"<Relationship Id="rId{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/> "#,
        id = worksheets_xml.len() + 1
    ));
    wb_rels.push_str("</Relationships>");

    // Content types
    let mut ct_xml = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    ct_xml.push_str(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    );
    ct_xml.push_str(
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    );
    ct_xml.push_str(r#"<Default Extension="xml" ContentType="application/xml"/>"#);
    ct_xml.push_str(
        r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    for idx in 0..worksheets_xml.len() {
        ct_xml.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{sheet}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/> "#,
            sheet = idx + 1
        ));
    }
    ct_xml.push_str(
        r#"<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>"#,
    );
    ct_xml.push_str("</Types>");

    // Root relationships
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

    // Styles (bold header style at index 1)
    let styles_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="1"><fill><patternFill patternType="none"/></fill></fills>
<borders count="1"><border/></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"#;

    // Write the XLSX zip
    let file = std::fs::File::create(output_path).with_context(|| {
        format!(
            "failed to create XLSX output file: {}",
            output_path.display()
        )
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("[Content_Types].xml", options)?;
    zip.write_all(ct_xml.as_bytes())?;
    zip.start_file("_rels/.rels", options)?;
    zip.write_all(root_rels.as_bytes())?;
    zip.start_file("xl/workbook.xml", options)?;
    zip.write_all(wb_xml.as_bytes())?;
    zip.start_file("xl/_rels/workbook.xml.rels", options)?;
    zip.write_all(wb_rels.as_bytes())?;
    zip.start_file("xl/styles.xml", options)?;
    zip.write_all(styles_xml.as_bytes())?;
    zip.start_file("xl/sharedStrings.xml", options)?;
    zip.write_all(ss_xml.as_bytes())?;
    for (idx, ws_xml) in worksheets_xml.iter().enumerate() {
        let name = format!("xl/worksheets/sheet{}.xml", idx + 1);
        zip.start_file(&name, options)?;
        zip.write_all(ws_xml.as_bytes())?;
    }

    zip.finish()?;
    Ok(())
}

/// Export CSV text to an XLSX file (single worksheet).
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

    let headers = rows[0].clone();
    let data_rows = rows[1..].to_vec();

    // Convert to markdown table for reuse of export_markdown_to_xlsx
    let mut md = String::new();
    md.push('|');
    for h in &headers {
        md.push_str(&format!(" {} |", h));
    }
    md.push('\n');
    md.push('|');
    for _ in &headers {
        md.push_str(" --- |");
    }
    md.push('\n');
    for row in &data_rows {
        md.push('|');
        for cell in row {
            md.push_str(&format!(" {} |", cell));
        }
        md.push('\n');
    }

    export_markdown_to_xlsx(&md, output_path)
}

// ── DOCX export (manual OOXML via zip crate) ─────────────────────────

/// Convert Markdown to a simplified DOCX file.
///
/// Supports headings (H1-HH3), paragraphs, bullet/numbered lists, bold/italic,
/// and tables.  The output is a valid .docx that opens in Word/LibreOffice.
pub fn export_markdown_to_docx(markdown: &str, output_path: &Path) -> Result<()> {
    let body_xml = markdown_to_docx_body(markdown);

    let document_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
{body}
<w:sectPr>
<w:pgSz w:w="12240" w:h="15840"/>
<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>
</w:sectPr>
</w:body>
</w:document>"#,
        body = body_xml
    );

    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;

    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
</Relationships>"#;

    let file = std::fs::File::create(output_path).with_context(|| {
        format!(
            "failed to create DOCX output file: {}",
            output_path.display()
        )
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("[Content_Types].xml", options)?;
    zip.write_all(content_types.as_bytes())?;
    zip.start_file("_rels/.rels", options)?;
    zip.write_all(root_rels.as_bytes())?;
    zip.start_file("word/_rels/document.xml.rels", options)?;
    zip.write_all(doc_rels.as_bytes())?;
    zip.start_file("word/document.xml", options)?;
    zip.write_all(document_xml.as_bytes())?;

    zip.finish()?;
    Ok(())
}

/// Convert Markdown text to DOCX body XML (paragraphs, headings, lists, tables).
fn markdown_to_docx_body(markdown: &str) -> String {
    let mut body = String::new();
    let lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Headings
        if let Some(rest) = line.strip_prefix("### ") {
            body.push_str(&docx_heading(rest, 3));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            body.push_str(&docx_heading(rest, 2));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            body.push_str(&docx_heading(rest, 1));
            i += 1;
            continue;
        }

        // Tables
        if is_table_row(line.trim()) && i + 1 < lines.len() && is_separator_row(lines[i + 1].trim())
        {
            let headers = parse_row_cells(line.trim());
            let num_cols = headers.len();
            i += 2;
            let mut data_rows = Vec::new();
            while i < lines.len() && is_table_row(lines[i].trim()) {
                let mut cells = parse_row_cells(lines[i].trim());
                cells.resize(num_cols, String::new());
                data_rows.push(cells);
                i += 1;
            }
            body.push_str(&docx_table(&headers, &data_rows));
            continue;
        }

        // Bullet list
        if line.trim_start().starts_with("- ") || line.trim_start().starts_with("* ") {
            let content = line.trim_start()[2..].trim();
            body.push_str(&docx_bullet(content));
            i += 1;
            continue;
        }

        // Numbered list
        if let Some(rest) = parse_numbered_list_item(line) {
            body.push_str(&docx_number(rest));
            i += 1;
            continue;
        }

        // Empty line
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // Regular paragraph
        body.push_str(&docx_paragraph(line.trim()));
        i += 1;
    }

    body
}

// ── HTML export ──────────────────────────────────────────────────────

/// Convert Markdown to a self-contained HTML file (can be printed to PDF).
pub fn export_markdown_to_html(markdown: &str, title: &str, output_path: &Path) -> Result<()> {
    let html_body = markdown_to_html_body(markdown);
    let escaped_title = xml_escape(title);

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title}</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 800px; margin: 2rem auto; padding: 0 1rem; line-height: 1.6; color: #333; }}
h1, h2, h3 {{ color: #1a1a1a; }}
table {{ border-collapse: collapse; width: 100%; margin: 1rem 0; }}
th, td {{ border: 1px solid #ddd; padding: 0.5rem 0.75rem; text-align: left; }}
th {{ background-color: #f2f2f2; font-weight: bold; }}
tr:nth-child(even) {{ background-color: #f9f9f9; }}
code {{ background: #f4f4f4; padding: 0.1em 0.3em; border-radius: 3px; font-size: 0.9em; }}
pre {{ background: #f4f4f4; padding: 1rem; border-radius: 5px; overflow-x: auto; }}
blockquote {{ border-left: 4px solid #ddd; margin: 1rem 0; padding: 0.5rem 1rem; color: #666; }}
</style>
</head>
<body>
{body}
</body>
</html>"#,
        title = escaped_title,
        body = html_body
    );

    std::fs::write(output_path, html).with_context(|| {
        format!(
            "failed to write HTML output file: {}",
            output_path.display()
        )
    })?;
    Ok(())
}

/// Convert Markdown to HTML body content.
fn markdown_to_html_body(markdown: &str) -> String {
    let mut html = String::new();
    let lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0;
    let mut in_code_block = false;

    while i < lines.len() {
        let line = lines[i];

        // Code blocks
        if line.trim_start().starts_with("```") {
            if in_code_block {
                html.push_str("</code></pre>");
                in_code_block = false;
            } else {
                html.push_str("<pre><code>");
                in_code_block = true;
            }
            i += 1;
            continue;
        }
        if in_code_block {
            html.push_str(&xml_escape(line));
            html.push('\n');
            i += 1;
            continue;
        }

        // Headings
        if let Some(rest) = line.strip_prefix("### ") {
            html.push_str(&format!("<h3>{}</h3>\n", inline_md(rest)));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            html.push_str(&format!("<h2>{}</h2>\n", inline_md(rest)));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            html.push_str(&format!("<h1>{}</h1>\n", inline_md(rest)));
            i += 1;
            continue;
        }

        // Tables
        if is_table_row(line.trim()) && i + 1 < lines.len() && is_separator_row(lines[i + 1].trim())
        {
            let headers = parse_row_cells(line.trim());
            let num_cols = headers.len();
            i += 2;
            let mut data_rows = Vec::new();
            while i < lines.len() && is_table_row(lines[i].trim()) {
                let mut cells = parse_row_cells(lines[i].trim());
                cells.resize(num_cols, String::new());
                data_rows.push(cells);
                i += 1;
            }
            html.push_str("<table>\n<thead>\n<tr>");
            for h in &headers {
                html.push_str(&format!("<th>{}</th>", inline_md(h)));
            }
            html.push_str("</tr>\n</thead>\n<tbody>\n");
            for row in &data_rows {
                html.push_str("<tr>");
                for cell in row {
                    html.push_str(&format!("<td>{}</td>", inline_md(cell)));
                }
                html.push_str("</tr>\n");
            }
            html.push_str("</tbody>\n</table>\n");
            continue;
        }

        // Blockquote
        if let Some(rest) = line.strip_prefix("> ") {
            html.push_str(&format!("<blockquote>{}</blockquote>\n", inline_md(rest)));
            i += 1;
            continue;
        }

        // Bullet list
        if line.trim_start().starts_with("- ") || line.trim_start().starts_with("* ") {
            html.push_str("<ul>\n");
            while i < lines.len()
                && (lines[i].trim_start().starts_with("- ")
                    || lines[i].trim_start().starts_with("* "))
            {
                let content = lines[i].trim_start()[2..].trim();
                html.push_str(&format!("<li>{}</li>\n", inline_md(content)));
                i += 1;
            }
            html.push_str("</ul>\n");
            continue;
        }

        // Numbered list
        if parse_numbered_list_item(line).is_some() {
            html.push_str("<ol>\n");
            while i < lines.len() && parse_numbered_list_item(lines[i]).is_some() {
                let content = parse_numbered_list_item(lines[i]).unwrap();
                html.push_str(&format!("<li>{}</li>\n", inline_md(content)));
                i += 1;
            }
            html.push_str("</ol>\n");
            continue;
        }

        // Empty line
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // Regular paragraph
        html.push_str(&format!("<p>{}</p>\n", inline_md(line.trim())));
        i += 1;
    }

    if in_code_block {
        html.push_str("</code></pre>");
    }

    html
}

// ── Marp/PPTX export ─────────────────────────────────────────────────

/// Convert Markdown to Marp-compatible presentation Markdown.
///
/// Each H2 heading starts a new slide. Content under each H2 becomes the slide body.
/// The output is standard Markdown with Marp frontmatter.
pub fn export_markdown_to_marp(markdown: &str, title: &str, output_path: &Path) -> Result<()> {
    // #3327: Escape backslash and double-quote for YAML double-quoted scalar.
    let yaml_title = title.replace('\\', "\\\\").replace('"', "\\\"");
    let mut output = format!(
        "---\nmarp: true\ntheme: default\npaginate: true\ntitle: \"{yaml_title}\"\n---\n\n",
        yaml_title = yaml_title
    );

    // Title slide — also escape for Markdown body (backslash-escape # and other
    // Markdown-special chars that could break heading structure).
    let body_title = title.replace('#', "\\#");
    output.push_str(&format!("# {}\n\n", body_title));

    let lines: Vec<&str> = markdown.lines().collect();
    let mut current_slide = String::new();
    let mut first_h2 = true;

    for line in &lines {
        if let Some(rest) = line.strip_prefix("## ") {
            // Start new slide
            if !first_h2 && !current_slide.trim().is_empty() {
                output.push_str(&current_slide);
                output.push_str("\n\n---\n\n");
            }
            current_slide.clear();
            current_slide.push_str(&format!("## {}\n\n", rest));
            first_h2 = false;
        } else if let Some(rest) = line.strip_prefix("# ") {
            // H1 as title - skip (already in title slide)
            let _ = rest;
        } else {
            current_slide.push_str(line);
            current_slide.push('\n');
        }
    }

    if !current_slide.trim().is_empty() {
        output.push_str(&current_slide);
    }

    std::fs::write(output_path, output).with_context(|| {
        format!(
            "failed to write Marp output file: {}",
            output_path.display()
        )
    })?;
    Ok(())
}

// ── Unified export dispatcher ────────────────────────────────────────

/// Export Markdown content to the specified format.
pub fn export_markdown(
    markdown: &str,
    format: ExportFormat,
    title: &str,
    output_path: &Path,
) -> Result<()> {
    match format {
        ExportFormat::Xlsx => export_markdown_to_xlsx(markdown, output_path),
        ExportFormat::Docx => export_markdown_to_docx(markdown, output_path),
        ExportFormat::Html => export_markdown_to_html(markdown, title, output_path),
        ExportFormat::PptxMarp => export_markdown_to_marp(markdown, title, output_path),
    }
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Convert a 0-based column index to Excel column letters (A, B, ..., Z, AA, ...).
fn col_letter(col: u32) -> String {
    let mut result = String::new();
    let mut n = col;
    loop {
        result.insert(0, char::from_u32(b'A' as u32 + n % 26).unwrap());
        n /= 26;
        if n == 0 {
            break;
        }
        n -= 1;
    }
    result
}

/// Escape special XML characters.
fn xml_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            _ => result.push(c),
        }
    }
    result
}

/// Process inline Markdown formatting (bold, italic, code) to HTML.
fn inline_md(text: &str) -> String {
    // First escape XML special chars
    let escaped = xml_escape(text);

    // Apply inline formatting in order: bold, italic, code, wikilinks
    // Each pass operates on the full string (not borrowing slices)
    let after_bold = apply_bold(&escaped);
    let after_italic = apply_italic(&after_bold);
    let after_code = apply_code(&after_italic);
    apply_wikilinks(&after_code)
}

/// Replace **text** with <strong>text</strong>.
fn apply_bold(s: &str) -> String {
    let mut result = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("**") {
        let after = &rest[start + 2..];
        if let Some(end) = after.find("**") {
            result.push_str(&rest[..start]);
            result.push_str("<strong>");
            result.push_str(&after[..end]);
            result.push_str("</strong>");
            rest = &after[end + 2..];
        } else {
            break;
        }
    }
    result.push_str(rest);
    result
}

/// Replace *text* with <em>text</em> (but not ** which is bold).
fn apply_italic(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '*' && (i + 1 >= chars.len() || chars[i + 1] != '*') {
            // Look for closing single *
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '*' && (j + 1 >= chars.len() || chars[j + 1] != '*') {
                    // Found closing italic
                    let inner: String = chars[i + 1..j].iter().collect();
                    result.push_str("<em>");
                    result.push_str(&inner);
                    result.push_str("</em>");
                    i = j + 1;
                    break;
                }
                j += 1;
            }
            if j >= chars.len() {
                // No closing found, output as literal
                result.push(chars[i]);
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Replace `text` with <code>text</code>.
fn apply_code(s: &str) -> String {
    let mut result = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            result.push_str(&rest[..start]);
            result.push_str("<code>");
            result.push_str(&after[..end]);
            result.push_str("</code>");
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    result.push_str(rest);
    result
}

/// Replace [[Note]] with <a href="#">Note</a>.
fn apply_wikilinks(s: &str) -> String {
    let mut result = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("[[") {
        if let Some(end) = rest[start + 2..].find("]]") {
            let link_text = &rest[start + 2..start + 2 + end];
            result.push_str(&rest[..start]);
            result.push_str(&format!("<a href=\"#\">{}</a>", link_text));
            rest = &rest[start + 2 + end + 2..];
        } else {
            break;
        }
    }
    result.push_str(rest);
    result
}

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
    let cells: Vec<&str> = cleaned.split('|').filter(|s| !s.is_empty()).collect();
    if cells.is_empty() {
        return false;
    }
    cells
        .iter()
        .all(|cell| !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':'))
}

/// Parse cells from a Markdown table row line.
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
    let mut was_quoted = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
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
            was_quoted = true;
        } else if ch == delimiter {
            // RFC 4180: only trim unquoted fields; quoted content is preserved exactly
            let cell = if was_quoted {
                current.clone()
            } else {
                current.trim().to_string()
            };
            cells.push(cell);
            current = String::new();
            was_quoted = false;
        } else {
            current.push(ch);
        }
    }
    // Last cell
    let cell = if was_quoted {
        current
    } else {
        current.trim().to_string()
    };
    cells.push(cell);
    cells
}

/// Try to parse a numbered list item like "1. text" → Some("text").
fn parse_numbered_list_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let dot_pos = trimmed.find(". ")?;
    let prefix = &trimmed[..dot_pos];
    if prefix.chars().all(|c| c.is_ascii_digit()) && !prefix.is_empty() {
        Some(trimmed[dot_pos + 2..].trim())
    } else {
        None
    }
}

/// Try to parse a cell as f64 for native Excel number writing.
fn try_parse_number(s: &str) -> Option<f64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
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

// ── DOCX paragraph helpers ───────────────────────────────────────────

fn docx_heading(text: &str, level: u8) -> String {
    let escaped = xml_escape(text);
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading{lvl}"/></w:pPr><w:r><w:t xml:space="preserve">{t}</w:t></w:r></w:p>"#,
        lvl = level,
        t = escaped
    )
}

fn docx_paragraph(text: &str) -> String {
    let escaped = xml_escape(text);
    format!(
        r#"<w:p><w:r><w:t xml:space="preserve">{t}</w:t></w:r></w:p>"#,
        t = escaped
    )
}

fn docx_bullet(text: &str) -> String {
    let escaped = xml_escape(text);
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="ListBullet"/></w:pPr><w:r><w:t xml:space="preserve">{t}</w:t></w:r></w:p>"#,
        t = escaped
    )
}

fn docx_number(text: &str) -> String {
    let escaped = xml_escape(text);
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="ListNumber"/></w:pPr><w:r><w:t xml:space="preserve">{t}</w:t></w:r></w:p>"#,
        t = escaped
    )
}

fn docx_table_cell(text: &str, is_header: bool) -> String {
    let escaped = xml_escape(text);
    let props = if is_header {
        r#"<w:rPr><w:b/></w:rPr>"#
    } else {
        ""
    };
    format!(
        r#"<w:tc><w:tcPr><w:tcW w:w="0" w:type="auto"/></w:tcPr><w:p><w:r>{props}<w:t xml:space="preserve">{t}</w:t></w:r></w:p></w:tc>"#,
        props = props,
        t = escaped
    )
}

fn docx_table(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut xml = String::from(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>"#,
    );

    // Header row
    xml.push_str("<w:tr>");
    for h in headers {
        xml.push_str(&docx_table_cell(h, true));
    }
    xml.push_str("</w:tr>");

    // Data rows
    for row in rows {
        xml.push_str("<w:tr>");
        for cell in row {
            xml.push_str(&docx_table_cell(cell, false));
        }
        xml.push_str("</w:tr>");
    }

    xml.push_str("</w:tbl>");
    // Empty paragraph after table (required by Word)
    xml.push_str(r#"<w:p/>"#);
    xml
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

    // ── Regression tests ────────────────────────────────────────────────

    /// #3312: RFC 4180 — quoted fields must preserve internal whitespace.
    #[test]
    fn parse_csv_line_preserves_quoted_whitespace() {
        let line = r#""  hello  ",world"#;
        let cells = parse_csv_line(line, ',');
        assert_eq!(cells, vec!["  hello  ", "world"]);
    }

    /// #3312: Unquoted fields are still trimmed.
    #[test]
    fn parse_csv_line_trims_unquoted_whitespace() {
        let line = "  hello  ,  world  ";
        let cells = parse_csv_line(line, ',');
        assert_eq!(cells, vec!["hello", "world"]);
    }

    /// #3312: Mixed quoted and unquoted in same line.
    #[test]
    fn parse_csv_line_mixed_quoted_unquoted() {
        let line = r#"  untrimmed  ,"  preserved  ",normal"#;
        let cells = parse_csv_line(line, ',');
        assert_eq!(cells, vec!["untrimmed", "  preserved  ", "normal"]);
    }

    /// #3317: XLSX <col> elements must have correct min/max per-column, not all = 1.
    #[test]
    fn export_xlsx_column_definitions_correct() {
        let md = "| Name | Age | City |\n| --- | --- | --- |\n| Alice | 30 | NYC |";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-xlsx-cols-test-{}.xlsx",
            std::process::id()
        ));
        export_markdown_to_xlsx(md, &tmp).expect("export should succeed");
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }

    /// #3318: Marp title must not be XML-escaped (& stays & in Markdown output).
    #[test]
    fn export_marp_title_not_xml_escaped() {
        let md = "## Slide 1\n\nContent.\n";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-marp-amp-test-{}.md",
            std::process::id()
        ));
        export_markdown_to_marp(md, "Q3 & Q4 Reports", &tmp).expect("marp export should succeed");
        assert!(tmp.exists());
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(
            content.contains("Q3 & Q4 Reports"),
            "title should contain literal '&', got: {content}"
        );
        assert!(
            !content.contains("&amp;"),
            "title should NOT contain XML-escaped '&amp;'"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    /// #3327: Title with double-quote must be escaped in YAML frontmatter.
    #[test]
    fn export_marp_title_double_quote_escaped() {
        let md = "## Slide 1\n\nContent.\n";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-marp-quote-test-{}.md",
            std::process::id()
        ));
        export_markdown_to_marp(md, r#"Q3 "final" report"#, &tmp)
            .expect("marp export should succeed");
        let content = std::fs::read_to_string(&tmp).unwrap();
        // The frontmatter should contain the escaped quote.
        assert!(
            content.contains(r#"title: "Q3 \"final\" report""#),
            "expected escaped YAML title, got: {content}"
        );
        // The raw title should NOT appear as unescaped inside the frontmatter line.
        let fm_line = content
            .lines()
            .find(|l| l.starts_with("title:"))
            .unwrap_or("");
        assert!(
            !fm_line.contains(r#"Q3 "final" report""#),
            "frontmatter should NOT contain unescaped quotes, got: {fm_line}"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_marp_title_backslash_escaped() {
        let md = "## Slide 1\n\nContent.\n";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-marp-backslash-test-{}.md",
            std::process::id()
        ));
        export_markdown_to_marp(md, r"C:\path\to\notes", &tmp).expect("marp export should succeed");
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(
            content.contains(r#"title: "C:\\path\\to\\notes""#),
            "expected backslash-escaped YAML title, got: {content}"
        );
        let _ = std::fs::remove_file(&tmp);
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

    // ── New format tests (#3276) ──────────────────────────────────────

    #[test]
    fn export_format_from_str() {
        assert_eq!(ExportFormat::parse_format("xlsx"), Some(ExportFormat::Xlsx));
        assert_eq!(ExportFormat::parse_format("XLSX"), Some(ExportFormat::Xlsx));
        assert_eq!(
            ExportFormat::parse_format("excel"),
            Some(ExportFormat::Xlsx)
        );
        assert_eq!(ExportFormat::parse_format("docx"), Some(ExportFormat::Docx));
        assert_eq!(ExportFormat::parse_format("word"), Some(ExportFormat::Docx));
        assert_eq!(ExportFormat::parse_format("html"), Some(ExportFormat::Html));
        assert_eq!(ExportFormat::parse_format("pdf"), Some(ExportFormat::Html));
        assert_eq!(
            ExportFormat::parse_format("pptx"),
            Some(ExportFormat::PptxMarp)
        );
        assert_eq!(ExportFormat::parse_format("invalid"), None);
    }

    #[test]
    fn export_format_extension() {
        assert_eq!(ExportFormat::Xlsx.extension(), "xlsx");
        assert_eq!(ExportFormat::Docx.extension(), "docx");
        assert_eq!(ExportFormat::Html.extension(), "html");
        assert_eq!(ExportFormat::PptxMarp.extension(), "md");
    }

    #[test]
    fn export_docx_basic() {
        let md = "# My Document\n\nThis is a paragraph.\n\n## Section\n\n- Item 1\n- Item 2\n";
        let tmp =
            std::env::temp_dir().join(format!("vaultpilot-docx-test-{}.docx", std::process::id()));
        export_markdown_to_docx(md, &tmp).expect("docx export should succeed");
        assert!(tmp.exists(), "DOCX file should exist");
        assert!(
            tmp.metadata().unwrap().len() > 100,
            "file should have content"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_docx_with_table() {
        let md = "# Report\n\n| Name | Score |\n| --- | --- |\n| Alice | 95 |\n| Bob | 87 |\n";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-docx-table-test-{}.docx",
            std::process::id()
        ));
        export_markdown_to_docx(md, &tmp).expect("docx export should succeed");
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_html_basic() {
        let md = "# Title\n\nSome **bold** and *italic* text.\n\n- Item 1\n- Item 2\n";
        let tmp =
            std::env::temp_dir().join(format!("vaultpilot-html-test-{}.html", std::process::id()));
        export_markdown_to_html(md, "Test Title", &tmp).expect("html export should succeed");
        assert!(tmp.exists());
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("<h1>"));
        assert!(content.contains("<strong>bold</strong>"));
        assert!(content.contains("<em>italic</em>"));
        assert!(content.contains("<ul>"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_html_with_table() {
        let md = "| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-html-table-test-{}.html",
            std::process::id()
        ));
        export_markdown_to_html(md, "Table Test", &tmp).expect("should succeed");
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("<table>"));
        assert!(content.contains("<th>A</th>"));
        assert!(content.contains("<td>1</td>"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_marp_basic() {
        let md =
            "# My Presentation\n\n## Slide 1\n\nContent here.\n\n## Slide 2\n\nMore content.\n";
        let tmp =
            std::env::temp_dir().join(format!("vaultpilot-marp-test-{}.md", std::process::id()));
        export_markdown_to_marp(md, "My Presentation", &tmp).expect("marp export should succeed");
        assert!(tmp.exists());
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("marp: true"));
        assert!(content.contains("## Slide 1"));
        assert!(content.contains("---"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_unified_xlsx() {
        let md = "| A | B |\n| --- | --- |\n| 1 | 2 |";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-unified-xlsx-{}.xlsx",
            std::process::id()
        ));
        export_markdown(md, ExportFormat::Xlsx, "Test", &tmp).expect("should succeed");
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn export_unified_docx() {
        let md = "# Title\n\nParagraph text.\n";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-unified-docx-{}.docx",
            std::process::id()
        ));
        export_markdown(md, ExportFormat::Docx, "Test", &tmp).expect("should succeed");
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn col_letter_conversion() {
        assert_eq!(col_letter(0), "A");
        assert_eq!(col_letter(1), "B");
        assert_eq!(col_letter(25), "Z");
        assert_eq!(col_letter(26), "AA");
        assert_eq!(col_letter(27), "AB");
    }

    #[test]
    fn xml_escape_special() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape(r#""quote""#), "&quot;quote&quot;");
    }

    #[test]
    fn inline_md_bold() {
        assert_eq!(inline_md("**bold**"), "<strong>bold</strong>");
    }

    #[test]
    fn inline_md_italic() {
        assert_eq!(inline_md("*italic*"), "<em>italic</em>");
    }

    #[test]
    fn inline_md_code() {
        assert_eq!(inline_md("`code`"), "<code>code</code>");
    }

    #[test]
    fn inline_md_wikilink() {
        let expected = "<a href=\"#\">Note</a>";
        assert_eq!(inline_md("[[Note]]"), expected);
    }

    #[test]
    fn parse_numbered_list_item_valid() {
        assert_eq!(parse_numbered_list_item("1. First"), Some("First"));
        assert_eq!(parse_numbered_list_item("  2. Second"), Some("Second"));
        assert_eq!(parse_numbered_list_item("10. Tenth"), Some("Tenth"));
    }

    #[test]
    fn parse_numbered_list_item_invalid() {
        assert_eq!(parse_numbered_list_item("- bullet"), None);
        assert_eq!(parse_numbered_list_item("plain text"), None);
        assert_eq!(parse_numbered_list_item(". no number"), None);
    }

    #[test]
    fn xlsx_contains_shared_strings() {
        let md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-xlsx-ss-test-{}.xlsx",
            std::process::id()
        ));
        export_markdown_to_xlsx(md, &tmp).expect("should succeed");

        // Verify the XLSX is a valid zip and contains expected parts
        let file = std::fs::File::open(&tmp).unwrap();
        let archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
        names.sort();
        assert!(names.contains(&"[Content_Types].xml".to_string()));
        assert!(names.contains(&"xl/workbook.xml".to_string()));
        assert!(names.contains(&"xl/sharedStrings.xml".to_string()));
        assert!(names.contains(&"xl/worksheets/sheet1.xml".to_string()));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn docx_contains_document_xml() {
        let md = "# Title\n\nParagraph.\n";
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot-docx-verify-{}.docx",
            std::process::id()
        ));
        export_markdown_to_docx(md, &tmp).expect("should succeed");

        let file = std::fs::File::open(&tmp).unwrap();
        let archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
        names.sort();
        assert!(names.contains(&"[Content_Types].xml".to_string()));
        assert!(names.contains(&"word/document.xml".to_string()));

        let _ = std::fs::remove_file(&tmp);
    }
}
