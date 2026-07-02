import { isRetryable } from "./clientUtils";

export interface StreamChunk {
  content?: string;
  tool_calls?: Array<{ id: string; name: string; arguments: string }>;
  done: boolean;
}

export interface ParseSSEOptions {
  /** Maximum reconnection attempts on network error (default: 3) */
  maxRetries?: number;
  /** Base delay for exponential backoff in ms (default: 1000) */
  baseDelay?: number;
  /** AbortSignal for cancellation */
  signal?: AbortSignal;
  /** Transform the raw response body before SSE parsing (e.g. Anthropic→OpenAI wrapper) */
  transformBody?: (
    body: ReadableStream<Uint8Array>,
  ) => ReadableStream<Uint8Array>;
  /** Called when a chunk fails JSON parse. Allows callers to detect data loss. */
  onParseError?: (data: string, error: unknown) => void;
  /** Reject the stream after this many consecutive parse errors (default: 3) */
  maxParseErrors?: number;
}

/**
 * Parse an SSE stream with automatic reconnection support.
 * Unifies the previous duplicate implementations in client.ts and sse.ts.
 */
export function parseSSEStream(
  stream: ReadableStream<Uint8Array>,
  onChunk: (chunk: StreamChunk) => void,
  options?: ParseSSEOptions,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const reader = stream.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let doneReceived = false;
    const dataParts: string[] = [];
    let parseErrorCount = 0;
    let rejected = false;
    const maxParseErrors = options?.maxParseErrors ?? 3;

    // Propagate AbortSignal to reader so reader.read() unblocks on abort
    const onAbort = () => {
      reader.cancel("abort").catch(() => {});
    };
    options?.signal?.addEventListener("abort", onAbort, { once: true });

    function processBuffer() {
      const lines = buffer.split(/\r\n|\r|\n/);
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (line.startsWith("data:")) {
          dataParts.push(line.slice(5).trimStart());
          continue;
        }
        // Empty line = end of event; process accumulated data parts
        if (line.trim() === "" && dataParts.length > 0) {
          const data = dataParts.join("\n");
          dataParts.length = 0;
          if (data === "[DONE]") {
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
            parseErrorCount = 0; // Reset on successful parse
          } catch (e) {
            parseErrorCount++;
            console.warn("[SSE] Failed to parse chunk:", data.slice(0, 100), e);
            options?.onParseError?.(data, e);
            if (parseErrorCount >= maxParseErrors) {
              rejected = true;
              reader.cancel("max parse errors").catch(() => {});
              reject(
                new Error(
                  `[SSE] ${parseErrorCount} consecutive parse errors — aborting stream`,
                ),
              );
              return;
            }
          }
        }
      }
    }

    (async () => {
      try {
        while (true) {
          const { value, done } = await reader.read();
          if (done) {
            buffer += decoder.decode();
            // FIX(#2342): reader.cancel('abort') causes reader.read() to resolve
            // with { done: true } instead of rejecting. Check for external abort
            // here and propagate AbortError instead of emitting a spurious done.
            if (options?.signal?.aborted) {
              rejected = true;
              reader.releaseLock();
              reject(new DOMException("Aborted", "AbortError"));
              return;
            }
            break;
          }
          buffer += decoder.decode(value, { stream: true });
          processBuffer();
          if (doneReceived) break;
        }
        if (options?.signal?.aborted && !rejected) {
          rejected = true;
          reader.releaseLock();
          reject(new DOMException("Aborted", "AbortError"));
          return;
        }
        if (!doneReceived && buffer.trim().length > 0) {
          buffer += "\n\n"; // Ensure last data line is processed even without trailing newline
          processBuffer();
        }
        if (!doneReceived && !rejected) {
          console.warn("[SSE] Stream ended without [DONE] signal");
          onChunk({ done: true });
        }
        reader.releaseLock();
        resolve();
      } catch (err) {
        await reader
          .cancel()
          .catch((e) => console.warn("[SSE] reader.cancel failed:", e));
        reader.releaseLock();
        if (options?.signal?.aborted) {
          reject(new DOMException("Aborted", "AbortError"));
        } else {
          reject(err);
        }
      } finally {
        options?.signal?.removeEventListener("abort", onAbort);
      }
    })();
  });
}

/**
 * Parse an SSE stream with reconnection on network errors.
 * Creates a new fetch for each retry attempt.
 */
export async function parseSSEStreamWithReconnect(
  url: string,
  fetchInit: RequestInit,
  onChunk: (chunk: StreamChunk) => void,
  options?: ParseSSEOptions,
): Promise<void> {
  const maxRetries = options?.maxRetries ?? 3;
  const baseDelay = options?.baseDelay ?? 1000;

  // Deduplicate done:true across retries — prevent caller from receiving
  // multiple stream-end signals when a retry follows a partial completion.
  let doneSignaled = false;
  // Track whether any content was delivered — if so, don't retry to avoid
  // sending duplicate content (the server re-sends from the beginning).
  let contentDelivered = false;
  const wrappedOnChunk = (chunk: StreamChunk) => {
    if (chunk.done) {
      if (doneSignaled) return;
      doneSignaled = true;
    }
    if (!chunk.done && (chunk.content || chunk.tool_calls)) {
      contentDelivered = true;
    }
    onChunk(chunk);
  };

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    if (options?.signal?.aborted) {
      throw new DOMException("Aborted", "AbortError");
    }

    try {
      const res = await fetch(url, { ...fetchInit, signal: options?.signal });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        const err = new Error(`API ${res.status}: ${text}`) as Error & {
          status: number;
        };
        err.status = res.status;
        throw err;
      }
      if (!res.body) throw new Error("No response body");

      const streamBody = options?.transformBody
        ? options.transformBody(res.body)
        : res.body;
      await parseSSEStream(streamBody, wrappedOnChunk, options);
      return; // Success
    } catch (err: unknown) {
      if (err instanceof DOMException && err.name === "AbortError") throw err;

      // Don't retry client errors (4xx) — they won't succeed on retry
      const status = (err as { status?: number }).status;
      if (
        status !== undefined &&
        status >= 400 &&
        status < 500 &&
        !isRetryable(status)
      )
        throw err;

      // If content was already delivered, retrying would send duplicate text.
      // End the stream gracefully instead of retrying.
      if (contentDelivered) {
        console.warn(
          "[SSE] Connection lost after content delivery — ending stream (no retry to avoid duplicates)",
        );
        if (!doneSignaled) {
          doneSignaled = true;
          onChunk({ done: true });
        }
        return;
      }

      if (attempt < maxRetries) {
        const delay = baseDelay * Math.pow(2, attempt);
        console.warn(
          `[SSE] Connection lost (attempt ${attempt + 1}/${maxRetries}), retrying in ${delay}ms:`,
          err instanceof Error ? err.message : String(err),
        );
        await new Promise<void>((resolve, reject) => {
          const onAbort = () => {
            clearTimeout(timer);
            reject(new DOMException("Aborted", "AbortError"));
          };
          const timer = setTimeout(() => {
            options?.signal?.removeEventListener("abort", onAbort);
            resolve();
          }, delay);
          options?.signal?.addEventListener("abort", onAbort, { once: true });
        });
      } else {
        throw err;
      }
    }
  }
}
