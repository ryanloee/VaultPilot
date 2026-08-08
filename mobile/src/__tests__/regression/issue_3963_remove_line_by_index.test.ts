/**
 * Regression test for #3963 (mobile) — removeLineByIndex pure utility.
 *
 * Root cause: the old delete path filtered content by text equality
 * (`filter(l => l !== imageLine)`), deleting ALL lines matching the image's
 * markdown — including duplicate image lines the user did NOT select.
 *
 * Fix: `removeLineByIndex(content, lineIndex)` removes exactly one line by
 * its 0-based index, preserving any identical duplicates.
 */
import { removeLineByIndex } from '../../utils/imageMarkdown';

describe('#3963 removeLineByIndex', () => {
  it('removes only the line at the given index', () => {
    const content = 'line0\nline1\nline2\nline3';
    expect(removeLineByIndex(content, 2)).toBe('line0\nline1\nline3');
  });

  it('removes the FIRST occurrence when duplicate lines exist', () => {
    // Two identical image lines — deleting index 0 must keep index 2.
    const content = '![img](a.png)\nsome text\n![img](a.png)';
    const result = removeLineByIndex(content, 0);
    expect(result).toBe('some text\n![img](a.png)');
    // The remaining duplicate image line is preserved.
    expect(result.split('\n').filter((l) => l === '![img](a.png)').length).toBe(1);
  });

  it('removes a MIDDLE occurrence of duplicate lines, keeping both neighbors', () => {
    const content = '![img](a.png)\n![img](a.png)\n![img](a.png)';
    const result = removeLineByIndex(content, 1);
    expect(result).toBe('![img](a.png)\n![img](a.png)');
    expect(result.split('\n').length).toBe(2);
  });

  it('removes the LAST occurrence of duplicate lines', () => {
    const content = '![img](a.png)\ntext\n![img](a.png)';
    const result = removeLineByIndex(content, 2);
    expect(result).toBe('![img](a.png)\ntext');
  });

  it('handles single-line content', () => {
    expect(removeLineByIndex('only line', 0)).toBe('');
  });

  it('returns content unchanged for out-of-range positive index', () => {
    const content = 'a\nb\nc';
    expect(removeLineByIndex(content, 5)).toBe(content);
  });

  it('returns content unchanged for negative index', () => {
    const content = 'a\nb\nc';
    expect(removeLineByIndex(content, -1)).toBe(content);
  });

  it('preserves empty lines (only target removed)', () => {
    const content = 'a\n\n\nb';
    expect(removeLineByIndex(content, 1)).toBe('a\n\nb');
  });

  it('old text-equality filter would delete BOTH duplicates; index-based keeps one', () => {
    // This test documents the exact regression scenario from #3963.
    const content = '![photo](https://example.com/p.png)\nmiddle text\n![photo](https://example.com/p.png)';
    const imageLine = '![photo](https://example.com/p.png)';

    // OLD (buggy) behavior — would delete both:
    const buggyResult = content.split('\n').filter((l) => l !== imageLine).join('\n');
    expect(buggyResult).toBe('middle text'); // ← both images lost (data loss!)

    // NEW (fixed) behavior — deletes only the selected first image:
    const fixedResult = removeLineByIndex(content, 0);
    expect(fixedResult).toBe('middle text\n![photo](https://example.com/p.png)');
    // Second image (user did NOT select) is preserved.
  });
});
