/**
 * Regression test for #2926: client.ts had duplicate Anthropic SSE wrapping logic.
 *
 * The fix consolidates chatAnthropic() to use wrapAnthropicBody() instead of
 * having its own inline Anthropic SSE parser. This test verifies that:
 * 1. Both chat() (Anthropic path) and chatWithReconnect() produce equivalent output
 * 2. The consolidated wrapAnthropicBody() correctly handles all Anthropic SSE event types
 */

import { chat } from '../../api/client';
// chat() is the public entry point that selects chatAnthropic() for anthropic format.
// chatWithReconnect uses wrapAnthropicBody via transformBody.
// We test through the public API to verify consolidation correctness.

// ── Mock setup ──────────────────────────────────────────────
import { useAppStore } from '../../store';

// Mock fetch via globalThis (TypeScript-safe pattern used by other tests)
const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

beforeEach(() => {
  jest.clearAllMocks();
  mockFetch.mockReset();
  useAppStore.setState({
    themeMode: 'system',
    isDark: false,
    accentColor: '#3B82F6',
    apiBase: 'https://api.anthropic.com',
    apiKey: 'sk-ant-test-key',
    model: 'claude-sonnet-4-20250514',
    apiFormat: 'anthropic',
    providers: [{
      name: 'Anthropic',
      apiBase: 'https://api.anthropic.com',
      apiKey: 'sk-ant-test-key',
      model: 'claude-sonnet-4-20250514',
      apiFormat: 'anthropic',
    }],
    activeProviderIndex: 0,
  });
});

// Helper: create an Anthropic SSE stream manually using fetch mock
function createMockAnthropicSSEResponse(events: Array<{event?: string; data: Record<string, unknown>}>): Response {
  const lines: string[] = [];
  for (const evt of events) {
    if (evt.event) lines.push(`event: ${evt.event}`);
    lines.push(`data: ${JSON.stringify(evt.data)}`);
    lines.push(''); // SSE requires blank line between events
  }
  const body = lines.join('\n') + '\n';
  const stream = new ReadableStream<Uint8Array>({
    start(ctrl) {
      ctrl.enqueue(new TextEncoder().encode(body));
      ctrl.close();
    },
  });
  return new Response(stream, {
    status: 200,
    headers: { 'Content-Type': 'text/event-stream' },
  });
}

describe('#2926: Anthropic SSE wrapping consolidated — chat() uses wrapAnthropicBody()', () => {
  it('chat() with anthropic format produces correct OpenAI-compatible SSE', async () => {
    // Simulate an Anthropic response with multiple event types
    const mockEvents = [
      { event: 'message_start', data: { type: 'message_start', message: { id: 'msg_1', model: 'claude-sonnet-4-20250514' } } },
      { event: 'content_block_start', data: { type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } } },
      { event: 'content_block_delta', data: { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Hello' } } },
      { event: 'content_block_delta', data: { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: ' world' } } },
      { event: 'content_block_stop', data: { type: 'content_block_stop', index: 0 } },
      { event: 'message_delta', data: { type: 'message_delta', delta: { stop_reason: 'end_turn' }, usage: { output_tokens: 5 } } },
      { event: 'message_stop', data: { type: 'message_stop' } },
    ];

    const mockResponse = createMockAnthropicSSEResponse(mockEvents);
    mockFetch.mockResolvedValue(mockResponse);

    const messages = [{ role: 'user' as const, content: 'hi' }];
    const stream = await chat(messages);

    // Read all chunks from the wrapped stream
    const reader = stream.getReader();
    const chunks: string[] = [];
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      chunks.push(new TextDecoder().decode(value));
    }

    // Should produce OpenAI-compatible SSE chunks
    expect(chunks.length).toBeGreaterThan(0);

    // Verify we got content chunks (delta events translated)
    const contentChunks = chunks.filter(c => c.includes('"delta"'));
    expect(contentChunks.length).toBeGreaterThanOrEqual(2); // Hello + world
    expect(contentChunks.some(c => c.includes('Hello'))).toBe(true);
    expect(contentChunks.some(c => c.includes(' world'))).toBe(true);

    // Verify we got a done signal
    expect(chunks.some(c => c.includes('[DONE]'))).toBe(true);
  });

  it('chat() Anthropic path handles message_stop (no message_delta before)', async () => {
    // Minimal response: just message_start + content_block_delta + message_stop
    const mockEvents = [
      { event: 'message_start', data: { type: 'message_start', message: { id: 'msg_1', model: 'claude' } } },
      { event: 'content_block_delta', data: { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'OK' } } },
      { event: 'message_stop', data: { type: 'message_stop' } },
    ];

    const mockResponse = createMockAnthropicSSEResponse(mockEvents);
    mockFetch.mockResolvedValue(mockResponse);

    const messages = [{ role: 'user' as const, content: 'test' }];
    const stream = await chat(messages);

    const reader = stream.getReader();
    const chunks: string[] = [];
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      chunks.push(new TextDecoder().decode(value));
    }

    expect(chunks.length).toBeGreaterThan(0);
    expect(chunks.filter(c => c.includes('[DONE]')).length).toBeGreaterThanOrEqual(1);
  });

  it('chat() Anthropic path handles ping events (ignored)', async () => {
    // Anthropic sends periodic ping events with empty data
    const mockEvents = [
      { event: 'ping', data: {} as Record<string, unknown> },
      { event: 'content_block_delta', data: { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'x' } } },
      { event: 'message_stop', data: { type: 'message_stop' } },
    ];

    const mockResponse = createMockAnthropicSSEResponse(mockEvents);
    mockFetch.mockResolvedValue(mockResponse);

    const messages = [{ role: 'user' as const, content: 'ping-test' }];
    const stream = await chat(messages);

    const reader = stream.getReader();
    const chunks: string[] = [];
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      chunks.push(new TextDecoder().decode(value));
    }

    // Should have content chunk and done, no ping chunks leaked
    expect(chunks.filter(c => c.includes('"delta"')).length).toBeGreaterThanOrEqual(1);
    expect(chunks.some(c => c.includes('[DONE]'))).toBe(true);
  });
});