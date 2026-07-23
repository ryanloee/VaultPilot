//! Regression test for #3342 — Markdown Callouts support.
//!
//! Verifies that Obsidian-style callout syntax (`> [!NOTE]`, `> [!WARNING]-`, etc.)
//! is correctly parsed and rendered as styled HTML divs instead of plain blockquotes.
//!
//! Tests cover all 12 Obsidian callout types plus known aliases,
//! collapse modifiers (-/+), optional titles, case insensitivity,
//! and fallback behavior for unknown types.

use crate::export::{
    callout_css_class, export_markdown, export_markdown_to_html, markdown_to_html_body,
    parse_callout_line, ExportFormat,
};

#[test]
fn regression_3342_all_callout_types_parsed() {
    let types = [
        "NOTE",
        "ABSTRACT",
        "SUMMARY",
        "TLDR",
        "INFO",
        "TODO",
        "TIP",
        "HINT",
        "IMPORTANT",
        "SUCCESS",
        "CHECK",
        "DONE",
        "QUESTION",
        "HELP",
        "FAQ",
        "WARNING",
        "CAUTION",
        "ATTENTION",
        "FAILURE",
        "FAIL",
        "MISSING",
        "DANGER",
        "ERROR",
        "BUG",
        "EXAMPLE",
        "QUOTE",
        "CITE",
    ];

    for callout_type in types {
        let line = format!("> [!{callout_type}]");
        let (is_callout, typ, _title, _collapse) = parse_callout_line(&line);
        assert!(is_callout, "should detect [!{callout_type}] as callout");
        assert_eq!(
            typ.to_uppercase(),
            callout_type.to_uppercase(),
            "type mismatch for [!{callout_type}]"
        );
    }
}

#[test]
fn regression_3342_all_callout_css_classes_exist() {
    let aliases = [
        ("note", "callout-note"),
        ("abstract", "callout-abstract"),
        ("summary", "callout-abstract"),
        ("tldr", "callout-abstract"),
        ("info", "callout-info"),
        ("todo", "callout-info"),
        ("tip", "callout-tip"),
        ("hint", "callout-tip"),
        ("important", "callout-tip"),
        ("success", "callout-success"),
        ("check", "callout-success"),
        ("done", "callout-success"),
        ("question", "callout-question"),
        ("help", "callout-question"),
        ("faq", "callout-question"),
        ("warning", "callout-warning"),
        ("caution", "callout-warning"),
        ("attention", "callout-warning"),
        ("failure", "callout-failure"),
        ("fail", "callout-failure"),
        ("missing", "callout-failure"),
        ("danger", "callout-danger"),
        ("error", "callout-danger"),
        ("bug", "callout-bug"),
        ("example", "callout-example"),
        ("quote", "callout-quote"),
        ("cite", "callout-quote"),
    ];

    for (alias, expected_class) in aliases {
        assert_eq!(
            callout_css_class(alias),
            expected_class,
            "CSS class mismatch for '{alias}'"
        );
    }

    // Unknown type falls back to callout-note
    assert_eq!(callout_css_class("UNKNOWN"), "callout-note");
    assert_eq!(callout_css_class("custom"), "callout-note");
}

#[test]
fn regression_3342_collapse_modifier_both_positions() {
    // Inside brackets: Obsidian standard syntax
    let line1 = "> [!NOTE]-";
    let (is, typ1, title1, coll1) = parse_callout_line(line1);
    assert!(is);
    assert_eq!(typ1, "NOTE");
    assert!(title1.is_empty());
    assert!(coll1);

    // After brackets (also supported for flexibility)
    let line2 = "> [!NOTE]- Title Here";
    let (is2, typ2, title2, coll2) = parse_callout_line(line2);
    assert!(is2);
    assert_eq!(typ2, "NOTE");
    assert_eq!(title2, "Title Here");
    assert!(coll2);

    // Explicit expand (+)
    let line3 = "> [!TIP]+ Expanded tip";
    let (is3, typ3, title3, coll3) = parse_callout_line(line3);
    assert!(is3);
    assert_eq!(typ3, "TIP");
    assert_eq!(title3, "Expanded tip");
    assert!(!coll3);
}

#[test]
fn regression_3342_callout_html_roundtrip() {
    // Full roundtrip: markdown input → HTML output → verify structure
    let md = "\
> [!WARNING] 重要提示
> 请勿在生产环境中使用此功能。
> 这可能导致数据丢失。

Some text after callout.
";

    let html = markdown_to_html_body(md);

    // Callout div structure
    assert!(
        html.contains("class=\"callout callout-warning\""),
        "should have callout + callout-warning classes"
    );
    assert!(
        html.contains("class=\"callout-title\""),
        "should have title div"
    );
    assert!(html.contains("重要提示"), "should include title text");
    assert!(
        html.contains("class=\"callout-body\""),
        "should have body div"
    );
    assert!(html.contains("请勿在生产环境中使用此功能"));
    assert!(html.contains("这可能导致数据丢失"));

    // Regular paragraph should still follow
    assert!(html.contains("<p>Some text after callout.</p>"));

    // Should NOT contain <blockquote> (callouts are divs, not blockquotes)
    assert!(
        !html.contains("<blockquote>"),
        "callouts should NOT render as blockquote: {html}"
    );
}

#[test]
fn regression_3342_mixed_callouts_and_blockquotes() {
    // Mix of callout and regular blockquote
    let md = "\
> [!INFO]
> This is a callout.

> This is a regular blockquote.
> With multiple lines.
";

    let html = markdown_to_html_body(md);

    assert!(
        html.contains("class=\"callout callout-info\""),
        "should contain callout div"
    );
    assert!(
        html.contains("<blockquote>"),
        "should contain regular blockquote"
    );
    assert!(html.contains("regular blockquote"));
}

#[test]
fn regression_3342_callout_in_html_export_file() {
    // Full HTML export to a file and verify callout CSS is included
    let md = "> [!TIP] 使用 Ctrl+Shift+P 打开命令面板。\n";
    let tmp = std::env::temp_dir().join(format!(
        "vaultpilot-callout-html-{}.html",
        std::process::id()
    ));

    export_markdown_to_html(md, "Callout Test", &tmp).expect("export should succeed");
    let content = std::fs::read_to_string(&tmp).unwrap();

    // CSS classes should be present in the style block
    assert!(
        content.contains(".callout-note"),
        "CSS callout-note missing"
    );
    assert!(
        content.contains(".callout-warning"),
        "CSS callout-warning missing"
    );
    assert!(content.contains(".callout-tip"), "CSS callout-tip missing");
    assert!(
        content.contains(".callout-danger"),
        "CSS callout-danger missing"
    );
    assert!(content.contains(".callout"), "base callout CSS missing");

    // HTML body should contain the callout div
    assert!(content.contains("callout-tip"), "tip callout not rendered");

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn regression_3342_callout_via_unified_export() {
    // Export via the unified export_markdown function (HTML format)
    let md = "> [!EXAMPLE] Example callout\n";
    let tmp = std::env::temp_dir().join(format!(
        "vaultpilot-callout-unified-{}.html",
        std::process::id()
    ));

    export_markdown(md, ExportFormat::Html, "Test", &tmp).expect("should succeed");
    let content = std::fs::read_to_string(&tmp).unwrap();

    assert!(
        content.contains("callout-example"),
        "callout-example class not in exported HTML"
    );
    assert!(content.contains("Example callout"));

    let _ = std::fs::remove_file(&tmp);
}
