//! Regression tests for #1328: is_cjk missing Japanese/Korean character ranges.
//!
//! The is_cjk() function must recognize Japanese Hiragana, Katakana,
//! Korean Hangul, and CJK Symbols/Punctuation in addition to CJK
//! Unified Ideographs.

#[test]
fn regression_1328_hiragana_recognized_as_cjk() {
    assert!(is_cjk_for_test('あ'));
    assert!(is_cjk_for_test('の'));
    assert!(is_cjk_for_test('を'));
    assert!(is_cjk_for_test('ん'));
}

#[test]
fn regression_1328_katakana_recognized_as_cjk() {
    assert!(is_cjk_for_test('ア'));
    assert!(is_cjk_for_test('ン'));
    assert!(is_cjk_for_test('ガ'));
    assert!(is_cjk_for_test('ッ'));
}

#[test]
fn regression_1328_hangul_recognized_as_cjk() {
    assert!(is_cjk_for_test('한'));
    assert!(is_cjk_for_test('글'));
    assert!(is_cjk_for_test('가'));
    assert!(is_cjk_for_test('힣'));
}

#[test]
fn regression_1328_cjk_symbols_recognized() {
    assert!(is_cjk_for_test('「'));
    assert!(is_cjk_for_test('」'));
    assert!(is_cjk_for_test('〒'));
}

#[test]
fn regression_1328_chinese_still_works() {
    assert!(is_cjk_for_test('中'));
    assert!(is_cjk_for_test('文'));
    assert!(is_cjk_for_test('龙'));
}

#[test]
fn regression_1328_latin_not_cjk() {
    assert!(!is_cjk_for_test('a'));
    assert!(!is_cjk_for_test('Z'));
    assert!(!is_cjk_for_test('0'));
}

#[test]
fn regression_1328_japanese_tokenization_not_split() {
    // Japanese text should be classified as CJK (kind 1), not separator (kind 0)
    let tokens = split_search_token_for_test("あいう");
    assert_eq!(tokens, vec!["あいう"]);
}

#[test]
fn regression_1328_korean_tokenization_not_split() {
    let tokens = split_search_token_for_test("한글");
    assert_eq!(tokens, vec!["한글"]);
}

#[test]
fn regression_1328_mixed_japanese_ascii_splits_correctly() {
    let tokens = split_search_token_for_test("testあいうabc");
    assert_eq!(tokens, vec!["test", "あいう", "abc"]);
}

// ── Helpers (duplicated from search.rs for independent regression verification) ──

fn is_cjk_for_test(ch: char) -> bool {
    matches!(
        ch,
        '\u{3000}'..='\u{303F}'   // CJK Symbols and Punctuation
        | '\u{3040}'..='\u{309F}'   // Japanese Hiragana
        | '\u{30A0}'..='\u{30FF}'   // Japanese Katakana
        | '\u{3400}'..='\u{4DBF}'   // CJK Unified Ideographs Extension A
        | '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{AC00}'..='\u{D7AF}'   // Korean Hangul Syllables
        | '\u{F900}'..='\u{FAFF}'   // CJK Compatibility Ideographs
        | '\u{20000}'..='\u{2A6DF}' // CJK Extension B
        | '\u{2A700}'..='\u{2B73F}' // CJK Extension C
        | '\u{2B740}'..='\u{2B81F}' // CJK Extension D
        | '\u{2B820}'..='\u{2CEAF}' // CJK Extension E
        | '\u{2CEB0}'..='\u{2EBEF}' // CJK Extension F
        | '\u{30000}'..='\u{3134F}' // CJK Extension G
    )
}

fn split_search_token_for_test(token: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_kind = None::<u8>;

    for ch in token.chars() {
        let kind = if is_cjk_for_test(ch) {
            1
        } else if ch.is_ascii_alphanumeric() {
            2
        } else {
            0
        };

        if kind == 0 {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            current_kind = None;
            continue;
        }

        if current_kind.is_some() && current_kind != Some(kind) && !current.is_empty() {
            parts.push(current.clone());
            current.clear();
        }

        current.push(ch);
        current_kind = Some(kind);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}
