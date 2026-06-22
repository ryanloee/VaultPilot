/**
 * Regression test for #1330: mobile RAG isCJK regex missing Japanese/Korean ranges.
 *
 * The isCJK regex in extractKeywords must recognize Japanese Hiragana, Katakana,
 * and Korean Hangul in addition to CJK Unified Ideographs.
 */

// Inline the CJK regex patterns to test independently (no import of rag.ts needed
// since extractKeywords is not exported).

const OLD_CJK_RE = /[\u4e00-\u9fff]/;
const NEW_CJK_RE = /[\u3000-\u9fff\uac00-\ud7af]/;

describe('issue_1330: isCJK regex coverage', () => {
  // Chinese (should match with both old and new)
  test.each(['中', '文', '龙'])('Chinese char %s matches old regex', (ch) => {
    expect(OLD_CJK_RE.test(ch)).toBe(true);
    expect(NEW_CJK_RE.test(ch)).toBe(true);
  });

  // Japanese Hiragana (should NOT match old, SHOULD match new)
  test.each(['あ', 'の', 'を', 'ん'])('Hiragana %s matches new regex', (ch) => {
    expect(OLD_CJK_RE.test(ch)).toBe(false);
    expect(NEW_CJK_RE.test(ch)).toBe(true);
  });

  // Japanese Katakana (should NOT match old, SHOULD match new)
  test.each(['ア', 'ン', 'ガ', 'ッ'])('Katakana %s matches new regex', (ch) => {
    expect(OLD_CJK_RE.test(ch)).toBe(false);
    expect(NEW_CJK_RE.test(ch)).toBe(true);
  });

  // Korean Hangul (should NOT match old, SHOULD match new)
  test.each(['한', '글', '가', '힣'])('Hangul %s matches new regex', (ch) => {
    expect(OLD_CJK_RE.test(ch)).toBe(false);
    expect(NEW_CJK_RE.test(ch)).toBe(true);
  });

  // Latin (should NOT match either)
  test.each(['a', 'Z', '0'])('Latin %s matches neither regex', (ch) => {
    expect(OLD_CJK_RE.test(ch)).toBe(false);
    expect(NEW_CJK_RE.test(ch)).toBe(false);
  });

  // CJK Symbols and Punctuation
  test.each(['「', '」', '〒'])('CJK Symbol %s matches new regex', (ch) => {
    expect(NEW_CJK_RE.test(ch)).toBe(true);
  });
});

describe('issue_1330: tokenization regex', () => {
  const OLD_SPLIT_RE = /(?<=[\u4e00-\u9fff])(?=[^\u4e00-\u9fff])|(?<=[^\u4e00-\u9fff])(?=[\u4e00-\u9fff])/;
  const NEW_SPLIT_RE = /(?<=[\u3000-\u9fff\uac00-\ud7af])(?=[^\u3000-\u9fff\uac00-\ud7af])|(?<=[^\u3000-\u9fff\uac00-\ud7af])(?=[\u3000-\u9fff\uac00-\ud7af])/;

  test('Japanese text splits correctly with new regex', () => {
    const text = 'テスト文章';
    const parts = text.split(NEW_SPLIT_RE);
    // All CJK, should not split
    expect(parts).toEqual(['テスト文章']);
  });

  test('Mixed Japanese-ASCII splits correctly with new regex', () => {
    const text = 'helloあいうworld';
    const parts = text.split(NEW_SPLIT_RE);
    expect(parts).toEqual(['hello', 'あいう', 'world']);
  });

  test('Korean text splits correctly with new regex', () => {
    const text = 'hello한글world';
    const parts = text.split(NEW_SPLIT_RE);
    expect(parts).toEqual(['hello', '한글', 'world']);
  });

  test('Old regex does NOT split Japanese (bug)', () => {
    const text = 'helloあいうworld';
    const parts = text.split(OLD_SPLIT_RE);
    // Old regex treats Hiragana as non-CJK, so no split happens
    expect(parts).toEqual(['helloあいうworld']);
  });
});
