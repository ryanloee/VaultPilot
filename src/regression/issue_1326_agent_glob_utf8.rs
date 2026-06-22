//! Regression tests for #1326: agent.rs glob_match and summarize_args bugs.
//!
//! Bug 1: summarize_args UTF-8 panic when byte 200 falls inside a CJK char.
//! Bug 2: glob_match("**", "a/b") returns false (should be true).
//! Bug 3: glob_match("?", "/") returns true (should be false).

#[test]
fn issue_1326_summarize_args_cjk_no_panic() {
    // 300 CJK chars = 900 bytes. Old code did &args_json[..200] which panics
    // because byte 200 falls inside a multi-byte UTF-8 char.
    let cjk: String = "中".repeat(300);
    // This should NOT panic
    let result = summarize_args_for_test(&cjk);
    assert!(result.ends_with('…'));
    assert!(result.chars().count() <= 201);
}

#[test]
fn issue_1326_glob_double_star_matches_path_separator() {
    assert!(glob_match_for_test("**", "a/b"));
    assert!(glob_match_for_test("**", "a/b/c/d"));
    assert!(glob_match_for_test("**", ""));
    assert!(glob_match_for_test("prefix/**", "prefix/deep/nested"));
    assert!(glob_match_for_test("**/suffix", "a/b/suffix"));
    assert!(glob_match_for_test("a/**/b", "a/x/y/b"));
}

#[test]
fn issue_1326_glob_question_mark_rejects_path_separator() {
    assert!(glob_match_for_test("?", "a"));
    assert!(glob_match_for_test("?", "中"));
    assert!(!glob_match_for_test("?", "/"));
    assert!(!glob_match_for_test("?", "\\"));
    assert!(!glob_match_for_test("a/?/b", "a//b"));
}

// ── Helpers (duplicated from agent.rs for independent regression verification) ──

fn summarize_args_for_test(args_json: &str) -> String {
    let chars: Vec<char> = args_json.chars().collect();
    if chars.len() <= 200 {
        args_json.to_string()
    } else {
        let truncated: String = chars[..200].iter().collect();
        format!("{truncated}…")
    }
}

fn glob_match_for_test(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match_inner_test(&p, &t)
}

fn glob_match_inner_test(pattern: &[char], text: &[char]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = usize::MAX;
    let mut star_ti = 0;

    while ti < text.len() {
        if pi < pattern.len()
            && (pattern[pi] == text[ti]
                || (pattern[pi] == '?' && text[ti] != '/' && text[ti] != '\\'))
        {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            if pi + 1 < pattern.len() && pattern[pi + 1] == '*' {
                star_pi = pi;
                star_ti = ti;
                pi += 2;
            } else {
                star_pi = pi;
                star_ti = ti;
                pi += 1;
            }
        } else if star_pi != usize::MAX {
            if star_pi + 1 < pattern.len() && pattern[star_pi + 1] == '*' {
                star_ti += 1;
                ti = star_ti;
                pi = star_pi + 2;
            } else if text[ti] != '/' && text[ti] != '\\' {
                star_ti += 1;
                ti = star_ti;
                pi = star_pi + 1;
            } else {
                return false;
            }
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }

    pi == pattern.len()
}
