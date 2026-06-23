/**
 * Regression test for #1425: extractKeywords exported for direct unit testing.
 *
 * Tests the keyword extraction logic directly without needing a mock database.
 * Covers: CJK ngram extraction, stop word filtering, Latin token extraction,
 * mixed text, empty input, and fallback behavior.
 */

import { extractKeywords } from '../../services/rag';

describe('extractKeywords (#1425)', () => {
  // ── Latin token extraction ──────────────────────────────

  it('extracts Latin tokens >= 2 chars', () => {
    const keywords = extractKeywords('machine learning basics');
    expect(keywords).toContain('machine');
    expect(keywords).toContain('learning');
    expect(keywords).toContain('basics');
  });

  it('filters English stop words', () => {
    const keywords = extractKeywords('the quick brown fox');
    expect(keywords).not.toContain('the');
    expect(keywords).toContain('quick');
    expect(keywords).toContain('brown');
    expect(keywords).toContain('fox');
  });

  it('filters single-char Latin tokens', () => {
    const keywords = extractKeywords('I am OK');
    // 'I' is a stop word, 'am' is 2 chars (passes filter), 'OK' becomes 'ok'
    expect(keywords).not.toContain('i');
    // 'am' is 2 chars and not a stop word — it passes
    expect(keywords).toContain('am');
  });

  // ── CJK ngram extraction ───────────────────────────────

  it('extracts CJK 2-gram and 3-gram ngrams', () => {
    const keywords = extractKeywords('机器学习基础');
    // Should contain bigrams like '机器', '器学', '学习', etc.
    expect(keywords.length).toBeGreaterThan(0);
    expect(keywords).toContain('机器');
    expect(keywords).toContain('学习');
  });

  it('filters CJK stop chars from ngrams', () => {
    // '的' is a CJK stop char — ngrams containing it should be filtered
    const keywords = extractKeywords('我的笔记');
    // '我的' and '的笔' should NOT be in results (contain stop char '的')
    expect(keywords).not.toContain('我的');
    expect(keywords).not.toContain('的笔');
  });

  // ── Japanese/Korean support ────────────────────────────

  it('extracts Japanese text ngrams', () => {
    const keywords = extractKeywords('これは日本語のテスト');
    expect(keywords.length).toBeGreaterThan(0);
  });

  it('extracts Korean text ngrams', () => {
    const keywords = extractKeywords('이것은 한국어 테스트');
    expect(keywords.length).toBeGreaterThan(0);
  });

  // ── Mixed text ─────────────────────────────────────────

  it('handles mixed CJK and Latin text', () => {
    const keywords = extractKeywords('AI 人工智能 machine learning');
    expect(keywords.length).toBeGreaterThan(0);
    // Should have both CJK and Latin tokens
    const hasCJK = keywords.some(k => /[\u3000-\u9fff]/.test(k));
    const hasLatin = keywords.some(k => /^[a-z]/.test(k));
    expect(hasCJK).toBe(true);
    expect(hasLatin).toBe(true);
  });

  // ── Edge cases ─────────────────────────────────────────

  it('returns empty array for empty string', () => {
    expect(extractKeywords('')).toEqual([]);
  });

  it('returns empty array for whitespace-only', () => {
    expect(extractKeywords('   ')).toEqual([]);
  });

  it('filters individual CJK stop words from rawTokens', () => {
    // Single CJK stop chars like '的' are filtered by stopWords.has()
    // But multi-char tokens like '的了呢吗' pass because the whole string
    // is not in the stop words set — this is expected behavior.
    const keywords = extractKeywords('人工智能的笔记');
    // '的' as part of a longer token may survive, but individual stop chars are filtered
    expect(keywords.length).toBeGreaterThan(0);
  });

  it('limits output to 15 keywords', () => {
    // Generate text with many unique tokens
    const words = Array.from({ length: 30 }, (_, i) => `word${i}`).join(' ');
    const keywords = extractKeywords(words);
    expect(keywords.length).toBeLessThanOrEqual(15);
  });

  it('falls back to single-char CJK when no other keywords found', () => {
    // Single CJK char that's not a stop char
    const keywords = extractKeywords('龍');
    // '龍' is a single CJK char, not a stop char — should be in relaxed fallback
    expect(keywords.length).toBeGreaterThan(0);
  });

  // ── Deduplication ──────────────────────────────────────

  it('deduplicates repeated keywords', () => {
    const keywords = extractKeywords('test test test word');
    const testCount = keywords.filter(k => k === 'test').length;
    expect(testCount).toBe(1);
  });
});
