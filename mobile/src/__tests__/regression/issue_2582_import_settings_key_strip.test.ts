/**
 * Regression test for issue #2582:
 * importSettings strips API keys from in-memory providers — switching provider
 * after import loses key.
 *
 * Root cause: importSettings() explicitly set apiKey: '' on all providers when
 * syncing the in-memory Zustand store, even though keys were present in the
 * import data. The persist middleware already strips keys for AsyncStorage via
 * partialize → sanitizeForPersistence, so keeping them in memory is safe.
 *
 * After the fix, providers in the in-memory store retain their apiKey from the
 * import data, and setActiveProvider() correctly populates the legacy flat
 * apiKey field.
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

// Mock api/client saveSettings to avoid AsyncStorage side effects during import
jest.mock('../../api/client', () => ({
  saveSettings: jest.fn(),
}));

import { importSettings } from '../../utils/settingsSync';
import { useAppStore } from '../../store';

const mockAsyncStorage = AsyncStorage as jest.Mocked<typeof AsyncStorage>;
const mockSecureStore = SecureStore as jest.Mocked<typeof SecureStore>;

const IMPORT_WITH_TWO_KEYS = JSON.stringify({
  version: 1,
  exportedAt: new Date().toISOString(),
  themeMode: 'dark',
  accentColor: '#3B82F6',
  providers: [
    { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: 'sk-key-a', model: 'gpt-4o', apiFormat: 'openai' },
    { name: 'Anthropic', apiBase: 'https://api.anthropic.com', apiKey: 'sk-key-b', model: 'claude-sonnet-4-20250514', apiFormat: 'anthropic' },
  ],
  activeProviderIndex: 0,
});

beforeEach(() => {
  jest.clearAllMocks();
  mockAsyncStorage.getItem.mockResolvedValue(null);
  mockAsyncStorage.setItem.mockResolvedValue(undefined);
  mockSecureStore.getItemAsync.mockResolvedValue(null);
  mockSecureStore.setItemAsync.mockResolvedValue(undefined);
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

describe('importSettings preserves API keys in memory (#2582)', () => {
  it('does NOT strip apiKey from in-memory providers after import', async () => {
    await importSettings(IMPORT_WITH_TWO_KEYS);

    const providers = useAppStore.getState().providers;
    expect(providers).toHaveLength(2);
    // Both providers must retain their keys — this is the regression: previously
    // these were '' because importSettings explicitly stripped them.
    expect(providers[0].apiKey).toBe('sk-key-a');
    expect(providers[1].apiKey).toBe('sk-key-b');
  });

  it('allows setActiveProvider to switch to a non-active provider without losing key', async () => {
    await importSettings(IMPORT_WITH_TWO_KEYS);

    // Switch to the second (non-active) provider
    useAppStore.getState().setActiveProvider(1);

    // The legacy flat apiKey must now reflect the second provider's key.
    // Before the fix this was '' because the provider's apiKey was stripped.
    expect(useAppStore.getState().apiKey).toBe('sk-key-b');
    expect(useAppStore.getState().activeProviderIndex).toBe(1);
  });

  it('populates the active provider key into legacy flat apiKey on import', async () => {
    await importSettings(IMPORT_WITH_TWO_KEYS);

    // Active provider is index 0 → legacy apiKey should be sk-key-a
    expect(useAppStore.getState().apiKey).toBe('sk-key-a');
  });

  it('still persists providers with keys stripped to AsyncStorage (persistence layer)', async () => {
    await importSettings(IMPORT_WITH_TWO_KEYS);

    // The AsyncStorage write (from persist + importSettings's explicit write)
    // must have stripped keys — keys go to SecureStore only.
    const setItemCalls = mockAsyncStorage.setItem.mock.calls;
    const persistedJson = setItemCalls
      .map(([, value]) => value as string)
      .find(v => v.includes('providers'));
    expect(persistedJson).toBeDefined();
    const persisted = JSON.parse(persistedJson!);
    const persistedState = persisted.state ?? persisted;
    for (const p of persistedState.providers) {
      expect(p.apiKey).toBe('');
    }
  });
});
