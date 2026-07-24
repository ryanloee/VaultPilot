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
      const legacy = await AsyncStorage.getItem(KEYS.apiKey);
      if (legacy) {
        await SecureStore.setItemAsync(KEYS.apiKey, legacy);
        await AsyncStorage.removeItem(KEYS.apiKey);
        _migrated = true;
        return legacy;
      }
      _migrated = true;
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
// Cache lives in settingsCache.ts (shared with store.ts to avoid circular imports).

import { invalidateSettingsCache, getSettingsCache, setSettingsCache } from './settingsCache';
export { invalidateSettingsCache } from './settingsCache';

export async function getSettings(): Promise<{ apiBase: string; apiKey: string; model: string; apiFormat: ApiFormat }> {
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
  } catch (e: unknown) {
    console.warn('[Settings] Failed to read from store:', e instanceof Error ? e.message : e);
  }

  const cached = getSettingsCache();
  if (cached) return cached;
  const [base, key, model, fmt] = await Promise.all([
    AsyncStorage.getItem(KEYS.apiBase),
    getApiKey(),
    AsyncStorage.getItem(KEYS.model),
    AsyncStorage.getItem(KEYS.apiFormat),
  ]);
  const settings = {
    apiBase: base || DEFAULTS.apiBase,
    apiKey: key,
    model: model || DEFAULTS.model,
    apiFormat: (fmt as ApiFormat) || 'openai',
  } as const;
  setSettingsCache(settings);
  return settings;
}

export async function saveSettings(s: { apiBase?: string; apiKey?: string; model?: string; apiFormat?: ApiFormat }) {
  const ops: Promise<void>[] = [];
  if (s.apiBase !== undefined) ops.push(AsyncStorage.setItem(KEYS.apiBase, s.apiBase));
  if (s.apiKey !== undefined) ops.push(setApiKey(s.apiKey));
  if (s.model !== undefined) ops.push(AsyncStorage.setItem(KEYS.model, s.model));
  if (s.apiFormat !== undefined) ops.push(AsyncStorage.setItem(KEYS.apiFormat, s.apiFormat));
  await Promise.all(ops);
  invalidateSettingsCache();
  // Also sync the in-memory Zustand store so getSettings() doesn't return stale data (#2507).
  // Only do this when at least one field was actually provided — otherwise skip to avoid
  // triggering unnecessary persist middleware writes that break tests expecting no side effects.
  if (s.apiBase !== undefined || s.apiKey !== undefined || s.model !== undefined || s.apiFormat !== undefined) {
    const store = useAppStore.getState();
    if (store.apiBase || store.providers.length > 0) {
      await store.setApiSettings(s);
    }
  }
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
  let abortedByTimeout = false;
  const timeout = setTimeout(() => { abortedByTimeout = true; controller.abort(); }, CHAT_TIMEOUT_MS);
  let onSignalAbort: (() => void) | undefined;
  if (signal) {
    onSignalAbort = () => controller.abort(signal.reason);
    if (signal.aborted) controller.abort(signal.reason);
    else signal.addEventListener('abort', onSignalAbort, { once: true });
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
  let started = false; // tracks whether the ReadableStream was returned (its own finally handles cleanup)

  try {
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

    if (!res) throw new Error('请求失败，已重试多次');

    if (!res.ok) {
      const text = await res.text().catch(() => '');
      throw new Error(sanitizeApiError(res.status, text));
    }

    if (!res.body) throw new Error('No response body');

    // Wrap Anthropic SSE into OpenAI-compatible format using the shared
    // wrapAnthropicBody function (consolidates duplicate SSE parsing with
    // chatWithReconnect's transformBody path — #2926).
    const wrapped = wrapAnthropicBody(res.body);
    // #3224: Only mark `started = true` AFTER wrapAnthropicBody() succeeds.
    // If wrapAnthropicBody throws, the finally block must clean up the
    // timeout and signal listener (set started=true only when the
    // ReadableStream is actually being returned).
    started = true;
    let wrappedReader: ReadableStreamDefaultReader<Uint8Array> | null = null;
    return new ReadableStream<Uint8Array>({
      start(ctrl) {
        const reader = wrapped.getReader();
        wrappedReader = reader;
        const onAbort = () => {
          reader.cancel('abort').catch(() => {});
          ctrl.error(abortedByTimeout ? new DOMException('Timeout', 'AbortError') : new DOMException('Cancelled', 'AbortError'));
        };
        controller.signal.addEventListener('abort', onAbort, { once: true });

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
            controller.signal.removeEventListener('abort', onAbort);
            if (onSignalAbort) signal?.removeEventListener('abort', onSignalAbort);
          }
        })();
      },
      cancel(reason) {
        wrappedReader?.cancel(reason).catch(() => {});
      },
    });
  } catch (e: unknown) {
    // #3036: Convert fetch-phase AbortError (timeout firing before the
    // ReadableStream is returned) into a friendly Chinese message, matching
    // chatOpenAI's behaviour. The stream-consumption path (started === true)
    // never reaches here because the ReadableStream is returned synchronously.
    if (e instanceof Error && e.name === 'AbortError' && !signal?.aborted) {
      throw new Error('请求超时（2 分钟），请检查网络或服务端状态');
    }
    throw e;
  } finally {
    if (!started) {
      clearTimeout(timeout);
      if (onSignalAbort) signal?.removeEventListener('abort', onSignalAbort);
    }
  }
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
  if (sig) {
    if (sig.aborted) controller.abort(sig.reason);
    else sig.addEventListener('abort', onSignalAbort!, { once: true });
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
      await res.body?.cancel().catch(() => {}); // Release connection back to pool
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
    let openaiReader: ReadableStreamDefaultReader<Uint8Array> | null = null;
    return new ReadableStream<Uint8Array>({
      start(ctrl) {
        const reader = body.getReader();
        openaiReader = reader;
        const onTimeout = () => { reader.cancel('timeout').catch(() => {}); ctrl.error(new DOMException('Timeout', 'AbortError')); };
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
            if (onSignalAbort) sig?.removeEventListener('abort', onSignalAbort);
          }
        })();
      },
      cancel(reason) {
        openaiReader?.cancel(reason).catch(() => {});
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
 *
 * This is the canonical implementation — used by both chatAnthropic()
 * (via stream passthrough) and chatWithReconnect() (via transformBody).
 * Consolidation from #2926 — previously duplicated inline in chatAnthropic().
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
              reader.cancel().catch(() => {});
              ctrl.close();
              return;
            }
            if (result) ctrl.enqueue(encoder.encode(result));
          }
        }
      } catch (e) { reader.cancel().catch(() => {}); ctrl.error(e); }
    },
    cancel() { reader.cancel().catch(() => {}); },
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
    if (signal) {
      if (signal.aborted) controller.abort(signal.reason);
      else signal.addEventListener('abort', onSignalAbort!, { once: true });
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
  if (signal) {
    if (signal.aborted) controller.abort(signal.reason);
    else signal.addEventListener('abort', onSignalAbort!, { once: true });
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
  // Avoid unsafe type assertion: when params is not provided, getSettings()
  // does not return signal or model fields. Use params for caller-provided
  // fields and fall back to getSettings() for stored settings (#2927).
  const provided = params;
  const stored = provided ? undefined : await getSettings();
  const apiKey = provided?.apiKey ?? stored?.apiKey ?? '';
  const signal = provided?.signal;
  const apiBase = provided?.apiBase ?? stored?.apiBase ?? '';
  const format = provided?.apiFormat ?? stored?.apiFormat ?? 'openai';
  const model = provided?.model ?? stored?.model;

  if (!apiKey) return { ok: false, error: '未配置 API Key' };

  // Hermes-compatible timeout: AbortSignal.timeout() / AbortSignal.any() are
  // unavailable on React Native Hermes (Hermes 0.12 / RN 0.73). Use setTimeout +
  // a manual AbortController instead to combine timeout and user signal (#2329).
  const timeoutController = new AbortController();
  const timer = setTimeout(() => timeoutController.abort(), CHECK_API_TIMEOUT_MS);
  const onUserAbort = () => timeoutController.abort(signal?.reason);
  if (signal) {
    if (signal.aborted) timeoutController.abort(signal.reason);
    else signal.addEventListener('abort', onUserAbort, { once: true });
  }
  const effectiveSignal = timeoutController.signal;

  try {
    if (format === 'anthropic') {
      // Anthropic GET /v1/models is free (no token cost), matching OpenAI's /models pattern (#3421)
      const base = normalizeAnthropicBase(normalizeApiBase(apiBase));
      const res = await fetch(`${base}/v1/models`, {
        method: 'GET',
        headers: {
          'x-api-key': apiKey,
          'anthropic-version': '2023-06-01',
        },
        signal: effectiveSignal,
      });
      return { ok: res.ok, error: res.ok ? undefined : `HTTP ${res.status}` };
    }

    const res = await fetch(`${normalizeApiBase(apiBase)}/models`, {
      headers: { Authorization: `Bearer ${apiKey}` },
      signal: effectiveSignal,
    });
    return { ok: res.ok, error: res.ok ? undefined : `HTTP ${res.status}` };
  } catch (e: unknown) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  } finally {
    clearTimeout(timer);
    if (signal) signal.removeEventListener('abort', onUserAbort);
  }
}
