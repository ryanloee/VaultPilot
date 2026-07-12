/**
 * CJK character detection utilities.
 *
 * Extracted from rag.ts isCJK() (#2100) for shared use across modules.
 * Covers all CJK extension blocks (A through G), Hiragana, Katakana,
 * Hangul, Compatibility Ideographs, and Symbols & Punctuation.
 */

/** CJK stop characters — matching Rust is_cjk_stop_char in search.rs */
export const CJK_STOP_CHARS = new Set([
  '的', '了', '呢', '吗', '啊', '呀', '吧', '么', '我', '你',
]);

/**
 * Check if a single character is CJK (Chinese, Japanese, Korean).
 *
 * Covers:
 * - CJK Symbols and Punctuation       U+3000–U+303F
 * - Japanese Hiragana                 U+3040–U+309F
 * - Japanese Katakana                 U+30A0–U+30FF
 * - CJK Extension A                   U+3400–U+4DBF
 * - CJK Unified Ideographs            U+4E00–U+9FFF
 * - Korean Hangul                     U+AC00–U+D7AF
 * - CJK Compatibility Ideographs      U+F900–U+FAFF
 * - CJK Extension B                   U+20000–U+2A6DF
 * - CJK Extension C                   U+2A700–U+2B73F
 * - CJK Extension D                   U+2B740–U+2B81F
 * - CJK Extension E                   U+2B820–U+2CEAF
 * - CJK Extensions F & G              U+2CEB0–U+2EBEF
 * - CJK Compatibility Ideographs Supp U+2F800–U+2FA1F
 */
export function isCJK(ch: string): boolean {
  const cp = ch.codePointAt(0);
  if (cp === undefined) return false;
  const code = cp;
  return (code >= 0x3000 && code <= 0x303F)   // CJK Symbols and Punctuation
    || (code >= 0x3040 && code <= 0x309F)      // Japanese Hiragana
    || (code >= 0x30A0 && code <= 0x30FF)      // Japanese Katakana
    || (code >= 0x3400 && code <= 0x4DBF)      // CJK Extension A
    || (code >= 0x4E00 && code <= 0x9FFF)      // CJK Unified Ideographs
    || (code >= 0xAC00 && code <= 0xD7AF)      // Korean Hangul
    || (code >= 0xF900 && code <= 0xFAFF)      // CJK Compatibility Ideographs
    || (code >= 0x20000 && code <= 0x2A6DF)    // CJK Extension B
    || (code >= 0x2A700 && code <= 0x2B73F)    // CJK Extension C
    || (code >= 0x2B740 && code <= 0x2B81F)    // CJK Extension D
    || (code >= 0x2B820 && code <= 0x2CEAF)    // CJK Extension E
    || (code >= 0x2CEB0 && code <= 0x2EBEF)    // CJK Extensions F & G
    || (code >= 0x2F800 && code <= 0x2FA1F);   // CJK Compatibility Ideographs Supplement
}
