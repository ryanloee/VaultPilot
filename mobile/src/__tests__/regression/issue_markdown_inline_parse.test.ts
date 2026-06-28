/**
 * Unit tests for utils/markdown.ts — inline markdown parsing.
 *
 * Extracted from MarkdownPreview.tsx for testability.
 */

import { parseInline } from '../../utils/markdown';

// ── parseInline ──────────────────────────────────────────

describe('parseInline', () => {
  // Plain text
  it('returns single text element for plain text', () => {
    expect(parseInline('hello world')).toEqual([
      { type: 'text', text: 'hello world' },
    ]);
  });

  it('handles empty string', () => {
    expect(parseInline('')).toEqual([]);
  });

  // Bold
  it('parses bold text', () => {
    expect(parseInline('**bold**')).toEqual([
      { type: 'bold', text: 'bold' },
    ]);
  });

  it('parses bold with surrounding text', () => {
    expect(parseInline('before **bold** after')).toEqual([
      { type: 'text', text: 'before ' },
      { type: 'bold', text: 'bold' },
      { type: 'text', text: ' after' },
    ]);
  });

  // Italic
  it('parses italic text', () => {
    expect(parseInline('*italic*')).toEqual([
      { type: 'italic', text: 'italic' },
    ]);
  });

  it('parses italic with surrounding text', () => {
    expect(parseInline('before *italic* after')).toEqual([
      { type: 'text', text: 'before ' },
      { type: 'italic', text: 'italic' },
      { type: 'text', text: ' after' },
    ]);
  });

  // Code
  it('parses inline code', () => {
    expect(parseInline('`code`')).toEqual([
      { type: 'code', text: 'code' },
    ]);
  });

  it('parses code with surrounding text', () => {
    expect(parseInline('use `console.log` here')).toEqual([
      { type: 'text', text: 'use ' },
      { type: 'code', text: 'console.log' },
      { type: 'text', text: ' here' },
    ]);
  });

  // Link
  it('parses links', () => {
    expect(parseInline('[Google](https://google.com)')).toEqual([
      { type: 'link', text: 'Google', url: 'https://google.com' },
    ]);
  });

  it('parses link with surrounding text', () => {
    expect(parseInline('visit [Google](https://google.com) now')).toEqual([
      { type: 'text', text: 'visit ' },
      { type: 'link', text: 'Google', url: 'https://google.com' },
      { type: 'text', text: ' now' },
    ]);
  });

  // Mixed formatting
  it('parses bold and italic together', () => {
    expect(parseInline('**bold** and *italic*')).toEqual([
      { type: 'bold', text: 'bold' },
      { type: 'text', text: ' and ' },
      { type: 'italic', text: 'italic' },
    ]);
  });

  // Code has highest priority — text before code is not re-parsed for bold/italic
  it('code regex consumes preceding text literally', () => {
    // When code matches, everything before the backtick becomes plain text
    // This is the original MarkdownPreview.tsx behavior
    const result = parseInline('**bold** and `code`');
    expect(result).toEqual([
      { type: 'text', text: '**bold** and ' },
      { type: 'code', text: 'code' },
    ]);
  });

  it('parses link alongside bold', () => {
    expect(parseInline('see **docs** at [site](url)')).toEqual([
      { type: 'text', text: 'see ' },
      { type: 'bold', text: 'docs' },
      { type: 'text', text: ' at ' },
      { type: 'link', text: 'site', url: 'url' },
    ]);
  });

  // Edge cases
  it('handles text starting with unmatched backtick', () => {
    const result = parseInline('`unmatched');
    // The backtick is emitted as text, then the rest
    expect(result.some(e => e.text.includes('`'))).toBe(true);
  });

  it('handles multiple consecutive bold segments', () => {
    expect(parseInline('**a** and **b**')).toEqual([
      { type: 'bold', text: 'a' },
      { type: 'text', text: ' and ' },
      { type: 'bold', text: 'b' },
    ]);
  });

  it('handles multiple consecutive code segments', () => {
    expect(parseInline('`a` and `b`')).toEqual([
      { type: 'code', text: 'a' },
      { type: 'text', text: ' and ' },
      { type: 'code', text: 'b' },
    ]);
  });

  // Real-world examples
  it('parses a realistic markdown line', () => {
    const input = 'Use `npm install` to install **dependencies** from [npm](https://npmjs.com)';
    const result = parseInline(input);
    // Code regex: text before code is literal, text after code is re-parsed
    expect(result).toEqual([
      { type: 'text', text: 'Use ' },
      { type: 'code', text: 'npm install' },
      { type: 'text', text: ' to install ' },
      { type: 'bold', text: 'dependencies' },
      { type: 'text', text: ' from ' },
      { type: 'link', text: 'npm', url: 'https://npmjs.com' },
    ]);
  });

  it('parses bold + link without code interference', () => {
    const result = parseInline('**bold** and [link](url)');
    expect(result).toEqual([
      { type: 'bold', text: 'bold' },
      { type: 'text', text: ' and ' },
      { type: 'link', text: 'link', url: 'url' },
    ]);
  });

  it('preserves order of inline elements', () => {
    const result = parseInline('a `b` c **d** e *f* g [h](i)');
    // code has highest priority, text before code is literal
    expect(result.map(e => e.type)).toEqual([
      'text', 'code', 'text', 'bold', 'text', 'italic', 'text', 'link',
    ]);
  });

  // ── Wikilinks and Block References ───────────────────────

  it('parses simple wikilink [[Note Name]]', () => {
    const result = parseInline('See [[My Note]] for details');
    expect(result).toEqual([
      { type: 'text', text: 'See ' },
      { type: 'wikilink', text: 'My Note', url: 'My Note' },
      { type: 'text', text: ' for details' },
    ]);
  });

  it('parses wikilink with display text [[Note|display]]', () => {
    const result = parseInline('[[My Note|click here]]');
    expect(result).toEqual([
      { type: 'wikilink', text: 'click here', url: 'My Note' },
    ]);
  });

  it('parses block reference [[Note#^blockid]]', () => {
    const result = parseInline('[[Tasks#^abc123]]');
    expect(result).toHaveLength(1);
    expect(result[0].type).toBe('blockref');
    expect(result[0].noteName).toBe('Tasks');
    expect(result[0].blockId).toBe('abc123');
  });

  it('parses block reference with display text', () => {
    const result = parseInline('[[Tasks#^abc123|see task]]');
    expect(result).toHaveLength(1);
    expect(result[0].type).toBe('blockref');
    expect(result[0].text).toBe('see task');
    expect(result[0].blockId).toBe('abc123');
  });

  it('parses heading anchor [[Note#Heading]]', () => {
    const result = parseInline('[[Guide#Installation]]');
    expect(result).toHaveLength(1);
    expect(result[0].type).toBe('blockref');
    expect(result[0].noteName).toBe('Guide');
    expect(result[0].blockId).toBe('Installation');
  });

  it('parses wikilink alongside other formatting', () => {
    const result = parseInline('**bold** and [[note]] text');
    // Wikilink has higher priority than bold — text before wikilink is literal
    // (same behavior as code: text before code is not re-parsed)
    expect(result.map(e => e.type)).toEqual(['text', 'wikilink', 'text']);
  });
});
