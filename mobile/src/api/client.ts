import AsyncStorage from '@react-native-async-storage/async-storage';

// Settings keys
const KEYS = {
  apiBase: 'cfg_api_base',
  apiKey: 'cfg_api_key',
  model: 'cfg_model',
} as const;

// Defaults
const DEFAULTS = {
  apiBase: 'https://api.openai.com/v1',
  model: 'gpt-4o-mini',
};

// ── Settings ──────────────────────────────────────────────
export async function getSettings() {
  const [base, key, model] = await Promise.all([
    AsyncStorage.getItem(KEYS.apiBase),
    AsyncStorage.getItem(KEYS.apiKey),
    AsyncStorage.getItem(KEYS.model),
  ]);
  return {
    apiBase: base || DEFAULTS.apiBase,
    apiKey: key || '',
    model: model || DEFAULTS.model,
  };
}

export async function saveSettings(s: { apiBase?: string; apiKey?: string; model?: string }) {
  const ops: Promise<void>[] = [];
  if (s.apiBase !== undefined) ops.push(AsyncStorage.setItem(KEYS.apiBase, s.apiBase));
  if (s.apiKey !== undefined) ops.push(AsyncStorage.setItem(KEYS.apiKey, s.apiKey));
  if (s.model !== undefined) ops.push(AsyncStorage.setItem(KEYS.model, s.model));
  await Promise.all(ops);
}

// ── Chat ──────────────────────────────────────────────────
export interface ChatMessage {
  role: 'system' | 'user' | 'assistant';
  content: string;
}

export async function chat(
  messages: ChatMessage[],
  signal?: AbortSignal
): Promise<ReadableStream<Uint8Array>> {
  const { apiBase, apiKey, model } = await getSettings();
  if (!apiKey) throw new Error('请先在设置中填写 API Key');

  const res = await fetch(`${apiBase}/chat/completions`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${apiKey}`,
    },
    body: JSON.stringify({ model, messages, stream: true }),
    signal,
  });

  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new Error(`API ${res.status}: ${text}`);
  }
  if (!res.body) throw new Error('No response body');
  return res.body;
}

// ── SSE Parser ────────────────────────────────────────────
export interface StreamChunk {
  content?: string;
  done: boolean;
}

export async function readStream(
  stream: ReadableStream<Uint8Array>,
  onChunk: (c: StreamChunk) => void
): Promise<void> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      const lines = buf.split('\n');
      buf = lines.pop() || '';
      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;
        const data = line.slice(6).trim();
        if (data === '[DONE]') { onChunk({ done: true }); return; }
        try {
          const delta = JSON.parse(data).choices?.[0]?.delta;
          if (delta?.content) onChunk({ content: delta.content, done: false });
        } catch {}
      }
    }
  } finally {
    reader.releaseLock();
  }
}

// ── Health Check ──────────────────────────────────────────
export async function checkApi(): Promise<{ ok: boolean; error?: string }> {
  try {
    const { apiBase, apiKey, model } = await getSettings();
    if (!apiKey) return { ok: false, error: '未配置 API Key' };
    const res = await fetch(`${apiBase}/models`, {
      headers: { Authorization: `Bearer ${apiKey}` },
      signal: AbortSignal.timeout(8000),
    });
    return { ok: res.ok, error: res.ok ? undefined : `HTTP ${res.status}` };
  } catch (e: any) {
    return { ok: false, error: e.message };
  }
}
