/**
 * Regression tests for settings export/import (#1222).
 *
 * Verifies:
 * 1. exportSettings produces valid JSON with correct shape
 * 2. exportSettings excludes API keys by default
 * 3. exportSettings includes keys when requested
 * 4. importSettings restores settings correctly
 * 5. importSettings rejects invalid version
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

// Mock modules
jest.mock('@react-native-async-storage/async-storage', () => ({
  getItem: jest.fn(),
  setItem: jest.fn(),
}));

jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
}));

import { exportSettings, importSettings } from '../../utils/settingsSync';

const mockAsyncStorage = AsyncStorage as jest.Mocked<typeof AsyncStorage>;
const mockSecureStore = SecureStore as jest.Mocked<typeof SecureStore>;

const SAMPLE_STORE = JSON.stringify({
  state: {
    themeMode: 'dark',
    accentColor: '#8B5CF6',
    providers: [
      { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: '', model: 'gpt-4o', apiFormat: 'openai' },
      { name: 'Anthropic', apiBase: 'https://api.anthropic.com', apiKey: '', model: 'claude-sonnet-4-20250514', apiFormat: 'anthropic' },
    ],
    activeProviderIndex: 0,
  },
});

beforeEach(() => {
  jest.clearAllMocks();
  mockAsyncStorage.getItem.mockResolvedValue(SAMPLE_STORE);
  mockSecureStore.getItemAsync.mockResolvedValue(JSON.stringify(['sk-test-key-1', 'sk-test-key-2']));
});

describe('Settings Export (#1222)', () => {
  it('should export valid JSON with version 1', async () => {
    const json = await exportSettings();
    const parsed = JSON.parse(json);
    expect(parsed.version).toBe(1);
    expect(parsed.exportedAt).toBeDefined();
  });

  it('should include theme and accent color', async () => {
    const json = await exportSettings();
    const parsed = JSON.parse(json);
    expect(parsed.themeMode).toBe('dark');
    expect(parsed.accentColor).toBe('#8B5CF6');
  });

  it('should exclude API keys by default', async () => {
    const json = await exportSettings(false);
    const parsed = JSON.parse(json);
    for (const p of parsed.providers) {
      expect(p.apiKey).toBeUndefined();
    }
  });

  it('should include API keys when requested', async () => {
    const json = await exportSettings(true);
    const parsed = JSON.parse(json);
    expect(parsed.providers[0].apiKey).toBe('sk-test-key-1');
    expect(parsed.providers[1].apiKey).toBe('sk-test-key-2');
  });

  it('should export provider count', async () => {
    const json = await exportSettings();
    const parsed = JSON.parse(json);
    expect(parsed.providers).toHaveLength(2);
    expect(parsed.providers[0].name).toBe('OpenAI');
  });
});

describe('Settings Import (#1222)', () => {
  const validExport = JSON.stringify({
    version: 1,
    exportedAt: new Date().toISOString(),
    themeMode: 'light',
    accentColor: '#10B981',
    providers: [
      { name: 'TestProvider', apiBase: 'https://test.com/v1', apiKey: 'sk-imported', model: 'test-model', apiFormat: 'openai' },
    ],
    activeProviderIndex: 0,
  });

  it('should import settings and return provider count', async () => {
    mockAsyncStorage.setItem.mockResolvedValue(undefined);
    mockSecureStore.setItemAsync.mockResolvedValue(undefined);
    const result = await importSettings(validExport);
    expect(result.providersImported).toBe(1);
  });

  it('should update AsyncStorage with new settings', async () => {
    mockAsyncStorage.setItem.mockResolvedValue(undefined);
    mockSecureStore.setItemAsync.mockResolvedValue(undefined);
    await importSettings(validExport);
    expect(mockAsyncStorage.setItem).toHaveBeenCalled();
    const savedArg = mockAsyncStorage.setItem.mock.calls[0][1];
    const saved = JSON.parse(savedArg);
    expect(saved.state.themeMode).toBe('light');
    expect(saved.state.accentColor).toBe('#10B981');
  });

  it('should save API keys to SecureStore', async () => {
    mockAsyncStorage.setItem.mockResolvedValue(undefined);
    mockSecureStore.setItemAsync.mockResolvedValue(undefined);
    await importSettings(validExport);
    expect(mockSecureStore.setItemAsync).toHaveBeenCalledWith(
      'vaultpilot_provider_keys',
      expect.stringContaining('sk-imported')
    );
  });

  it('should reject unsupported version', async () => {
    const badJson = JSON.stringify({ version: 99, providers: [] });
    await expect(importSettings(badJson)).rejects.toThrow('Unsupported settings version');
  });

  it('should reject invalid JSON', async () => {
    await expect(importSettings('not json')).rejects.toThrow();
  });

  it('should clamp activeProviderIndex to valid range', async () => {
    const exportWithHighIndex = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [
        { name: 'Provider1', apiBase: 'https://api1.com', model: 'm1', apiFormat: 'openai' },
        { name: 'Provider2', apiBase: 'https://api2.com', model: 'm2', apiFormat: 'openai' },
      ],
      activeProviderIndex: 99, // Out of range
    });

    mockAsyncStorage.setItem.mockResolvedValue(undefined);
    mockSecureStore.setItemAsync.mockResolvedValue(undefined);
    await importSettings(exportWithHighIndex);

    const savedArg = mockAsyncStorage.setItem.mock.calls[0][1];
    const saved = JSON.parse(savedArg);
    // Should be clamped to last valid index (1)
    expect(saved.state.activeProviderIndex).toBe(1);
  });

  it('should handle corrupted existing store gracefully', async () => {
    // Existing store has corrupted JSON
    mockAsyncStorage.getItem.mockResolvedValueOnce('corrupted-json{{{');
    mockAsyncStorage.setItem.mockResolvedValue(undefined);
    mockSecureStore.setItemAsync.mockResolvedValue(undefined);

    const result = await importSettings(validExport);
    expect(result.providersImported).toBe(1);
    // Should still save successfully with fresh state
    expect(mockAsyncStorage.setItem).toHaveBeenCalled();
  });

  it('should not save keys to SecureStore when all keys are empty', async () => {
    const exportNoKeys = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [
        { name: 'Provider1', apiBase: 'https://api1.com', model: 'm1', apiFormat: 'openai' },
      ],
      activeProviderIndex: 0,
    });

    mockAsyncStorage.setItem.mockResolvedValue(undefined);
    await importSettings(exportNoKeys);

    // Should NOT call SecureStore when no keys are present
    expect(mockSecureStore.setItemAsync).not.toHaveBeenCalled();
  });
});
