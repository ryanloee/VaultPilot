import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

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

// ── Secure helpers ───────────────────────────────────────
async function getApiKey(): Promise<string> {
  try {
    return (await SecureStore.getItemAsync(KEYS.apiKey)) || '';
  } catch (e: any) {
    console.warn('[SecureStore] Failed to read API key:', e.message);
    return '';
  }
}

async function setApiKey(value: string): Promise<void> {
  try {
    await SecureStore.setItemAsync(KEYS.apiKey, value);
  } catch (e: any) {
    console.warn('[SecureStore] Failed to write API key:', e.message);
    throw new Error('无法安全存储 API Key，请检查设备安全设置');
  }
}

/** Chat request timeout in ms (2 minutes) */
const CHAT_TIMEOUT_MS = 120_000;

// ── Settings ──────────────────────────────────────────────
let _settingsCache: { apiBase: string; apiKey: string; model: string } | null = null;

export function invalidateSettingsCache() {
  _settingsCache = null;
}

export async function getSettings() {
  if (_settingsCache) return _settingsCache;
  const [base, key, model] = await Promise.all([
    AsyncStorage.getItem(KEYS.apiBase),
    getApiKey(),
    AsyncStorage.getItem(KEYS.model),
  ]);
  _settingsCache = {
    apiBase: base || DEFAULTS.apiBase,
    apiKey: key,
    model: model || DEFAULTS.model,
  };
  return _settingsCache;
}

export async function saveSettings(s: { apiBase?: string; apiKey?: string; model?: string }) {
  const ops: Promise<void>[] = [];
  if (s.apiBase !== undefined) ops.push(AsyncStorage.setItem(KEYS.apiBase, s.apiBase));
  if (s.apiKey !== undefined) ops.push(setApiKey(s.apiKey));
  if (s.model !== undefined) ops.push(AsyncStorage.setItem(KEYS.model, s.model));
  await Promise.all(ops);
  invalidateSettingsCache();
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

// ── API Base Normalization ────────────────────────────────
/** Ensure apiBase ends with /v1 for consistent path construction.
 *  Preserves existing versioned paths (e.g. /v2) and validates input. */
function normalizeApiBase(raw: string): string {
  const trimmed = raw.trim().replace(/\/+$/, '');
  if (!trimmed) return DEFAULTS.apiBase;
  if (/\/v\d+($|\/)/.test(trimmed)) return trimmed;
  return trimmed + '/v1';
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
  if (signal?.aborted) {
    throw new DOMException('The operation was aborted.', 'AbortError');
  }

  const { apiBase, apiKey, model } = await getSettings();
  if (!apiKey) throw new Error('请先在设置中填写 API Key');

  const base = normalizeApiBase(apiBase);

  // Combine user signal with timeout
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), CHAT_TIMEOUT_MS);
  const sig = signal;
  const onSignalAbort = sig ? () => controller.abort(sig.reason) : undefined;
  if (sig && onSignalAbort) {
    sig.addEventListener('abort', onSignalAbort, { once: true });
  }

  try {
    // Try streaming first; fall back to non-streaming on 400
    // (VaultPilot HTTP bridge doesn't support stream=true yet)
    let res = await fetch(`${base}/chat/completions`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({ model, messages, stream: true }),
      signal: controller.signal,
    });

    if (res.status === 400) {
      await res.body?.cancel(); // Release connection back to pool
      res = await fetch(`${base}/chat/completions`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${apiKey}`,
        },
        body: JSON.stringify({ model, messages, stream: false }),
        signal: controller.signal,
      });
    }

    if (!res.ok) {
      const text = await res.text().catch(() => '');
      throw new Error(sanitizeApiError(res.status, text));
    }

    // If non-streaming fallback, wrap JSON response as a single-chunk SSE stream
    if (!res.headers.get('content-type')?.includes('text/event-stream')) {
      const json = await res.json();
      const message = json.choices?.[0]?.message ?? {};
      const delta: Record<string, unknown> = {};
      if (message.content) delta.content = message.content;
      if (message.tool_calls) delta.tool_calls = message.tool_calls;
      if (message.function_call) delta.function_call = message.function_call;
      const finish_reason = json.choices?.[0]?.finish_reason;
      const encoded = new TextEncoder().encode(`data: ${JSON.stringify({ choices: [{ delta, finish_reason }] })}\n\ndata: [DONE]\n\n`);
      return new ReadableStream({
        start(controller) { controller.enqueue(encoded); controller.close(); },
      });
    }

    if (!res.body) throw new Error('No response body');
    // Wrap body so the timeout still applies during stream reading
    const body = res.body;
    const timeoutController = controller;
    return new ReadableStream<Uint8Array>({
      start(ctrl) {
        const reader = body.getReader();
        const onTimeout = () => { reader.cancel('timeout'); ctrl.error(new DOMException('Timeout', 'AbortError')); };
        timeoutController.signal.addEventListener('abort', onTimeout, { once: true });
        (async () => {
          try {
            while (true) {
              const { value, done } = await reader.read();
              if (done) break;
              ctrl.enqueue(value);
            }
            ctrl.close();
          } catch (e) { ctrl.error(e); }
          finally {
            clearTimeout(timeout);
            timeoutController.signal.removeEventListener('abort', onTimeout);
          }
        })();
      },
    });
  } catch (e: any) {
    clearTimeout(timeout);
    if (e.name === 'AbortError' && !signal?.aborted) {
      throw new Error('请求超时（2 分钟），请检查网络或服务端状态');
    }
    throw e;
  } finally {
    clearTimeout(timeout);
    if (onSignalAbort) sig?.removeEventListener('abort', onSignalAbort);
  }
}

// ── Health Check ──────────────────────────────────────────
export async function checkApi(params?: { apiBase?: string; apiKey?: string }): Promise<{ ok: boolean; error?: string }> {
  try {
    const settings = params ?? await getSettings();
    const { apiKey } = settings;
    const apiBase = settings.apiBase ?? '';
    if (!apiKey) return { ok: false, error: '未配置 API Key' };
    const res = await fetch(`${normalizeApiBase(apiBase)}/models`, {
      headers: { Authorization: `Bearer ${apiKey}` },
      signal: AbortSignal.timeout(8000),
    });
    return { ok: res.ok, error: res.ok ? undefined : `HTTP ${res.status}` };
  } catch (e: any) {
    return { ok: false, error: e.message };
  }
}
