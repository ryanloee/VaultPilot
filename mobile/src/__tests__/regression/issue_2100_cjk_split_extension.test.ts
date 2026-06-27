/**
 * Regression test for #2100: extractKeywords CJK/Latin split regex range incomplete.
 *
 * The previous split regex only covered U+3000–U+9FFF and U+AC00–U+D7AF, so
 * characters from CJK Extension A/B/C-G were treated as non-CJK and never
 * separated from adjacent Latin tokens. This verified by mixing an extension
 * char directly with Latin text — "AI" and "test" must be split out separately.
 *
 * The fix replaces the regex with a character scan that reuses isCJK(), which
 * already covers every CJK extension block.
 */

import { extractKeywords, isCJK } from '../../services/rag';

describe('extractKeywords CJK extension split (#2100)', () => {
  // ── Extension B (U+20000) ────────────────────────────────
  it('splits Extension B char from adjacent Latin (AI𠀀test)', () => {
    // 𠀀 is U+20000 (CJK Extension B) — must be recognized as CJK
    expect(isCJK('𠀀')).toBe(true);
    // Before the fix, "AI𠀀test" stayed as a single token and neither
    // "ai" nor "test" was extracted as a standalone Latin keyword.
    const keywords = extractKeywords('AI𠀀test');
    expect(keywords).toContain('ai');
    expect(keywords).toContain('test');
  });

  // ── Extension A (U+3400) ────────────────────────────────
  it('splits Extension A char from adjacent Latin', () => {
    // 㐀 is U+3400 (CJK Extension A) — must be recognized as CJK
    expect(isCJK('㐀')).toBe(true);
    const keywords = extractKeywords('hello㐀world');
    expect(keywords).toContain('hello');
    expect(keywords).toContain('world');
  });

  // ── Regression: basic CJK + Latin still splits ──────────
  it('still splits basic CJK from Latin (AI人工智能)', () => {
    const keywords = extractKeywords('AI人工智能machine');
    expect(keywords).toContain('ai');
    expect(keywords).toContain('machine');
  });

  // ── Regression: pure Latin unaffected ───────────────────
  it('preserves pure Latin tokenization', () => {
    const keywords = extractKeywords('machine learning');
    expect(keywords).toContain('machine');
    expect(keywords).toContain('learning');
  });

  // ── Regression: pure CJK ngrams still extracted ─────────
  it('still extracts basic CJK ngrams', () => {
    const keywords = extractKeywords('机器学习');
    expect(keywords).toContain('机器');
    expect(keywords).toContain('学习');
  });
});
