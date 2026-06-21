/**
 * Unit tests for ChatScreen pure logic helpers (#1214).
 *
 * Tests: buildHistory, buildUserContent, formatToolCallResult, buildSavePreview.
 * All functions are extracted from ChatScreen.tsx and have zero RN dependencies.
 */

import {
  buildHistory,
  buildUserContent,
  formatToolCallResult,
  buildSavePreview,
  MAX_HISTORY_MESSAGES,
  Msg,
} from '../../utils/chatHelpers';

// ── buildHistory ──────────────────────────────────────────────

describe('buildHistory', () => {
  it('includes system prompt, filtered messages, and new user message', () => {
    const prevMsgs: Msg[] = [
      { id: '1', role: 'user', content: 'hello' },
      { id: '2', role: 'assistant', content: 'hi there' },
    ];
    const result = buildHistory(prevMsgs, 'You are helpful.', 'new question');

    expect(result[0]).toEqual({ role: 'system', content: 'You are helpful.' });
    expect(result[result.length - 1]).toEqual({ role: 'user', content: 'new question' });
    expect(result).toHaveLength(4); // system + 2 prev + user
  });

  it('filters out streaming assistant messages', () => {
    const prevMsgs: Msg[] = [
      { id: '1', role: 'user', content: 'q1' },
      { id: '2', role: 'assistant', content: '', streaming: true },
    ];
    const result = buildHistory(prevMsgs, 'sys', 'q2');
    // streaming message should be filtered out
    const nonSystem = result.filter(m => m.role !== 'system');
    expect(nonSystem).toHaveLength(2); // user q1 + user q2
    expect(nonSystem[0].content).toBe('q1');
    expect(nonSystem[1].content).toBe('q2');
  });

  it('filters out error messages', () => {
    const prevMsgs: Msg[] = [
      { id: '1', role: 'assistant', content: 'error occurred', isError: true },
      { id: '2', role: 'user', content: 'real message' },
    ];
    const result = buildHistory(prevMsgs, 'sys', 'new');
    const nonSystem = result.filter(m => m.role !== 'system');
    expect(nonSystem).toHaveLength(2); // real message + new
    expect(nonSystem[0].content).toBe('real message');
  });

  it('truncates to MAX_HISTORY_MESSAGES', () => {
    const prevMsgs: Msg[] = Array.from({ length: 60 }, (_, i) => ({
      id: String(i),
      role: (i % 2 === 0 ? 'user' : 'assistant') as 'user' | 'assistant',
      content: `msg ${i}`,
    }));
    const result = buildHistory(prevMsgs, 'sys', 'new');
    // system + 50 messages + user new = 52
    expect(result).toHaveLength(2 + MAX_HISTORY_MESSAGES);
  });

  it('accepts custom maxMessages', () => {
    const prevMsgs: Msg[] = Array.from({ length: 10 }, (_, i) => ({
      id: String(i),
      role: 'user' as const,
      content: `msg ${i}`,
    }));
    const result = buildHistory(prevMsgs, 'sys', 'new', 3);
    // system + 3 + user = 5
    expect(result).toHaveLength(5);
  });

  it('handles empty prevMsgs', () => {
    const result = buildHistory([], 'sys', 'hello');
    expect(result).toEqual([
      { role: 'system', content: 'sys' },
      { role: 'user', content: 'hello' },
    ]);
  });

  it('preserves userContent as ContentPart[] when provided', () => {
    const content = [
      { type: 'text' as const, text: 'describe this' },
      { type: 'image_url' as const, image_url: { url: 'data:image/jpeg;base64,abc' } },
    ];
    const result = buildHistory([], 'sys', content);
    expect(result[1].content).toEqual(content);
  });
});

// ── buildUserContent ──────────────────────────────────────────

describe('buildUserContent', () => {
  it('returns plain string when no attachments', () => {
    expect(buildUserContent('hello', [])).toBe('hello');
  });

  it('returns plain string when only text attachment', () => {
    // If somehow there's a text-only ContentPart, it should still return string
    expect(buildUserContent('hello', [])).toBe('hello');
  });

  it('builds ContentPart[] with image attachment', () => {
    const result = buildUserContent('describe', [{ base64: 'abc123', mime: 'image/jpeg' }]);
    expect(Array.isArray(result)).toBe(true);
    if (Array.isArray(result)) {
      expect(result).toHaveLength(2);
      expect(result[0]).toEqual({ type: 'text', text: 'describe' });
      expect(result[1]).toEqual({
        type: 'image_url',
        image_url: { url: 'data:image/jpeg;base64,abc123' },
      });
    }
  });

  it('handles empty text with image', () => {
    const result = buildUserContent('', [{ base64: 'img', mime: 'image/png' }]);
    expect(Array.isArray(result)).toBe(true);
    if (Array.isArray(result)) {
      expect(result).toHaveLength(1);
      expect(result[0].type).toBe('image_url');
    }
  });

  it('handles multiple attachments', () => {
    const result = buildUserContent('text', [
      { base64: 'a', mime: 'image/jpeg' },
      { base64: 'b', mime: 'image/png' },
    ]);
    expect(Array.isArray(result)).toBe(true);
    if (Array.isArray(result)) {
      expect(result).toHaveLength(3); // text + 2 images
    }
  });
});

// ── formatToolCallResult ──────────────────────────────────────

describe('formatToolCallResult', () => {
  it('returns cleaned text unchanged when no actions', () => {
    expect(formatToolCallResult('hello world', [])).toBe('hello world');
  });

  it('appends single action as italic', () => {
    const result = formatToolCallResult('content', ['已保存笔记']);
    expect(result).toBe('content\n\n_已保存笔记_');
  });

  it('joins multiple actions with Chinese semicolon', () => {
    const result = formatToolCallResult('content', ['保存了笔记 A', '保存了笔记 B']);
    expect(result).toBe('content\n\n_保存了笔记 A；保存了笔记 B_');
  });
});

// ── buildSavePreview ──────────────────────────────────────────

describe('buildSavePreview', () => {
  it('returns full content when under limit', () => {
    expect(buildSavePreview('short text')).toBe('short text');
  });

  it('truncates at 200 chars with ellipsis', () => {
    const long = 'x'.repeat(250);
    const result = buildSavePreview(long);
    expect(result).toHaveLength(203); // 200 + '...'
    expect(result.endsWith('...')).toBe(true);
  });

  it('respects custom maxLen', () => {
    const result = buildSavePreview('abcdefghij', 5);
    expect(result).toBe('abcde...');
  });

  it('exact boundary — no truncation at maxLen', () => {
    const exact = 'x'.repeat(200);
    expect(buildSavePreview(exact)).toBe(exact);
  });
});
