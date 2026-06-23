/**
 * Regression test for #1426: buildFtsQuery + escapeLikePattern exported for direct testing.
 *
 * Directly imports the pure functions from db.ts instead of replicating them.
 * Covers: FTS query building, LIKE escaping, edge cases.
 */

import { buildFtsQuery, escapeLikePattern } from '../../db';

// ── buildFtsQuery ─────────────────────────────────────────

describe('buildFtsQuery (#1426)', () => {
  it('wraps each term in double quotes and joins with OR', () => {
    expect(buildFtsQuery('hello world')).toBe('"hello" OR "world"');
  });

  it('escapes double quotes inside terms', () => {
    // Each " is replaced with "", then wrapped in outer "quotes"
    expect(buildFtsQuery('say "hello"')).toBe('"say" OR """hello"""');
  });

  it('returns null for empty string', () => {
    expect(buildFtsQuery('')).toBeNull();
  });

  it('returns null for whitespace-only input', () => {
    expect(buildFtsQuery('   ')).toBeNull();
  });

  it('handles single term', () => {
    expect(buildFtsQuery('test')).toBe('"test"');
  });

  it('handles multiple spaces between terms', () => {
    expect(buildFtsQuery('hello   world')).toBe('"hello" OR "world"');
  });

  it('handles CJK text', () => {
    expect(buildFtsQuery('机器学习')).toBe('"机器学习"');
  });

  it('handles mixed CJK and Latin', () => {
    expect(buildFtsQuery('AI 人工智能')).toBe('"AI" OR "人工智能"');
  });

  it('handles special FTS5 characters', () => {
    // Double quotes are escaped by doubling them
    expect(buildFtsQuery('test"ing')).toBe('"test""ing"');
  });

  it('handles leading/trailing whitespace', () => {
    expect(buildFtsQuery('  hello world  ')).toBe('"hello" OR "world"');
  });
});

// ── escapeLikePattern ─────────────────────────────────────

describe('escapeLikePattern (#1426)', () => {
  it('escapes % with backslash prefix', () => {
    expect(escapeLikePattern('foo%bar')).toBe('foo\\%bar');
  });

  it('escapes _ with backslash prefix', () => {
    expect(escapeLikePattern('foo_bar')).toBe('foo\\_bar');
  });

  it('escapes backslash with double backslash', () => {
    expect(escapeLikePattern('foo\\bar')).toBe('foo\\\\bar');
  });

  it('escapes multiple special chars', () => {
    expect(escapeLikePattern('%_\\')).toBe('\\%\\_\\\\');
  });

  it('passes through normal text unchanged', () => {
    expect(escapeLikePattern('hello world')).toBe('hello world');
  });

  it('handles empty string', () => {
    expect(escapeLikePattern('')).toBe('');
  });

  it('handles CJK text unchanged', () => {
    expect(escapeLikePattern('机器学习')).toBe('机器学习');
  });
});
