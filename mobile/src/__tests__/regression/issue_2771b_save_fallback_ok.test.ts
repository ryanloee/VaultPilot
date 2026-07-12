/**
 * Regression test for #2771: saveProviderKeysSecure queue resolves successfully
 * when save fails — race between error propagation and queue state.
 *
 * Test B: Verifies that when only SecureStore fails but AsyncStorage
 * fallback succeeds, the save completes without error (graceful degradation).
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

describe('#2771b: addProvider succeeds with AsyncStorage fallback', () => {
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
    AsyncStorage.setItem.mockResolvedValue(undefined);
  });

  it('succeeds when SecureStore fails but AsyncStorage works', async () => {
    SecureStore.setItemAsync.mockRejectedValue(new Error('SecureStore error'));
    // AsyncStorage remains healthy

    await expect(useAppStore.getState().addProvider({
      name: 'Test', apiBase: 'https://test.com', apiKey: 'sk-test',
      model: 'test-model', apiFormat: 'openai',
    })).resolves.toBeUndefined();

    // Provider was added to state despite SecureStore failure
    expect(useAppStore.getState().providers).toHaveLength(1);
    expect(useAppStore.getState().providers[0].name).toBe('Test');
  });
});
