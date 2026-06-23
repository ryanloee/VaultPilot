/**
 * Regression test for #1415: settingsSync.ts JSON.parse unprotected.
 *
 * importSettings and exportSettings must handle corrupted JSON gracefully
 * instead of throwing raw SyntaxError.
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

const mockGetItem = AsyncStorage.getItem as jest.MockedFunction<typeof AsyncStorage.getItem>;
const mockSetItem = AsyncStorage.setItem as jest.MockedFunction<typeof AsyncStorage.setItem>;

import { importSettings, exportSettings } from '../../utils/settingsSync';

describe('settingsSync JSON.parse safety (#1415)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  test('importSettings rejects invalid JSON with friendly error', async () => {
    await expect(importSettings('not json')).rejects.toThrow('导入数据格式无效');
  });

  test('importSettings rejects truncated JSON', async () => {
    await expect(importSettings('{"version":1,"providers":[')).rejects.toThrow('导入数据格式无效');
  });

  test('importSettings rejects unsupported version', async () => {
    const validButWrongVersion = JSON.stringify({ version: 99, providers: [] });
    await expect(importSettings(validButWrongVersion)).rejects.toThrow('Unsupported settings version');
  });

  test('importSettings accepts valid settings JSON', async () => {
    mockGetItem.mockResolvedValue(null);
    mockSetItem.mockResolvedValue(undefined);
    (SecureStore.setItemAsync as jest.Mock).mockResolvedValue(undefined);

    const valid = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [{ name: 'Test', apiBase: 'https://api.test.com', model: 'gpt-4', apiFormat: 'openai' }],
      activeProviderIndex: 0,
    });
    const result = await importSettings(valid);
    expect(result.providersImported).toBe(1);
  });

  test('exportSettings throws friendly error on corrupted AsyncStorage', async () => {
    mockGetItem.mockResolvedValue('corrupted{json');
    await expect(exportSettings()).rejects.toThrow('设置数据已损坏');
  });

  test('importSettings handles corrupted existing store gracefully', async () => {
    mockGetItem.mockResolvedValue('corrupted{json');
    mockSetItem.mockResolvedValue(undefined);
    (SecureStore.setItemAsync as jest.Mock).mockResolvedValue(undefined);

    const valid = JSON.stringify({
      version: 1,
      exportedAt: new Date().toISOString(),
      themeMode: 'dark',
      accentColor: '#3B82F6',
      providers: [{ name: 'Test', apiBase: 'https://api.test.com', model: 'gpt-4', apiFormat: 'openai' }],
      activeProviderIndex: 0,
    });
    // Should not throw — falls back to empty store
    const result = await importSettings(valid);
    expect(result.providersImported).toBe(1);
  });
});
