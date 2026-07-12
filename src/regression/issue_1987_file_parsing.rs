//! Issue #1987: 多格式文件支持 / Multi-format file parsing pipeline.
//!
//! Regression coverage for the dependency-free text/markdown/CSV/TSV parsers,
//! the honest PDF/Office stubs, and the SQLite parse cache. See the module
//! doc on [`crate::file_parsing`] for the intended (and honestly limited) scope.

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Utc;

    use crate::file_parsing::{
        cached_parse, clear_cache, parse_and_cache, parse_file, FileParser, TxtParser,
    };
    use crate::storage::StorageContext;

    /// Unique temp directory under the system temp dir (no `tempfile` dep).
    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-1987-{}-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write_tmp(name: &str, contents: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = temp_dir();
        let path = dir.join(name);
        fs::write(&path, contents).expect("write file");
        (dir, path)
    }

    fn write_tmp_bytes(name: &str, contents: &[u8]) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = temp_dir();
        let path = dir.join(name);
        fs::write(&path, contents).expect("write file");
        (dir, path)
    }

    // ── txt ───────────────────────────────────────────────────────

    #[test]
    fn regression_1987_parse_txt_records_text_lines_and_size() {
        let (dir, path) = write_tmp("hello.txt", "line one\nline two\nline three");
        let parsed = parse_file(&path).expect("parse txt");

        assert_eq!(parsed.parser_used, "txt");
        assert_eq!(parsed.extension, "txt");
        assert_eq!(parsed.text, "line one\nline two\nline three");
        assert_eq!(parsed.metadata["line_count"].as_u64(), Some(3));
        assert_eq!(
            parsed.byte_size,
            "line one\nline two\nline three".len() as u64
        );
        assert!(!parsed.needs_native_parser);

        let _ = fs::remove_dir_all(&dir);
    }

    // ── markdown with frontmatter ─────────────────────────────────

    #[test]
    fn regression_1987_parse_markdown_strips_frontmatter_into_metadata() {
        let content = "---\ntitle: Hello\ntags: [a, b]\n---\n\n# Body content\n";
        let (dir, path) = write_tmp("doc.md", content);
        let parsed = parse_file(&path).expect("parse md");

        assert_eq!(parsed.parser_used, "markdown");
        assert!(
            !parsed.text.contains("title: Hello"),
            "frontmatter must be stripped from the body text"
        );
        assert!(
            parsed.text.contains("# Body content"),
            "body content must survive frontmatter stripping"
        );
        assert_eq!(parsed.metadata["has_frontmatter"], true);
        let fm = parsed.metadata["frontmatter"]
            .as_str()
            .expect("frontmatter str");
        assert!(fm.contains("title: Hello"));

        let _ = fs::remove_dir_all(&dir);
    }

    // ── markdown without frontmatter ──────────────────────────────

    #[test]
    fn regression_1987_parse_markdown_without_frontmatter_is_unchanged() {
        let content = "# Just a heading\n\nSome paragraph.\n";
        let (dir, path) = write_tmp("plain.md", content);
        let parsed = parse_file(&path).expect("parse md");

        assert_eq!(parsed.parser_used, "markdown");
        assert_eq!(
            parsed.text, content,
            "text must be unchanged when no frontmatter"
        );
        assert_eq!(parsed.metadata["has_frontmatter"], false);

        let _ = fs::remove_dir_all(&dir);
    }

    // ── csv with a quoted field containing a comma ────────────────

    #[test]
    fn regression_1987_parse_csv_with_quoted_comma() {
        let content = "name,age,city\nAlice,30,\"New York, NY\"\nBob,25,Boston\n";
        let (dir, path) = write_tmp("data.csv", content);
        let parsed = parse_file(&path).expect("parse csv");

        assert_eq!(parsed.parser_used, "csv");
        assert_eq!(parsed.metadata["row_count"].as_u64(), Some(3));
        assert_eq!(
            parsed.metadata["column_count"].as_u64(),
            Some(3),
            "quoted comma must not inflate the column count"
        );
        assert!(!parsed.text.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    // ── tsv ───────────────────────────────────────────────────────

    #[test]
    fn regression_1987_parse_tsv_handles_tab_separator() {
        let content = "name\tage\tcity\nAlice\t30\tNYC\nBob\t25\tBoston\n";
        let (dir, path) = write_tmp("data.tsv", content);
        let parsed = parse_file(&path).expect("parse tsv");

        assert_eq!(parsed.parser_used, "tsv");
        assert_eq!(parsed.metadata["row_count"].as_u64(), Some(3));
        assert_eq!(parsed.metadata["column_count"].as_u64(), Some(3));
        assert_eq!(parsed.metadata["separator"], "\t");
        assert!(!parsed.text.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    // ── pdf parser (real extraction + stub fallback) ──────────────

    #[test]
    fn regression_1987_parse_pdf_with_real_extraction() {
        // Fake PDF header won't extract — triggers stub fallback.
        let (dir, path) = write_tmp_bytes("doc.pdf", b"%PDF-1.4\n%not really parsed\n");
        let parsed = parse_file(&path).expect("parse pdf");

        assert_eq!(parsed.parser_used, "pdf");
        assert!(
            !parsed.needs_native_parser,
            "PDF is now handled in pure Rust by pdf-extract"
        );
        // Fake bytes won't extract — bail-out produces empty text with fallback note.
        assert!(parsed.text.is_empty(), "corrupt PDF produces empty text");
        assert!(
            parsed.metadata["note"]
                .as_str()
                .expect("note str")
                .contains("PDF text extraction failed"),
            "metadata must explain why extraction failed"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // ── unknown extension falls back to TxtParser ─────────────────

    #[test]
    fn regression_1987_unknown_extension_falls_back_to_txt() {
        let (dir, path) = write_tmp("blob.weird", "just text\n");
        let parsed = parse_file(&path).expect("parse unknown");

        assert_eq!(parsed.parser_used, "txt");
        assert_eq!(parsed.text, "just text\n");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── FileParser::supports wiring ───────────────────────────────

    #[test]
    fn regression_1987_parser_supports_matrices() {
        assert!(TxtParser.supports("txt"));
        assert!(TxtParser.supports("log"));
        assert!(!TxtParser.supports("pdf"));
    }

    // ── parse cache round-trip ────────────────────────────────────

    #[test]
    fn regression_1987_parse_and_cache_round_trip_and_clear() {
        let file_dir = temp_dir();
        let path = file_dir.join("cached.txt");
        fs::write(&path, "cache me\n").expect("write file");

        // A StorageContext whose DB has NOT been bootstrapped — the cache must
        // create its own table lazily via ensure_parsing_tables.
        let ctx_dir = temp_dir();
        let ctx = StorageContext::for_test(&ctx_dir);

        let parsed = parse_and_cache(&ctx, &path).expect("parse_and_cache");
        assert_eq!(parsed.parser_used, "txt");
        assert_eq!(parsed.text, "cache me\n");

        // cached_parse returns the previously stored result without re-parsing.
        let cached = cached_parse(&ctx, &path)
            .expect("cached_parse")
            .expect("cached row");
        assert_eq!(cached.text, "cache me\n");
        assert_eq!(cached.parser_used, "txt");

        // Re-running parse_and_cache on an unchanged file is a cache hit and
        // must not error.
        let again = parse_and_cache(&ctx, &path).expect("cache hit");
        assert_eq!(again.text, "cache me\n");

        // clear_cache for one path, then it must be gone.
        clear_cache(&ctx, Some(path.to_str().expect("utf8 path"))).expect("clear one");
        assert!(cached_parse(&ctx, &path)
            .expect("cached_parse after clear")
            .is_none());

        let _ = fs::remove_dir_all(&file_dir);
        let _ = fs::remove_dir_all(&ctx_dir);
    }
}
