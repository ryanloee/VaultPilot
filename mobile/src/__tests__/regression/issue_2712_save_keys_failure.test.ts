// Regression test for issue #2712: saveProviderKeysSecure must not silently
// swallow SecureStore + AsyncStorage write failures.
//
// IMPORTANT: We must use a smart mock for AsyncStorage.setItem because zustand's
// persist middleware also uses AsyncStorage.setItem for store persistence.
// Mocking setItem to always reject breaks zustand's persistence, causing
// unhandled rejections. Instead, we only reject when the key matches our
// fallback key ID.

jest.mock('expo-secure-store', () => ({
  setItemAsync: jest.fn(),
  getItemAsync: jest.fn(),
}));
jest.mock('@react-native-async-storage/async-storage', () => ({
  __esModule: true,
  default: {
    setItem: jest.fn(),
    getItem: jest.fn(),
  },
}));

// eslint-disable-next-line @typescript-eslint/no-var-requires
const SecureStore = require('expo-secure-store');
// eslint-disable-next-line @typescript-eslint/no-var-requires
const AsyncStorage = require('@react-native-async-storage/async-storage').default;
// eslint-disable-next-line @typescript-eslint/no-var-requires
const { useAppStore } = require('../../store');
// eslint-disable-next-line @typescript-eslint/no-var-requires
const Alert = require('react-native').Alert;

const ASYNC_FALLBACK_KEY = 'vaultpilot_provider_keys_backup';

/**
 * Setup AsyncStorage mock: resolve for zustand persistence (any key),
 * reject only for our fallback key when requested.
 */
function setAsyncStorageRejectForFallback() {
  AsyncStorage.setItem.mockImplementation((key: string, _value: string) => {
    if (key === ASYNC_FALLBACK_KEY) {
      return Promise.reject(new Error('AsyncStorage fallback write error'));
    }
    return Promise.resolve(undefined);
  });
}

function setAsyncStorageResolve() {
  AsyncStorage.setItem.mockResolvedValue(undefined);
}

describe('Issue #2712 — saveProviderKeysSecure error propagation', () => {
  beforeEach(() => {
    useAppStore.setState({
      themeMode: 'system',
      isDark: false,
      accentColor: '#3B82F6',
      apiBase: 'https://opencode.ai/zen/v1',
      apiKey: '',
      model: 'deepseek-v4-flash-free',
      apiFormat: 'openai',
      providers: [],
      activeProviderIndex: 0,
    });
    Alert.alert = jest.fn();
    jest.clearAllMocks();
    SecureStore.setItemAsync.mockResolvedValue(undefined);
    setAsyncStorageResolve();
  });

  it('does NOT throw when both SecureStore and AsyncStorage succeed', async () => {
    await expect(
      useAppStore.getState().addProvider({
        name: 'Test', apiBase: 'https://test.com', apiKey: 'sk-test',
        model: 'test-model', apiFormat: 'openai',
      })
    ).resolves.toBeUndefined();

    expect(useAppStore.getState().providers[0].apiKey).toBe('sk-test');
    expect(SecureStore.setItemAsync).toHaveBeenCalled();
    expect(AsyncStorage.setItem).toHaveBeenCalled();
  });

  it('does NOT throw when only SecureStore fails but AsyncStorage succeeds (graceful)', async () => {
    SecureStore.setItemAsync.mockRejectedValue(new Error('SecureStore write error'));

    await expect(
      useAppStore.getState().addProvider({
        name: 'Test', apiBase: 'https://test.com', apiKey: 'sk-test',
        model: 'test-model', apiFormat: 'openai',
      })
    ).resolves.toBeUndefined();

    expect(useAppStore.getState().providers[0].apiKey).toBe('sk-test');
    expect(AsyncStorage.setItem).toHaveBeenCalled();
  });

  it('THROWS when BOTH SecureStore and AsyncStorage fallback fail', async () => {
    SecureStore.setItemAsync.mockRejectedValue(new Error('SecureStore down'));
    setAsyncStorageRejectForFallback();

    await expect(
      useAppStore.getState().addProvider({
        name: 'Test', apiBase: 'https://test.com', apiKey: 'sk-test',
        model: 'test-model', apiFormat: 'openai',
      })
    ).rejects.toThrow(/Critical.*provider keys.*lost on next restart/);

    // State mutation happened synchronously (zustand set is sync)
    expect(useAppStore.getState().providers[0].apiKey).toBe('sk-test');
  });

  it('THROWS when both stores fail during updateProvider', async () => {
    SecureStore.setItemAsync.mockResolvedValue(undefined);
    setAsyncStorageResolve();
    await useAppStore.getState().addProvider({
      name: 'Test', apiBase: 'https://test.com', apiKey: 'sk-test',
      model: 'test-model', apiFormat: 'openai',
    });

    SecureStore.setItemAsync.mockRejectedValue(new Error('SecureStore down'));
    setAsyncStorageRejectForFallback();

    await expect(
      useAppStore.getState().updateProvider(0, { apiKey: 'sk-new-key' })
    ).rejects.toThrow(/Critical.*provider keys.*lost on next restart/);

    expect(useAppStore.getState().providers[0].apiKey).toBe('sk-new-key');
  });

  it('THROWS when both stores fail during removeProvider', async () => {
    SecureStore.setItemAsync.mockResolvedValue(undefined);
    setAsyncStorageResolve();
    await useAppStore.getState().addProvider({
      name: 'A', apiBase: 'https://a.com', apiKey: 'sk-a', model: 'model-a', apiFormat: 'openai',
    });
    await useAppStore.getState().addProvider({
      name: 'B', apiBase: 'https://b.com', apiKey: 'sk-b', model: 'model-b', apiFormat: 'openai',
    });

    SecureStore.setItemAsync.mockRejectedValue(new Error('SecureStore down'));
    setAsyncStorageRejectForFallback();

    await expect(
      useAppStore.getState().removeProvider(0)
    ).rejects.toThrow(/Critical.*provider keys.*lost on next restart/);

    expect(useAppStore.getState().providers).toHaveLength(1);
    expect(useAppStore.getState().providers[0].name).toBe('B');
  });

  it('graceful: SecureStore down does NOT throw when AsyncStorage is healthy', async () => {
    SecureStore.setItemAsync.mockRejectedValue(new Error('SecureStore unavailable'));
    // AsyncStorage stays healthy (default from beforeEach)

    await expect(useAppStore.getState().addProvider({
      name: 'A', apiBase: 'https://a.com', apiKey: 'sk-a', model: 'm', apiFormat: 'openai',
    })).resolves.toBeUndefined();

    await expect(useAppStore.getState().addProvider({
      name: 'B', apiBase: 'https://b.com', apiKey: 'sk-b', model: 'm', apiFormat: 'openai',
    })).resolves.toBeUndefined();

    await expect(useAppStore.getState().updateProvider(0, { model: 'gpt-4' }))
      .resolves.toBeUndefined();

    expect(useAppStore.getState().providers).toHaveLength(2);
    expect(useAppStore.getState().providers[0].model).toBe('gpt-4');
  });
});