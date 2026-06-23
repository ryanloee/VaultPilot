/**
 * Regression test for #1372: autoTag.ts CJK regex missing Japanese/Korean ranges.
 *
 * The CJK regex in extractAutoTags must recognize Japanese Hiragana, Katakana,
 * and Korean Hangul in addition to CJK Unified Ideographs.
 */

import { extractAutoTags } from '../../utils/autoTag';

describe('issue_1372: autoTag CJK regex coverage', () => {
  test('Chinese text extracts tags', () => {
    const tags = extractAutoTags('笔记标题', '这是一篇关于人工智能的笔记内容');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('Japanese Hiragana text extracts tags', () => {
    const tags = extractAutoTags('あいうえお', 'これは日本語のテストです');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('Japanese Katakana text extracts tags', () => {
    const tags = extractAutoTags('カタカナ', 'テストの内容です');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('Korean Hangul text extracts tags', () => {
    const tags = extractAutoTags('한국어', '이것은 한국어 테스트입니다');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('mixed CJK and English extracts tags', () => {
    const tags = extractAutoTags('AI 人工智能', 'Machine learning 机器学习 basics');
    expect(tags.length).toBeGreaterThan(0);
    // Should contain both English and CJK tokens
    const hasEnglish = tags.some(t => /^[a-z]/.test(t));
    expect(hasEnglish).toBe(true);
  });

  test('English-only text still works', () => {
    const tags = extractAutoTags('Machine Learning', 'Deep learning neural networks');
    expect(tags.length).toBeGreaterThan(0);
  });

  test('stop words are excluded', () => {
    const tags = extractAutoTags('the a an', 'is are was were');
    expect(tags).toEqual([]);
  });

  test('maxTags limits output', () => {
    const tags = extractAutoTags(
      'word1 word2 word3 word4 word5 word6',
      'word1 word2 word3 word4 word5 word6 word7 word8',
      3,
    );
    expect(tags.length).toBeLessThanOrEqual(3);
  });

  test('title is weighted higher than content', () => {
    // Title keyword appears 2x (title weighted 2x) vs content keyword appears 1x
    const tags = extractAutoTags('important', 'content');
    // "important" should rank higher due to 2x title weight
    expect(tags[0]).toBe('important');
  });

  test('markdown is stripped before extraction', () => {
    const tags = extractAutoTags(
      '# Heading',
      '**bold** _italic_ `code` [link](http://example.com)',
    );
    // formatting chars and URLs should be stripped
    expect(tags).not.toContain('http');
    expect(tags).not.toContain('#');
  });
});
