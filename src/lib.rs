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
    finalize_chat_with_ai_answer, normalize_tool_path, prepare_chat_for_ai, PreparedChatContext,
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

        // 2f. Redact api-key query parameter: "api-key=" (hyphenated variant,
        //     used by Cloudflare and other services).
        if i + 8 <= len
            && bytes[i..i + 8].eq_ignore_ascii_case(b"api-key=")
            && (i == 0 || bytes[i - 1] == b'?' || bytes[i - 1] == b'&' || bytes[i - 1] == b' ')
        {
            let val_start = i + 8;
            let val_end = scan_until_ampersand_or_end(bytes, val_start);
            if val_end - val_start >= 8 {
                out.push_str("api-key=[REDACTED]");
                i = val_end;
                continue;
            }
        }

        // 2g. Redact access_token query parameter: "access_token=" (OAuth).
        if i + 13 <= len
            && bytes[i..i + 13].eq_ignore_ascii_case(b"access_token=")
            && (i == 0 || bytes[i - 1] == b'?' || bytes[i - 1] == b'&' || bytes[i - 1] == b' ')
        {
            let val_start = i + 13;
            let val_end = scan_until_ampersand_or_end(bytes, val_start);
            if val_end - val_start >= 8 {
                out.push_str("access_token=[REDACTED]");
                i = val_end;
                continue;
            }
        }

        // 2h. Redact secret query parameter: "secret=" (generic).
        if i + 7 <= len
            && bytes[i..i + 7].eq_ignore_ascii_case(b"secret=")
            && (i == 0 || bytes[i - 1] == b'?' || bytes[i - 1] == b'&' || bytes[i - 1] == b' ')
        {
            let val_start = i + 7;
            let val_end = scan_until_ampersand_or_end(bytes, val_start);
            if val_end - val_start >= 8 {
                out.push_str("secret=[REDACTED]");
                i = val_end;
                continue;
            }
        }

        // 2i. Redact token query parameter: "token=" (generic).
        if i + 6 <= len
            && bytes[i..i + 6].eq_ignore_ascii_case(b"token=")
            && (i == 0 || bytes[i - 1] == b'?' || bytes[i - 1] == b'&' || bytes[i - 1] == b' ')
        {
            let val_start = i + 6;
            let val_end = scan_until_ampersand_or_end(bytes, val_start);
            if val_end - val_start >= 8 {
                out.push_str("token=[REDACTED]");
                i = val_end;
                continue;
            }
        }

        // Determine the number of bytes for this UTF-8 character so that
        // multi-byte characters (e.g. CJK, emoji) are preserved intact.
        let char_len = match bytes[i] {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1, // continuation or invalid byte – emit as-is
        };
        let end = (i + char_len).min(len);
        out.push_str(&message[i..end]);
        i += char_len;
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
    fn sanitize_error_redacts_api_key_hyphenated() {
        let input = "error: https://api.cloudflare.com?api-key=abcdefghijklmnop";
        let result = sanitize_error(input);
        assert!(result.contains("api-key=[REDACTED]"));
        assert!(!result.contains("abcdefghijklmnop"));
    }

    #[test]
    fn sanitize_error_redacts_access_token() {
        let input = "error: https://oauth.example.com?access_token=abcdefghijklmnop";
        let result = sanitize_error(input);
        assert!(result.contains("access_token=[REDACTED]"));
        assert!(!result.contains("abcdefghijklmnop"));
    }

    #[test]
    fn sanitize_error_redacts_secret_param() {
        let input = "error: https://api.example.com?secret=abcdefghijklmnop";
        let result = sanitize_error(input);
        assert!(result.contains("secret=[REDACTED]"));
        assert!(!result.contains("abcdefghijklmnop"));
    }

    #[test]
    fn sanitize_error_redacts_token_param() {
        let input = "error: https://api.example.com?token=abcdefghijklmnop";
        let result = sanitize_error(input);
        assert!(result.contains("token=[REDACTED]"));
        assert!(!result.contains("abcdefghijklmnop"));
    }

    #[test]
    fn sanitize_error_redacts_multiple_params() {
        let input = "https://api.example.com?api_key=12345678&secret=abcdefgh&token=ijklmnop";
        let result = sanitize_error(input);
        assert!(result.contains("api_key=[REDACTED]"));
        assert!(result.contains("secret=[REDACTED]"));
        assert!(result.contains("token=[REDACTED]"));
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

    #[test]
    fn sanitize_error_preserves_multibyte_utf8() {
        let input = "错误：文件未找到";
        let result = sanitize_error(input);
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_error_preserves_mixed_ascii_and_utf8() {
        let input = "error: 文件 not found — 你好世界 🌍";
        let result = sanitize_error(input);
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_error_redacts_key_with_utf8_context() {
        let input = "错误 key sk-abcdefghijklmnopqrstuvwxyz end";
        let result = sanitize_error(input);
        assert!(result.contains("sk-[REDACTED]"));
        assert!(result.contains("错误"));
        assert!(result.contains("end"));
    }
}
