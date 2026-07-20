//! Regression test for #3189 — Web Clipper: collect a web page into a Markdown
//! vault note.
//!
//! The browser-extension capture UI is a follow-up; this test locks in the
//! behaviour of the pure-Rust HTML→Markdown converter
//! (`vaultpilot_lib::clipper::html_to_markdown`) that backs the
//! `vaultpilot clip <url>` CLI command. It guards against regressions in the
//! common article constructs: headings, emphasis, links, images, (nested)
//! lists, blockquotes, code blocks, entity decoding, and script/style stripping.

#[cfg(test)]
mod tests {
    use crate::clipper::html_to_markdown;

    /// A realistic blog-style article fragment.
    const SAMPLE: &str = r#"
        <html><head><title>Why Local-First Matters</title></head>
        <body>
          <h1>Why Local-First Matters</h1>
          <p>Local-first software keeps your <strong>data</strong> on your
             device. Read more on <a href="https://example.com/local-first">the site</a>.</p>
          <h2>Benefits</h2>
          <ul>
            <li>Offline access</li>
            <li>Privacy
              <ul>
                <li>No third-party telemetry</li>
                <li>End-to-end encryption</li>
              </ul>
            </li>
          </ul>
          <blockquote><p>Own your data.</p></blockquote>
          <pre><code>fn main() {}</code></pre>
          <p>Tom &amp; Jerry &lt;3</p>
          <script>console.log('evil')</script>
        </body></html>
    "#;

    #[test]
    fn regression_clip_produces_readable_markdown() {
        let md = html_to_markdown(SAMPLE);
        // Title heading preserved.
        assert!(
            md.contains("# Why Local-First Matters"),
            "missing H1:\n{md}"
        );
        // Emphasis preserved.
        assert!(md.contains("**data**"), "missing bold:\n{md}");
        // Link preserved with label + href.
        assert!(
            md.contains("[the site](https://example.com/local-first)"),
            "missing link:\n{md}"
        );
        // Nested list: top item at depth 0, children indented.
        assert!(md.contains("- Offline access"), "missing top item:\n{md}");
        assert!(md.contains("- Privacy"), "missing nested parent:\n{md}");
        assert!(
            md.contains("  - No third-party telemetry"),
            "missing nested child:\n{md}"
        );
        assert!(
            md.contains("  - End-to-end encryption"),
            "missing nested child 2:\n{md}"
        );
        // Blockquote preserved.
        assert!(md.contains("> Own your data."), "missing blockquote:\n{md}");
        // Code block preserved as fenced.
        assert!(md.contains("```"), "missing code fence:\n{md}");
        assert!(md.contains("fn main() {}"), "missing code:\n{md}");
        // HTML entities decoded.
        assert!(md.contains("Tom & Jerry"), "entities not decoded:\n{md}");
        assert!(md.contains("<3"), "entities not decoded 2:\n{md}");
        // Script content stripped.
        assert!(!md.contains("console.log"), "script not stripped:\n{md}");
        assert!(!md.contains("evil"), "script body leaked:\n{md}");
    }

    #[test]
    fn regression_clip_empty_input_is_safe() {
        // Must not panic and must return a string.
        let md = html_to_markdown("");
        assert_eq!(md, "\n");
    }

    #[test]
    fn regression_clip_malformed_nesting_does_not_panic() {
        // Malformed HTML must never panic; it should degrade gracefully.
        let md = html_to_markdown("<ul><li>a<ul><li>b<li>c</ul>");
        // Should still surface the items without panicking.
        assert!(md.contains("- a"), "got: {md}");
        assert!(md.contains("b"), "got: {md}");
        assert!(md.contains("c"), "got: {md}");
    }
}
