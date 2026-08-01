// @ts-nocheck
/**
 * Regression tests for #3696 — App.tsx initLocale() must complete BEFORE
 * the first UI render (setInitState('ready')).
 *
 * Bug: initLocale() was fire-and-forget AFTER setInitState('ready'), so the
 * first render always used the i18n default locale (zh-CN) until an unrelated
 * re-render happened. English-system users saw a Chinese UI on every cold
 * start. Tab bar labels were also hardcoded Chinese regardless of locale.
 *
 * Fix: `await initLocale()` before setInitState('ready') and migrate the
 * hardcoded tab labels to t('nav.*').
 *
 * This test renders the real App component with a stored user locale of 'en'
 * and asserts the tab bar renders the English labels on the very first render.
 */

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';

// ── Mock expo / navigation / heavy modules before importing App ──

jest.mock('expo-splash-screen', () => ({
  preventAutoHideAsync: jest.fn().mockResolvedValue(undefined),
  hideAsync: jest.fn().mockResolvedValue(undefined),
}));

jest.mock('expo-localization', () => ({
  getLocales: () => [{ languageCode: 'zh', countryCode: 'CN' }],
}));

jest.mock('react-native-safe-area-context', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    SafeAreaProvider: ({ children }: any) => React.createElement(View, null, children),
    SafeAreaView: ({ children }: any) => React.createElement(View, null, children),
  };
});

jest.mock('@react-navigation/native', () => {
  const React = require('react');
  return {
    NavigationContainer: ({ children }: any) => React.createElement(React.Fragment, null, children),
    createNavigationContainerRef: () => ({ isReady: () => false }),
  };
});

jest.mock('@react-navigation/bottom-tabs', () => {
  const React = require('react');
  const { Text } = require('react-native');
  const Navigator = ({ children }: any) => React.createElement(React.Fragment, null, children);
  // Render the tabBarLabel (if any) so tests can assert localized labels.
  const Screen = ({ options, children }: any) => {
    const label = options && options.tabBarLabel != null
      ? options.tabBarLabel
      : null;
    return React.createElement(React.Fragment, null,
      label != null ? React.createElement(Text, null, label) : null,
      children,
    );
  };
  return {
    createBottomTabNavigator: () => ({ Navigator, Screen }),
  };
});

jest.mock('@react-navigation/native-stack', () => {
  const React = require('react');
  const Navigator = ({ children }: any) => React.createElement(React.Fragment, null, children);
  const Screen = ({ children, component: Comp }: any) => {
    if (Comp) return React.createElement(Comp);
    return children || null;
  };
  return {
    createNativeStackNavigator: () => ({ Navigator, Screen }),
  };
});

// Mock all screens to keep the tree light.
jest.mock('../../screens/ChatScreen', () => () => null);
jest.mock('../../screens/SessionsScreen', () => () => null);
jest.mock('../../screens/NotesScreen', () => () => null);
jest.mock('../../screens/NoteEditorScreen', () => () => null);
jest.mock('../../screens/SearchScreen', () => () => null);
jest.mock('../../screens/SettingsScreen', () => () => null);
jest.mock('../../screens/ShareReceiveScreen', () => () => null);

// Mock DB / API / services so initApp resolves quickly without side effects.
jest.mock('../../db', () => ({
  getDb: jest.fn().mockResolvedValue({}),
  ensureDefaultTemplates: jest.fn().mockResolvedValue(undefined),
}));

jest.mock('../../api/client', () => ({
  getSettings: jest.fn().mockResolvedValue({
    apiBase: 'http://localhost',
    apiKey: '',
    model: 'gpt-4o-mini',
    apiFormat: 'openai',
  }),
}));

jest.mock('../../services/sync', () => ({
  autoSyncOnStartup: jest.fn().mockResolvedValue(undefined),
}));

jest.mock('../../services/backgroundSync', () => ({
  applyBackgroundSyncFromConfig: jest.fn().mockResolvedValue(undefined),
}));

import App from '../../../App';

describe('#3696 App cold-start locale — initLocale before first render', () => {
  beforeEach(async () => {
    await AsyncStorage.clear();
    await AsyncStorage.setItem('@vaultpilot:user-locale', 'en');
  });

  it('renders English tab bar labels on the very first render when user locale is en', async () => {
    const { findByText, queryByText } = await render(React.createElement(App));

    // With the fix, initLocale() is awaited before ready, so the first
    // render already uses the stored 'en' locale → t('nav.chat') = 'Chat'.
    expect(await findByText('Chat')).toBeTruthy();
    expect(await findByText('Notes')).toBeTruthy();
    expect(await findByText('Search')).toBeTruthy();
    expect(await findByText('Settings')).toBeTruthy();
    // Hardcoded Chinese labels must not appear (they were replaced by t()).
    expect(queryByText('对话')).toBeNull();
  });

  it('renders Chinese tab bar labels when stored locale is zh-CN', async () => {
    await AsyncStorage.setItem('@vaultpilot:user-locale', 'zh-CN');
    const { findByText, queryByText } = await render(React.createElement(App));

    expect(await findByText('笔记')).toBeTruthy();
    expect(await findByText('搜索')).toBeTruthy();
    expect(await findByText('设置')).toBeTruthy();
    expect(queryByText('Chat')).toBeNull();
  });
});
