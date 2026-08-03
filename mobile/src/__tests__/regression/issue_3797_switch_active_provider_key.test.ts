/**
 * Regression test for #3797: importSettings keeps OLD provider apiKey when the
 * import switches the active provider (key/base mismatch).
 *
 * Bug: `active.apiKey` comes from the import payload, which is `undefined` for
 * includeKeys=false exports. When the import switches activeProviderIndex to a
 * different provider, importSettings fell back to `fresh.apiKey` (the old
 * provider's key), leaving the NEW provider's apiBase/model paired with the OLD
 * provider's key → wrong endpoint + key pair → auth failure.
 *
 * Fix: resolve the active provider's key from the merged SecureStore record
 * (`keys[active.name]`) instead of the payload:
 *   const activeKey = keys[active.name ?? ''] ?? active.apiKey;
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

// Capture saveSettings calls so we can assert the cfg_api_key written.
jest.mock('../../api/client', () => ({
  saveSettings: jest.fn().mockResolvedValue(undefined),
}));

jest.mock('../../store', () => ({
  useAppStore: {
    getState: jest.fn(),
    setState: jest.fn(),
  },
  ApiFormat: {},
  ThemeMode: {},
  ProviderConfig: {},
  isValidThemeMode: () => true,
}));

import { importSettings } from '../../utils/settingsSync';
import { saveSettings } from '../../api/client';
import { useAppStore } from '../../store';

const mockSecureStore = SecureStore as jest.Mocked<typeof SecureStore>;
const mockAsyncStorage = AsyncStorage as jest.Mocked<typeof AsyncStorage>;
const mockSaveSettings = saveSettings as jest.MockedFunction<typeof saveSettings>;

// Device currently active on OpenAI (K_A); Anthropic also has a real key K_B.
const IMPORT_SWITCH_TO_ANTHROPIC = JSON.stringify({
  version: 1,
  exportedAt: new Date().toISOString(),
  themeMode: 'dark',
  accentColor: '#3B82F6',
  providers: [
    // includeKeys=false export → apiKey omitted entirely
    { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', model: 'gpt-4o', apiFormat: 'openai' },
    { name: 'Anthropic', apiBase: 'https://api.anthropic.com', model: 'claude-sonnet', apiFormat: 'anthropic' },
  ],
  activeProviderIndex: 1, // switch active to Anthropic (B)
});

beforeEach(() => {
  jest.clearAllMocks();
  // Existing app store: active = OpenAI with key K_A (old provider)
  mockAsyncStorage.getItem.mockResolvedValue(
    JSON.stringify({
      state: {
        themeMode: 'dark',
        accentColor: '#3B82F6',
        providers: [
          { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: '', model: 'gpt-4o', apiFormat: 'openai' },
          { name: 'Anthropic', apiBase: 'https://api.anthropic.com', apiKey: '', model: 'claude-sonnet', apiFormat: 'anthropic' },
        ],
        activeProviderIndex: 0,
      },
    })
  );
  mockAsyncStorage.setItem.mockResolvedValue(undefined);

  // Device SecureStore holds real keys for both providers.
  mockSecureStore.getItemAsync.mockResolvedValue(
    JSON.stringify({ OpenAI: 'K_A', Anthropic: 'K_B' })
  );
  mockSecureStore.setItemAsync.mockResolvedValue(undefined);

  // Fresh in-memory state reflects old active provider A (key K_A).
  (useAppStore.getState as jest.Mock).mockReturnValue({
    apiBase: 'https://api.openai.com/v1',
    apiKey: 'K_A',
    model: 'gpt-4o',
    apiFormat: 'openai',
  });
});

describe('#3797: switch active provider must resolve the NEW provider key (#3797)', () => {
  it('writes the new active provider key (K_B) to cfg_* via saveSettings, not the old K_A', async () => {
    await importSettings(IMPORT_SWITCH_TO_ANTHROPIC);

    // The cfg_* sync must pair the NEW provider (Anthropic) with ITS key.
    const saveCall = mockSaveSettings.mock.calls[0][0];
    expect(saveCall).toMatchObject({
      apiBase: 'https://api.anthropic.com',
      model: 'claude-sonnet',
      apiFormat: 'anthropic',
    });
    expect(saveCall.apiKey).toBe('K_B'); // not undefined, not K_A
    expect(saveCall.apiKey).not.toBe('K_A');
  });

  it('writes the new provider key into Zustand state via setState, not fresh.apiKey (old K_A)', async () => {
    await importSettings(IMPORT_SWITCH_TO_ANTHROPIC);

    const setStateCall = (useAppStore.setState as jest.Mock).mock.calls[0][0];
    expect(setStateCall.activeProviderIndex).toBe(1);
    expect(setStateCall.apiBase).toBe('https://api.anthropic.com');
    expect(setStateCall.apiKey).toBe('K_B'); // new key paired with new base
    expect(setStateCall.apiKey).not.toBe('K_A');
  });
});