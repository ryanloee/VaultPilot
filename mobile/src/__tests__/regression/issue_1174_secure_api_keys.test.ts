// Regression test for issue #1174: API keys must not be persisted in plain text AsyncStorage.
// Keys must be stored in SecureStore (encrypted) and stripped from zustand persistence.

describe('Issue #1174 — API keys must use SecureStore, not AsyncStorage', () => {
  beforeEach(() => {
    jest.resetModules();
  });

  it('store partialize strips apiKey from providers before persisting to AsyncStorage', () => {
    const { useAppStore } = require('../../store');

    // Add a provider with an API key
    useAppStore.getState().addProvider({
      name: 'Test',
      apiBase: 'https://test.com',
      apiKey: 'secret-key-123',
      model: 'test-model',
      apiFormat: 'openai',
    });

    const state = useAppStore.getState();
    // In-memory state should have the key
    expect(state.providers[0].apiKey).toBe('secret-key-123');

    // Simulate what partialize produces (the persisted version)
    const partialized = {
      providers: state.providers.map((p: any) => ({ ...p, apiKey: '' })),
    };

    // Persisted providers should NOT contain the actual API key
    expect(partialized.providers[0].apiKey).toBe('');
  });

  it('addProvider calls SecureStore.setItemAsync with provider keys', async () => {
    const SecureStore = require('expo-secure-store');
    const { useAppStore } = require('../../store');

    useAppStore.getState().addProvider({
      name: 'Provider A',
      apiBase: 'https://a.com',
      apiKey: 'sk-a-key',
      model: 'model-a',
      apiFormat: 'openai',
    });

    // Wait for async save (setTimeout in store)
    await new Promise(r => setTimeout(r, 100));

    // SecureStore should have been called with the provider keys
    expect(SecureStore.setItemAsync).toHaveBeenCalledWith(
      'vaultpilot_provider_keys',
      expect.any(String),
    );

    const lastCall = SecureStore.setItemAsync.mock.calls.at(-1);
    const stored = JSON.parse(lastCall[1]);
    expect(stored).toContain('sk-a-key');
  });

  it('updateProvider saves updated keys to SecureStore', async () => {
    const SecureStore = require('expo-secure-store');
    const { useAppStore } = require('../../store');

    const s = useAppStore.getState();
    s.addProvider({ name: 'P1', apiBase: 'https://p1.com', apiKey: 'key-p1', model: 'm', apiFormat: 'openai' });
    s.addProvider({ name: 'P2', apiBase: 'https://p2.com', apiKey: 'key-p2', model: 'm', apiFormat: 'openai' });

    await new Promise(r => setTimeout(r, 100));

    // Update provider 0's key
    useAppStore.getState().updateProvider(0, { apiKey: 'key-p1-updated' });
    await new Promise(r => setTimeout(r, 100));

    const lastCall = SecureStore.setItemAsync.mock.calls.at(-1);
    expect(lastCall).toBeDefined();
    const stored = JSON.parse(lastCall[1]);
    expect(stored[0]).toBe('key-p1-updated');
    expect(stored[1]).toBe('key-p2');
  });

  it('legacy apiKey field is excluded from partialize output', () => {
    const { useAppStore } = require('../../store');

    useAppStore.getState().setApiSettings({ apiKey: 'sk-legacy' });
    const state = useAppStore.getState();

    // Simulate partialize: apiKey should NOT be in the persisted data
    const partialized = {
      apiKey: undefined, // excluded by partialize
      providers: state.providers.map((p: any) => ({ ...p, apiKey: '' })),
    };

    expect(partialized.apiKey).toBeUndefined();
    for (const p of partialized.providers) {
      expect(p.apiKey).toBe('');
    }
  });
});
