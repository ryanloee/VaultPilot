/**
 * Regression test for #2771: saveProviderKeysSecure queue resolves successfully
 * when save fails — race between error propagation and queue state.
 *
 * Test A: Verifies that when both SecureStore and AsyncStorage fail,
 * the error propagates to the caller via addProvider rejection.
 *
 * Pattern follows the proven #2712 regression test approach.
 */

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

const ASYNC_FALLBACK_KEY = 'vaultpilot_provider_keys_backup';

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

describe('#2771a: addProvider rejects when both stores fail', () => {
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
    jest.clearAllMocks();
    SecureStore.setItemAsync.mockResolvedValue(undefined);
    setAsyncStorageResolve();
  });

  it('rejects with Critical error when both storages fail', async () => {
    SecureStore.setItemAsync.mockRejectedValue(new Error('SecureStore down'));
    setAsyncStorageRejectForFallback();

    await expect(
      useAppStore.getState().addProvider({
        name: 'Test', apiBase: 'https://test.com', apiKey: 'sk-test',
        model: 'test-model', apiFormat: 'openai',
      })
    ).rejects.toThrow(/Critical.*provider keys.*lost on next restart/);
  });
});
