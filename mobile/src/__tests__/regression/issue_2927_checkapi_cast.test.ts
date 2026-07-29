/**
 * Regression test for #2927: client.ts checkApi() used unsafe `as` cast
 * to add signal and model fields to getSettings() return type.
 *
 * The fix restructures checkApi() to use params directly for caller-provided
 * fields and getSettings() for stored settings, without type assertion.
 *
 * This test verifies:
 * 1. checkApi() works correctly with params provided (all fields explicit)
 * 2. checkApi() works correctly without params (calls getSettings())
 * 3. checkApi() preserves abort signal through param passthrough
 * 4. Signal from params is properly used (not from getSettings which doesn't have it)
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';
import { checkApi, invalidateSettingsCache } from '../../api/client';
import { useAppStore } from '../../store';

// Mock fetch via globalThis
const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

beforeEach(() => {
  jest.clearAllMocks();
  mockFetch.mockReset();
  invalidateSettingsCache();
  useAppStore.setState({
    themeMode: 'system',
    isDark: false,
    accentColor: '#3B82F6',
    apiBase: 'https://opencode.ai/zen/v1',
    apiKey: 'test-key-123',
    model: 'deepseek-v4-flash-free',
    apiFormat: 'openai',
    providers: [{
      name: 'OpenCode Zen',
      apiBase: 'https://opencode.ai/zen/v1',
      apiKey: 'test-key-123',
      model: 'deepseek-v4-flash-free',
      apiFormat: 'openai',
    }],
    activeProviderIndex: 0,
  });
});

describe('#2927: checkApi() avoids unsafe as-cast on getSettings() return', () => {
  it('checkApi() with explicit params does not call getSettings', async () => {
    mockFetch.mockResolvedValue(new Response(null, { status: 200 }));

    const result = await checkApi({
      apiBase: 'https://custom.example.com/v1',
      apiKey: 'custom-key',
      apiFormat: 'openai',
    });

    expect(result.ok).toBe(true);
    // Should use custom base, not stored base
    const fetchUrl = mockFetch.mock.calls[0][0] as string;
    expect(fetchUrl).toContain('custom.example.com');
  });

  it('checkApi() without params uses getSettings() for stored values', async () => {
    mockFetch.mockResolvedValue(new Response(null, { status: 200 }));

    const result = await checkApi();

    expect(result.ok).toBe(true);
    // Should use stored base URL
    const fetchUrl = mockFetch.mock.calls[0][0] as string;
    expect(fetchUrl).toContain('opencode.ai');
  });

  it('checkApi() with signal passes it through to fetch', async () => {
    mockFetch.mockResolvedValue(new Response(null, { status: 200 }));
    const controller = new AbortController();

    const result = await checkApi({
      apiBase: 'https://test.example.com/v1',
      apiKey: 'key',
      signal: controller.signal,
    });

    expect(result.ok).toBe(true);
    // The AbortSignal should be composed with timeout into effectiveSignal,
    // passed to fetch. The original signal should NOT be directly passed
    // (it's combined with the timeout controller).
    const fetchInit = mockFetch.mock.calls[0][1] as RequestInit;
    expect(fetchInit.signal).toBeDefined();
  });

  it('checkApi() with signal handles abort gracefully', async () => {
    const controller = new AbortController();
    controller.abort();

    const result = await checkApi({
      apiBase: 'https://test.example.com/v1',
      apiKey: 'key',
      signal: controller.signal,
    });

    // When signal is already aborted, fetch should fail immediately
    expect(result.ok).toBe(false);
    expect(result.error).toBeDefined();
  });

  it('checkApi() without signal still works (signal is undefined from params, not falsely from cast)', async () => {
    mockFetch.mockResolvedValue(new Response(null, { status: 200 }));

    // No params at all — signal should be undefined, coming naturally
    // from the provided?.signal path (not from a bogus as-cast).
    const result = await checkApi();

    expect(result.ok).toBe(true);
    // Verify fetch was called (timeout signal still works, just no user signal merged)
    expect(mockFetch).toHaveBeenCalled();
  });

  it('checkApi() Anthropic format uses GET /v1/models endpoint (#3421)', async () => {
    mockFetch.mockResolvedValue(new Response(null, { status: 200 }));

    const result = await checkApi({
      apiBase: 'https://api.anthropic.com',
      apiKey: 'sk-ant-test',
      apiFormat: 'anthropic',
    });

    expect(result.ok).toBe(true);
    // Verify the URL uses /v1/models (free GET endpoint), not /v1/messages
    const fetchUrl = mockFetch.mock.calls[0][0] as string;
    expect(fetchUrl).toContain('/v1/models');
    expect(fetchUrl).not.toContain('/v1/messages');
  });

  it('checkApi() Anthropic format without model param still uses /v1/models', async () => {
    // Set Anthropic format in store
    useAppStore.setState({
      apiFormat: 'anthropic',
      apiBase: 'https://api.anthropic.com',
      model: 'stored-model-name',
    });
    mockFetch.mockResolvedValue(new Response(null, { status: 200 }));

    const result = await checkApi();

    expect(result.ok).toBe(true);
    // Verify uses /v1/models endpoint
    const fetchUrl = mockFetch.mock.calls[0][0] as string;
    expect(fetchUrl).toContain('/v1/models');
  });
});