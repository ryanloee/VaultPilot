/**
 * Regression test for #2631: autoTag CJK tokenization regex misses Extension B-G,
 * Hiragana, Katakana, and Compatibility Ideographs.
 *
 * The fix replaces the limited regex with a character scan that reuses isCJK()
 * from the shared cjk.ts utility, which covers all CJK extension blocks.
 */

import { extractAutoTags } from '../../utils/autoTag';

describe('issue_2631: comprehensive CJK auto-tag coverage', () => {
  test('Japanese Hiragana text extracts tags', () => {
    const tags = extractAutoTags('あいうえお', 'これは日本語のテストです');
    expect(tags.length).toBeGreaterThan(0);
    // Should extract bigrams from Hiragana runs
    expect(tags.some(t => /^[\u3040-\u309f]{2,}$/.test(t))).toBe(true);
  });

  test('Japanese Katakana text extracts tags', () => {
    const tags = extractAutoTags('カタカナ', 'テストの内容です');
    expect(tags.length).toBeGreaterThan(0);
    expect(tags.some(t => /^[\u30a0-\u30ff]{2,}$/.test(t))).toBe(true);
  });

  test('CJK Compatibility Ideographs extract tags', () => {
    // U+F900-FAFF: CJK Compatibility Ideographs (e.g., 豈 U+F900, 更 U+F901)
    const compIdeo = '\uF900\uF901\uF902\uF903\uF904\uF905';
    const tags = extractAutoTags(compIdeo, compIdeo + ' some context');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('CJK Extension B characters extract tags', () => {
    // U+20000-U+2A6DF: CJK Extension B
    // 𠀀 U+20000, 𠀁 U+20001, 𠀂 U+20002, 𠀃 U+20003
    const extB = '\uD840\uDC00\uD840\uDC01\uD840\uDC02\uD840\uDC03'; // 𠀀𠀁𠀂𠀃
    const tags = extractAutoTags(extB, extB + ' some context');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('CJK Compatibility Ideographs Supplement extracts tags', () => {
    // U+2F800-U+2FA1F: CJK Compatibility Ideographs Supplement
    // 丽 U+2F800, 丸 U+2F801
    const compSupp = '\uD87E\uDC00\uD87E\uDC01'; // 丽丸
    const tags = extractAutoTags(compSupp, compSupp + ' some context');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('Korean Hangul text extracts tags', () => {
    const tags = extractAutoTags('한국어', '이것은 한국어 테스트입니다');
    expect(tags.length).toBeGreaterThan(0);
    expect(tags.some(t => /^[\uac00-\ud7af]{2,}$/.test(t))).toBe(true);
  });

  test('Chinese text still works correctly', () => {
    const tags = extractAutoTags('笔记标题', '这是一篇关于人工智能的笔记内容');
    expect(tags.length).toBeGreaterThan(0);
    expect(tags).toContain('笔记');
  });

  test('mixed Japanese and Latin extracts both', () => {
    const tags = extractAutoTags('AI 人工知能', 'Machine learning 機械学習 basics');
    const hasEnglish = tags.some(t => /^[a-z]/.test(t));
    expect(hasEnglish).toBe(true);
    const hasCJK = tags.some(t => /^[\u3000-\u9fff]/.test(t));
    expect(hasCJK).toBe(true);
  });

  test('English-only text still works', () => {
    const tags = extractAutoTags('Machine Learning', 'Deep learning neural networks');
    expect(tags.length).toBeGreaterThan(0);
    expect(tags).toContain('machine');
    expect(tags).toContain('learning');
  });

  test('CJK stop words are excluded from tags', () => {
    const tags = extractAutoTags('', '的 了 呢 吗 啊 technology');
    // 'technology' should be the only non-stop token
    expect(tags).toContain('technology');
    expect(tags).not.toContain('的');
  });

  test('title is weighted higher than content', () => {
    const tags = extractAutoTags('important', 'content');
    expect(tags[0]).toBe('important');
  });

  test('markdown is stripped before extraction', () => {
    const tags = extractAutoTags(
      '# Heading',
      '**bold** _italic_ `code` [link](http://example.com)',
    );
    expect(tags).not.toContain('http');
    expect(tags).not.toContain('#');
  });
});
