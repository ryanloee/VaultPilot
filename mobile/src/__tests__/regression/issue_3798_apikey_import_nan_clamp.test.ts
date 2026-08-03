/**
 * Regression test for #3798: importSettings activeProviderIndex clamp yields
 * NaN for missing/undefined field.
 *
 * Bug: `Math.max(0, Math.min(undefined, n))` === NaN (verified). If an imported
 * JSON omits `activeProviderIndex` (hand-edited or from an older exporter), the
 * NaN flowed into `data.providers[NaN]` → undefined → saveSettings skipped, and
 * Zustand received `activeProviderIndex: NaN`.
 *
 * Fix: guard with an integer check before clamping:
 *   const activeIdx = Number.isInteger(data.activeProviderIndex) ? data.activeProviderIndex : 0;
 * (Number.isInteger also rejects fractional indexes like 1.5.)
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

beforeEach(() => {
  jest.clearAllMocks();
  mockAsyncStorage.getItem.mockResolvedValue(
    JSON.stringify({
      state: {
        themeMode: 'dark',
        accentColor: '#3B82F6',
        providers: [
          { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', apiKey: '', model: 'gpt-4o', apiFormat: 'openai' },
        ],
        activeProviderIndex: 0,
      },
    })
  );
  mockAsyncStorage.setItem.mockResolvedValue(undefined);
  mockSecureStore.getItemAsync.mockResolvedValue(null);
  mockSecureStore.setItemAsync.mockResolvedValue(undefined);
  (useAppStore.getState as jest.Mock).mockReturnValue({
    apiBase: '',
    apiKey: '',
    model: '',
    apiFormat: 'openai',
  });
});

describe('#3798: activeProviderIndex clamp must not yield NaN (#3798)', () => {
  it('clamps a missing activeProviderIndex to 0 instead of NaN', async () => {
    const importWithoutIndex = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [
        { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', model: 'gpt-4o', apiFormat: 'openai' },
        { name: 'Anthropic', apiBase: 'https://api.anthropic.com', model: 'claude-sonnet', apiFormat: 'anthropic' },
      ],
      // activeProviderIndex omitted entirely → undefined
    });

    await importSettings(importWithoutIndex);

    // saveSettings must have been called (active = providers[0], not undefined)
    expect(mockSaveSettings).toHaveBeenCalledTimes(1);
    expect(mockSaveSettings.mock.calls[0][0]).toMatchObject({
      apiBase: 'https://api.openai.com/v1',
      model: 'gpt-4o',
    });

    // Zustand must receive a finite index — NOT NaN
    const setStateCall = (useAppStore.setState as jest.Mock).mock.calls[0][0];
    expect(setStateCall.activeProviderIndex).toBe(0);
    expect(Number.isNaN(setStateCall.activeProviderIndex)).toBe(false);
  });

  it('clamps a fractional activeProviderIndex (1.5) to a valid integer', async () => {
    const importWithFractionalIndex = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [
        { name: 'OpenAI', apiBase: 'https://api.openai.com/v1', model: 'gpt-4o', apiFormat: 'openai' },
        { name: 'Anthropic', apiBase: 'https://api.anthropic.com', model: 'claude-sonnet', apiFormat: 'anthropic' },
      ],
      activeProviderIndex: 1.5, // fractional → must be rejected, not passed through
    });

    await importSettings(importWithFractionalIndex);

    const setStateCall = (useAppStore.setState as jest.Mock).mock.calls[0][0];
    expect(Number.isInteger(setStateCall.activeProviderIndex)).toBe(true);
    expect(setStateCall.activeProviderIndex).toBe(0); // invalid → fall back to 0
  });
});