// @ts-nocheck
/**
 * Regression tests for #3696: cold start must show the correct locale.
 *
 * Previously App.tsx called initLocale() fire-and-forget AFTER
 * setInitState('ready'), so the first render always used the default
 * zh-CN locale — English-system users saw a Chinese UI on every cold
 * start until an unrelated re-render happened. The fix awaits
 * initLocale() BEFORE 'ready', and setLocale() bumps the store's
 * localeVersion so mounted locale-sensitive UI re-renders.
 */

// Mock all heavy native/Expo deps before importing App.
jest.mock('react-native', () => {
  const actual = jest.requireActual('react-native');
  return {
    ...actual,
    Appearance: { getColorScheme: () => 'light' },
  };
});

jest.mock('expo-splash-screen', () => ({
  preventAutoHideAsync: jest.fn().mockResolvedValue(undefined),
  hideAsync: jest.fn().mockResolvedValue(undefined),
}));

jest.mock('expo-localization', () => ({
  getLocales: () => [{ languageCode: 'en' }],
}));

jest.mock('react-native-safe-area-context', () => {
  const React = require('react');
  const SafeAreaProvider = ({ children }: any) => React.createElement(React.Fragment, null, children);
  const SafeAreaView = ({ children }: any) => React.createElement(React.Fragment, null, children);
  return { SafeAreaProvider, SafeAreaView };
});

jest.mock('@react-navigation/native', () => {
  const React = require('react');
  return {
    NavigationContainer: ({ children }: any) => React.createElement(React.Fragment, null, children),
    createNativeStackNavigator: () => {
      const Navigator = ({ children }: any) => React.createElement(React.Fragment, null, children);
      const Screen = ({ component: Component, children }: any) =>
        Component ? React.createElement(Component) : children ?? null;
      return { Navigator, Screen };
    },
  };
});

jest.mock('@react-navigation/native-stack', () => {
  const React = require('react');
  const createNativeStackNavigator = () => {
    const Navigator = ({ children }: any) => React.createElement(React.Fragment, null, children);
    const Screen = ({ component: Component, children }: any) =>
      Component ? React.createElement(Component) : children ?? null;
    return { Navigator, Screen };
  };
  return { createNativeStackNavigator };
});

jest.mock('@react-navigation/bottom-tabs', () => {
  const React = require('react');
  const { Text } = require('react-native');
  const createBottomTabNavigator = () => {
    const Navigator = ({ children }: any) => React.createElement(React.Fragment, null, children);
    const Screen = ({ children, component: Component, options }: any) =>
      React.createElement(
        'TabScreen',
        { label: options?.tabBarLabel },
        React.createElement(Text, null, options?.tabBarLabel),
        Component ? React.createElement(Component) : children ?? null
      );
    return { Navigator, Screen };
  };
  return { createBottomTabNavigator };
});

jest.mock('../../db', () => ({
  getDb: jest.fn().mockResolvedValue({}),
  ensureDefaultTemplates: jest.fn().mockResolvedValue(undefined),
}));

jest.mock('../../api/client', () => ({
  getSettings: jest.fn().mockResolvedValue({}),
}));

jest.mock('../../services/sync', () => ({
  autoSyncOnStartup: jest.fn().mockResolvedValue(undefined),
}));

jest.mock('../../services/backgroundSync', () => ({
  applyBackgroundSyncFromConfig: jest.fn().mockResolvedValue(undefined),
}));

// Mock screens to keep the render tree light
jest.mock('../../screens/ChatScreen', () => 'ChatScreen');
jest.mock('../../screens/SessionsScreen', () => 'SessionsScreen');
jest.mock('../../screens/NotesScreen', () => 'NotesScreen');
jest.mock('../../screens/NoteEditorScreen', () => 'NoteEditorScreen');
jest.mock('../../screens/SearchScreen', () => 'SearchScreen');
jest.mock('../../screens/SettingsScreen', () => 'SettingsScreen');
jest.mock('../../screens/ShareReceiveScreen', () => 'ShareReceiveScreen');

// Mock Icon to avoid SVG dep
jest.mock('../../components/Icon', () => {
  const { Text } = require('react-native');
  return function MockIcon(_props: any) {
    return React.createElement(Text, null, '[icon]');
  };
});

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import App from '../../../App';
import { useAppStore } from '../../store';
import * as i18n from '../../i18n';

const USER_LANG_KEY = '@vaultpilot:user-locale';

describe('App cold-start locale (#3696)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    useAppStore.setState({ localeVersion: 0 });
  });

  it('renders English tab labels on cold start when stored locale is en', async () => {
    await AsyncStorage.setItem(USER_LANG_KEY, 'en');
    // Reset i18n to a fresh state (as if app just launched)
    i18n.getCurrentLocale();

    const { findByText } = await render(React.createElement(App));

    // initLocale is awaited BEFORE 'ready', so the first render after
    // initialization must already show English labels (nav.chat = "Chat").
    expect(await findByText('Chat')).toBeTruthy();
    expect(await findByText('Notes')).toBeTruthy();
    expect(await findByText('Search')).toBeTruthy();
    expect(await findByText('Settings')).toBeTruthy();
  });

  it('renders zh-CN tab labels on cold start when stored locale is zh-CN', async () => {
    await AsyncStorage.setItem(USER_LANG_KEY, 'zh-CN');

    const { findByText } = await render(React.createElement(App));

    expect(await findByText('笔记')).toBeTruthy();
    expect(await findByText('搜索')).toBeTruthy();
    expect(await findByText('设置')).toBeTruthy();
  });

  it('setLocale bumps localeVersion so mounted UI re-renders', async () => {
    await AsyncStorage.setItem(USER_LANG_KEY, 'en');
    const before = useAppStore.getState().localeVersion;
    await i18n.setLocale('zh-CN');
    const after = useAppStore.getState().localeVersion;
    expect(after).toBe(before + 1);
    // cleanup — restore en to not affect other tests
    await i18n.setLocale('en');
  });
});
