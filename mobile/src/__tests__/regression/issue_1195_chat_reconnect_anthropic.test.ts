/**
 * Regression test for #1195:
 * - chatWithReconnect must respect apiFormat (Anthropic vs OpenAI)
 * - Anthropic users should hit /v1/messages, not /chat/completions
 * - checkApi should use Anthropic GET /v1/models (free endpoint, #3421)
 */
import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

jest.mock('@react-native-async-storage/async-storage', () => ({
  getItem: jest.fn(),
  setItem: jest.fn(),
}));

jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
}));

import { chatWithReconnect, checkApi, invalidateSettingsCache } from '../../api/client';

const MOCK_ANTHROPIC_STREAM = [
  'event: content_block_delta\n',
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}\n\n',
  'event: message_stop\n',
  'data: {"type":"message_stop"}\n\n',
];

const MOCK_OPENAI_STREAM = [
  'data: {"choices":[{"delta":{"content":"hi"}}]}\n\n',
  'data: [DONE]\n\n',
];

function makeStream(lines: string[]) {
  const encoder = new TextEncoder();
  let i = 0;
  return new ReadableStream({
    pull(controller) {
      if (i < lines.length) controller.enqueue(encoder.encode(lines[i++]));
      else controller.close();
    },
  });
}

describe('issue #1195 — chatWithReconnect respects apiFormat', () => {
  beforeEach(() => {
    jest.restoreAllMocks();
    invalidateSettingsCache();
  });

  it('routes to Anthropic /v1/messages when apiFormat=anthropic', async () => {
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com/v1');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve('claude-sonnet-4-20250514');
      return Promise.resolve(null);
    });
    (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test-key');

    let capturedUrl = '';
    let capturedHeaders: Record<string, string> = {};
    const mockFetch = jest.fn().mockImplementation((url: string, init: RequestInit) => {
      capturedUrl = url;
      capturedHeaders = init.headers as Record<string, string>;
      return Promise.resolve({
        ok: true,
        body: makeStream(MOCK_ANTHROPIC_STREAM),
      });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const chunks: string[] = [];
    await chatWithReconnect(
      [{ role: 'user', content: 'hi' }],
      (chunk) => { if (chunk.content) chunks.push(chunk.content); },
    );

    expect(capturedUrl).toBe('https://api.anthropic.com/v1/messages');
    expect(capturedHeaders['x-api-key']).toBe('sk-test-key');
    expect(capturedHeaders['anthropic-version']).toBe('2023-06-01');
    expect(chunks).toContain('hello');
  });

  it('routes to OpenAI /chat/completions when apiFormat=openai', async () => {
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.openai.com/v1');
      if (key === 'cfg_api_format') return Promise.resolve('openai');
      if (key === 'cfg_model') return Promise.resolve('gpt-4');
      return Promise.resolve(null);
    });
    (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test-key');

    let capturedUrl = '';
    let capturedHeaders: Record<string, string> = {};
    const mockFetch = jest.fn().mockImplementation((url: string, init: RequestInit) => {
      capturedUrl = url;
      capturedHeaders = init.headers as Record<string, string>;
      return Promise.resolve({
        ok: true,
        body: makeStream(MOCK_OPENAI_STREAM),
      });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    const chunks: string[] = [];
    await chatWithReconnect(
      [{ role: 'user', content: 'hi' }],
      (chunk) => { if (chunk.content) chunks.push(chunk.content); },
    );

    expect(capturedUrl).toBe('https://api.openai.com/v1/chat/completions');
    expect(capturedHeaders['Authorization']).toBe('Bearer sk-test-key');
    expect(chunks).toContain('hi');
  });

  it('uses correct Anthropic body format (system separated, max_tokens)', async () => {
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve('claude-3-opus-20240229');
      return Promise.resolve(null);
    });
    (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test');

    let capturedBody: Record<string, unknown> = {};
    const mockFetch = jest.fn().mockImplementation((_url: string, init: RequestInit) => {
      capturedBody = JSON.parse(init.body as string);
      return Promise.resolve({
        ok: true,
        body: makeStream(MOCK_ANTHROPIC_STREAM),
      });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    await chatWithReconnect(
      [
        { role: 'system', content: 'You are helpful' },
        { role: 'user', content: 'hi' },
      ],
      () => {},
    );

    expect(capturedBody.system).toBe('You are helpful');
    expect(capturedBody.max_tokens).toBe(4096);
    expect(capturedBody.messages).toEqual([{ role: 'user', content: 'hi' }]);
    // System message should NOT be in messages array
    expect((capturedBody.messages as Array<{ role: string }>).every(m => m.role !== 'system')).toBe(true);
  });

  it('does NOT double /v1 when base URL already contains /v1', async () => {
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com/v1');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve('claude-sonnet-4-20250514');
      return Promise.resolve(null);
    });
    (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test');

    let capturedUrl = '';
    const mockFetch = jest.fn().mockImplementation((url: string) => {
      capturedUrl = url;
      return Promise.resolve({ ok: true, body: makeStream(MOCK_ANTHROPIC_STREAM) });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    await chatWithReconnect([{ role: 'user', content: 'hi' }], () => {});

    expect(capturedUrl).toBe('https://api.anthropic.com/v1/messages');
    expect(capturedUrl).not.toContain('/v1/v1');
  });
});

describe('issue #1195 — checkApi uses configured model', () => {
  beforeEach(() => {
    jest.restoreAllMocks();
    invalidateSettingsCache();
  });

  it('uses Anthropic GET /v1/models (free endpoint) for health check', async () => {
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve('claude-3-haiku-20240307');
      return Promise.resolve(null);
    });
    (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test');

    let capturedUrl = '';
    const mockFetch = jest.fn().mockImplementation((url: string) => {
      capturedUrl = url;
      return Promise.resolve({ ok: true, status: 200 });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    await checkApi();

    // Anthropic checkApi now uses free GET /v1/models endpoint (#3421)
    expect(capturedUrl).toBe('https://api.anthropic.com/v1/models');
  });

  it('falls back to default model when none configured', async () => {
    (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com');
      if (key === 'cfg_api_format') return Promise.resolve('anthropic');
      if (key === 'cfg_model') return Promise.resolve(null);
      return Promise.resolve(null);
    });
    (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test');

    let capturedUrl = '';
    const mockFetch = jest.fn().mockImplementation((url: string) => {
      capturedUrl = url;
      return Promise.resolve({ ok: true, status: 200 });
    });
    jest.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    await checkApi();

    // Should still use /v1/models endpoint when no model configured
    expect(capturedUrl).toContain('/v1/models');
  });
});