/**
 * Regression test for issue #1337:
 * client.ts pure function extraction + unit tests.
 *
 * Tests: toAnthropicContent, convertAnthropicEvent,
 *        wrapNonStreamingResponse, normalizeAnthropicBase
 */
import {
  toAnthropicContent,
  convertAnthropicEvent,
  wrapNonStreamingResponse,
  normalizeAnthropicBase,
} from '../../api/clientPure';

// ── toAnthropicContent ──────────────────────────────────

describe('toAnthropicContent', () => {
  it('passes through plain string unchanged', () => {
    expect(toAnthropicContent('hello world')).toBe('hello world');
  });

  it('preserves text content parts', () => {
    const input = [{ type: 'text' as const, text: 'hello' }];
    const result = toAnthropicContent(input);
    expect(result).toEqual([{ type: 'text', text: 'hello' }]);
  });

  it('converts base64 image URL to Anthropic image block', () => {
    const input = [
      { type: 'image_url' as const, image_url: { url: 'data:image/png;base64,abc123' } },
    ];
    const result = toAnthropicContent(input) as Record<string, unknown>[];
    expect(result[0]).toEqual({
      type: 'image',
      source: { type: 'base64', media_type: 'image/png', data: 'abc123' },
    });
  });

  it('converts non-base64 image URL to placeholder text', () => {
    const input = [
      { type: 'image_url' as const, image_url: { url: 'https://example.com/photo.jpg' } },
    ];
    const result = toAnthropicContent(input) as Record<string, unknown>[];
    expect(result[0]).toEqual({ type: 'text', text: '[image unavailable]' });
  });

  it('handles mixed text and image parts', () => {
    const input = [
      { type: 'text' as const, text: 'look at this:' },
      { type: 'image_url' as const, image_url: { url: 'data:image/jpeg;base64,/9j/abc' } },
    ];
    const result = toAnthropicContent(input) as Record<string, unknown>[];
    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({ type: 'text', text: 'look at this:' });
    expect(result[1]).toEqual({
      type: 'image',
      source: { type: 'base64', media_type: 'image/jpeg', data: '/9j/abc' },
    });
  });

  it('handles empty content parts array', () => {
    expect(toAnthropicContent([])).toEqual([]);
  });

  it('handles image with webp format', () => {
    const input = [{ type: 'image_url' as const, image_url: { url: 'data:image/webp;base64,UklGR' } }];
    const result = toAnthropicContent(input) as Record<string, unknown>[];
    expect((result[0] as any).source.media_type).toBe('image/webp');
  });
});

// ── convertAnthropicEvent ────────────────────────────────

describe('convertAnthropicEvent', () => {
  it('converts content_block_delta to OpenAI SSE format', () => {
    const data = JSON.stringify({
      type: 'content_block_delta',
      delta: { type: 'text_delta', text: 'hello' },
    });
    const result = convertAnthropicEvent('content_block_delta', data);
    expect(result).toBe(
      `data: ${JSON.stringify({ choices: [{ delta: { content: 'hello' } }] })}\n\n`
    );
  });

  it('converts message_stop to [DONE]', () => {
    const data = JSON.stringify({ type: 'message_stop' });
    expect(convertAnthropicEvent('message_stop', data)).toBe('data: [DONE]\n\n');
  });

  it('returns null for ignored event types', () => {
    const data = JSON.stringify({ type: 'message_start', message: {} });
    expect(convertAnthropicEvent('message_start', data)).toBeNull();
  });

  it('returns null for invalid JSON', () => {
    expect(convertAnthropicEvent('test', 'not json')).toBeNull();
  });

  it('returns null when delta has no text', () => {
    const data = JSON.stringify({
      type: 'content_block_delta',
      delta: { type: 'input_json_delta', partial_json: '{}' },
    });
    expect(convertAnthropicEvent('content_block_delta', data)).toBeNull();
  });

  it('returns null for content_block_delta without delta', () => {
    const data = JSON.stringify({ type: 'content_block_delta' });
    expect(convertAnthropicEvent('content_block_delta', data)).toBeNull();
  });

  it('accumulates multiple text deltas correctly', () => {
    const deltas = ['Hello', ' world', '!'];
    const results = deltas.map(text => {
      const data = JSON.stringify({
        type: 'content_block_delta',
        delta: { type: 'text_delta', text },
      });
      return convertAnthropicEvent('content_block_delta', data);
    });
    expect(results).toHaveLength(3);
    results.forEach(r => expect(r).toMatch(/^data: /));
  });
});

// ── wrapNonStreamingResponse ─────────────────────────────

describe('wrapNonStreamingResponse', () => {
  it('wraps a simple text response into SSE format', () => {
    const json = {
      choices: [{ message: { content: 'Hello!' }, finish_reason: 'stop' }],
    };
    const result = new TextDecoder().decode(wrapNonStreamingResponse(json));
    expect(result).toContain('data: ');
    expect(result).toContain('[DONE]');

    // Parse the first data line
    const lines = result.split('\n').filter(l => l.startsWith('data: ') && !l.includes('[DONE]'));
    const parsed = JSON.parse(lines[0].replace('data: ', ''));
    expect(parsed.choices[0].delta.content).toBe('Hello!');
    expect(parsed.choices[0].finish_reason).toBe('stop');
  });

  it('includes tool_calls in delta', () => {
    const toolCalls = [{ id: 'call_1', type: 'function', function: { name: 'test' } }];
    const json = {
      choices: [{ message: { tool_calls: toolCalls }, finish_reason: 'tool_calls' }],
    };
    const result = new TextDecoder().decode(wrapNonStreamingResponse(json));
    const lines = result.split('\n').filter(l => l.startsWith('data: ') && !l.includes('[DONE]'));
    const parsed = JSON.parse(lines[0].replace('data: ', ''));
    expect(parsed.choices[0].delta.tool_calls).toEqual(toolCalls);
  });

  it('handles missing message gracefully', () => {
    const json = { choices: [{}] };
    const result = new TextDecoder().decode(wrapNonStreamingResponse(json));
    expect(result).toContain('[DONE]');
  });

  it('handles empty choices array', () => {
    const json = { choices: [] };
    const result = new TextDecoder().decode(wrapNonStreamingResponse(json));
    expect(result).toContain('[DONE]');
  });

  it('handles undefined choices', () => {
    const json = {};
    const result = new TextDecoder().decode(wrapNonStreamingResponse(json));
    expect(result).toContain('[DONE]');
  });

  it('includes function_call in delta when present', () => {
    const functionCall = { name: 'test', arguments: '{}' };
    const json = {
      choices: [{ message: { function_call: functionCall }, finish_reason: 'stop' }],
    };
    const result = new TextDecoder().decode(wrapNonStreamingResponse(json));
    const lines = result.split('\n').filter(l => l.startsWith('data: ') && !l.includes('[DONE]'));
    const parsed = JSON.parse(lines[0].replace('data: ', ''));
    expect(parsed.choices[0].delta.function_call).toEqual(functionCall);
  });
});

// ── normalizeAnthropicBase ───────────────────────────────

describe('normalizeAnthropicBase', () => {
  it('strips /v1 suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v1')).toBe('https://api.anthropic.com');
  });

  it('strips /v2 suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v2')).toBe('https://api.anthropic.com');
  });

  it('strips /v1-beta suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v1-beta')).toBe('https://api.anthropic.com');
  });

  it('returns URL unchanged when no version suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com')).toBe('https://api.anthropic.com');
  });

  it('handles empty string', () => {
    expect(normalizeAnthropicBase('')).toBe('');
  });

  it('preserves path before version', () => {
    expect(normalizeAnthropicBase('https://proxy.example.com/anthropic/v1')).toBe('https://proxy.example.com/anthropic');
  });

  it('strips trailing path after version', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v1/messages')).toBe('https://api.anthropic.com');
  });
});
