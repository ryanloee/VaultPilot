/**
 * Regression tests for issue #1441: clientPure.ts pure function unit tests.
 * Covers all 5 Anthropic API adapter functions (20+ tests).
 */

import {
  toAnthropicContent,
  convertAnthropicEvent,
  wrapNonStreamingResponse,
  extractTextContent,
  normalizeAnthropicBase,
} from '../../api/clientPure';
import type { ContentPart } from '../../api/client';

// ── toAnthropicContent ────────────────────────────────────────

describe('toAnthropicContent', () => {
  it('returns string content unchanged', () => {
    expect(toAnthropicContent('hello')).toBe('hello');
  });

  it('converts text ContentPart to passthrough', () => {
    const parts: ContentPart[] = [{ type: 'text', text: 'hi' }];
    const result = toAnthropicContent(parts) as Record<string, unknown>[];
    expect(result).toEqual([{ type: 'text', text: 'hi' }]);
  });

  it('converts base64 image_url to Anthropic image block', () => {
    const parts: ContentPart[] = [
      { type: 'image_url', image_url: { url: 'data:image/png;base64,AAAA' } },
    ];
    const result = toAnthropicContent(parts) as Record<string, unknown>[];
    expect(result[0]).toEqual({
      type: 'image',
      source: { type: 'base64', media_type: 'image/png', data: 'AAAA' },
    });
  });

  it('returns fallback text for non-data-URI images', () => {
    const parts: ContentPart[] = [
      { type: 'image_url', image_url: { url: 'https://example.com/img.png' } },
    ];
    const result = toAnthropicContent(parts) as Record<string, unknown>[];
    expect(result[0]).toEqual({ type: 'text', text: '[image unavailable]' });
  });

  it('handles mixed text and image parts', () => {
    const parts: ContentPart[] = [
      { type: 'text', text: 'describe this' },
      { type: 'image_url', image_url: { url: 'data:image/jpeg;base64,/9j/4AAQ' } },
    ];
    const result = toAnthropicContent(parts) as Record<string, unknown>[];
    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({ type: 'text', text: 'describe this' });
    expect(result[1]).toEqual({
      type: 'image',
      source: { type: 'base64', media_type: 'image/jpeg', data: '/9j/4AAQ' },
    });
  });
});

// ── convertAnthropicEvent ─────────────────────────────────────

describe('convertAnthropicEvent', () => {
  it('converts content_block_delta to OpenAI SSE format', () => {
    const data = JSON.stringify({ type: 'content_block_delta', delta: { text: 'Hi' } });
    const result = convertAnthropicEvent('message', data);
    expect(result).toBe(`data: ${JSON.stringify({ choices: [{ delta: { content: 'Hi' } }] })}\n\n`);
  });

  it('converts message_stop to [DONE] sentinel', () => {
    const data = JSON.stringify({ type: 'message_stop' });
    expect(convertAnthropicEvent('message', data)).toBe('data: [DONE]\n\n');
  });

  it('returns null for message_start event', () => {
    const data = JSON.stringify({ type: 'message_start', message: {} });
    expect(convertAnthropicEvent('message', data)).toBeNull();
  });

  it('returns null for content_block_start event', () => {
    const data = JSON.stringify({ type: 'content_block_start', index: 0 });
    expect(convertAnthropicEvent('message', data)).toBeNull();
  });

  it('returns null for unparseable JSON', () => {
    expect(convertAnthropicEvent('message', 'not-json')).toBeNull();
  });

  it('returns null when delta has no text field', () => {
    const data = JSON.stringify({ type: 'content_block_delta', delta: { type: 'text' } });
    expect(convertAnthropicEvent('message', data)).toBeNull();
  });
});

// ── wrapNonStreamingResponse ──────────────────────────────────

describe('wrapNonStreamingResponse', () => {
  it('wraps content into SSE data + [DONE]', () => {
    const json = { choices: [{ message: { content: 'Hello' }, finish_reason: 'stop' }] };
    const bytes = wrapNonStreamingResponse(json);
    const text = new TextDecoder().decode(bytes);
    const lines = text.split('\n\n').filter(Boolean);

    expect(lines).toHaveLength(2);

    const parsed = JSON.parse(lines[0].replace('data: ', ''));
    expect(parsed.choices[0].delta.content).toBe('Hello');
    expect(parsed.choices[0].finish_reason).toBe('stop');
    expect(lines[1]).toBe('data: [DONE]');
  });

  it('wraps tool_calls into delta', () => {
    const toolCalls = [{ id: 'tc1', type: 'function', function: { name: 'search' } }];
    const json = { choices: [{ message: { tool_calls: toolCalls }, finish_reason: 'stop' }] };
    const bytes = wrapNonStreamingResponse(json);
    const text = new TextDecoder().decode(bytes);
    const parsed = JSON.parse(text.split('\n\n')[0].replace('data: ', ''));
    expect(parsed.choices[0].delta.tool_calls).toEqual(toolCalls);
  });

  it('handles missing message gracefully', () => {
    const json = { choices: [{}] };
    const bytes = wrapNonStreamingResponse(json);
    const text = new TextDecoder().decode(bytes);
    expect(text).toContain('data: [DONE]');
  });

  it('handles empty choices array', () => {
    const json = { choices: [] };
    const bytes = wrapNonStreamingResponse(json);
    const text = new TextDecoder().decode(bytes);
    expect(text).toContain('data: [DONE]');
  });

  it('handles missing choices', () => {
    const json = {};
    const bytes = wrapNonStreamingResponse(json);
    const text = new TextDecoder().decode(bytes);
    expect(text).toContain('data: [DONE]');
  });
});

// ── extractTextContent ────────────────────────────────────────

describe('extractTextContent', () => {
  it('returns string content unchanged', () => {
    expect(extractTextContent('hello world')).toBe('hello world');
  });

  it('extracts text from text parts', () => {
    const parts: ContentPart[] = [
      { type: 'text', text: 'line 1' },
      { type: 'text', text: 'line 2' },
    ];
    expect(extractTextContent(parts)).toBe('line 1\nline 2');
  });

  it('ignores image_url parts', () => {
    const parts: ContentPart[] = [
      { type: 'text', text: 'before' },
      { type: 'image_url', image_url: { url: 'data:image/png;base64,AAA' } },
      { type: 'text', text: 'after' },
    ];
    expect(extractTextContent(parts)).toBe('before\nafter');
  });

  it('returns empty string for all-image content', () => {
    const parts: ContentPart[] = [
      { type: 'image_url', image_url: { url: 'data:image/png;base64,AAA' } },
    ];
    expect(extractTextContent(parts)).toBe('');
  });

  it('returns empty string for empty array', () => {
    expect(extractTextContent([])).toBe('');
  });
});

// ── normalizeAnthropicBase ────────────────────────────────────

describe('normalizeAnthropicBase', () => {
  it('strips /v1 suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v1')).toBe('https://api.anthropic.com');
  });

  it('strips /v2 suffix', () => {
    expect(normalizeAnthropicBase('https://custom.api.com/v2')).toBe('https://custom.api.com');
  });

  it('returns unchanged when no /vN suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com')).toBe('https://api.anthropic.com');
  });

  it('handles trailing slash before /v1', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com//v1')).toBe('https://api.anthropic.com/');
  });

  it('handles empty string', () => {
    expect(normalizeAnthropicBase('')).toBe('');
  });
});
