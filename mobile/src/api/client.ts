import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

import { ApiFormat, useAppStore } from '../store';
import { isRetryable, sanitizeApiError, normalizeApiBase, DEFAULTS } from './clientUtils';

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

// ── Secure helpers ───────────────────────────────────────
let _migrated = false;

async function getApiKey(): Promise<string> {
  try {
    const key = await SecureStore.getItemAsync(KEYS.apiKey);
    if (key) return key;

    // One-time migration: move legacy key from AsyncStorage → SecureStore
    if (!_migrated) {
      _migrated = true;
      const legacy = await AsyncStorage.getItem(KEYS.apiKey);
      if (legacy) {
        await SecureStore.setItemAsync(KEYS.apiKey, legacy);
        await AsyncStorage.removeItem(KEYS.apiKey);
        return legacy;
      }
    }
    return '';
  } catch (e: unknown) {
    console.warn('[SecureStore] Failed to read API key:', e instanceof Error ? e.message : e);
    return '';
  }
}

async function setApiKey(value: string): Promise<void> {
  try {
    await SecureStore.setItemAsync(KEYS.apiKey, value);
  } catch (e: unknown) {
    console.warn('[SecureStore] Failed to write API key:', e instanceof Error ? e.message : e);
    throw new Error('无法安全存储 API Key，请检查设备安全设置');
  }
}

/** Chat request timeout in ms (2 minutes) */
const CHAT_TIMEOUT_MS = 120_000;

/** Health-check (checkApi) timeout in ms (8 seconds) */
const CHECK_API_TIMEOUT_MS = 8_000;

/** Max retries for transient network/server errors */
const MAX_RETRIES = 2;
const RETRY_BASE_MS = 1000;

// ── Settings ──────────────────────────────────────────────
let _settingsCache: { apiBase: string; apiKey: string; model: string; apiFormat: ApiFormat } | null = null;

export function invalidateSettingsCache() {
  _settingsCache = null;
}

export async function getSettings() {
  // Use Zustand store as the primary source of truth — this ensures that
  // any changes made via the UI (setApiSettings, setActiveProvider, etc.)
  // are immediately reflected in API calls.
  try {
    const storeState = useAppStore.getState();
    if (storeState.apiBase && storeState.providers.length > 0) {
      return {
        apiBase: storeState.apiBase,
        apiKey: storeState.apiKey || '',
        model: storeState.model,
        apiFormat: storeState.apiFormat || 'openai',
      };
    }
  } catch {
    // Store not available — fall through to legacy keys
  }

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

// ── Chat ──────────────────────────────────────────────────
export type ContentPart =
  | { type: 'text'; text: string }
  | { type: 'image_url'; image_url: { url: string } };

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant';
  content: string | ContentPart[];
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

// Re-export pure conversion functions from clientPure.ts
export { toAnthropicContent, convertAnthropicEvent, wrapNonStreamingResponse, normalizeAnthropicBase, extractTextContent } from './clientPure';
import { toAnthropicContent, convertAnthropicEvent, wrapNonStreamingResponse, normalizeAnthropicBase, extractTextContent } from './clientPure';

async function chatAnthropic(
  apiBase: string, apiKey: string, model: string,
  messages: ChatMessage[], signal?: AbortSignal
): Promise<ReadableStream<Uint8Array>> {
  const systemMsgs = messages.filter(m => m.role === 'system');
  const nonSystem = messages.filter(m => m.role !== 'system');
  const systemText = systemMsgs.map(m => extractTextContent(m.content)).join('\n') || undefined;

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), CHAT_TIMEOUT_MS);
  let onSignalAbort: (() => void) | undefined;
  if (signal) {
    onSignalAbort = () => controller.abort(signal.reason);
    signal.addEventListener('abort', onSignalAbort, { once: true });
  }

  const base = normalizeAnthropicBase(normalizeApiBase(apiBase));
  const body: Record<string, unknown> = {
    model,
    max_tokens: 4096,
    messages: nonSystem.map(m => ({ role: m.role, content: toAnthropicContent(m.content) })),
    stream: true,
  };
  if (systemText) body.system = systemText;

  let res: Response | null = null;
  let lastError: Error | null = null;

  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
    if (attempt > 0) {
      const delay = RETRY_BASE_MS * Math.pow(2, attempt - 1);
      await new Promise(r => setTimeout(r, delay));
      if (controller.signal.aborted) throw new DOMException('Aborted', 'AbortError');
    }

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
    } catch (fetchErr: unknown) {
      lastError = fetchErr instanceof Error ? fetchErr : new Error(String(fetchErr));
      if (lastError.name === 'AbortError') {
        clearTimeout(timeout);
        signal?.removeEventListener('abort', onSignalAbort!);
        throw fetchErr;
      }
      continue; // network error → retry
    }

    // Retry on transient server errors
    if (isRetryable(res.status)) {
      lastError = new Error(sanitizeApiError(res.status, ''));
      await res.body?.cancel().catch(() => {});
      res = null;
      continue;
    }
    break;
  }

  if (!res) {
    clearTimeout(timeout);
    signal?.removeEventListener('abort', onSignalAbort!);
    throw new Error('请求失败，已重试多次');
  }

  if (!res.ok) {
    clearTimeout(timeout);
    signal?.removeEventListener('abort', onSignalAbort!);
    const text = await res.text().catch(() => '');
    throw new Error(sanitizeApiError(res.status, text));
  }

  if (!res.body) { clearTimeout(timeout); signal?.removeEventListener('abort', onSignalAbort!); throw new Error('No response body'); }

  // Wrap Anthropic SSE into OpenAI-compatible format so parseSSEStream works
  const anthropicBody = res.body;
  return new ReadableStream<Uint8Array>({
    start(ctrl) {
      const reader = anthropicBody.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      let currentEvent = '';
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


            for (const line of lines) {
              if (line.startsWith('event:')) {
                currentEvent = line.slice(6).trim();
                continue;
              }
              if (!line.startsWith('data:')) continue;
              const data = line.slice(5).trimStart();
              const result = convertAnthropicEvent(currentEvent, data);
              if (result) ctrl.enqueue(new TextEncoder().encode(result));
            }
          }
          ctrl.close();
        } catch (e) { ctrl.error(e); }
        finally {
          clearTimeout(timeout);
          controller.signal.removeEventListener('abort', onTimeout);
          signal?.removeEventListener('abort', onSignalAbort!);
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

  let streamingReturned = false;
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
      } catch (fetchErr: unknown) {
        lastError = fetchErr instanceof Error ? fetchErr : new Error(String(fetchErr));
        if (lastError.name === 'AbortError') throw fetchErr;
        continue; // network error → retry
      }

      // Retry on transient server errors
      if (isRetryable(res.status)) {
        lastError = new Error(sanitizeApiError(res.status, ''));
        await res.body?.cancel().catch(() => {});
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
      const encoded = wrapNonStreamingResponse(json);
      return new ReadableStream({
        start(controller) { controller.enqueue(encoded); controller.close(); },
      });
    }

    if (!res.body) throw new Error('No response body');
    // Wrap body so the timeout still applies during stream reading
    const body = res.body;
    const timeoutController = controller;
    streamingReturned = true;
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
            sig?.removeEventListener('abort', onSignalAbort!);
          }
        })();
      },
    });
  } catch (e: unknown) {
    clearTimeout(timeout);
    if (e instanceof Error && e.name === 'AbortError' && !signal?.aborted) {
      throw new Error('请求超时（2 分钟），请检查网络或服务端状态');
    }
    throw e;
  } finally {
    // Only clean up here if no stream was returned — stream's finally handles its own lifecycle
    if (!streamingReturned) {
      clearTimeout(timeout);
      if (onSignalAbort) sig?.removeEventListener('abort', onSignalAbort);
    }
  }
}

/**
 * Chat with SSE stream reconnection.
 * If the stream drops mid-response, automatically re-fetches with exponential backoff.
 * Unlike chat(), this does not wrap the response in a ReadableStream — it calls
 * onChunk directly via parseSSEStreamWithReconnect.
 */
/**
 * Wrap an Anthropic SSE response body into OpenAI-compatible SSE format,
 * so parseSSEStream can consume it uniformly.
 */
function wrapAnthropicBody(body: ReadableStream<Uint8Array>): ReadableStream<Uint8Array> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  const encoder = new TextEncoder();
  let buffer = '';
  let currentEvent = '';

  return new ReadableStream<Uint8Array>({
    async pull(ctrl) {
      try {
        while (true) {
          const { value, done } = await reader.read();
          if (done) { ctrl.close(); return; }
          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split(/\r\n|\r|\n/);
          buffer = lines.pop() || '';


          for (const line of lines) {
            if (line.startsWith('event:')) { currentEvent = line.slice(6).trim(); continue; }
            if (!line.startsWith('data:')) continue;
            const data = line.slice(5).trimStart();
            const result = convertAnthropicEvent(currentEvent, data);
            if (result === 'data: [DONE]\n\n') {
              ctrl.enqueue(encoder.encode(result));
              ctrl.close();
              return;
            }
            if (result) ctrl.enqueue(encoder.encode(result));
          }
        }
      } catch (e) { reader.cancel().catch(() => {}); ctrl.error(e); }
    },
    cancel() { reader.cancel(); },
  });
}

export async function chatWithReconnect(
  messages: ChatMessage[],
  onChunk: (chunk: import('./sse').StreamChunk) => void,
  signal?: AbortSignal,
  options?: { maxRetries?: number; baseDelay?: number },
): Promise<void> {
  if (signal?.aborted) {
    throw new DOMException('The operation was aborted.', 'AbortError');
  }

  const { apiBase, apiKey, model, apiFormat } = await getSettings();
  if (!apiKey) throw new Error('请先在设置中填写 API Key');
  const base = normalizeApiBase(apiBase);

  if (apiFormat === 'anthropic') {
    // Anthropic Messages API format
    const systemMsgs = messages.filter(m => m.role === 'system');
    const nonSystem = messages.filter(m => m.role !== 'system');
    const systemText = systemMsgs.map(m => extractTextContent(m.content)).join('\n') || undefined;
    const anthropicBase = normalizeAnthropicBase(base);
    const body: Record<string, unknown> = {
      model,
      max_tokens: 4096,
      messages: nonSystem.map(m => ({ role: m.role, content: toAnthropicContent(m.content) })),
      stream: true,
    };
    if (systemText) body.system = systemText;

    // Wrap signal with timeout (same pattern as chatAnthropic)
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), CHAT_TIMEOUT_MS);
    const onSignalAbort = signal ? () => controller.abort(signal.reason) : undefined;
    if (signal && onSignalAbort) {
      signal.addEventListener('abort', onSignalAbort, { once: true });
    }

    try {
      await parseSSEStreamWithReconnect(
        `${anthropicBase}/v1/messages`,
        {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'x-api-key': apiKey,
            'anthropic-version': '2023-06-01',
          },
          body: JSON.stringify(body),
        },
        onChunk,
        { ...options, signal: controller.signal, transformBody: wrapAnthropicBody },
      );
    } catch (e: unknown) {
      if (e instanceof Error && e.name === 'AbortError' && !signal?.aborted) {
        throw new Error('请求超时（2 分钟），请检查网络或服务端状态');
      }
      throw e;
    } finally {
      clearTimeout(timeout);
      if (onSignalAbort) signal?.removeEventListener('abort', onSignalAbort);
    }
    return;
  }

  // OpenAI-compatible format (default)
  const body = JSON.stringify({ model, messages, stream: true });
  // Wrap signal with timeout (same pattern as chatOpenAI)
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), CHAT_TIMEOUT_MS);
  const onSignalAbort = signal ? () => controller.abort(signal.reason) : undefined;
  if (signal && onSignalAbort) {
    signal.addEventListener('abort', onSignalAbort, { once: true });
  }

  try {
    await parseSSEStreamWithReconnect(
      `${base}/chat/completions`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${apiKey}` },
        body,
      },
      onChunk,
      { ...options, signal: controller.signal },
    );
  } catch (e: unknown) {
    if (e instanceof Error && e.name === 'AbortError' && !signal?.aborted) {
      throw new Error('请求超时（2 分钟），请检查网络或服务端状态');
    }
    throw e;
  } finally {
    clearTimeout(timeout);
    if (onSignalAbort) signal?.removeEventListener('abort', onSignalAbort);
  }
}

// ── Health Check ──────────────────────────────────────────
export async function checkApi(params?: { apiBase?: string; apiKey?: string; model?: string; apiFormat?: ApiFormat; signal?: AbortSignal }): Promise<{ ok: boolean; error?: string }> {
  try {
    const settings = (params ?? await getSettings()) as { apiBase?: string; apiKey?: string; model?: string; apiFormat?: ApiFormat; signal?: AbortSignal };
    const apiKey = settings.apiKey ?? '';
    const signal = settings.signal;
    const apiBase = settings.apiBase ?? '';
    const format = settings.apiFormat ?? 'openai';
    if (!apiKey) return { ok: false, error: '未配置 API Key' };

    // Hermes-compatible timeout: AbortSignal.timeout() / AbortSignal.any() are
    // unavailable on React Native Hermes (Hermes 0.12 / RN 0.73). Use setTimeout +
    // a manual AbortController instead to combine timeout and user signal (#2329).
    const timeoutController = new AbortController();
    const timer = setTimeout(() => timeoutController.abort(), CHECK_API_TIMEOUT_MS);
    if (signal) {
      if (signal.aborted) timeoutController.abort(signal.reason);
      else signal.addEventListener('abort', () => timeoutController.abort(signal.reason), { once: true });
    }
    const effectiveSignal = timeoutController.signal;

    if (format === 'anthropic') {
      // Anthropic doesn't have a /models endpoint; just verify the base URL is reachable
      const base = normalizeAnthropicBase(normalizeApiBase(apiBase));
      const res = await fetch(`${base}/v1/messages`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'x-api-key': apiKey,
          'anthropic-version': '2023-06-01',
        },
        body: JSON.stringify({ model: settings.model ?? 'claude-sonnet-4-20250514', max_tokens: 1, messages: [{ role: 'user', content: 'hi' }] }),
        signal: effectiveSignal,
      });
      // 400 = bad request but API is reachable; 200 = ok; anything else = auth/network error
      return { ok: res.ok || res.status === 400, error: res.ok || res.status === 400 ? undefined : `HTTP ${res.status}` };
    }

    const res = await fetch(`${normalizeApiBase(apiBase)}/models`, {
      headers: { Authorization: `Bearer ${apiKey}` },
      signal: effectiveSignal,
    });
    return { ok: res.ok, error: res.ok ? undefined : `HTTP ${res.status}` };
  } catch (e: unknown) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}
