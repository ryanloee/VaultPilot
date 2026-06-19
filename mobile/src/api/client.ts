import AsyncStorage from '@react-native-async-storage/async-storage';

// Re-export unified SSE types from sse.ts (single implementation)
export type { StreamChunk } from './sse';
export { parseSSEStream } from './sse';

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

/** Chat request timeout in ms (2 minutes) */
const CHAT_TIMEOUT_MS = 120_000;

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

// ── Error Sanitization ───────────────────────────────────
const STATUS_MESSAGES: Record<number, string> = {
  400: '请求格式错误',
  401: 'API Key 无效或已过期',
  403: '访问被拒绝，请检查权限',
  404: '请求的资源不存在',
  408: '请求超时，请稍后重试',
  429: '请求过于频繁，请稍后重试',
  500: '服务器内部错误',
  502: '服务暂时不可用',
  503: '服务暂时不可用，请稍后重试',
  504: '服务响应超时，请稍后重试',
};

function sanitizeApiError(status: number, rawBody: string): string {
  // Log full error in development for debugging
  if (__DEV__) {
    console.warn(`[API Error ${status}]`, rawBody);
  }
  const friendly = STATUS_MESSAGES[status];
  if (friendly) return `API 错误 (${status}): ${friendly}`;
  if (status >= 500) return `API 错误 (${status}): 服务端异常，请稍后重试`;
  if (status >= 400) return `API 错误 (${status}): 请求有误，请检查参数`;
  return `API 错误 (${status})`;
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

  // Combine user signal with timeout
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), CHAT_TIMEOUT_MS);
  if (signal) {
    signal.addEventListener('abort', () => controller.abort(signal.reason), { once: true });
  }

  try {
    const res = await fetch(`${apiBase}/chat/completions`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({ model, messages, stream: true }),
      signal: controller.signal,
    });

    if (!res.ok) {
      const text = await res.text().catch(() => '');
      throw new Error(sanitizeApiError(res.status, text));
    }
    if (!res.body) throw new Error('No response body');
    return res.body;
  } catch (e: any) {
    clearTimeout(timeout);
    if (e.name === 'AbortError' && !signal?.aborted) {
      throw new Error('请求超时（2 分钟），请检查网络或服务端状态');
    }
    throw e;
  } finally {
    clearTimeout(timeout);
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
