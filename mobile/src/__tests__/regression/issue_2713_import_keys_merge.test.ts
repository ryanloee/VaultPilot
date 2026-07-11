/**
 * Regression test for issue #2713:
 * importSettings replaces entire SecureStore, losing unlisted provider keys.
 *
 * Root cause: importSettings constructed a fresh keys Record and called
 * SecureStore.setItemAsync without first reading/merging existing keys.
 * If the device already had keys for providers A, B, C and the import only
 * contains A, B, then C's key was permanently lost.
 *
 * Fix: read existing SecureStore content before constructing the merged keys
 * object, using spread {...existingKeys, ...importedKeys}.
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

jest.mock('../../api/client', () => ({
  saveSettings: jest.fn(),
}));

import { importSettings } from '../../utils/settingsSync';
import { useAppStore } from '../../store';

const mockAsyncStorage = AsyncStorage as jest.Mocked<typeof AsyncStorage>;
const mockSecureStore = SecureStore as jest.Mocked<typeof SecureStore>;

const IMPORT_TWO_PROVIDERS = JSON.stringify({
  version: 1,
  exportedAt: new Date().toISOString(),
  themeMode: 'light',
  accentColor: '#3B82F6',
  providers: [
    { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: 'sk-imported-openai', model: 'gpt-4o', apiFormat: 'openai' },
    { name: 'Anthropic', apiBase: 'https://api.anthropic.com', apiKey: 'sk-imported-claude', model: 'claude-sonnet-4-20250514', apiFormat: 'anthropic' },
  ],
  activeProviderIndex: 0,
});

beforeEach(() => {
  jest.clearAllMocks();
  mockAsyncStorage.getItem.mockResolvedValue(null);
  mockAsyncStorage.setItem.mockResolvedValue(undefined);
  mockSecureStore.getItemAsync.mockResolvedValue(null);
  mockSecureStore.setItemAsync.mockResolvedValue(undefined);
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

describe('importSettings merges existing SecureStore keys (#2713)', () => {
  it('preserves keys for providers not present in the import data', async () => {
    // Simulate existing SecureStore with 3 providers: OpenAI, Anthropic, Groq
    const existingKeys = JSON.stringify({
      OpenAI: 'sk-existing-openai',
      Anthropic: 'sk-existing-claude',
      Groq: 'sk-groq-legacy',
    });
    mockSecureStore.getItemAsync.mockResolvedValue(existingKeys);

    await importSettings(IMPORT_TWO_PROVIDERS);

    // SecureStore.setItemAsync should receive merged keys:
    // Groq key preserved, OpenAI+Anthropic overwritten by import values
    const setItemCalls = mockSecureStore.setItemAsync.mock.calls;
    expect(setItemCalls.length).toBeGreaterThanOrEqual(1);

    const [storeKey, storedJson] = setItemCalls[0];
    expect(storeKey).toBe('vaultpilot_provider_keys');

    const merged = JSON.parse(storedJson as string);
    // Imported providers get their new keys
    expect(merged.OpenAI).toBe('sk-imported-openai');
    expect(merged.Anthropic).toBe('sk-imported-claude');
    // Groq — not in import — must survive
    expect(merged.Groq).toBe('sk-groq-legacy');
  });

  it('does not crash when existing SecureStore is corrupt JSON', async () => {
    mockSecureStore.getItemAsync.mockResolvedValue('{not valid json!!!');

    await importSettings(IMPORT_TWO_PROVIDERS);

    // Should still succeed — import keys become the whole store
    const setItemCalls = mockSecureStore.setItemAsync.mock.calls;
    expect(setItemCalls.length).toBeGreaterThanOrEqual(1);
    const merged = JSON.parse(setItemCalls[0][1] as string);
    expect(merged.OpenAI).toBe('sk-imported-openai');
    expect(merged.Anthropic).toBe('sk-imported-claude');
  });

  it('still imports keys when there is no existing SecureStore content', async () => {
    // No existing keys — fresh import
    mockSecureStore.getItemAsync.mockResolvedValue(null);

    await importSettings(IMPORT_TWO_PROVIDERS);

    const setItemCalls = mockSecureStore.setItemAsync.mock.calls;
    expect(setItemCalls.length).toBeGreaterThanOrEqual(1);
    const merged = JSON.parse(setItemCalls[0][1] as string);
    expect(merged.OpenAI).toBe('sk-imported-openai');
    expect(merged.Anthropic).toBe('sk-imported-claude');
    // No spurious extra keys
    expect(Object.keys(merged)).toHaveLength(2);
  });
});