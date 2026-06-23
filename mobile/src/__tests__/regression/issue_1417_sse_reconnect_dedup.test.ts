/**
 * Regression test for #1417: SSE reconnection duplicate content.
 *
 * When a stream delivers content chunks and then fails, the retry must NOT
 * re-fetch and send duplicate content. Instead, it should end the stream
 * gracefully.
 */

import { parseSSEStream, parseSSEStreamWithReconnect, type StreamChunk } from '../../api/sse';

// Helper: create a ReadableStream from chunks that may error mid-way
function createStreamFromChunks(chunks: Uint8Array[], errorAfter?: number): ReadableStream<Uint8Array> {
  let index = 0;
  return new ReadableStream({
    pull(ctrl) {
      if (errorAfter !== undefined && index >= errorAfter) {
        ctrl.error(new Error('Connection lost'));
        return;
      }
      if (index < chunks.length) {
        ctrl.enqueue(chunks[index++]);
      } else {
        ctrl.close();
      }
    },
  });
}

const encoder = new TextEncoder();
const encode = (s: string) => encoder.encode(s);

describe('parseSSEStreamWithReconnect dedup (#1417)', () => {
  const g = globalThis as typeof globalThis & { fetch: typeof fetch };
  let originalFetch: typeof fetch;

  beforeEach(() => {
    originalFetch = g.fetch;
  });

  afterEach(() => {
    g.fetch = originalFetch;
  });

  test('retries when no content delivered yet', async () => {
    let callCount = 0;
    const chunks: StreamChunk[] = [];

    g.fetch = jest.fn().mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        // First attempt: stream errors immediately (no content)
        return { ok: true, body: createStreamFromChunks([], 0) };
      }
      // Second attempt: success
      const sseData = 'data: {"choices":[{"delta":{"content":"hello"}}]}\n\ndata: [DONE]\n\n';
      return { ok: true, body: createStreamFromChunks([encode(sseData)]) };
    });

    await parseSSEStreamWithReconnect(
      'https://api.test.com/chat',
      { method: 'POST' },
      (chunk) => chunks.push(chunk),
      { maxRetries: 2, baseDelay: 1 },
    );

    const contentChunks = chunks.filter(c => c.content);
    expect(contentChunks).toHaveLength(1);
    expect(contentChunks[0].content).toBe('hello');
    expect(callCount).toBe(2);
  });

  test('does NOT retry when content already delivered', async () => {
    const chunks: StreamChunk[] = [];

    // Stream delivers partial content then errors
    const partialSSE = 'data: {"choices":[{"delta":{"content":"hello"}}]}\n\n';
    let callCount = 0;
    g.fetch = jest.fn().mockImplementation(async () => {
      callCount++;
      return { ok: true, body: createStreamFromChunks([encode(partialSSE)], 1) };
    });

    await parseSSEStreamWithReconnect(
      'https://api.test.com/chat',
      { method: 'POST' },
      (chunk) => chunks.push(chunk),
      { maxRetries: 3, baseDelay: 1 },
    );

    const contentChunks = chunks.filter(c => c.content);
    const doneChunks = chunks.filter(c => c.done);
    expect(contentChunks).toHaveLength(1);
    expect(contentChunks[0].content).toBe('hello');
    expect(doneChunks).toHaveLength(1);
    // fetch should only be called once (no retry)
    expect(callCount).toBe(1);
  });

  test('parseSSEStream handles normal stream correctly', async () => {
    const sseData = 'data: {"choices":[{"delta":{"content":"hi"}}]}\n\ndata: {"choices":[{"delta":{"content":" there"}}]}\n\ndata: [DONE]\n\n';
    const stream = createStreamFromChunks([encode(sseData)]);
    const chunks: StreamChunk[] = [];

    await parseSSEStream(stream, (chunk) => chunks.push(chunk));

    expect(chunks).toHaveLength(3);
    expect(chunks[0].content).toBe('hi');
    expect(chunks[1].content).toBe(' there');
    expect(chunks[2].done).toBe(true);
  });
});
