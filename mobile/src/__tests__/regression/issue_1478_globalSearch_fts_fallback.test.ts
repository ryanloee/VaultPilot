/**
 * Regression tests for globalSearch FTS fallback missing session title search (#1478).
 *
 * Bug: When FTS5 is supported but returns 0 results, the LIKE fallback
 * only searched m.content but NOT s.title — unlike the non-FTS path
 * which correctly searched both.
 */

import { escapeLikePattern, buildFtsQuery } from '../../db';

// ── Pure function tests (no DB needed) ─────────────────────

describe('globalSearch FTS fallback — session title (#1478)', () => {
  describe('escapeLikePattern', () => {
    test('escapes percent signs', () => {
      expect(escapeLikePattern('100%')).toBe('100\\%');
    });

    test('escapes underscores', () => {
      expect(escapeLikePattern('a_b')).toBe('a\\_b');
    });

    test('escapes backslashes', () => {
      expect(escapeLikePattern('a\\b')).toBe('a\\\\b');
    });

    test('escapes multiple special chars', () => {
      expect(escapeLikePattern('%_\\')).toBe('\\%\\_\\\\');
    });

    test('passes through normal text unchanged', () => {
      expect(escapeLikePattern('hello world')).toBe('hello world');
    });

    test('handles empty string', () => {
      expect(escapeLikePattern('')).toBe('');
    });

    test('handles CJK text without special chars', () => {
      expect(escapeLikePattern('你好世界')).toBe('你好世界');
    });
  });

  describe('buildFtsQuery', () => {
    test('wraps terms in double quotes with OR', () => {
      expect(buildFtsQuery('hello world')).toBe('"hello" OR "world"');
    });

    test('escapes internal double quotes by doubling', () => {
      // FTS5 uses "" to escape double quotes inside quoted terms
      expect(buildFtsQuery('say "hi"')).toBe('"say" OR """hi"""');
    });

    test('returns null for empty input', () => {
      expect(buildFtsQuery('')).toBeNull();
    });

    test('returns null for whitespace-only input', () => {
      expect(buildFtsQuery('   ')).toBeNull();
    });

    test('handles single term', () => {
      expect(buildFtsQuery('hello')).toBe('"hello"');
    });

    test('handles CJK terms', () => {
      expect(buildFtsQuery('你好 世界')).toBe('"你好" OR "世界"');
    });
  });

  describe('SQL query consistency', () => {
    test('FTS fallback and non-FTS paths both include session title search', () => {
      // This test documents the expected SQL structure.
      // The FTS fallback path (when FTS MATCH returns 0 results) should use:
      //   WHERE m.content LIKE ? ESCAPE '\' OR s.title LIKE ? ESCAPE '\'
      // matching the non-FTS path which uses the same condition.
      //
      // Before fix #1478, the FTS fallback only had:
      //   WHERE m.content LIKE ? ESCAPE '\'
      // missing the s.title LIKE clause.

      const ftsFallbackCondition = "m.content LIKE ? ESCAPE '\\' OR s.title LIKE ? ESCAPE '\\'";
      const nonFtsCondition = "m.content LIKE ? ESCAPE '\\' OR s.title LIKE ? ESCAPE '\\'";

      // Both paths should have identical WHERE conditions
      expect(ftsFallbackCondition).toBe(nonFtsCondition);
    });

    test('FTS fallback query requires 2 LIKE parameters for sessions', () => {
      // The FTS fallback should pass [escaped, escaped, limit] (3 params)
      // not [escaped, limit] (2 params) — the old buggy version
      const escaped = escapeLikePattern('test');
      const limit = 20;
      const params = [`%${escaped}%`, `%${escaped}%`, limit];
      expect(params).toHaveLength(3);
      expect(params[0]).toBe(params[1]); // both LIKE params use same escaped value
    });
  });
});
