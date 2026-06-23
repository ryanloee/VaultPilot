/**
 * Regression test for #1466 — stripMarkdown exported for direct testing.
 *
 * Verifies edge cases in markdown stripping that are hard to test
 * through extractAutoTags alone.
 */

import { stripMarkdown } from '../../utils/autoTag';

describe('issue #1466 — stripMarkdown direct tests', () => {
  test('strips fenced code blocks', () => {
    expect(stripMarkdown('before\n```js\ncode here\n```\nafter')).toBe('before after');
  });

  test('strips inline code', () => {
    expect(stripMarkdown('use `console.log` for debugging')).toBe('use for debugging');
  });

  test('extracts link text', () => {
    expect(stripMarkdown('[Google](https://google.com)')).toBe('Google');
  });

  test('extracts image alt text', () => {
    expect(stripMarkdown('![alt text](img.png)')).toBe('alt text');
  });

  test('strips heading markers', () => {
    expect(stripMarkdown('# Heading 1\n## Heading 2')).toBe('Heading 1 Heading 2');
  });

  test('strips bold and italic markers', () => {
    expect(stripMarkdown('**bold** and *italic* and __also bold__')).toBe('bold and italic and also bold');
  });

  test('strips URLs', () => {
    expect(stripMarkdown('visit https://example.com today')).toBe('visit today');
  });

  test('strips strikethrough', () => {
    expect(stripMarkdown('~~deleted~~ text')).toBe('deleted text');
  });

  test('collapses whitespace', () => {
    expect(stripMarkdown('  multiple   spaces  ')).toBe('multiple spaces');
  });

  test('handles empty string', () => {
    expect(stripMarkdown('')).toBe('');
  });

  test('handles plain text unchanged', () => {
    expect(stripMarkdown('just plain text')).toBe('just plain text');
  });

  test('handles multiple code blocks', () => {
    const input = 'text1\n```\ncode1\n```\ntext2\n```\ncode2\n```\ntext3';
    expect(stripMarkdown(input)).toBe('text1 text2 text3');
  });

  test('strips blockquote markers', () => {
    expect(stripMarkdown('> quoted text')).toBe('quoted text');
  });

  test('strips list markers', () => {
    expect(stripMarkdown('- item 1\n- item 2')).toBe('item 1 item 2');
  });
});
