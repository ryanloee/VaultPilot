/**
 * Regression test for #1172:
 * - Anthropic API double /v1 path: base URL containing /v1 should not become /v1/v1/messages
 * - Leaked abort listener: signal listener must be removed on error paths
 */
import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

// Mock modules before imports
jest.mock('@react-native-async-storage/async-storage', () => ({
  getItem: jest.fn(),
  setItem: jest.fn(),
}));

jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
}));

// Must import after mocks are set up
import { chat, invalidateSettingsCache } from '../../api/client';

const MOCK_ANTHROPIC_STREAM = [
  'event: content_block_delta\n',
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}\n\n',
  'event: message_stop\n',
  'data: {"type":"message_stop"}\n\n',
];

function makeAnthropicStream() {
  const encoder = new TextEncoder();
  let i = 0;
  return new ReadableStream({
    pull(controller) {
      if (i < MOCK_ANTHROPIC_STREAM.length) {
        controller.enqueue(encoder.encode(MOCK_ANTHROPIC_STREAM[i++]));
      } else {
        controller.close();
      }
    },
  });
}

describe('issue #1172 — Anthropic double /v1 path', () => {
  beforeEach(() => {
    jest.restoreAllMocks();
    invalidateSettingsCache();
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com/v1');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve('claude-sonnet-4-20250514');
      return Promise.resolve(null);
    });
    (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test-key');
  });

  it('does NOT double /v1 when base URL already contains /v1', async () => {
    let capturedUrl = '';
    const mockFetch = jest.fn().mockImplementation((url: string) => {
      capturedUrl = url;
      return Promise.resolve({
        ok: true,
        body: makeAnthropicStream(),
        headers: new Map([['content-type', 'text/event-stream']]),
      });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const controller = new AbortController();
    try {
      await chat([{ role: 'user', content: 'hi' }], controller.signal);
    } catch {
      // Stream may error on close, that's ok
    }

    // Should be https://api.anthropic.com/v1/messages, NOT .../v1/v1/messages
    expect(capturedUrl).toBe('https://api.anthropic.com/v1/messages');
    expect(capturedUrl).not.toContain('/v1/v1');
  });

  it('appends /v1/messages when base URL has no version suffix', async () => {
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve('claude-sonnet-4-20250514');
      return Promise.resolve(null);
    });
    invalidateSettingsCache();

    let capturedUrl = '';
    const mockFetch = jest.fn().mockImplementation((url: string) => {
      capturedUrl = url;
      return Promise.resolve({
        ok: true,
        body: makeAnthropicStream(),
        headers: new Map([['content-type', 'text/event-stream']]),
      });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const controller = new AbortController();
    try {
      await chat([{ role: 'user', content: 'hi' }], controller.signal);
    } catch {}

    expect(capturedUrl).toBe('https://api.anthropic.com/v1/messages');
  });

  it('strips trailing slashes before appending /v1/messages', async () => {
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com/v1/');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve('claude-sonnet-4-20250514');
      return Promise.resolve(null);
    });
    invalidateSettingsCache();

    let capturedUrl = '';
    const mockFetch = jest.fn().mockImplementation((url: string) => {
      capturedUrl = url;
      return Promise.resolve({
        ok: true,
        body: makeAnthropicStream(),
        headers: new Map([['content-type', 'text/event-stream']]),
      });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const controller = new AbortController();
    try {
      await chat([{ role: 'user', content: 'hi' }], controller.signal);
    } catch {}

    expect(capturedUrl).toBe('https://api.anthropic.com/v1/messages');
    expect(capturedUrl).not.toMatch(/\/v1\/.*\/v1/);
  });
});

describe('issue #1172 — Anthropic abort listener cleanup', () => {
  beforeEach(() => {
    jest.restoreAllMocks();
    invalidateSettingsCache();
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com/v1');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve('claude-sonnet-4-20250514');
      return Promise.resolve(null);
    });
    (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test-key');
  });

  it('cleans up signal listener when fetch throws', async () => {
    const mockFetch = jest.fn().mockRejectedValue(new TypeError('Network error'));
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const controller = new AbortController();
    const addSpy = jest.spyOn(controller.signal, 'addEventListener');
    const removeSpy = jest.spyOn(controller.signal, 'removeEventListener');

    await expect(chat([{ role: 'user', content: 'hi' }], controller.signal)).rejects.toThrow('请求失败，已重试多次');

    // Listener should have been added and then removed
    expect(addSpy).toHaveBeenCalled();
    expect(removeSpy).toHaveBeenCalledWith('abort', expect.any(Function));
  });

  it('cleans up signal listener when response is not ok', async () => {
    const mockFetch = jest.fn().mockResolvedValue({
      ok: false,
      status: 401,
      text: jest.fn().mockResolvedValue('Unauthorized'),
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const controller = new AbortController();
    const removeSpy = jest.spyOn(controller.signal, 'removeEventListener');

    await expect(chat([{ role: 'user', content: 'hi' }], controller.signal)).rejects.toThrow();

    expect(removeSpy).toHaveBeenCalledWith('abort', expect.any(Function));
  });
});
