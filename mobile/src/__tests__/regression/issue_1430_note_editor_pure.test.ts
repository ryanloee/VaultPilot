/**
 * Regression tests for #1430: NoteEditorScreen pure function extraction + unit tests.
 *
 * Covers applyFormat, buildClipboardText, buildAiPrefill, shouldAutoTag, parseNewTag.
 */

import {
  applyFormat,
  buildClipboardText,
  buildAiPrefill,
  shouldAutoTag,
  parseNewTag,
} from '../../utils/noteEditorPure';

// ── applyFormat ───────────────────────────────────────────

describe('applyFormat', () => {
  test('wraps selected text with wrap syntax (**)', () => {
    const result = applyFormat('hello world', 0, 5, '**');
    expect(result.content).toBe('**hello** world');
    expect(result.cursorPos).toBe(9); // 0 + 2(len **) + 5(len hello) + 2(len **)
  });

  test('wraps empty selection with wrap syntax', () => {
    const result = applyFormat('hello', 5, 5, '**');
    expect(result.content).toBe('hello****');
    expect(result.cursorPos).toBe(9); // 5 + 2 + 0 + 2
  });

  test('inserts prefix syntax (# ) before selection', () => {
    const result = applyFormat('title', 0, 5, '# ');
    expect(result.content).toBe('# title');
    expect(result.cursorPos).toBe(7);
  });

  test('inserts prefix syntax (- ) for list', () => {
    const result = applyFormat('item', 0, 4, '- ');
    expect(result.content).toBe('- item');
    expect(result.cursorPos).toBe(6);
  });

  test('wraps middle selection', () => {
    const result = applyFormat('hello world', 6, 11, '*');
    expect(result.content).toBe('hello *world*');
    expect(result.cursorPos).toBe(13);
  });

  test('handles code block syntax', () => {
    const result = applyFormat('code', 0, 4, '`');
    expect(result.content).toBe('`code`');
    expect(result.cursorPos).toBe(6);
  });

  test('handles link syntax []()', () => {
    const result = applyFormat('text', 0, 4, '[]()');
    expect(result.content).toBe('[]()text[]()');
    expect(result.cursorPos).toBe(12);
  });
});

// ── buildClipboardText ────────────────────────────────────

describe('buildClipboardText', () => {
  test('combines title and content', () => {
    expect(buildClipboardText('My Title', 'Body text')).toBe('My Title\n\nBody text');
  });

  test('returns content only when title is empty', () => {
    expect(buildClipboardText('', 'Body text')).toBe('Body text');
  });

  test('returns empty string when content is empty', () => {
    expect(buildClipboardText('Title', '')).toBe('');
  });

  test('returns empty string when both are empty', () => {
    expect(buildClipboardText('', '')).toBe('');
  });

  test('handles whitespace-only title', () => {
    expect(buildClipboardText('   ', 'content')).toBe('   \n\ncontent');
  });
});

// ── buildAiPrefill ────────────────────────────────────────

describe('buildAiPrefill', () => {
  test('builds prefill with content', () => {
    const result = buildAiPrefill('请润色：', 'Hello world', '');
    expect(result).toBe('请润色：\n\nHello world');
  });

  test('uses title when content is empty', () => {
    const result = buildAiPrefill('请总结：', '', 'My Title');
    expect(result).toBe('请总结：\n\nMy Title');
  });

  test('returns empty string when both content and title are empty', () => {
    expect(buildAiPrefill('请润色：', '', '')).toBe('');
  });

  test('returns empty string when only whitespace', () => {
    expect(buildAiPrefill('请润色：', '   ', '')).toBe('');
  });

  test('truncates content to 2000 chars', () => {
    const longContent = 'a'.repeat(3000);
    const result = buildAiPrefill('请润色：', longContent, '');
    expect(result).toContain('请润色：\n\n');
    // The noteText portion should be 2000 chars
    const notePart = result.split('\n\n')[1];
    expect(notePart).toHaveLength(2000);
  });
});

// ── shouldAutoTag ─────────────────────────────────────────

describe('shouldAutoTag', () => {
  test('returns true when no tags and non-empty title', () => {
    expect(shouldAutoTag([], 'My Note')).toBe(true);
  });

  test('returns false when tags exist', () => {
    expect(shouldAutoTag(['tag1'], 'My Note')).toBe(false);
  });

  test('returns false when title is empty', () => {
    expect(shouldAutoTag([], '')).toBe(false);
  });

  test('returns false when title is whitespace', () => {
    expect(shouldAutoTag([], '   ')).toBe(false);
  });

  test('returns false when tags exist and title empty', () => {
    expect(shouldAutoTag(['tag1'], '')).toBe(false);
  });
});

// ── parseNewTag ───────────────────────────────────────────

describe('parseNewTag', () => {
  test('returns trimmed tag when valid', () => {
    expect(parseNewTag('  my-tag  ', [])).toBe('my-tag');
  });

  test('returns null for empty string', () => {
    expect(parseNewTag('', [])).toBeNull();
  });

  test('returns null for whitespace-only', () => {
    expect(parseNewTag('   ', [])).toBeNull();
  });

  test('returns null for duplicate tag', () => {
    expect(parseNewTag('existing', ['existing', 'other'])).toBeNull();
  });

  test('returns tag when not duplicate', () => {
    expect(parseNewTag('new-tag', ['existing'])).toBe('new-tag');
  });

  test('handles CJK tags', () => {
    expect(parseNewTag('笔记', [])).toBe('笔记');
  });

  test('trims surrounding whitespace from CJK tags', () => {
    expect(parseNewTag('  标签  ', [])).toBe('标签');
  });
});
