/**
 * Regression test for #1384: LIKE ESCAPE backslash mismatch.
 *
 * db.ts non-FTS LIKE queries used ESCAPE '\\' (two backslashes in SQL)
 * but escapeLikePattern() uses single \ as the escape prefix.
 * This test verifies the escape logic is consistent with ESCAPE '\'.
 */

/** Replicate escapeLikePattern from db.ts */
function escapeLikePattern(pattern: string): string {
  return pattern.replace(/[\\%_]/g, ch => `\\${ch}`);
}

describe('LIKE ESCAPE consistency (#1384)', () => {
  it('escapeLikePattern escapes % with single backslash prefix', () => {
    expect(escapeLikePattern('foo%bar')).toBe('foo\\%bar');
  });

  it('escapeLikePattern escapes _ with single backslash prefix', () => {
    expect(escapeLikePattern('foo_bar')).toBe('foo\\_bar');
  });

  it('escapeLikePattern escapes \\ with double backslash', () => {
    expect(escapeLikePattern('foo\\bar')).toBe('foo\\\\bar');
  });

  it('escapeLikePattern escapes multiple special chars', () => {
    expect(escapeLikePattern('%_\\')).toBe('\\%\\_\\\\');
  });

  it('escapeLikePattern passes through normal text unchanged', () => {
    expect(escapeLikePattern('hello world')).toBe('hello world');
  });

  it('template literal ESCAPE resolves to single backslash', () => {
    // This is the key assertion: in a template literal, '\\' produces '\'
    // which is a single backslash character in the resulting string.
    const escapeClause = `ESCAPE '\\'`;
    // The SQL string should contain ESCAPE followed by a quoted single backslash
    expect(escapeClause).toBe("ESCAPE '\\'");
    // The escape character itself should be a single backslash
    const match = escapeClause.match(/ESCAPE '(.+)'/);
    expect(match).not.toBeNull();
    expect(match![1]).toBe('\\');
    expect(match![1].length).toBe(1);
  });

  it('old template literal ESCAPE incorrectly resolves to two backslashes', () => {
    // The bug: '\\\\' in template literal produces '\\' (two backslashes)
    const buggyClause = `ESCAPE '\\\\'`;
    const match = buggyClause.match(/ESCAPE '(.+)'/);
    expect(match).not.toBeNull();
    expect(match![1]).toBe('\\\\');
    expect(match![1].length).toBe(2); // Two backslashes — WRONG for escapeLikePattern
  });
});
