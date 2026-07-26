/**
 * Regression test for #3483: settingsSync import overwrites SecureStore key with empty string.
 *
 * Bug: importSettings() used `p.apiKey !== undefined` to guard the SecureStore
 * write. An empty string '' passes this check, overwriting an existing valid
 * key with ''. The real API key is then permanently lost.
 *
 * Fix: Use truthiness check `if (p.apiKey)` so empty strings are skipped,
 * preserving any existing key in SecureStore.
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

// Mock saveSettings so importSettings doesn't hit AsyncStorage cfg_* writes.
jest.mock('../../api/client', () => ({
  saveSettings: jest.fn().mockResolvedValue(undefined),
}));

// Mock store so importSettings's dynamic import doesn't fail.
jest.mock('../../store', () => ({
  useAppStore: {
    getState: () => ({
      apiBase: '',
      apiKey: '',
      model: '',
      apiFormat: 'openai',
    }),
    setState: jest.fn(),
  },
  ApiFormat: {},
  ThemeMode: {},
  ProviderConfig: {},
  isValidThemeMode: () => true,
}));

import { importSettings } from '../../utils/settingsSync';

const mockSecureStore = SecureStore as jest.Mocked<typeof SecureStore>;
const mockAsyncStorage = AsyncStorage as jest.Mocked<typeof AsyncStorage>;

beforeEach(() => {
  jest.clearAllMocks();
  // Existing store already has a valid key for OpenAI
  mockAsyncStorage.getItem.mockResolvedValue(
    JSON.stringify({
      state: {
        themeMode: 'dark',
        accentColor: '#3B82F6',
        providers: [
          { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: '', model: 'gpt-4o', apiFormat: 'openai' },
        ],
        activeProviderIndex: 0,
      },
    })
  );
  // SecureStore already has the real key
  mockSecureStore.getItemAsync.mockResolvedValue(JSON.stringify({ OpenAI: 'sk-real-valid-key' }));
  mockAsyncStorage.setItem.mockResolvedValue(undefined);
  mockSecureStore.setItemAsync.mockResolvedValue(undefined);
});

describe('#3483: empty-string apiKey must not overwrite SecureStore', () => {
  it('should NOT overwrite existing SecureStore key when import has empty apiKey', async () => {
    const exportWithEmptyKey = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [
        // apiKey is empty string — should be SKIPPED, not overwrite
        { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: '', model: 'gpt-4o', apiFormat: 'openai' },
      ],
      activeProviderIndex: 0,
    });

    await importSettings(exportWithEmptyKey);

    // The existing key must be preserved — the merged SecureStore write
    // should still contain 'sk-real-valid-key', NOT ''.
    expect(mockSecureStore.setItemAsync).toHaveBeenCalledWith(
      'vaultpilot_provider_keys',
      expect.stringContaining('sk-real-valid-key')
    );
    // The written value must NOT contain an empty value for OpenAI
    const writtenValue = mockSecureStore.setItemAsync.mock.calls[0][1];
    const written = JSON.parse(writtenValue);
    expect(written.OpenAI).toBe('sk-real-valid-key');
    expect(written.OpenAI).not.toBe('');
  });

  it('should overwrite when import has a real (non-empty) apiKey', async () => {
    const exportWithRealKey = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [
        { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: 'sk-new-key', model: 'gpt-4o', apiFormat: 'openai' },
      ],
      activeProviderIndex: 0,
    });

    await importSettings(exportWithRealKey);

    // The new real key should overwrite the old one
    const writtenValue = mockSecureStore.setItemAsync.mock.calls[0][1];
    const written = JSON.parse(writtenValue);
    expect(written.OpenAI).toBe('sk-new-key');
  });

  it('should preserve existing keys when apiKey is undefined (omitted)', async () => {
    const exportNoKey = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [
        // apiKey omitted entirely
        { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', model: 'gpt-4o', apiFormat: 'openai' },
      ],
      activeProviderIndex: 0,
    });

    await importSettings(exportNoKey);

    // Existing key must be preserved (merge behavior from #2713)
    const writtenValue = mockSecureStore.setItemAsync.mock.calls[0][1];
    const written = JSON.parse(writtenValue);
    expect(written.OpenAI).toBe('sk-real-valid-key');
  });
});
