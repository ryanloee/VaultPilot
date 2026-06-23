/**
 * Regression test for #1396: system message ContentPart[] content produces [object Object].
 *
 * client.ts used systemMsgs.map(m => m.content).join('\n') which calls .toString()
 * on ContentPart objects, producing [object Object] instead of actual text.
 *
 * extractTextContent() properly extracts text from both string and ContentPart[] content.
 */

import { extractTextContent } from '../../api/clientPure';

describe('extractTextContent (#1396)', () => {
  it('returns string content unchanged', () => {
    expect(extractTextContent('hello world')).toBe('hello world');
  });

  it('returns empty string unchanged', () => {
    expect(extractTextContent('')).toBe('');
  });

  it('extracts text from single text ContentPart', () => {
    const content = [{ type: 'text' as const, text: 'hello' }];
    expect(extractTextContent(content)).toBe('hello');
  });

  it('joins multiple text ContentParts with newline', () => {
    const content = [
      { type: 'text' as const, text: 'line 1' },
      { type: 'text' as const, text: 'line 2' },
    ];
    expect(extractTextContent(content)).toBe('line 1\nline 2');
  });

  it('ignores image_url parts and extracts only text', () => {
    const content = [
      { type: 'text' as const, text: 'before image' },
      { type: 'image_url' as const, image_url: { url: 'data:image/png;base64,abc' } },
      { type: 'text' as const, text: 'after image' },
    ];
    expect(extractTextContent(content)).toBe('before image\nafter image');
  });

  it('returns empty string when only image parts present', () => {
    const content = [
      { type: 'image_url' as const, image_url: { url: 'https://example.com/img.png' } },
    ];
    expect(extractTextContent(content)).toBe('');
  });

  it('returns empty string for empty ContentPart array', () => {
    expect(extractTextContent([])).toBe('');
  });

  it('handles single text part with multiline content', () => {
    const content = [{ type: 'text' as const, text: 'line 1\nline 2\nline 3' }];
    expect(extractTextContent(content)).toBe('line 1\nline 2\nline 3');
  });

  it('produces correct result where old code produced [object Object]', () => {
    // This is the exact scenario that was broken:
    // Old code: systemMsgs.map(m => m.content).join('\n')
    // would produce "[object Object]" for ContentPart[] content
    const content = [
      { type: 'text' as const, text: 'You are a helpful assistant.' },
    ];
    const result = extractTextContent(content);
    expect(result).not.toContain('[object');
    expect(result).toBe('You are a helpful assistant.');
  });
});
