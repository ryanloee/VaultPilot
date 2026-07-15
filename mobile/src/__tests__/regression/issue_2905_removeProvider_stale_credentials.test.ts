/**
 * Regression test for #2905: removeProvider leaves stale API credentials
 * in flat fields when last provider removed.
 *
 * When the last provider is removed, apiBase/apiKey/model/apiFormat must
 * be reset to defaults instead of retaining the removed provider's values.
 */

// Mock expo-secure-store to avoid native module issues.
jest.mock('expo-secure-store', () => ({
  setItemAsync: jest.fn().mockResolvedValue(undefined),
  getItemAsync: jest.fn().mockResolvedValue(null),
  deleteItemAsync: jest.fn().mockResolvedValue(undefined),
}));

import { useAppStore } from '../../store';

beforeEach(() => {
  // Reset to clean state
  useAppStore.setState({
    providers: [],
    activeProviderIndex: -1,
    apiBase: '',
    apiKey: '',
    model: '',
    apiFormat: 'openai',
  } as any);
});

/** Set up state with N providers, all pointing at index 0. */
function seedProviders(n: number) {
  const providers = Array.from({ length: n }, (_, i) => ({
    name: `P${i}`,
    apiBase: i === 0 ? 'https://api0.example.com' : `https://api${i}.example.com`,
    apiKey: i === 0 ? 'key-0' : `key-${i}`,
    model: i === 0 ? 'model-0' : `model-${i}`,
    apiFormat: 'openai' as const,
  }));
  useAppStore.setState({
    providers,
    activeProviderIndex: 0,
    apiBase: providers[0].apiBase,
    apiKey: providers[0].apiKey,
    model: providers[0].model,
    apiFormat: providers[0].apiFormat,
  } as any);
}

describe('#2905 removeProvider clears flat fields on last provider removal', () => {
  it('resets apiBase/apiKey/model/apiFormat when last provider removed', async () => {
    seedProviders(1);

    await useAppStore.getState().removeProvider(0);

    const s = useAppStore.getState();
    expect(s.providers).toHaveLength(0);
    expect(s.activeProviderIndex).toBe(-1);
    expect(s.apiBase).toBe('');
    expect(s.apiKey).toBe('');
    expect(s.model).toBe('');
    expect(s.apiFormat).toBe('openai');
  });

  it('keeps flat fields pointing to active provider when other providers remain', async () => {
    seedProviders(3);

    // Remove index 2 (third provider); activeProviderIndex stays 0
    await useAppStore.getState().removeProvider(2);

    const s = useAppStore.getState();
    expect(s.providers).toHaveLength(2);
    expect(s.activeProviderIndex).toBe(0);
    expect(s.apiBase).toBe('https://api0.example.com');
    expect(s.apiKey).toBe('key-0');
  });
});
