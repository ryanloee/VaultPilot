/**
 * Regression test for issue #3273 / #3483:
 * importSettings apiKey handling in settingsSync.ts.
 *
 * History:
 *   #3273 originally changed `if (p.apiKey)` → `if (p.apiKey !== undefined)`
 *   so that an explicit `apiKey: ""` would clear SecureStore keys on import.
 *   However #3483 revealed this causes data loss: an export with includeKeys=true
 *   from a device that has NO key produces `apiKey: ""`, and importing that on
 *   a device WITH a real key would silently overwrite it with "".
 *
 * Resolution: truthiness check (`if (p.apiKey)`) is the correct behavior.
 * An empty-string apiKey in an export means "no key on source device", NOT an
 * intentional clear. Empty/undefined apiKeys are both skipped to preserve
 * existing SecureStore keys. Users who want to clear a key should do so via UI.
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

/** Import data where the provider has apiKey: "" (e.g. exported from a device with no key set). */
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

describe('importSettings skips empty-string apiKey to preserve existing keys (#3273 / #3483)', () => {
  it('does NOT write to SecureStore when import has apiKey: "" (no existing keys)', async () => {
    await importSettings(IMPORT_WITH_EMPTY_KEY);

    // #3483: An empty-string apiKey means "no key on source device", NOT an
    // intentional clear. It must be skipped to avoid overwriting existing
    // SecureStore keys with "" on devices that DO have a real key.
    // When there are no existing keys either, SecureStore.setItemAsync should
    // not be called at all (nothing to write).
    const setCalls = mockSecureStore.setItemAsync.mock.calls;
    const keysCalls = setCalls.filter(c => c[0] === 'vaultpilot_provider_keys');
    expect(keysCalls.length).toBe(0);
  });

  it('does NOT include apiKey in saveSettings when import has apiKey: ""', async () => {
    await importSettings(IMPORT_WITH_EMPTY_KEY);

    // #3483: empty-string apiKey is skipped (truthiness check), so
    // saveSettings must NOT include apiKey — preserving existing cfg_* value.
    expect(mockSaveSettings).toHaveBeenCalled();
    const callArg = mockSaveSettings.mock.calls[0][0];
    expect(callArg.apiKey).toBeUndefined();
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