import React, { useCallback, useEffect, useRef, useState } from 'react';
import { StatusBar, useColorScheme, Text, View, ActivityIndicator, TouchableOpacity, StyleSheet } from 'react-native';
import Ionicons from '@expo/vector-icons/Ionicons';
import * as SplashScreen from 'expo-splash-screen';
import { SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';
import { NavigationContainer } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { useAppStore, ThemeMode, isValidThemeMode } from './src/store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { getDb, ensureDefaultTemplates } from './src/db';
import { getSettings } from './src/api/client';
import ErrorBoundary from './src/components/ErrorBoundary';
import { autoSyncOnStartup } from './src/services/sync';
import { applyBackgroundSyncFromConfig } from './src/services/backgroundSync';

import ChatScreen from './src/screens/ChatScreen';
import SessionsScreen from './src/screens/SessionsScreen';
import NotesScreen from './src/screens/NotesScreen';
import NoteEditorScreen from './src/screens/NoteEditorScreen';
import SearchScreen from './src/screens/SearchScreen';
import SettingsScreen from './src/screens/SettingsScreen';

import ShareReceiveScreen from './src/screens/ShareReceiveScreen';

/**
 * Deep-link configuration for widget / Quick Settings tile (#2915).
 * Supports vaultpilot:// scheme with routes:
 *   vaultpilot://note/new  → create + open new note
 *   vaultpilot://note/:id  → open existing note
 *   vaultpilot://chat/new  → new chat session
 *   vaultpilot://search    → global search
 */
const linking: any = {
  prefixes: ['vaultpilot://', 'expo-sharing://'],
  config: {
    screens: {
      Main: {
        screens: {
          Chat: {
            screens: {
              ChatMain: 'chat',
              ChatNew: 'chat/new',
              Sessions: 'chat/sessions',
            },
          },
          Notes: {
            screens: {
              NotesList: 'note',
              NoteEdit: 'note/:noteId',
            },
          },
          Search: 'search',
          Settings: 'settings',
        },
      },
      ShareReceive: 'share-receive',
    },
  },
};

const Tab = createBottomTabNavigator();
const Stack = createNativeStackNavigator();
const RootStack = createNativeStackNavigator();

SplashScreen.preventAutoHideAsync();

function NotesStack() {
  return (
    <Stack.Navigator screenOptions={{ headerShown: false }}>
      <Stack.Screen name="NotesList" component={NotesScreen} />
      <Stack.Screen name="NoteEdit" component={NoteEditorScreen} />
    </Stack.Navigator>
  );
}

function ChatStack() {
  return (
    <Stack.Navigator screenOptions={{ headerShown: false }}>
      <Stack.Screen name="ChatMain" component={ChatScreen} />
      <Stack.Screen name="ChatNew" component={ChatScreen} initialParams={{ action: 'new' }} />
      <Stack.Screen name="Sessions" component={SessionsScreen} />
    </Stack.Navigator>
  );
}

function MainTabs() {
  const { isDark, accentColor } = useAppStore();
  return (
    <Tab.Navigator
      screenOptions={{
        headerShown: false,
        tabBarStyle: {
          backgroundColor: isDark ? '#000' : '#FFF',
          borderTopColor: isDark ? '#1F2937' : '#E5E7EB',
        },
        tabBarActiveTintColor: accentColor,
        tabBarInactiveTintColor: isDark ? '#6B7280' : '#9CA3AF',
      }}
    >
      <Tab.Screen name="Chat" component={ChatStack} options={{
        tabBarLabel: '对话',
        tabBarIcon: ({ color }) => <TabIcon name="chatbubble-outline" color={color} />,
      }} />
      <Tab.Screen name="Notes" component={NotesStack} options={{
        tabBarLabel: '笔记',
        tabBarIcon: ({ color }) => <TabIcon name="document-text-outline" color={color} />,
      }} />
      <Tab.Screen name="Search" component={SearchScreen} options={{
        tabBarLabel: '搜索',
        tabBarIcon: ({ color }) => <TabIcon name="search-outline" color={color} />,
      }} />
      <Tab.Screen name="Settings" component={SettingsScreen} options={{
        tabBarLabel: '设置',
        tabBarIcon: ({ color }) => <TabIcon name="settings-outline" color={color} />,
      }} />
    </Tab.Navigator>
  );
}

function TabIcon({ name, color }: { name: React.ComponentProps<typeof Ionicons>['name']; color: string }) {
  return <Ionicons name={name} size={22} color={color} />;
}

export default function App() {
  const { isDark, setIsDark, themeMode, accentColor } = useAppStore();
  const systemScheme = useColorScheme();
  const [initState, setInitState] = useState<'loading' | 'ready' | 'error'>('loading');
  const [errorMsg, setErrorMsg] = useState('');
  const loadedRef = useRef(false);

  // Initialize the app — reusable so retry button can call it again.
  const initApp = useCallback(async () => {
    loadedRef.current = false;
    setInitState('loading');
    setErrorMsg('');
    try {
      await getDb(); // Initialize database
      await ensureDefaultTemplates(); // #2154 — seed built-in note templates on first launch
    } catch (e) {
      console.error('[App] DB init failed:', e);
      setErrorMsg(String(e));
      setInitState('error');
      await SplashScreen.hideAsync();
      return;
    }
    try {
      // Load API settings from cfg_* keys (matches SettingsScreen's saveSettings)
      const apiSettings = await getSettings();
      useAppStore.getState().setApiSettings(apiSettings);

      // Load theme settings from cfg_* keys
      const [savedTheme, savedColor] = await Promise.all([
        AsyncStorage.getItem('cfg_theme_mode'),
        AsyncStorage.getItem('cfg_accent_color'),
      ]);
      if (savedTheme && isValidThemeMode(savedTheme)) {
        useAppStore.getState().setThemeMode(savedTheme);
        if (savedTheme === 'system') {
          setIsDark(systemScheme === 'dark');
        } else {
          setIsDark(savedTheme === 'dark');
        }
      }
      if (savedColor) useAppStore.getState().setAccentColor(savedColor);
    } catch (e) {
      console.warn('[App] Failed to load settings:', e);
    }
    loadedRef.current = true;
    setInitState('ready');
    await SplashScreen.hideAsync();

    // #3158 — Background sync: kick off (a) a one-shot foreground auto-sync
    // now that the DB/settings are ready, and (b) re-apply the persisted
    // background-fetch configuration so the OS scheduler matches user prefs.
    // Both are fire-and-forget; errors are caught inside the helpers.
    autoSyncOnStartup().catch((e) => console.warn('[App] autoSyncOnStartup:', e));
    applyBackgroundSyncFromConfig().catch((e) => console.warn('[App] applyBackgroundSyncFromConfig:', e));
  }, [setIsDark]);

  // Load saved settings on startup
  useEffect(() => { initApp(); }, [initApp]);

  // Follow system theme — only after initial load to avoid overriding saved preference
  useEffect(() => {
    if (loadedRef.current && themeMode === 'system') {
      setIsDark(systemScheme === 'dark');
    }
  }, [systemScheme, themeMode]);

  if (initState === 'error') {
    return (
      <SafeAreaProvider>
        <SafeAreaView style={{ flex: 1, justifyContent: 'center', alignItems: 'center', backgroundColor: isDark ? '#000' : '#FFF', padding: 24 }}>
          <Text style={{ fontSize: 18, fontWeight: '600', color: isDark ? '#F87171' : '#DC2626', marginBottom: 8 }}>数据库初始化失败</Text>
          <Text style={{ fontSize: 14, color: isDark ? '#9CA3AF' : '#6B7280', textAlign: 'center', marginBottom: 16 }}>{errorMsg}</Text>
          <TouchableOpacity
            onPress={() => { initApp(); }}
            style={{ paddingHorizontal: 24, paddingVertical: 12, backgroundColor: '#3B82F6', borderRadius: 8 }}
          >
            <Text style={{ color: '#FFF', fontWeight: '600' }}>重试</Text>
          </TouchableOpacity>
        </SafeAreaView>
      </SafeAreaProvider>
    );
  }

  return (
    <SafeAreaProvider>
      <ErrorBoundary>
        <StatusBar barStyle={isDark ? 'light-content' : 'dark-content'} />
        <NavigationContainer linking={linking}>
          <RootStack.Navigator screenOptions={{ headerShown: false }}>
            <RootStack.Screen name="Main" component={MainTabs} />
            <RootStack.Screen
              name="ShareReceive"
              component={ShareReceiveScreen}
              options={{ presentation: 'modal' }}
            />
          </RootStack.Navigator>
        </NavigationContainer>
      </ErrorBoundary>
    </SafeAreaProvider>
  );
}
