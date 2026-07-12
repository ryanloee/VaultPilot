/**
 * Regression test for #2741: extractKeywords misclassifies CJK astral-plane
 * characters (Extension B-G, U+20000+) as Latin via `token[0]`.
 *
 * `token[0]` is a UTF-16 code unit; for astral chars it is merely a *surrogate*
 * (e.g. "𠀀"[0] === "\uD840"), which isCJK() does not recognise, so the whole
 * token fell into the Latin branch and was treated as a Latin keyword.
 *
 * The fix passes the whole token to isCJK() (which decodes via codePointAt(0)),
 * so astral CJK is classified as CJK. This guard verifies that astral-plane CJK
 * is preserved as a retrievable CJK term (not dropped) and that genuine Latin
 * keywords are still extracted.
 */

import { extractKeywords } from '../../services/rag';

describe('issue_2741: extractKeywords astral-plane CJK preserved as CJK', () => {
  test('astral-plane Extension B token is preserved as a keyword', () => {
    // 𠀀𠀁𠀂𠀃 = U+20000..U+20003 (CJK Extension B)
    const astral = '𠀀𠀁𠀂𠀃';
    const kws = extractKeywords(`${astral} machine learning`);
    // Astral CJK must remain retrievable (correctly classified, not dropped).
    expect(kws).toContain(astral);
    // Genuine Latin keywords are still extracted.
    expect(kws).toContain('machine');
    expect(kws).toContain('learning');
  });

  test('astral token mixed into a sentence is preserved', () => {
    const astral = '𠀀𠀁';
    const kws = extractKeywords(`the ${astral} note about 人工智能`);
    // Astral CJK must remain retrievable even when mixed with Latin/BMP CJK.
    expect(kws).toContain(astral);
    expect(kws).toContain('人工智能');
  });

  test('BMP CJK and Latin still both extracted (no regression)', () => {
    const kws = extractKeywords('人工智能 machine learning basics');
    // BMP CJK token is preserved as a CJK term.
    expect(kws).toContain('人工智能');
    expect(kws).toContain('machine');
    expect(kws).toContain('learning');
    expect(kws).toContain('basics');
  });
});
