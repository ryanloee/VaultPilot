//! Regression test for #3276: Agent multi-format file export.
//!
//! Verifies that:
//! 1. ExportFormat enum parses all expected format strings
//! 2. XLSX export creates valid ZIP with expected OOXML parts
//! 3. DOCX export creates valid ZIP with document.xml
//! 4. HTML export produces well-formed HTML
//! 5. Marp export includes frontmatter
//! 6. ExportDocument AiActionType is registered and has correct metadata
//! 7. CLI export-format handler path works end-to-end

use crate::ai::actions::{AiActionRequest, AiActionType};
use crate::export::{
    self, export_csv_to_xlsx, export_markdown_to_docx, export_markdown_to_html,
    export_markdown_to_marp, export_markdown_to_xlsx, parse_markdown_tables, ExportFormat,
};

#[test]
fn issue_3276_export_format_all_variants_parse() {
    assert_eq!(ExportFormat::parse_format("xlsx"), Some(ExportFormat::Xlsx));
    assert_eq!(ExportFormat::parse_format("docx"), Some(ExportFormat::Docx));
    assert_eq!(ExportFormat::parse_format("html"), Some(ExportFormat::Html));
    assert_eq!(ExportFormat::parse_format("pdf"), Some(ExportFormat::Html));
    assert_eq!(
        ExportFormat::parse_format("pptx"),
        Some(ExportFormat::PptxMarp)
    );
    // Case-insensitive
    assert_eq!(ExportFormat::parse_format("XLSX"), Some(ExportFormat::Xlsx));
    assert_eq!(ExportFormat::parse_format("Docx"), Some(ExportFormat::Docx));
    // Aliases
    assert_eq!(
        ExportFormat::parse_format("excel"),
        Some(ExportFormat::Xlsx)
    );
    assert_eq!(ExportFormat::parse_format("word"), Some(ExportFormat::Docx));
    assert_eq!(
        ExportFormat::parse_format("slides"),
        Some(ExportFormat::PptxMarp)
    );
    // Invalid
    assert_eq!(ExportFormat::parse_format("rtf"), None);
    assert_eq!(ExportFormat::parse_format(""), None);
}

#[test]
fn issue_3276_export_action_type_registered() {
    // The ExportDocument variant must be in the all() list
    let all = AiActionType::all();
    assert!(
        all.contains(&AiActionType::ExportDocument),
        "ExportDocument must be in AiActionType::all()"
    );
}

#[test]
fn issue_3276_export_action_metadata() {
    let action = AiActionType::ExportDocument;
    assert_eq!(action.id(), "exportDocument");
    assert_eq!(action.label(), "导出文档");
    assert_eq!(AiActionType::from_id("exportDocument"), Some(action));
    assert_eq!(AiActionType::from_id("export_document"), Some(action));
    assert_eq!(AiActionType::from_id("export"), Some(action));
}

#[test]
fn issue_3276_export_request_has_format_field() {
    let req = AiActionRequest {
        action: AiActionType::ExportDocument,
        text: "Some note content".to_string(),
        target_language: None,
        tone: None,
        note_id: None,
        instruction: None,
        model: None,
        export_format: Some("xlsx".to_string()),
    };
    assert_eq!(req.export_format.as_deref(), Some("xlsx"));
}

#[test]
fn issue_3276_export_request_serializes_format() {
    let req = AiActionRequest {
        action: AiActionType::ExportDocument,
        text: "content".to_string(),
        target_language: None,
        tone: None,
        note_id: None,
        instruction: None,
        model: None,
        export_format: Some("docx".to_string()),
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"exportFormat\":\"docx\""));
}

#[test]
fn issue_3276_export_request_deserializes_without_format() {
    // Backward compatibility: requests without export_format should still work
    let json = r#"{"action":"summarize","text":"hello"}"#;
    let req: AiActionRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.export_format, None);
}

#[test]
fn issue_3276_xlsx_creates_valid_zip() {
    let md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |";
    let tmp = std::env::temp_dir().join(format!(
        "issue3276-xlsx-{}-{}.xlsx",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export_markdown_to_xlsx(md, &tmp).expect("XLSX export should succeed");
    assert!(tmp.exists(), "XLSX file must exist");
    assert!(
        tmp.metadata().unwrap().len() > 200,
        "XLSX should have content"
    );

    // Verify it's a valid ZIP with expected OOXML structure
    let file = std::fs::File::open(&tmp).unwrap();
    let archive = zip::ZipArchive::new(file).expect("should be valid ZIP");
    let names: Vec<&str> = archive.file_names().collect();
    assert!(
        names.contains(&"[Content_Types].xml"),
        "must have content types"
    );
    assert!(names.contains(&"xl/workbook.xml"), "must have workbook.xml");
    assert!(
        names.contains(&"xl/worksheets/sheet1.xml"),
        "must have sheet1.xml"
    );
    assert!(
        names.contains(&"xl/sharedStrings.xml"),
        "must have sharedStrings.xml"
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn issue_3276_xlsx_with_numbers_and_strings() {
    let md = "| Product | Qty | Price |\n| --- | --- | --- |\n| Widget | 10 | 9.99 |\n| Gadget | 5 | 19.99 |";
    let tmp = std::env::temp_dir().join(format!(
        "issue3276-xlsx-types-{}-{}.xlsx",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export_markdown_to_xlsx(md, &tmp).expect("export should succeed");
    assert!(tmp.exists());
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn issue_3276_xlsx_no_table_errors() {
    let md = "Just some text without any tables.";
    let tmp = std::env::temp_dir().join("issue3276-xlsx-no-table.xlsx");
    let result = export_markdown_to_xlsx(md, &tmp);
    assert!(result.is_err(), "should error when no tables found");
}

#[test]
fn issue_3276_csv_to_xlsx() {
    let csv = "Name,Age,City\nAlice,30,NYC\nBob,25,LA";
    let tmp = std::env::temp_dir().join(format!(
        "issue3276-csv-{}-{}.xlsx",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export_csv_to_xlsx(csv, &tmp, ',').expect("CSV export should succeed");
    assert!(tmp.exists());
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn issue_3276_docx_creates_valid_zip() {
    let md = "# My Report\n\nThis is a paragraph.\n\n## Section\n\n- Item 1\n- Item 2\n\n1. First\n2. Second\n";
    let tmp = std::env::temp_dir().join(format!(
        "issue3276-docx-{}-{}.docx",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export_markdown_to_docx(md, &tmp).expect("DOCX export should succeed");
    assert!(tmp.exists());
    assert!(
        tmp.metadata().unwrap().len() > 200,
        "DOCX should have content"
    );

    let file = std::fs::File::open(&tmp).unwrap();
    let archive = zip::ZipArchive::new(file).expect("should be valid ZIP");
    let names: Vec<&str> = archive.file_names().collect();
    assert!(
        names.contains(&"word/document.xml"),
        "must have document.xml"
    );
    assert!(
        names.contains(&"[Content_Types].xml"),
        "must have content types"
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn issue_3276_docx_with_table() {
    let md = "# Report\n\n| Name | Score |\n| --- | --- |\n| Alice | 95 |\n| Bob | 87 |\n";
    let tmp = std::env::temp_dir().join(format!(
        "issue3276-docx-table-{}-{}.docx",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export_markdown_to_docx(md, &tmp).expect("DOCX with table should succeed");
    assert!(tmp.exists());

    // Verify table XML exists
    let file = std::fs::File::open(&tmp).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut doc = String::new();
    use std::io::Read;
    archive
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut doc)
        .unwrap();
    assert!(doc.contains("<w:tbl>"), "document must contain table XML");

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn issue_3276_html_export_basic() {
    let md = "# Title\n\nParagraph with **bold** and *italic*.\n\n- List item\n";
    let tmp = std::env::temp_dir().join(format!(
        "issue3276-html-{}-{}.html",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export_markdown_to_html(md, "Test Doc", &tmp).expect("HTML export should succeed");
    assert!(tmp.exists());

    let content = std::fs::read_to_string(&tmp).unwrap();
    assert!(content.contains("<!DOCTYPE html>"));
    assert!(content.contains("<title>Test Doc</title>"));
    assert!(content.contains("<h1"));
    assert!(content.contains("<strong>bold</strong>"));
    assert!(content.contains("<em>italic</em>"));
    assert!(content.contains("<ul>"));
    assert!(content.contains("<li>List item</li>"));

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn issue_3276_html_with_table_and_code() {
    let md = "# Data\n\n| Col A | Col B |\n| --- | --- |\n| 1 | 2 |\n\nSome `inline code`.\n";
    let tmp = std::env::temp_dir().join(format!(
        "issue3276-html-table-{}-{}.html",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export_markdown_to_html(md, "Table Code", &tmp).expect("should succeed");
    let content = std::fs::read_to_string(&tmp).unwrap();
    assert!(content.contains("<table>"));
    assert!(content.contains("<th>Col A</th>"));
    assert!(content.contains("<td>1</td>"));
    assert!(content.contains("<code>inline code</code>"));

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn issue_3276_marp_export() {
    let md = "# Presentation\n\n## Slide One\n\nContent for slide 1.\n\n## Slide Two\n\nContent for slide 2.\n";
    let tmp = std::env::temp_dir().join(format!(
        "issue3276-marp-{}-{}.md",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export_markdown_to_marp(md, "Presentation", &tmp).expect("Marp export should succeed");
    assert!(tmp.exists());

    let content = std::fs::read_to_string(&tmp).unwrap();
    assert!(content.contains("marp: true"));
    assert!(content.contains("theme: default"));
    assert!(content.contains("# Presentation"));
    assert!(content.contains("## Slide One"));
    assert!(content.contains("## Slide Two"));

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn issue_3276_unified_export_dispatcher() {
    let md = "| A | B |\n| --- | --- |\n| 1 | 2 |";

    // XLSX via dispatcher
    let xlsx_path = std::env::temp_dir().join(format!(
        "issue3276-dispatch-xlsx-{}-{}.xlsx",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export::export_markdown(md, ExportFormat::Xlsx, "Test", &xlsx_path).expect("should succeed");
    assert!(xlsx_path.exists());
    let _ = std::fs::remove_file(&xlsx_path);

    // DOCX via dispatcher
    let docx_md = "# Title\n\nParagraph.\n";
    let docx_path = std::env::temp_dir().join(format!(
        "issue3276-dispatch-docx-{}-{}.docx",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export::export_markdown(docx_md, ExportFormat::Docx, "Test", &docx_path)
        .expect("should succeed");
    assert!(docx_path.exists());
    let _ = std::fs::remove_file(&docx_path);

    // HTML via dispatcher
    let html_path = std::env::temp_dir().join(format!(
        "issue3276-dispatch-html-{}-{}.html",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export::export_markdown(docx_md, ExportFormat::Html, "Test", &html_path)
        .expect("should succeed");
    assert!(html_path.exists());
    let _ = std::fs::remove_file(&html_path);

    // Marp via dispatcher
    let marp_path = std::env::temp_dir().join(format!(
        "issue3276-dispatch-marp-{}-{}.md",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    export::export_markdown(docx_md, ExportFormat::PptxMarp, "Test", &marp_path)
        .expect("should succeed");
    assert!(marp_path.exists());
    let _ = std::fs::remove_file(&marp_path);
}

#[test]
fn issue_3276_system_prompt_for_export() {
    // Verify the system prompt is non-empty and mentions export
    let prompt = crate::ai::actions::system_prompt(AiActionType::ExportDocument);
    assert!(!prompt.is_empty(), "system prompt must not be empty");
    assert!(
        prompt.to_lowercase().contains("export"),
        "must mention export"
    );
    assert!(prompt.to_lowercase().contains("xlsx"), "must mention xlsx");
    assert!(prompt.to_lowercase().contains("docx"), "must mention docx");
}

#[test]
fn issue_3276_user_prompt_includes_format() {
    let req = AiActionRequest {
        action: AiActionType::ExportDocument,
        text: "Some content".to_string(),
        target_language: None,
        tone: None,
        note_id: None,
        instruction: None,
        model: None,
        export_format: Some("xlsx".to_string()),
    };
    let prompt = crate::ai::actions::user_prompt(AiActionType::ExportDocument, &req);
    assert!(
        prompt.contains("xlsx"),
        "user prompt must mention the format"
    );
    assert!(
        prompt.contains("Some content"),
        "user prompt must include the text"
    );
}

#[test]
fn issue_3276_user_prompt_falls_back_to_instruction() {
    let req = AiActionRequest {
        action: AiActionType::ExportDocument,
        text: "Some content".to_string(),
        target_language: None,
        tone: None,
        note_id: None,
        instruction: Some("docx".to_string()),
        model: None,
        export_format: None,
    };
    let prompt = crate::ai::actions::user_prompt(AiActionType::ExportDocument, &req);
    assert!(
        prompt.contains("docx"),
        "should fall back to instruction field"
    );
}

#[test]
fn issue_3276_table_parser_preserves_data() {
    let md = "| Product | Price | Stock |\n| --- | --- | --- |\n| Laptop | $999 | 42 |\n| Phone | $699 | 100 |";
    let tables = parse_markdown_tables(md);
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].headers, vec!["Product", "Price", "Stock"]);
    assert_eq!(tables[0].rows.len(), 2);
    assert_eq!(tables[0].rows[0], vec!["Laptop", "$999", "42"]);
    assert_eq!(tables[0].rows[1], vec!["Phone", "$699", "100"]);
}

#[test]
fn issue_3276_export_format_extensions() {
    assert_eq!(ExportFormat::Xlsx.extension(), "xlsx");
    assert_eq!(ExportFormat::Docx.extension(), "docx");
    assert_eq!(ExportFormat::Html.extension(), "html");
    assert_eq!(ExportFormat::PptxMarp.extension(), "md");
}

#[test]
fn issue_3276_export_format_labels() {
    assert_eq!(ExportFormat::Xlsx.label(), "Excel (XLSX)");
    assert_eq!(ExportFormat::Docx.label(), "Word (DOCX)");
    assert_eq!(ExportFormat::Html.label(), "HTML");
    assert_eq!(ExportFormat::PptxMarp.label(), "Slides (Marp Markdown)");
}
