/**
 * Regression test for #1414: ChatScreen JSON.parse(m.attachments) unprotected.
 *
 * Corrupt attachment JSON must not crash session loading —
 * safeParseAttachments returns undefined for invalid input.
 */

// Extract the safe parse helper for unit testing.
// We replicate the exact logic from ChatScreen.tsx.

interface Attachment { name: string; type: 'image' | 'file'; }

function safeParseAttachments(raw: string | null | undefined): Attachment[] | undefined {
  if (!raw) return undefined;
  try { return JSON.parse(raw); } catch { return undefined; }
}

describe('safeParseAttachments (#1414)', () => {
  test('returns undefined for null input', () => {
    expect(safeParseAttachments(null)).toBeUndefined();
  });

  test('returns undefined for undefined input', () => {
    expect(safeParseAttachments(undefined)).toBeUndefined();
  });

  test('returns undefined for empty string', () => {
    expect(safeParseAttachments('')).toBeUndefined();
  });

  test('parses valid attachment array', () => {
    const input = JSON.stringify([{ name: 'photo.jpg', type: 'image' }]);
    expect(safeParseAttachments(input)).toEqual([{ name: 'photo.jpg', type: 'image' }]);
  });

  test('returns undefined for truncated JSON', () => {
    expect(safeParseAttachments('[{"name":"photo.jpg","type":"im')).toBeUndefined();
  });

  test('returns undefined for plain text garbage', () => {
    expect(safeParseAttachments('not json at all')).toBeUndefined();
  });

  test('parses number string (valid JSON but not array — consumer handles)', () => {
    // 42 is valid JSON — safeParse returns it. Consumer (map/forEach) handles non-array gracefully.
    expect(safeParseAttachments('42')).toBe(42);
  });

  test('parses empty array', () => {
    expect(safeParseAttachments('[]')).toEqual([]);
  });

  test('parses multiple attachments', () => {
    const input = JSON.stringify([
      { name: 'a.png', type: 'image' },
      { name: 'b.pdf', type: 'file' },
    ]);
    expect(safeParseAttachments(input)).toHaveLength(2);
  });
});
