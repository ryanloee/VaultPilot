export interface StreamChunk {
  content?: string;
  tool_calls?: Array<{ id: string; name: string; arguments: string }>;
  done: boolean;
}

interface ParseSSEOptions {
  /** Maximum reconnection attempts on network error (default: 3) */
  maxRetries?: number;
  /** Base delay for exponential backoff in ms (default: 1000) */
  baseDelay?: number;
  /** AbortSignal for cancellation */
  signal?: AbortSignal;
}

/**
 * Parse an SSE stream with automatic reconnection support.
 * Unifies the previous duplicate implementations in client.ts and sse.ts.
 */
export function parseSSEStream(
  stream: ReadableStream<Uint8Array>,
  onChunk: (chunk: StreamChunk) => void,
  options?: ParseSSEOptions
): Promise<void> {
  return new Promise((resolve, reject) => {
    const reader = stream.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let doneReceived = false;

    function processBuffer() {
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';

      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;
        const data = line.slice(6).trim();
        if (data === '[DONE]') {
          doneReceived = true;
          onChunk({ done: true });
          return;
        }
        try {
          const parsed = JSON.parse(data);
          const delta = parsed.choices?.[0]?.delta;
          if (delta) {
            onChunk({
              content: delta.content,
              tool_calls: delta.tool_calls,
              done: false,
            });
          }
        } catch (e) {
          console.warn('[SSE] Failed to parse chunk:', data.slice(0, 100), e);
        }
      }
    }

    (async () => {
      try {
        while (true) {
          if (options?.signal?.aborted) {
            reader.releaseLock();
            reject(new DOMException('Aborted', 'AbortError'));
            return;
          }
          const { value, done } = await reader.read();
          if (done) {
            // Flush any remaining bytes in the TextDecoder (multi-byte tail)
            buffer += decoder.decode();
            break;
          }
          buffer += decoder.decode(value, { stream: true });
          processBuffer();
          if (doneReceived) break;
        }
        // Process any remaining data left in the buffer after the stream ends.
        // This handles the case where the last SSE message doesn't end with \n.
        if (!doneReceived && buffer.trim().length > 0) {
          processBuffer();
        }
        // Stream ended without [DONE] — emit done callback so UI can recover
        if (!doneReceived) {
          console.warn('[SSE] Stream ended without [DONE] signal');
          onChunk({ done: true });
        }
        reader.releaseLock();
        resolve();
      } catch (err) {
        reader.releaseLock();
        reject(err);
      }
    })();
  });
}

/**
 * Backward-compatible alias for parseSSEStream.
 * @deprecated Use parseSSEStream directly.
 */
export const readStream_compat = parseSSEStream;

/**
 * Parse an SSE stream with reconnection on network errors.
 * Creates a new fetch for each retry attempt.
 */
export async function parseSSEStreamWithReconnect(
  url: string,
  fetchInit: RequestInit,
  onChunk: (chunk: StreamChunk) => void,
  options?: ParseSSEOptions
): Promise<void> {
  const maxRetries = options?.maxRetries ?? 3;
  const baseDelay = options?.baseDelay ?? 1000;

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    if (options?.signal?.aborted) {
      throw new DOMException('Aborted', 'AbortError');
    }

    try {
      const res = await fetch(url, { ...fetchInit, signal: options?.signal });
      if (!res.ok) {
        const text = await res.text().catch(() => '');
        throw new Error(`API ${res.status}: ${text}`);
      }
      if (!res.body) throw new Error('No response body');

      await parseSSEStream(res.body, onChunk, options);
      return; // Success
    } catch (err: any) {
      if (err.name === 'AbortError') throw err;

      if (attempt < maxRetries) {
        const delay = baseDelay * Math.pow(2, attempt);
        console.warn(`[SSE] Connection lost (attempt ${attempt + 1}/${maxRetries}), retrying in ${delay}ms:`, err.message);
        await new Promise(r => setTimeout(r, delay));
      } else {
        throw err;
      }
    }
  }
}
