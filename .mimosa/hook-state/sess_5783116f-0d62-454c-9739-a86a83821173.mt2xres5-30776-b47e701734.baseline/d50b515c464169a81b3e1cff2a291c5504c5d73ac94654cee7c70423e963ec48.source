// Regression tests for #3684: Markdown footnote rendering in HTML export.
//
// GFM-style footnotes: [^id] references in text + [^id]: definition at end.
// The markdown_to_html_body function now extracts definitions during a first
// pass and renders them as a <section class="footnotes"> at the bottom.

#[cfg(test)]
mod tests {
    use crate::export::markdown_to_html_body;

    #[test]
    fn regression_3684_footnote_refs_replaced_with_sup_links() {
        let input = "Some text with a footnote[^1] here.";
        let html = markdown_to_html_body(input);
        eprintln!("=== HTML ===\n{html}");
        assert!(
            html.contains("<sup id=\"fnref-1\">"),
            "should contain sup link for fnref-1, got: {html}"
        );
        assert!(
            html.contains("<a href=\"#fn-1\">[1]</a>"),
            "should link to [1], got: {html}"
        );
    }

    #[test]
    fn regression_3684_footnote_definitions_rendered_at_end() {
        let input = "Paragraph.[^hi]\n\n[^hi]: This is a footnote definition.";
        let html = markdown_to_html_body(input);
        assert!(
            html.contains("<section class=\"footnotes\">"),
            "should contain footnotes section"
        );
        assert!(
            html.contains("id=\"fn-hi\""),
            "should contain fn-hi definition"
        );
        assert!(
            html.contains("This is a footnote definition"),
            "should include footnote text"
        );
        assert!(html.contains("↩"), "should have back-reference link");
    }

    #[test]
    fn regression_3684_footnote_ids_sorted_numerically() {
        let input = "[^2]: second\n[^10]: tenth\n[^1]: first\n[^3]: third";
        let html = markdown_to_html_body(input);
        // Verify definitions appear and are ordered
        let fn1 = html.find("id=\"fn-1\"").expect("fn-1 should exist");
        let fn2 = html.find("id=\"fn-2\"").expect("fn-2 should exist");
        let fn3 = html.find("id=\"fn-3\"").expect("fn-3 should exist");
        let fn10 = html.find("id=\"fn-10\"").expect("fn-10 should exist");
        assert!(fn1 < fn2, "1 before 2");
        assert!(fn2 < fn3, "2 before 3");
        assert!(fn3 < fn10, "3 before 10");
    }

    #[test]
    fn regression_3684_footnote_with_tab_separator() {
        let input = "[^1]:\tHere is a footnote with tab separator.";
        let html = markdown_to_html_body(input);
        assert!(html.contains("Here is a footnote with tab separator"));
    }

    #[test]
    fn regression_3684_no_footnotes_produces_no_section() {
        let input = "Just a plain paragraph.";
        let html = markdown_to_html_body(input);
        assert!(!html.contains("class=\"footnotes\""));
    }

    #[test]
    fn regression_3684_footnote_ref_in_definition_text() {
        // Inline_md_footnote should not expand [^ref] references
        // within the definition body (to avoid cycles).
        // markdown_to_html_body should still render them as sup-refs.
        let input = "[^ref]: Here is a cross-reference to [^other] within the footnote.";
        let html = markdown_to_html_body(input);
        // There should be no sup ref inside the definition (inline_md_footnote)
        // The main content has no refs, but the footnote definition
        // is rendered by inline_md_footnote which skips expanding refs.
        assert!(
            html.contains("Here is a cross-reference to [^other]"),
            "footnote def should not expand nested refs"
        );
    }
}
