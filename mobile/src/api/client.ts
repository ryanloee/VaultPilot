import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

import { ApiFormat } from '../store';

// Re-export unified SSE types from sse.ts (single implementation)
export type { StreamChunk } from './sse';
export { parseSSEStream } from './sse';
import { parseSSEStreamWithReconnect } from './sse';

// Settings keys
const KEYS = {
  apiBase: 'cfg_api_base',
  apiKey: 'cfg_api_key',
  model: 'cfg_model',
  apiFormat: 'cfg_api_format',
} as const;

// Defaults
const DEFAULTS = {
  apiBase: 'https://opencode.ai/zen/v1',
  model: 'deepseek-v4-flash-free',
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

/** Max retries for transient network/server errors */
const MAX_RETRIES = 2;
const RETRY_BASE_MS = 1000;

function isRetryable(status: number): boolean {
  return status === 429 || status === 502 || status === 503 || status === 504;
}

// ── Settings ──────────────────────────────────────────────
let _settingsCache: { apiBase: string; apiKey: string; model: string; apiFormat: ApiFormat } | null = null;

export function invalidateSettingsCache() {
  _settingsCache = null;
}

export async function getSettings() {
  if (_settingsCache) return _settingsCache;
  const [base, key, model, fmt] = await Promise.all([
    AsyncStorage.getItem(KEYS.apiBase),
    getApiKey(),
    AsyncStorage.getItem(KEYS.model),
    AsyncStorage.getItem(KEYS.apiFormat),
  ]);
  _settingsCache = {
    apiBase: base || DEFAULTS.apiBase,
    apiKey: key,
    model: model || DEFAULTS.model,
    apiFormat: (fmt as ApiFormat) || 'openai',
  };
  return _settingsCache;
}

export async function saveSettings(s: { apiBase?: string; apiKey?: string; model?: string; apiFormat?: ApiFormat }) {
  const ops: Promise<void>[] = [];
  if (s.apiBase !== undefined) ops.push(AsyncStorage.setItem(KEYS.apiBase, s.apiBase));
  if (s.apiKey !== undefined) ops.push(setApiKey(s.apiKey));
  if (s.model !== undefined) ops.push(AsyncStorage.setItem(KEYS.model, s.model));
  if (s.apiFormat !== undefined) ops.push(AsyncStorage.setItem(KEYS.apiFormat, s.apiFormat));
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
  if (/\/v\d+[\w-]*($|\/)/.test(trimmed)) return trimmed;
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

  const { apiBase, apiKey, model, apiFormat } = await getSettings();
  if (!apiKey) throw new Error('请先在设置中填写 API Key');

  if (apiFormat === 'anthropic') {
    return chatAnthropic(apiBase, apiKey, model, messages, signal);
  }
  return chatOpenAI(apiBase, apiKey, model, messages, signal);
}

// ── Anthropic Messages API ────────────────────────────────
async function chatAnthropic(
  apiBase: string, apiKey: string, model: string,
  messages: ChatMessage[], signal?: AbortSignal
): Promise<ReadableStream<Uint8Array>> {
  const systemMsgs = messages.filter(m => m.role === 'system');
  const nonSystem = messages.filter(m => m.role !== 'system');
  const systemText = systemMsgs.map(m => m.content).join('\n') || undefined;

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), CHAT_TIMEOUT_MS);
  if (signal) {
    const onAbort = () => controller.abort(signal.reason);
    signal.addEventListener('abort', onAbort, { once: true });
  }

  const base = apiBase.replace(/\/+$/, '');
  const body: Record<string, unknown> = {
    model,
    max_tokens: 4096,
    messages: nonSystem.map(m => ({ role: m.role, content: m.content })),
    stream: true,
  };
  if (systemText) body.system = systemText;

  let res: Response;
  try {
    res = await fetch(`${base}/v1/messages`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'x-api-key': apiKey,
        'anthropic-version': '2023-06-01',
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } catch (e: any) {
    clearTimeout(timeout);
    if (e.name === 'AbortError') throw e;
    throw new Error('网络请求失败，请检查连接');
  }

  if (!res.ok) {
    clearTimeout(timeout);
    const text = await res.text().catch(() => '');
    throw new Error(sanitizeApiError(res.status, text));
  }

  if (!res.body) { clearTimeout(timeout); throw new Error('No response body'); }

  // Wrap Anthropic SSE into OpenAI-compatible format so parseSSEStream works
  const anthropicBody = res.body;
  return new ReadableStream<Uint8Array>({
    start(ctrl) {
      const reader = anthropicBody.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      const onTimeout = () => { reader.cancel('timeout'); ctrl.error(new DOMException('Timeout', 'AbortError')); };
      controller.signal.addEventListener('abort', onTimeout, { once: true });

      (async () => {
        try {
          while (true) {
            const { value, done } = await reader.read();
            if (done) break;
            buffer += decoder.decode(value, { stream: true });
            const lines = buffer.split(/\r\n|\r|\n/);
            buffer = lines.pop() || '';

            let currentEvent = '';
            for (const line of lines) {
              if (line.startsWith('event:')) {
                currentEvent = line.slice(6).trim();
                continue;
              }
              if (!line.startsWith('data:')) continue;
              const data = line.slice(5).trimStart();
              try {
                const parsed = JSON.parse(data);
                if (parsed.type === 'content_block_delta' && parsed.delta?.text) {
                  // Convert to OpenAI format
                  const openai = JSON.stringify({ choices: [{ delta: { content: parsed.delta.text } }] });
                  ctrl.enqueue(new TextEncoder().encode(`data: ${openai}\n\n`));
                } else if (parsed.type === 'message_stop') {
                  ctrl.enqueue(new TextEncoder().encode('data: [DONE]\n\n'));
                }
              } catch {}
            }
          }
          ctrl.close();
        } catch (e) { ctrl.error(e); }
        finally {
          clearTimeout(timeout);
          controller.signal.removeEventListener('abort', onTimeout);
        }
      })();
    },
  });
}

// ── OpenAI-compatible Chat Completions API ─────────────────
async function chatOpenAI(
  apiBase: string, apiKey: string, model: string,
  messages: ChatMessage[], signal?: AbortSignal
): Promise<ReadableStream<Uint8Array>> {
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
    let res: Response | null = null;
    let lastError: Error | null = null;

    for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
      if (attempt > 0) {
        const delay = RETRY_BASE_MS * Math.pow(2, attempt - 1);
        await new Promise(r => setTimeout(r, delay));
        if (controller.signal.aborted) throw new DOMException('Aborted', 'AbortError');
      }

      // Try streaming first; fall back to non-streaming on 400
      try {
        res = await fetch(`${base}/chat/completions`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${apiKey}` },
          body: JSON.stringify({ model, messages, stream: true }),
          signal: controller.signal,
        });
      } catch (fetchErr: any) {
        lastError = fetchErr;
        if (fetchErr.name === 'AbortError') throw fetchErr;
        continue; // network error → retry
      }

      // Retry on transient server errors
      if (isRetryable(res.status)) {
        lastError = new Error(sanitizeApiError(res.status, ''));
        res = null;
        continue;
      }
      break;
    }

    if (!res) throw lastError ?? new Error('请求失败，已重试多次');

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

/**
 * Chat with SSE stream reconnection.
 * If the stream drops mid-response, automatically re-fetches with exponential backoff.
 * Unlike chat(), this does not wrap the response in a ReadableStream — it calls
 * onChunk directly via parseSSEStreamWithReconnect.
 */
export async function chatWithReconnect(
  messages: ChatMessage[],
  onChunk: (chunk: import('./sse').StreamChunk) => void,
  signal?: AbortSignal,
  options?: { maxRetries?: number; baseDelay?: number },
): Promise<void> {
  if (signal?.aborted) {
    throw new DOMException('The operation was aborted.', 'AbortError');
  }

  const { apiBase, apiKey, model } = await getSettings();
  if (!apiKey) throw new Error('请先在设置中填写 API Key');
  const base = normalizeApiBase(apiBase);

  const body = JSON.stringify({ model, messages, stream: true });
  const fetchInit: RequestInit = {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${apiKey}` },
    body,
  };

  await parseSSEStreamWithReconnect(
    `${base}/chat/completions`,
    fetchInit,
    onChunk,
    { ...options, signal },
  );
}

// ── Health Check ──────────────────────────────────────────
export async function checkApi(params?: { apiBase?: string; apiKey?: string; apiFormat?: ApiFormat }): Promise<{ ok: boolean; error?: string }> {
  try {
    const settings = params ?? await getSettings();
    const { apiKey } = settings;
    const apiBase = settings.apiBase ?? '';
    const format = ('apiFormat' in settings) ? (settings as any).apiFormat : 'openai';
    if (!apiKey) return { ok: false, error: '未配置 API Key' };

    if (format === 'anthropic') {
      // Anthropic doesn't have a /models endpoint; just verify the base URL is reachable
      const base = apiBase.replace(/\/+$/, '');
      const res = await fetch(`${base}/v1/messages`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'x-api-key': apiKey,
          'anthropic-version': '2023-06-01',
        },
        body: JSON.stringify({ model: 'claude-sonnet-4-20250514', max_tokens: 1, messages: [{ role: 'user', content: 'hi' }] }),
        signal: AbortSignal.timeout(8000),
      });
      // 400 = bad request but API is reachable; 200 = ok; anything else = auth/network error
      return { ok: res.ok || res.status === 400, error: res.ok || res.status === 400 ? undefined : `HTTP ${res.status}` };
    }

    const res = await fetch(`${normalizeApiBase(apiBase)}/models`, {
      headers: { Authorization: `Bearer ${apiKey}` },
      signal: AbortSignal.timeout(8000),
    });
    return { ok: res.ok, error: res.ok ? undefined : `HTTP ${res.status}` };
  } catch (e: any) {
    return { ok: false, error: e.message };
  }
}
