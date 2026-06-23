/**
 * Regression test for #1381: chatHelpers.ts pure function unit tests.
 *
 * Tests all 4 exported functions:
 * - buildHistory: builds ChatMessage[] from local messages
 * - buildUserContent: builds content payload with/without attachments
 * - formatToolCallResult: appends tool call action summaries
 * - buildSavePreview: truncates content for save confirmation
 */

import {
  buildHistory,
  buildUserContent,
  formatToolCallResult,
  buildSavePreview,
  MAX_HISTORY_MESSAGES,
  type Msg,
} from '../../utils/chatHelpers';
import type { ContentPart } from '../../api/client';

describe('buildHistory', () => {
  const systemPrompt = 'You are a helpful assistant.';

  it('includes system prompt as first message', () => {
    const result = buildHistory([], systemPrompt, 'hello');
    expect(result[0]).toEqual({ role: 'system', content: systemPrompt });
  });

  it('includes user content as last message', () => {
    const result = buildHistory([], systemPrompt, 'hello');
    expect(result[result.length - 1]).toEqual({ role: 'user', content: 'hello' });
  });

  it('includes previous messages between system and user', () => {
    const prev: Msg[] = [
      { id: '1', role: 'user', content: 'hi' },
      { id: '2', role: 'assistant', content: 'hello!' },
    ];
    const result = buildHistory(prev, systemPrompt, 'how are you?');
    expect(result).toHaveLength(4); // system + 2 prev + user
    expect(result[1]).toEqual({ role: 'user', content: 'hi' });
    expect(result[2]).toEqual({ role: 'assistant', content: 'hello!' });
  });

  it('filters out streaming assistant messages', () => {
    const prev: Msg[] = [
      { id: '1', role: 'assistant', content: 'partial...', streaming: true },
      { id: '2', role: 'assistant', content: 'complete answer', streaming: false },
    ];
    const result = buildHistory(prev, systemPrompt, 'question');
    // system + 1 non-streaming + user
    expect(result).toHaveLength(3);
    expect(result[1].content).toBe('complete answer');
  });

  it('filters out error messages', () => {
    const prev: Msg[] = [
      { id: '1', role: 'assistant', content: 'error occurred', isError: true },
      { id: '2', role: 'user', content: 'real message' },
    ];
    const result = buildHistory(prev, systemPrompt, 'question');
    // system + 1 non-error + user
    expect(result).toHaveLength(3);
    expect(result[1].content).toBe('real message');
  });

  it('respects maxMessages limit', () => {
    const prev: Msg[] = Array.from({ length: 60 }, (_, i) => ({
      id: String(i),
      role: (i % 2 === 0 ? 'user' : 'assistant') as 'user' | 'assistant',
      content: `msg ${i}`,
    }));
    const result = buildHistory(prev, systemPrompt, 'last', 10);
    // system + 10 + user = 12
    expect(result).toHaveLength(12);
    // Should include the last 10 messages
    expect(result[1].content).toBe('msg 50');
  });

  it('uses default MAX_HISTORY_MESSAGES when not specified', () => {
    expect(MAX_HISTORY_MESSAGES).toBe(50);
  });

  it('handles empty previous messages', () => {
    const result = buildHistory([], systemPrompt, 'hello');
    expect(result).toEqual([
      { role: 'system', content: systemPrompt },
      { role: 'user', content: 'hello' },
    ]);
  });

  it('supports ContentPart[] as userContent', () => {
    const content: ContentPart[] = [
      { type: 'text', text: 'describe this' },
      { type: 'image_url', image_url: { url: 'data:image/png;base64,abc' } },
    ];
    const result = buildHistory([], systemPrompt, content);
    expect(result[result.length - 1].content).toEqual(content);
  });
});

describe('buildUserContent', () => {
  it('returns plain string when no attachments', () => {
    const result = buildUserContent('hello', []);
    expect(result).toBe('hello');
    expect(typeof result).toBe('string');
  });

  it('returns ContentPart[] when attachments present', () => {
    const result = buildUserContent('describe', [
      { base64: 'abc123', mime: 'image/png' },
    ]);
    expect(Array.isArray(result)).toBe(true);
    const parts = result as ContentPart[];
    expect(parts[0]).toEqual({ type: 'text', text: 'describe' });
    expect(parts[1]).toEqual({
      type: 'image_url',
      image_url: { url: 'data:image/png;base64,abc123' },
    });
  });

  it('returns ContentPart[] with multiple images', () => {
    const result = buildUserContent('compare', [
      { base64: 'img1', mime: 'image/png' },
      { base64: 'img2', mime: 'image/jpeg' },
    ]);
    const parts = result as ContentPart[];
    expect(parts).toHaveLength(3); // text + 2 images
    expect(parts[1]).toEqual({
      type: 'image_url',
      image_url: { url: 'data:image/png;base64,img1' },
    });
    expect(parts[2]).toEqual({
      type: 'image_url',
      image_url: { url: 'data:image/jpeg;base64,img2' },
    });
  });

  it('returns ContentPart[] even with empty text if images present', () => {
    const result = buildUserContent('', [
      { base64: 'abc', mime: 'image/png' },
    ]);
    expect(Array.isArray(result)).toBe(true);
    const parts = result as ContentPart[];
    // No text part since text is empty
    expect(parts).toHaveLength(1);
    expect(parts[0].type).toBe('image_url');
  });

  it('returns empty string when both text and attachments are empty', () => {
    const result = buildUserContent('', []);
    expect(result).toBe('');
  });
});

describe('formatToolCallResult', () => {
  it('returns cleaned text when no actions', () => {
    expect(formatToolCallResult('Hello world', [])).toBe('Hello world');
  });

  it('appends single action as italic', () => {
    const result = formatToolCallResult('Done', ['保存了笔记「测试」']);
    expect(result).toBe('Done\n\n_保存了笔记「测试」_');
  });

  it('joins multiple actions with semicolons', () => {
    const result = formatToolCallResult('OK', ['保存了笔记A', '搜索了关键词B']);
    expect(result).toBe('OK\n\n_保存了笔记A；搜索了关键词B_');
  });

  it('handles empty cleaned text with actions', () => {
    const result = formatToolCallResult('', ['action1']);
    expect(result).toBe('\n\n_action1_');
  });
});

describe('buildSavePreview', () => {
  it('returns full content when under limit', () => {
    expect(buildSavePreview('short text')).toBe('short text');
  });

  it('returns full content when exactly at limit', () => {
    const text = 'a'.repeat(200);
    expect(buildSavePreview(text)).toBe(text);
    expect(buildSavePreview(text)).toHaveLength(200);
  });

  it('truncates content exceeding default limit (200) with ellipsis', () => {
    const text = 'a'.repeat(250);
    const result = buildSavePreview(text);
    expect(result).toHaveLength(203); // 200 + '...'
    expect(result).toBe('a'.repeat(200) + '...');
  });

  it('respects custom maxLen', () => {
    const text = 'hello world this is a long text';
    const result = buildSavePreview(text, 10);
    expect(result).toBe('hello worl...');
  });

  it('handles empty content', () => {
    expect(buildSavePreview('')).toBe('');
  });

  it('handles CJK characters correctly', () => {
    const text = '这是一个测试文本，用于验证中文截断是否正确处理。';
    const result = buildSavePreview(text, 10);
    expect(result).toHaveLength(13); // 10 CJK chars + '...'
    expect(result.endsWith('...')).toBe(true);
  });
});
