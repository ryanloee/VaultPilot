/**
 * Regression test for #2631: autoTag.ts CJK regex misses Extension B-G,
 * Hiragana, Katakana — inconsistent with rag.ts comprehensive isCJK().
 *
 * Verifies that all CJK ranges covered by isCJK() in rag.ts are also
 * recognized by extractAutoTags in autoTag.ts.
 */

import { extractAutoTags } from '../../utils/autoTag';

describe('issue_2631: autoTag CJK full coverage', () => {
  test('CJK Extension B (U+20000+) characters', () => {
    // 𠀀 (U+20000) CJK Extension B — first character
    const tags = extractAutoTags('𠀀𠀁𠀂', 'test');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('CJK Compatibility Ideographs (U+F900+)', () => {
    // 豈 (U+F900) CJK Compatibility Ideograph
    const tags = extractAutoTags('豈更車', 'test');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('Japanese Hiragana extracts tags', () => {
    const tags = extractAutoTags('ひらがな', 'これは日本語のテストです');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('Japanese Katakana extracts tags', () => {
    const tags = extractAutoTags('カタカナ', 'テストの内容');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('mixed extended CJK with English still works', () => {
    const tags = extractAutoTags(
      'test title',
      'Contains CJK Extension B: 𠀀𠀁𠀂 and Hiragana: あいうえお'
    );
    expect(tags.length).toBeGreaterThan(0);
  });

  test('Japanese text with mixed Hiragana/Katakana/Kanji', () => {
    const tags = extractAutoTags(
      '日本語タイトル',
      'これは人工知能に関するテスト記事です。機械学習について説明します。'
    );
    expect(tags.length).toBeGreaterThan(0);
    // Should have bigrams from the content
    const hasCjkTag = tags.some(t => /^[\u3000-\u9fff]/.test(t));
    expect(hasCjkTag).toBe(true);
  });

  test('Korean Hangul still works', () => {
    const tags = extractAutoTags('한국어', '이것은 한국어 테스트입니다');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('CJK Compatibility Supplement (U+2F800+)', () => {
    // 丽 (U+2F800) CJK Compatibility Ideograph Supplement
    const tags = extractAutoTags('丽丸乁', 'test content');
    expect(tags.length).toBeGreaterThan(0);
  });
});
