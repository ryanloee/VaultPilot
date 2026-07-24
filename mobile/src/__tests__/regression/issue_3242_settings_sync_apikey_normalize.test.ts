/**
 * Regression test for #3242
 *
 * Bug: When importing settings exported with includeKeys=false (default),
 * `apiKey` is undefined on the source objects. `importSettings` previously
 * spread `...p` directly into the Zustand `providers` array, propagating
 * `apiKey: undefined`. `ProviderConfig.apiKey` is typed as `string`
 * (required), so any downstream consumer using `.trim()` / `.length()`
 * would throw `TypeError: Cannot read properties of undefined`.
 *
 * Fix: normalize `apiKey` to `''` in the providers map inside
 * `useAppStore.setState` (mirroring the AsyncStorage write path and
 * `restoreProviderKeys` in store.ts).
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

// Mock the dynamic store import so we can capture what importSettings
// writes into Zustand. The real module is imported via `await import('../store')`
// so we must register the mock before that import resolves.
const setStateCaptured: { providers: Array<{ name: string; apiKey: string }> } = {
  providers: [],
};

const mockUseAppStore = {
  getState: jest.fn(() => ({
    apiBase: 'https://legacy.com',
    apiKey: 'legacy-key',
    model: 'legacy-model',
    apiFormat: 'openai',
  })),
  setState: jest.fn((partial: { providers?: Array<{ name: string; apiKey: string }> }) => {
    if (partial.providers) {
      setStateCaptured.providers = partial.providers;
    }
  }),
};

jest.mock('../../store', () => ({
  useAppStore: mockUseAppStore,
  // re-export types used by settingsSync.ts import
  ApiFormat: {},
  ThemeMode: {},
  ProviderConfig: {},
  // settingsSync.ts imports isValidThemeMode (#3423) — must provide it in the mock.
  isValidThemeMode: (v: string) => ['light', 'dark', 'system'].includes(v),
  __esModule: true,
}));

// Mock the api/client saveSettings to avoid hitting the network.
jest.mock('../../api/client', () => ({
  saveSettings: jest.fn().mockResolvedValue(undefined),
}));

import { importSettings } from '../../utils/settingsSync';

const mockAsyncStorage = AsyncStorage as jest.Mocked<typeof AsyncStorage>;
const mockSecureStore = SecureStore as jest.Mocked<typeof SecureStore>;

beforeEach(() => {
  jest.clearAllMocks();
  setStateCaptured.providers = [];
  // Existing store is empty/missing — the imported providers are what matter.
  mockAsyncStorage.getItem.mockResolvedValue(
    JSON.stringify({
      state: {
        themeMode: 'system',
        accentColor: '#3B82F6',
        providers: [],
        activeProviderIndex: 0,
      },
    }),
  );
  mockSecureStore.getItemAsync.mockResolvedValue('{}');
  mockAsyncStorage.setItem.mockResolvedValue(undefined);
  mockSecureStore.setItemAsync.mockResolvedValue(undefined);
});

describe('#3242: importSettings normalizes undefined apiKey to ""', () => {
  it('writes apiKey="" (not undefined) into Zustand when source omits apiKey', async () => {
    // Mirrors an export produced by `exportSettings(false)` — apiKey is
    // stripped to undefined before serialization.
    const exportedNoKeys = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#8B5CF6',
      providers: [
        { name: 'Provider1', apiBase: 'https://api1.com/v1', model: 'gpt-4o', apiFormat: 'openai' },
        { name: 'Provider2', apiBase: 'https://api2.com/v1', model: 'claude-sonnet-4-20250514', apiFormat: 'anthropic' },
      ],
      activeProviderIndex: 0,
    });

    await importSettings(exportedNoKeys);

    expect(mockUseAppStore.setState).toHaveBeenCalled();
    expect(setStateCaptured.providers).toHaveLength(2);
    for (const p of setStateCaptured.providers) {
      // Must be exactly the empty string — NOT undefined, NOT null.
      expect(p.apiKey).toBe('');
      // Regression guard: a strict-equality check distinguishes '' from undefined.
      expect(p.apiKey).not.toBeUndefined();
    }
  });

  it('still preserves real apiKey when source includes it (includeKeys=true)', async () => {
    const exportedWithKeys = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#8B5CF6',
      providers: [
        { name: 'Provider1', apiBase: 'https://api1.com/v1', apiKey: 'sk-real-123', model: 'gpt-4o', apiFormat: 'openai' },
      ],
      activeProviderIndex: 0,
    });

    await importSettings(exportedWithKeys);

    expect(setStateCaptured.providers).toHaveLength(1);
    expect(setStateCaptured.providers[0].apiKey).toBe('sk-real-123');
  });
});