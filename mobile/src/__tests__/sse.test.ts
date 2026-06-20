import { parseSSEStream, parseSSEStreamWithReconnect, StreamChunk } from '../api/sse';

function makeStream(chunks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  let i = 0;
  return new ReadableStream({
    pull(controller) {
      if (i < chunks.length) {
        controller.enqueue(encoder.encode(chunks[i++]));
      } else {
        controller.close();
      }
    },
  });
}

describe('parseSSEStream', () => {
  it('parses a simple data event and signals done on stream end', async () => {
    const stream = makeStream(['data: {"choices":[{"delta":{"content":"hello"}}]}\n\n']);
    const chunks: StreamChunk[] = [];
    await parseSSEStream(stream, c => chunks.push(c));
    expect(chunks[0]).toEqual({ content: 'hello', tool_calls: undefined, done: false });
    expect(chunks[chunks.length - 1]).toEqual({ done: true });
  });

  it('handles [DONE] signal', async () => {
    const stream = makeStream(['data: [DONE]\n\n']);
    const chunks: StreamChunk[] = [];
    await parseSSEStream(stream, c => chunks.push(c));
    expect(chunks).toEqual([{ done: true }]);
  });

  it('handles multi-line data (concatenated with newlines)', async () => {
    const stream = makeStream(['data: {"choices":[{"delta":{"content":"line1"}}]}\ndata: extra\n\n']);
    const chunks: StreamChunk[] = [];
    await parseSSEStream(stream, c => chunks.push(c));
    expect(chunks.length).toBeGreaterThanOrEqual(1);
  });

  it('handles chunked stream (data split across reads)', async () => {
    const stream = makeStream([
      'data: {"choices":[{"del',
      'ta":{"content":"hello"}}]}\n\n',
    ]);
    const chunks: StreamChunk[] = [];
    await parseSSEStream(stream, c => chunks.push(c));
    expect(chunks[0]).toEqual({ content: 'hello', tool_calls: undefined, done: false });
  });

  it('handles empty stream', async () => {
    const stream = makeStream([]);
    const chunks: StreamChunk[] = [];
    await parseSSEStream(stream, c => chunks.push(c));
    expect(chunks).toEqual([{ done: true }]);
  });

  it('ignores malformed JSON gracefully', async () => {
    const consoleSpy = jest.spyOn(console, 'warn').mockImplementation();
    const stream = makeStream(['data: not-json\n\ndata: [DONE]\n\n']);
    const chunks: StreamChunk[] = [];
    await parseSSEStream(stream, c => chunks.push(c));
    expect(chunks).toEqual([{ done: true }]);
    expect(consoleSpy).toHaveBeenCalled();
    consoleSpy.mockRestore();
  });

  it('parses tool_calls in delta', async () => {
    const toolCall = { id: 'call_1', name: 'search', arguments: '{"q":"test"}' };
    const data = JSON.stringify({ choices: [{ delta: { tool_calls: [toolCall] } }] });
    const stream = makeStream([`data: ${data}\n\n`, 'data: [DONE]\n\n']);
    const chunks: StreamChunk[] = [];
    await parseSSEStream(stream, c => chunks.push(c));
    expect(chunks[0].tool_calls).toEqual([toolCall]);
    expect(chunks[0].done).toBe(false);
  });

  it('handles multiple data events in sequence', async () => {
    const stream = makeStream([
      'data: {"choices":[{"delta":{"content":"a"}}]}\n\n',
      'data: {"choices":[{"delta":{"content":"b"}}]}\n\n',
      'data: [DONE]\n\n',
    ]);
    const chunks: StreamChunk[] = [];
    await parseSSEStream(stream, c => chunks.push(c));
    expect(chunks[0].content).toBe('a');
    expect(chunks[1].content).toBe('b');
    expect(chunks[2]).toEqual({ done: true });
  });
});

describe('parseSSEStreamWithReconnect', () => {
  beforeEach(() => {
    jest.restoreAllMocks();
  });

  it('succeeds on first attempt', async () => {
    const body = makeStream(['data: {"choices":[{"delta":{"content":"ok"}}]}\n\n', 'data: [DONE]\n\n']);
    const mockFetch = jest.fn().mockResolvedValue({ ok: true, body });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const chunks: StreamChunk[] = [];
    await parseSSEStreamWithReconnect('http://test.com', {}, c => chunks.push(c));
    expect(chunks.some(c => c.content === 'ok')).toBe(true);
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('retries on 500 error', async () => {
    const failRes = { ok: false, status: 500, text: jest.fn().mockResolvedValue('error') };
    const body = makeStream(['data: {"choices":[{"delta":{"content":"ok"}}]}\n\n', 'data: [DONE]\n\n']);
    const okRes = { ok: true, body };
    const mockFetch = jest.fn().mockResolvedValueOnce(failRes).mockResolvedValueOnce(okRes);
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const chunks: StreamChunk[] = [];
    await parseSSEStreamWithReconnect('http://test.com', {}, c => chunks.push(c), { baseDelay: 1 });
    expect(mockFetch).toHaveBeenCalledTimes(2);
    expect(chunks.some(c => c.content === 'ok')).toBe(true);
  });

  it('does NOT retry on 400 error (client error)', async () => {
    const failRes = { ok: false, status: 400, text: jest.fn().mockResolvedValue('bad request') };
    const mockFetch = jest.fn().mockResolvedValueOnce(failRes);
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    await expect(
      parseSSEStreamWithReconnect('http://test.com', {}, () => {}, { baseDelay: 1 })
    ).rejects.toThrow('API 400');
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('signals done only once across retries', async () => {
    const encoder = new TextEncoder();
    const partialStream = new ReadableStream({
      pull(controller) {
        controller.enqueue(encoder.encode('data: {"choices":[{"delta":{"content":"p"}}]}\n\n'));
        controller.error(new Error('network'));
      },
    });
    const fullStream = makeStream([
      'data: {"choices":[{"delta":{"content":"f"}}]}\n\n',
      'data: [DONE]\n\n',
    ]);

    const mockFetch = jest.fn()
      .mockResolvedValueOnce({ ok: true, body: partialStream })
      .mockResolvedValueOnce({ ok: true, body: fullStream });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const chunks: StreamChunk[] = [];
    await parseSSEStreamWithReconnect('http://test.com', {}, c => chunks.push(c), { baseDelay: 1 });
    const doneChunks = chunks.filter(c => c.done);
    expect(doneChunks.length).toBeLessThanOrEqual(1);
  });

  it('aborts immediately if signal already aborted', async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(
      parseSSEStreamWithReconnect('http://test.com', {}, () => {}, { signal: controller.signal })
    ).rejects.toThrow();
  });
});
