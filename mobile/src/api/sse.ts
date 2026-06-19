export interface StreamChunk {
  content?: string;
  tool_calls?: Array<{ id: string; name: string; arguments: string }>;
  done: boolean;
}

export function parseSSEStream(
  stream: ReadableStream<Uint8Array>,
  onChunk: (chunk: StreamChunk) => void
): Promise<void> {
  return new Promise((resolve, reject) => {
    const reader = stream.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    function processBuffer() {
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';

      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;
        const data = line.slice(6);
        if (data === '[DONE]') {
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
        } catch {
          // ignore parse errors in stream
        }
      }
    }

    (async () => {
      try {
        while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          processBuffer();
        }
        resolve();
      } catch (err) {
        reject(err);
      }
    })();
  });
}
