/**
 * Unit tests for client.ts — API layer (#1207).
 *
 * Tests: getSettings/cache, saveSettings, checkApi, normalizeApiBase (via checkApi).
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';
import { getSettings, saveSettings, invalidateSettingsCache, checkApi } from '../../api/client';

// Mock global fetch
const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

beforeEach(() => {
  jest.clearAllMocks();
  invalidateSettingsCache();
  mockFetch.mockReset();
});

// ── getSettings / saveSettings / cache ──────────────────────

describe('getSettings', () => {
  it('returns defaults when storage is empty', async () => {
    const s = await getSettings();
    expect(s.apiBase).toBe('https://opencode.ai/zen/v1');
    expect(s.model).toBe('deepseek-v4-flash-free');
    expect(s.apiFormat).toBe('openai');
  });

  it('caches settings on subsequent calls', async () => {
    await getSettings();
    await getSettings();
    // AsyncStorage.getItem called 3x (apiBase, model, apiFormat); apiKey uses SecureStore
    expect(AsyncStorage.getItem).toHaveBeenCalledTimes(3);
  });

  it('re-reads after cache invalidation', async () => {
    await getSettings();
    invalidateSettingsCache();
    await getSettings();
    // Each getSettings call reads 3 AsyncStorage keys
    expect(AsyncStorage.getItem).toHaveBeenCalledTimes(6);
  });
});

describe('saveSettings', () => {
  it('saves apiBase to AsyncStorage', async () => {
    await saveSettings({ apiBase: 'https://custom.api.com/v1' });
    expect(AsyncStorage.setItem).toHaveBeenCalledWith('cfg_api_base', 'https://custom.api.com/v1');
  });

  it('saves apiKey to SecureStore', async () => {
    await saveSettings({ apiKey: 'sk-test-key' });
    expect(SecureStore.setItemAsync).toHaveBeenCalledWith('cfg_api_key', 'sk-test-key');
  });

  it('saves model to AsyncStorage', async () => {
    await saveSettings({ model: 'gpt-4' });
    expect(AsyncStorage.setItem).toHaveBeenCalledWith('cfg_model', 'gpt-4');
  });

  it('saves apiFormat to AsyncStorage', async () => {
    await saveSettings({ apiFormat: 'anthropic' });
    expect(AsyncStorage.setItem).toHaveBeenCalledWith('cfg_api_format', 'anthropic');
  });

  it('invalidates cache after save', async () => {
    await getSettings(); // populate cache
    await saveSettings({ model: 'new-model' });
    await getSettings(); // should re-read
    expect(AsyncStorage.getItem).toHaveBeenCalledTimes(3 + 3); // first read + second read
  });

  it('does nothing for undefined fields', async () => {
    await saveSettings({});
    expect(AsyncStorage.setItem).not.toHaveBeenCalled();
    expect(SecureStore.setItemAsync).not.toHaveBeenCalled();
  });
});

// ── checkApi ────────────────────────────────────────────────

describe('checkApi', () => {
  it('returns ok when OpenAI /models returns 200', async () => {
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    const result = await checkApi({ apiBase: 'https://api.openai.com', apiKey: 'sk-test', apiFormat: 'openai' });
    expect(result.ok).toBe(true);
    expect(result.error).toBeUndefined();
  });

  it('returns error when OpenAI /models returns 401', async () => {
    mockFetch.mockResolvedValueOnce({ ok: false, status: 401 });
    const result = await checkApi({ apiBase: 'https://api.openai.com', apiKey: 'bad-key', apiFormat: 'openai' });
    expect(result.ok).toBe(false);
    expect(result.error).toContain('401');
  });

  it('returns ok when Anthropic returns 200', async () => {
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    const result = await checkApi({ apiBase: 'https://api.anthropic.com', apiKey: 'sk-ant-test', apiFormat: 'anthropic' });
    expect(result.ok).toBe(true);
  });

  it('returns error when Anthropic returns 400 (bad API key or request)', async () => {
    mockFetch.mockResolvedValueOnce({ ok: false, status: 400 });
    const result = await checkApi({ apiBase: 'https://api.anthropic.com', apiKey: 'key', apiFormat: 'anthropic' });
    // Anthropic checkApi now uses GET /v1/models and returns res.ok directly (#3421)
    expect(result.ok).toBe(false);
    expect(result.error).toContain('400');
  });

  it('returns error when no apiKey configured', async () => {
    const result = await checkApi({ apiBase: 'https://api.openai.com', apiKey: '', apiFormat: 'openai' });
    expect(result.ok).toBe(false);
    expect(result.error).toBeDefined();
  });

  it('returns error on network failure', async () => {
    mockFetch.mockRejectedValueOnce(new Error('Network error'));
    const result = await checkApi({ apiBase: 'https://api.openai.com', apiKey: 'sk-test', apiFormat: 'openai' });
    expect(result.ok).toBe(false);
    expect(result.error).toContain('Network error');
  });

  it('appends /v1 to apiBase without version path', async () => {
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    await checkApi({ apiBase: 'https://api.openai.com', apiKey: 'sk-test', apiFormat: 'openai' });
    expect(mockFetch).toHaveBeenCalledWith(
      expect.stringContaining('/v1/models'),
      expect.any(Object),
    );
  });

  it('does not double /v1 for apiBase already ending in /v1', async () => {
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    await checkApi({ apiBase: 'https://api.openai.com/v1', apiKey: 'sk-test', apiFormat: 'openai' });
    const calledUrl = mockFetch.mock.calls[0][0];
    expect(calledUrl).not.toMatch(/\/v1\/v1/);
  });

  it('appends /v1 to apiBase with trailing slashes', async () => {
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    await checkApi({ apiBase: 'https://api.openai.com///', apiKey: 'sk-test', apiFormat: 'openai' });
    const calledUrl = mockFetch.mock.calls[0][0];
    expect(calledUrl).toContain('/v1/models');
    expect(calledUrl).not.toMatch(/\/\/\/v1/); // no triple slashes before /v1
  });

  it('preserves /v2 versioned path without appending /v1', async () => {
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    await checkApi({ apiBase: 'https://api.openai.com/v2', apiKey: 'sk-test', apiFormat: 'openai' });
    const calledUrl = mockFetch.mock.calls[0][0];
    expect(calledUrl).toContain('/v2/models');
    expect(calledUrl).not.toMatch(/\/v1\/models/);
  });

  it('falls back to default apiBase when empty string provided', async () => {
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    await checkApi({ apiBase: '', apiKey: 'sk-test', apiFormat: 'openai' });
    const calledUrl = mockFetch.mock.calls[0][0];
    // Should use default: https://opencode.ai/zen/v1
    expect(calledUrl).toContain('opencode.ai');
  });

  it('strips trailing slashes from Anthropic base before appending /v1/models', async () => {
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    await checkApi({ apiBase: 'https://api.anthropic.com/', apiKey: 'sk-ant-test', apiFormat: 'anthropic' });
    const calledUrl = mockFetch.mock.calls[0][0];
    // Anthropic checkApi now uses GET /v1/models (free endpoint, #3421)
    expect(calledUrl).toContain('/v1/models');
    expect(calledUrl).not.toMatch(/\/\/v1/); // no double slash before v1
  });
});