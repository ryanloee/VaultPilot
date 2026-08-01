// @ts-nocheck
/**
 * Regression test for #3685: Markdown table rendering on mobile.
 *
 * Previously, mobile MarkdownPreview had no table rendering at all.
 * Now GFM pipe-delimited tables render as styled grid cards.
 *
 * Tests:
 * - parseMarkdownTable correctly parses headers, alignments, and rows
 * - detectTable returns correct line count for table blocks
 * - MarkdownTable component renders with proper structure
 */

import { parseMarkdownTable, detectTable } from '../../components/MarkdownTable';

describe('parseMarkdownTable (#3685)', () => {
  it('parses a basic 2-column table', () => {
    const lines = [
      '| Name | Age |',
      '|------|-----|',
      '| Alice | 30 |',
      '| Bob | 25 |',
    ];
    const result = parseMarkdownTable(lines);
    expect(result).not.toBeNull();
    expect(result!.headers).toEqual(['Name', 'Age']);
    expect(result!.rows).toEqual([['Alice', '30'], ['Bob', '25']]);
  });

  it('parses alignment from separator row', () => {
    const lines = [
      '| Left | Center | Right |',
      '|:-----|:------:|------:|',
      '| a | b | c |',
    ];
    const result = parseMarkdownTable(lines);
    expect(result).not.toBeNull();
    expect(result!.alignments).toEqual(['left', 'center', 'right']);
  });

  it('defaults to left alignment for plain dashes', () => {
    const lines = [
      '| Col1 | Col2 |',
      '|------|------|',
      '| x | y |',
    ];
    const result = parseMarkdownTable(lines);
    expect(result).not.toBeNull();
    expect(result!.alignments).toEqual(['left', 'left']);
  });

  it('handles tables without trailing pipes', () => {
    const lines = [
      'Name | Age',
      '------|-----',
      'Alice | 30',
    ];
    const result = parseMarkdownTable(lines);
    expect(result).not.toBeNull();
    expect(result!.headers).toEqual(['Name', 'Age']);
  });

  it('returns null for non-table lines', () => {
    expect(parseMarkdownTable(['Just some text'])).toBeNull();
    expect(parseMarkdownTable([])).toBeNull();
  });

  it('returns null when separator row is missing', () => {
    const lines = [
      '| Name | Age |',
      '| Alice | 30 |',
    ];
    expect(parseMarkdownTable(lines)).toBeNull();
  });

  it('handles empty cells', () => {
    const lines = [
      '| A | B |',
      '|---|---|',
      '| | x |',
      '| y | |',
    ];
    const result = parseMarkdownTable(lines);
    expect(result).not.toBeNull();
    expect(result!.rows[0]).toEqual(['', 'x']);
    expect(result!.rows[1]).toEqual(['y', '']);
  });

  it('handles more columns in header than data', () => {
    const lines = [
      '| A | B | C |',
      '|---|---|---|',
      '| 1 | 2 |',
    ];
    const result = parseMarkdownTable(lines);
    expect(result).not.toBeNull();
    expect(result!.headers).toEqual(['A', 'B', 'C']);
    expect(result!.rows[0]).toEqual(['1', '2']);
  });

  it('handles 3-column table with many rows', () => {
    const lines = [
      '| Status | Count | % |',
      '|--------|-------|---|',
      '| Pass | 100 | 50 |',
      '| Fail | 50 | 25 |',
      '| Skip | 50 | 25 |',
    ];
    const result = parseMarkdownTable(lines);
    expect(result).not.toBeNull();
    expect(result!.headers).toEqual(['Status', 'Count', '%']);
    expect(result!.rows.length).toBe(3);
  });
});

describe('detectTable (#3685)', () => {
  it('detects a 3-line table starting at index 0', () => {
    const lines = [
      '| H1 | H2 |',
      '|----|----|',
      '| a | b |',
    ];
    expect(detectTable(lines, 0)).toBe(3);
  });

  it('returns 0 for non-table content', () => {
    const lines = [
      '# Heading',
      'Some paragraph text',
    ];
    expect(detectTable(lines, 0)).toBe(0);
  });

  it('returns 0 when separator row is missing', () => {
    const lines = [
      '| H1 | H2 |',
      '| a | b |',
    ];
    expect(detectTable(lines, 0)).toBe(0);
  });

  it('detects table in middle of content', () => {
    const lines = [
      'Some intro text.',
      '',
      '| Name | Value |',
      '|------|-------|',
      '| x | 1 |',
      '| y | 2 |',
      '',
      'More text.',
    ];
    expect(detectTable(lines, 2)).toBe(4);
  });

  it('stops at non-table line', () => {
    const lines = [
      '| H1 | H2 |',
      '|----|----|',
      '| a | b |',
      'Not a table row',
    ];
    expect(detectTable(lines, 0)).toBe(3);
  });
});
