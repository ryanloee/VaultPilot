/**
 * Regression test for issue #1274: Legacy API key migration from AsyncStorage → SecureStore.
 *
 * When a user upgrades from a version that stored the API key in AsyncStorage (plain text),
 * getApiKey() must automatically migrate the key to SecureStore and delete the old value.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

const KEYS = { apiKey: 'cfg_api_key' } as const;

// We need fresh module instances per test so the _migrated flag resets
function loadClient() {
  jest.resetModules();
  // Re-mock after resetModules
  jest.mock('@react-native-async-storage/async-storage', () => ({
    __esModule: true,
    default: {
      getItem: jest.fn().mockResolvedValue(null),
      setItem: jest.fn().mockResolvedValue(undefined),
      removeItem: jest.fn().mockResolvedValue(undefined),
    },
  }));
  jest.mock('expo-secure-store', () => ({
    __esModule: true,
    getItemAsync: jest.fn().mockResolvedValue(null),
    setItemAsync: jest.fn().mockResolvedValue(undefined),
    deleteItemAsync: jest.fn().mockResolvedValue(undefined),
  }));
  return {
    ...require('../../api/client'),
    AsyncStorage: require('@react-native-async-storage/async-storage').default,
    SecureStore: require('expo-secure-store'),
  };
}

describe('Issue #1274 — AsyncStorage → SecureStore migration', () => {
  it('migrates legacy key from AsyncStorage to SecureStore on first read', async () => {
    const { getSettings, invalidateSettingsCache, AsyncStorage: AS, SecureStore: SS } = loadClient();

    // Simulate legacy state: key exists in AsyncStorage, not in SecureStore
    (SS.getItemAsync as jest.Mock).mockResolvedValue(null);
    (AS.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === KEYS.apiKey) return Promise.resolve('sk-legacy-key');
      return Promise.resolve(null);
    });

    const s = await getSettings();

    // Key should be returned from the migration
    expect(s.apiKey).toBe('sk-legacy-key');

    // Should have written to SecureStore
    expect(SS.setItemAsync).toHaveBeenCalledWith(KEYS.apiKey, 'sk-legacy-key');

    // Should have deleted from AsyncStorage
    expect(AS.removeItem).toHaveBeenCalledWith(KEYS.apiKey);
  });

  it('skips migration when SecureStore already has the key', async () => {
    const { getSettings, AsyncStorage: AS, SecureStore: SS } = loadClient();

    (SS.getItemAsync as jest.Mock).mockResolvedValue('sk-existing');

    const s = await getSettings();

    expect(s.apiKey).toBe('sk-existing');
    // Should NOT check AsyncStorage for the legacy key
    const apiKeyCalls = (AS.getItem as jest.Mock).mock.calls.filter(
      (c: string[]) => c[0] === KEYS.apiKey,
    );
    expect(apiKeyCalls).toHaveLength(0);
    expect(AS.removeItem).not.toHaveBeenCalled();
  });

  it('returns empty string when no key exists anywhere', async () => {
    const { getSettings, AsyncStorage: AS, SecureStore: SS } = loadClient();

    (SS.getItemAsync as jest.Mock).mockResolvedValue(null);
    (AS.getItem as jest.Mock).mockResolvedValue(null);

    const s = await getSettings();

    expect(s.apiKey).toBe('');
    expect(SS.setItemAsync).not.toHaveBeenCalled();
    expect(AS.removeItem).not.toHaveBeenCalled();
  });

  it('only migrates once per app session (flag guards repeated checks)', async () => {
    const { getSettings, invalidateSettingsCache, AsyncStorage: AS, SecureStore: SS } = loadClient();

    (SS.getItemAsync as jest.Mock).mockResolvedValue(null);
    (AS.getItem as jest.Mock).mockImplementation((key: string) => {
      if (key === KEYS.apiKey) return Promise.resolve('sk-legacy');
      return Promise.resolve(null);
    });

    // First call — migration happens
    await getSettings();
    invalidateSettingsCache();

    // Second call — SecureStore now returns the key, so no migration
    (SS.getItemAsync as jest.Mock).mockResolvedValue('sk-migrated');
    await getSettings();

    // AsyncStorage.getItem for apiKey should only have been called once (during migration)
    const apiKeyCalls = (AS.getItem as jest.Mock).mock.calls.filter(
      (c: string[]) => c[0] === KEYS.apiKey,
    );
    expect(apiKeyCalls).toHaveLength(1);
  });
});
