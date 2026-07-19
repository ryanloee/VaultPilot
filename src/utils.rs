use deunicode::deunicode;
use sha2::{Digest, Sha256};

/// Generate a file-system-safe slug from a title string.
///
/// Behaviour:
/// - CJK and other non-ASCII scripts are transliterated to ASCII via `deunicode`.
/// - Output is lowercased.
/// - Allowed characters: `[a-z0-9_-]`. Everything else becomes `-`.
/// - Consecutive dashes are collapsed into a single dash.
/// - Leading/trailing dashes are trimmed.
/// - If the result is empty, a hash-based fallback (`note-<hash>`) is returned.
///
/// This is the single canonical slugify used across the entire codebase.
/// Both the agent loop (path generation) and the storage layer (lookup)
/// produce identical slugs, preventing silent note-not-found errors.
pub fn slugify(value: &str) -> String {
    let ascii = deunicode(value);
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in ascii.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let cleaned = slug.trim_matches('-').to_string();
    if cleaned.is_empty() {
        // Use SHA-256 (not DefaultHasher) so the fallback name is stable
        // across Rust releases. DefaultHasher's algorithm is unspecified and
        // may change between compiler versions. (#3166)
        let hash = Sha256::digest(value.as_bytes());
        let hex = format!("{:016x}", u64::from_be_bytes(hash[..8].try_into().unwrap()));
        format!("note-{hex}")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_ascii_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("test.md"), "test-md");
        assert_eq!(slugify("hello"), "hello");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Hello! @#$% World"), "hello-world");
        assert_eq!(slugify("path/to/file"), "path-to-file");
        assert_eq!(slugify("a/b\\c:d*e?f"), "a-b-c-d-e-f");
    }

    #[test]
    fn slugify_consecutive_dashes_collapsed() {
        assert_eq!(slugify("a---b"), "a-b");
        assert_eq!(slugify("--test--"), "test");
        assert_eq!(slugify("---hello---"), "hello");
    }

    #[test]
    fn slugify_preserves_underscore_and_digits() {
        assert_eq!(slugify("my-note_v2"), "my-note_v2");
        assert_eq!(slugify("2024-01-15"), "2024-01-15");
    }

    #[test]
    fn slugify_empty_returns_hash_fallback() {
        assert!(slugify("").starts_with("note-"));
        assert!(slugify("---").starts_with("note-"));
    }

    #[test]
    fn slugify_empty_hash_is_stable_across_runs() {
        // DefaultHasher is non-deterministic across Rust versions;
        // SHA-256 ensures the same title always produces the same slug. (#3166)
        let h1 = slugify("");
        let h2 = slugify("");
        assert_eq!(h1, h2, "slugify('') must be deterministic");

        let p1 = slugify("---");
        let p2 = slugify("---");
        assert_eq!(p1, p2, "slugify('---') must be deterministic");

        let u1 = slugify("\u{3001}\u{3002}\u{300C}\u{300D}");
        let u2 = slugify("\u{3001}\u{3002}\u{300C}\u{300D}");
        assert_eq!(u1, u2, "slugify(punctuation-only) must be deterministic");
    }

    #[test]
    fn slugify_empty_hash_format() {
        // SHA-256 hex is 16 characters (8 bytes)
        let slug = slugify("");
        assert!(slug.starts_with("note-"));
        assert_eq!(slug.len(), "note-".len() + 16);
    }

    #[test]
    fn slugify_cjk_transliterated() {
        let result = slugify("测试中文");
        assert_eq!(result, "ce-shi-zhong-wen");
    }

    #[test]
    fn slugify_cjk_fallback_with_hash() {
        let result = slugify("\u{3001}\u{3002}\u{300C}\u{300D}");
        assert!(result.starts_with("note-"));
        assert!(result.len() > "note-".len());
    }
}
