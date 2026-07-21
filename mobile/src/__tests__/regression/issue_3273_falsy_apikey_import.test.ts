/**
 * Regression test for issue #3273:
 * importSettings falsy check on apiKey prevents clearing keys (settingsSync.ts).
 *
 * Root cause: `if (p.apiKey)` at lines 129 and 154 treats empty string ""
 * the same as undefined, so when an import explicitly sets apiKey to ""
 * (to clear a key), the SecureStore key is preserved and the saveSettings
 * call omits apiKey, preventing the clearing from taking effect.
 *
 * Fix: change both checks to `if (p.apiKey !== undefined)` so that ""
 * is treated as an intentional value (clearing the key).
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

// Mock AsyncStorage
jest.mock('@react-native-async-storage/async-storage', () => ({
  getItem: jest.fn(),
  setItem: jest.fn(),
}));

// Mock SecureStore
jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
}));

// Mock api/client saveSettings
jest.mock('../../api/client', () => ({
  saveSettings: jest.fn(),
}));

import { importSettings } from '../../utils/settingsSync';
import { useAppStore } from '../../store';
import { saveSettings } from '../../api/client';

const mockAsyncStorage = AsyncStorage as jest.Mocked<typeof AsyncStorage>;
const mockSecureStore = SecureStore as jest.Mocked<typeof SecureStore>;
const mockSaveSettings = saveSettings as jest.MockedFunction<typeof saveSettings>;

/** Import data where active provider explicitly clears its key (apiKey: ""). */
const IMPORT_WITH_EMPTY_KEY = JSON.stringify({
  version: 1,
  exportedAt: new Date().toISOString(),
  themeMode: 'dark',
  accentColor: '#3B82F6',
  providers: [
    { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: '', model: 'gpt-4o', apiFormat: 'openai' },
  ],
  activeProviderIndex: 0,
});

/** Export data where the only provider has no apiKey field at all (undefined, from includeKeys=false). */
const IMPORT_WITHOUT_APIKEY_FIELD = JSON.stringify({
  version: 1,
  exportedAt: new Date().toISOString(),
  themeMode: 'dark',
  accentColor: '#3B82F6',
  providers: [
    { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', model: 'gpt-4o', apiFormat: 'openai' },
  ],
  activeProviderIndex: 0,
});

beforeEach(() => {
  jest.clearAllMocks();
  mockAsyncStorage.getItem.mockResolvedValue(null);
  mockAsyncStorage.setItem.mockResolvedValue(undefined);
  mockSecureStore.getItemAsync.mockResolvedValue(null);
  mockSecureStore.setItemAsync.mockResolvedValue(undefined);
  mockSaveSettings.mockResolvedValue(undefined);
  // Reset store to clean defaults
  useAppStore.setState({
    themeMode: 'system',
    isDark: false,
    accentColor: '#3B82F6',
    apiBase: '',
    apiKey: '',
    model: '',
    apiFormat: 'openai' as const,
    providers: [],
    activeProviderIndex: 0,
  });
});

describe('importSettings allows clearing keys with empty string (#3273)', () => {
  it('writes empty-string apiKey to SecureStore when import has apiKey: ""', async () => {
    await importSettings(IMPORT_WITH_EMPTY_KEY);

    // SecureStore must have been written with the empty key (""), not skipped.
    const setCalls = mockSecureStore.setItemAsync.mock.calls;
    expect(setCalls.length).toBeGreaterThan(0);

    const keysArg = setCalls.find(c => c[0] === 'vaultpilot_provider_keys');
    expect(keysArg).toBeDefined();
    const keysRecord = JSON.parse(keysArg![1] as string);
    expect(keysRecord['OpenAI']).toBe('');
  });

  it('calls saveSettings with apiKey: "" when import has apiKey: ""', async () => {
    await importSettings(IMPORT_WITH_EMPTY_KEY);

    // saveSettings must be called WITH apiKey set to "" (explicit clear).
    // Before the fix, the falsy check skipped this entirely.
    expect(mockSaveSettings).toHaveBeenCalled();
    const callArg = mockSaveSettings.mock.calls[0][0];
    expect(callArg.apiKey).toBe('');
  });

  it('skips saveSettings apiKey when apiKey is undefined (no field at all)', async () => {
    await importSettings(IMPORT_WITHOUT_APIKEY_FIELD);

    // When the provider has NO apiKey field (undefined), saveSettings
    // should NOT include apiKey — to preserve whatever the device already has.
    expect(mockSaveSettings).toHaveBeenCalled();
    const callArg = mockSaveSettings.mock.calls[0][0];
    expect(callArg.apiKey).toBeUndefined();
  });
});