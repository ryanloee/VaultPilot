/**
 * Regression test for issue #1338:
 * SettingsScreen.tsx split into sub-components.
 */

// Mock react-native so component imports don't crash
jest.mock('react-native', () => ({
  View: 'View',
  Text: 'Text',
  TextInput: 'TextInput',
  TouchableOpacity: 'TouchableOpacity',
  ScrollView: 'ScrollView',
  Modal: 'Modal',
  Linking: { openURL: jest.fn() },
  Platform: { OS: 'android' },
  ActivityIndicator: 'ActivityIndicator',
  Alert: { alert: jest.fn() },
  StyleSheet: { create: (s: any) => s },
}));

jest.mock('react-native-safe-area-context', () => ({
  SafeAreaView: 'SafeAreaView',
}));

jest.mock('../../store', () => ({
  useAppStore: () => ({
    isDark: false,
    accentColor: '#007AFF',
    providers: [{ name: 'Test', apiBase: 'https://test.com', apiKey: '', model: 'gpt-4', apiFormat: 'openai' }],
    activeProviderIndex: 0,
    themeMode: 'system',
    addProvider: jest.fn(),
    updateProvider: jest.fn(),
    removeProvider: jest.fn(),
    setActiveProvider: jest.fn(),
    setThemeMode: jest.fn(),
    setIsDark: jest.fn(),
    setAccentColor: jest.fn(),
  }),
  getColors: () => ({
    bg: '#FFF', bgSecondary: '#F5F5F5', text: '#000', textSecondary: '#666',
    border: '#E0E0E0', inputBg: '#F9F9F9', card: '#FFF',
  }),
  ACCENT_COLORS: [{ name: 'Blue', value: '#007AFF' }],
  PROVIDERS: [{ name: 'OpenAI', base: 'https://api.openai.com/v1', models: ['gpt-4'], format: 'openai' }],
  isValidThemeMode: () => true,
}));

jest.mock('../../api/client', () => ({
  checkApi: jest.fn(),
  getSettings: jest.fn(),
  saveSettings: jest.fn(),
}));

jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
}));

jest.mock('@react-native-async-storage/async-storage', () => ({
  getItem: jest.fn(),
  setItem: jest.fn(),
}));

jest.mock('../../utils/updateChecker', () => ({
  checkForUpdate: jest.fn(),
}));

jest.mock('../../utils/settingsSync', () => ({
  exportSettings: jest.fn(),
  importSettings: jest.fn(),
}));

jest.mock('expo-clipboard', () => ({
  setStringAsync: jest.fn(),
  getStringAsync: jest.fn(),
}));

jest.mock('../../services/sync', () => ({
  getServerConfig: jest.fn(),
  setServerConfig: jest.fn(),
  syncNotesFromServer: jest.fn(),
  getLastSyncTime: jest.fn(),
}));

import { ProviderList, ProviderEditor, ThemeSection, UpdateModal, AddProviderModal } from '../../components/settings';

describe('issue #1338 — SettingsScreen.tsx split into sub-components', () => {
  it('ProviderList is exported as a function component', () => {
    expect(typeof ProviderList).toBe('function');
    expect(ProviderList.name).toBe('ProviderList');
  });

  it('ProviderEditor is exported as a function component', () => {
    expect(typeof ProviderEditor).toBe('function');
    expect(ProviderEditor.name).toBe('ProviderEditor');
  });

  it('ThemeSection is exported as a function component', () => {
    expect(typeof ThemeSection).toBe('function');
    expect(ThemeSection.name).toBe('ThemeSection');
  });

  it('UpdateModal is exported as a function component', () => {
    expect(typeof UpdateModal).toBe('function');
    expect(UpdateModal.name).toBe('UpdateModal');
  });

  it('AddProviderModal is exported as a function component', () => {
    expect(typeof AddProviderModal).toBe('function');
    expect(AddProviderModal.name).toBe('AddProviderModal');
  });
});
