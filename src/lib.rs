pub mod agent;
pub mod ai;
pub mod crypto;
pub mod models;
pub mod orchestration;
pub mod plugin;
pub mod prompting;
pub mod search_rules;
pub mod storage;

// Re-export public API from orchestration module for backward compatibility
pub use orchestration::{
    ask_with_ai_with_context, chat_with_ai_with_context, compress_chat_history_with_context,
    normalize_tool_path,
};
#[cfg(test)]
mod regression;

/// Redact sensitive substrings — API keys, bearer tokens, and secret query
/// parameters — from an error or log message before it is shown to the user
/// or written to a crash log.
///
/// The function is intentionally simple and conservative: it only redacts
/// patterns it is confident about to avoid mangling unrelated messages.
pub fn sanitize_error(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let bytes = message.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // 1. Redact OpenAI / Anthropic-style secret keys: "sk-" followed by
        //    20+ base64url characters (letters, digits, hyphen, underscore).
        if i + 3 <= len
            && &bytes[i..i + 3] == b"sk-"
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
        {
            let key_end = scan_secret_token(bytes, i + 3);
            if key_end - (i + 3) >= 20 {
                out.push_str("sk-[REDACTED]");
                i = key_end;
                continue;
            }
        }

        // 2. Redact Bearer tokens: "Bearer " followed by 20+ non-whitespace
        //    characters.
        if i + 7 <= len && bytes[i..i + 7].eq_ignore_ascii_case(b"Bearer ") {
            let tok_start = i + 7;
            let tok_end = scan_until_whitespace(bytes, tok_start);
            if tok_end - tok_start >= 20 {
                out.push_str("Bearer [REDACTED]");
                i = tok_end;
                continue;
            }
        }

        // 2b. Redact HTTP Basic auth credentials: "Basic " followed by 20+
        //     base64 characters.
        if i + 6 <= len && bytes[i..i + 6].eq_ignore_ascii_case(b"Basic ") {
            let tok_start = i + 6;
            let tok_end = scan_until_whitespace(bytes, tok_start);
            if tok_end - tok_start >= 20 {
                out.push_str("Basic [REDACTED]");
                i = tok_end;
                continue;
            }
        }

        // 2c. Redact x-api-key header values (used by Anthropic and custom
        //     providers).  Match "x-api-key:" followed by optional space and
        //     8+ non-whitespace characters.
        if i + 10 <= len && bytes[i..i + 10].eq_ignore_ascii_case(b"x-api-key:") {
            let mut val_start = i + 10;
            if val_start < len && bytes[val_start] == b' ' {
                val_start += 1;
            }
            let val_end = scan_until_whitespace(bytes, val_start);
            if val_end - val_start >= 8 {
                out.push_str("x-api-key: [REDACTED]");
                i = val_end;
                continue;
            }
        }

        // 2d. Redact API key query parameter: "key=" followed by 8+
        //     non-whitespace, non-& characters.
        if i + 4 <= len
            && bytes[i..i + 4].eq_ignore_ascii_case(b"key=")
            && (i == 0 || bytes[i - 1] == b'?' || bytes[i - 1] == b'&' || bytes[i - 1] == b' ')
        {
            let val_start = i + 4;
            let val_end = scan_until_ampersand_or_end(bytes, val_start);
            if val_end - val_start >= 8 {
                out.push_str("key=[REDACTED]");
                i = val_end;
                continue;
            }
        }

        // 2e. Redact api_key query parameter: "api_key=" followed by 8+
        //     non-whitespace, non-& characters.
        if i + 8 <= len
            && bytes[i..i + 8].eq_ignore_ascii_case(b"api_key=")
            && (i == 0 || bytes[i - 1] == b'?' || bytes[i - 1] == b'&' || bytes[i - 1] == b' ')
        {
            let val_start = i + 8;
            let val_end = scan_until_ampersand_or_end(bytes, val_start);
            if val_end - val_start >= 8 {
                out.push_str("api_key=[REDACTED]");
                i = val_end;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

/// Scan forward from `pos` over base64url characters (a-z A-Z 0-9 - _ .).
/// Returns the index just past the last such character.
fn scan_secret_token(bytes: &[u8], pos: usize) -> usize {
    let mut j = pos;
    while j < bytes.len() {
        match bytes[j] {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => j += 1,
            _ => break,
        }
    }
    j
}

/// Scan forward from `pos` until whitespace or end-of-string.
fn scan_until_whitespace(bytes: &[u8], pos: usize) -> usize {
    let mut j = pos;
    while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    j
}

/// Scan forward from `pos` until '&' or end-of-string.
fn scan_until_ampersand_or_end(bytes: &[u8], pos: usize) -> usize {
    let mut j = pos;
    while j < bytes.len() && bytes[j] != b'&' {
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::sanitize_error;

    #[test]
    fn sanitize_error_redacts_openai_key() {
        let input = "error: invalid key sk-abcdefghijklmnopqrstuvwxyz123456";
        let result = sanitize_error(input);
        assert!(result.contains("sk-[REDACTED]"));
        assert!(!result.contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn sanitize_error_redacts_bearer_token() {
        let input = "error: unauthorized Bearer abcdefghijklmnopqrstuvwxyz123456";
        let result = sanitize_error(input);
        assert!(result.contains("Bearer [REDACTED]"));
    }

    #[test]
    fn sanitize_error_redacts_basic_auth() {
        let input = "error: unauthorized Basic abcdefghijklmnopqrstuvwxyz123456";
        let result = sanitize_error(input);
        assert!(result.contains("Basic [REDACTED]"));
    }

    #[test]
    fn sanitize_error_redacts_x_api_key() {
        let input = "error: x-api-key: abcdefghijklmnop";
        let result = sanitize_error(input);
        assert!(result.contains("x-api-key: [REDACTED]"));
    }

    #[test]
    fn sanitize_error_redacts_key_query_param() {
        let input = "error: https://api.example.com?key=abcdefghijklmnop";
        let result = sanitize_error(input);
        assert!(result.contains("key=[REDACTED]"));
    }

    #[test]
    fn sanitize_error_redacts_api_key_query_param() {
        let input = "error: https://api.example.com?api_key=abcdefghijklmnop";
        let result = sanitize_error(input);
        assert!(result.contains("api_key=[REDACTED]"));
    }

    #[test]
    fn sanitize_error_preserves_short_keys() {
        let input = "error: sk-short";
        let result = sanitize_error(input);
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_error_preserves_normal_text() {
        let input = "error: file not found";
        let result = sanitize_error(input);
        assert_eq!(result, input);
    }
}
