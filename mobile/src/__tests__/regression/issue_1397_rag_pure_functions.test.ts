/**
 * Unit tests for rag.ts pure functions (#1397).
 *
 * Tests: looksLikeSmallTalk, extractCJKNgrams, isCJK.
 * These are pure functions with no IO dependencies.
 */

import { looksLikeSmallTalk, extractCJKNgrams, isCJK } from '../../services/rag';

// ── isCJK ────────────────────────────────────────────────────

describe('isCJK', () => {
  it('returns true for CJK Unified Ideographs (Chinese)', () => {
    expect(isCJK('你')).toBe(true);
    expect(isCJK('好')).toBe(true);
  });

  it('returns true for Japanese Hiragana', () => {
    expect(isCJK('あ')).toBe(true);
    expect(isCJK('の')).toBe(true);
  });

  it('returns true for Japanese Katakana', () => {
    expect(isCJK('ア')).toBe(true);
    expect(isCJK('カ')).toBe(true);
  });

  it('returns true for Korean Hangul', () => {
    expect(isCJK('한')).toBe(true);
    expect(isCJK('글')).toBe(true);
  });

  it('returns false for ASCII letters', () => {
    expect(isCJK('a')).toBe(false);
    expect(isCJK('Z')).toBe(false);
  });

  it('returns false for digits', () => {
    expect(isCJK('0')).toBe(false);
    expect(isCJK('9')).toBe(false);
  });

  it('returns false for space and punctuation', () => {
    expect(isCJK(' ')).toBe(false);
    expect(isCJK('.')).toBe(false);
    expect(isCJK('!')).toBe(false);
  });
});

// ── looksLikeSmallTalk ───────────────────────────────────────

describe('looksLikeSmallTalk', () => {
  it('detects Chinese greetings', () => {
    expect(looksLikeSmallTalk('你好')).toBe(true);
    expect(looksLikeSmallTalk('嗨')).toBe(true);
    expect(looksLikeSmallTalk('哈喽')).toBe(true);
    expect(looksLikeSmallTalk('早上好')).toBe(true);
    expect(looksLikeSmallTalk('下午好')).toBe(true);
    expect(looksLikeSmallTalk('晚上好')).toBe(true);
  });

  it('detects English greetings', () => {
    expect(looksLikeSmallTalk('hi')).toBe(true);
    expect(looksLikeSmallTalk('hello')).toBe(true);
    expect(looksLikeSmallTalk('hey')).toBe(true);
    expect(looksLikeSmallTalk('thanks')).toBe(true);
    expect(looksLikeSmallTalk('thank you')).toBe(true);
  });

  it('detects greetings with punctuation', () => {
    expect(looksLikeSmallTalk('hi!')).toBe(true);
    expect(looksLikeSmallTalk('hello。')).toBe(true);
    expect(looksLikeSmallTalk('你好!')).toBe(true);
  });

  it('is case-insensitive', () => {
    expect(looksLikeSmallTalk('HI')).toBe(true);
    expect(looksLikeSmallTalk('Hello')).toBe(true);
    expect(looksLikeSmallTalk('BYE')).toBe(true);
  });

  it('detects farewell messages', () => {
    expect(looksLikeSmallTalk('再见')).toBe(true);
    expect(looksLikeSmallTalk('bye')).toBe(true);
    expect(looksLikeSmallTalk('拜拜')).toBe(true);
    expect(looksLikeSmallTalk('晚安')).toBe(true);
  });

  it('returns false for non-trivial messages', () => {
    expect(looksLikeSmallTalk('请帮我写一篇关于AI的文章')).toBe(false);
    expect(looksLikeSmallTalk('how to use Rust async')).toBe(false);
    expect(looksLikeSmallTalk('你好，请问一下这个问题怎么解决')).toBe(false);
  });

  it('returns false for empty string', () => {
    expect(looksLikeSmallTalk('')).toBe(false);
  });

  it('returns false for whitespace-only', () => {
    expect(looksLikeSmallTalk('   ')).toBe(false);
  });

  it('detects agreement messages', () => {
    expect(looksLikeSmallTalk('好的')).toBe(true);
    expect(looksLikeSmallTalk('ok')).toBe(true);
    expect(looksLikeSmallTalk('okay')).toBe(true);
    expect(looksLikeSmallTalk('嗯')).toBe(true);
  });
});

// ── extractCJKNgrams ─────────────────────────────────────────

describe('extractCJKNgrams', () => {
  it('extracts 2-char and 3-char ngrams from CJK text', () => {
    const ngrams = extractCJKNgrams('人工智能');
    // Chars: 人, 工, 智, 能 (none are stop chars)
    // 2-grams: 人工, 工智, 智能
    // 3-grams: 人工智, 工智能
    expect(ngrams).toContain('人工');
    expect(ngrams).toContain('工智');
    expect(ngrams).toContain('智能');
    expect(ngrams).toContain('人工智');
    expect(ngrams).toContain('工智能');
    // 4-char ngram not generated (only 2 and 3)
    expect(ngrams).not.toContain('人工智能');
  });

  it('filters CJK stop characters', () => {
    // 的 and 了 are stop chars
    const ngrams = extractCJKNgrams('我的电脑了');
    // Chars after filtering: 电, 脑 (我的 and 了 are filtered)
    expect(ngrams).not.toContain('我的');
    expect(ngrams).not.toContain('的电');
    expect(ngrams).toContain('电脑');
  });

  it('returns empty array for pure Latin text', () => {
    expect(extractCJKNgrams('hello world')).toEqual([]);
  });

  it('returns empty array for digits only', () => {
    expect(extractCJKNgrams('12345')).toEqual([]);
  });

  it('extracts ngrams from mixed CJK/Latin text', () => {
    const ngrams = extractCJKNgrams('使用Rust编程');
    // CJK chars: 使, 用, 编, 程 (Rust is Latin)
    expect(ngrams).toContain('使用');
    expect(ngrams).toContain('编程');
    // Rust chars are not CJK, so no ngrams span across
  });

  it('returns empty for short CJK text (< 2 chars)', () => {
    expect(extractCJKNgrams('你')).toEqual([]);
  });

  it('returns empty for empty string', () => {
    expect(extractCJKNgrams('')).toEqual([]);
  });

  it('generates ngrams from Japanese text', () => {
    const ngrams = extractCJKNgrams('東京大学');
    // Chars: 東, 京, 大, 学
    expect(ngrams).toContain('東京');
    expect(ngrams).toContain('大学');
    expect(ngrams).toContain('東京大');
    expect(ngrams).toContain('京大学');
  });

  it('generates ngrams from Korean text', () => {
    const ngrams = extractCJKNgrams('한국어');
    // Chars: 한, 국, 어 (all Hangul, none are CJK_STOP_CHARS)
    expect(ngrams).toContain('한국');
    expect(ngrams).toContain('국어');
    expect(ngrams).toContain('한국어');
  });

  it('deduplicates ngrams', () => {
    // 人人人 → chars: 人, 人, 人
    // 2-grams: 人人, 人人 (duplicate)
    // 3-grams: 人人人
    const ngrams = extractCJKNgrams('人人人');
    const count = ngrams.filter(n => n === '人人').length;
    expect(count).toBe(1); // deduplicated
  });
});
