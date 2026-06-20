import React, { useEffect, useRef, useState } from 'react';
import { StatusBar, useColorScheme, Text, View, ActivityIndicator, TouchableOpacity, StyleSheet } from 'react-native';
import * as SplashScreen from 'expo-splash-screen';
import { SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';
import { NavigationContainer } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { useAppStore, ThemeMode, isValidThemeMode } from './src/store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { getDb } from './src/db';
import { getSettings } from './src/api/client';
import ErrorBoundary from './src/components/ErrorBoundary';

import ChatScreen from './src/screens/ChatScreen';
import SessionsScreen from './src/screens/SessionsScreen';
import NotesScreen from './src/screens/NotesScreen';
import NoteEditorScreen from './src/screens/NoteEditorScreen';
import SearchScreen from './src/screens/SearchScreen';
import SettingsScreen from './src/screens/SettingsScreen';

import type { ChatStackParamList, NotesStackParamList, RootTabParamList } from './src/navigation/types';

const Tab = createBottomTabNavigator<RootTabParamList>();
const ChatNativeStack = createNativeStackNavigator<ChatStackParamList>();
const NotesNativeStack = createNativeStackNavigator<NotesStackParamList>();

SplashScreen.preventAutoHideAsync();

function NotesStack() {
  return (
    <NotesNativeStack.Navigator screenOptions={{ headerShown: false }}>
      <NotesNativeStack.Screen name="NotesList" component={NotesScreen} />
      <NotesNativeStack.Screen name="NoteEdit" component={NoteEditorScreen} />
    </NotesNativeStack.Navigator>
  );
}

function ChatStack() {
  return (
    <ChatNativeStack.Navigator screenOptions={{ headerShown: false }}>
      <ChatNativeStack.Screen name="ChatMain" component={ChatScreen} />
      <ChatNativeStack.Screen name="Sessions" component={SessionsScreen} />
    </ChatNativeStack.Navigator>
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
        tabBarIcon: ({ color }) => <TabIcon label="💬" color={color} />,
      }} />
      <Tab.Screen name="Search" component={SearchScreen} options={{
        tabBarLabel: '搜索',
        tabBarIcon: ({ color }) => <TabIcon label="🔍" color={color} />,
      }} />
      <Tab.Screen name="Notes" component={NotesStack} options={{
        tabBarLabel: '笔记',
        tabBarIcon: ({ color }) => <TabIcon label="📝" color={color} />,
      }} />
      <Tab.Screen name="Settings" component={SettingsScreen} options={{
        tabBarLabel: '设置',
        tabBarIcon: ({ color }) => <TabIcon label="⚙️" color={color} />,
      }} />
    </Tab.Navigator>
  );
}

function TabIcon({ label, color }: { label: string; color: string }) {
  return <Text style={{ fontSize: 20, color }}>{label}</Text>;
}

export default function App() {
  const { isDark, setIsDark, themeMode, accentColor } = useAppStore();
  const systemScheme = useColorScheme();
  const [initState, setInitState] = useState<'loading' | 'ready' | 'error'>('loading');
  const [errorMsg, setErrorMsg] = useState('');
  const loadedRef = useRef(false);

  // Load saved settings on startup
  useEffect(() => {
    (async () => {
      try {
        await getDb(); // Initialize database
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
    })();
  }, []);

  // Follow system theme — only after initial load to avoid overriding saved preference
  useEffect(() => {
    if (loadedRef.current && themeMode === 'system') {
      setIsDark(systemScheme === 'dark');
    }
  }, [systemScheme, themeMode]);

  if (initState === 'error') {
    return (
      <SafeAreaView style={{ flex: 1, justifyContent: 'center', alignItems: 'center', backgroundColor: isDark ? '#000' : '#FFF', padding: 24 }}>
        <Text style={{ fontSize: 18, fontWeight: '600', color: isDark ? '#F87171' : '#DC2626', marginBottom: 8 }}>数据库初始化失败</Text>
        <Text style={{ fontSize: 14, color: isDark ? '#9CA3AF' : '#6B7280', textAlign: 'center', marginBottom: 16 }}>{errorMsg}</Text>
        <TouchableOpacity
          onPress={() => { setInitState('loading'); setErrorMsg(''); }}
          style={{ paddingHorizontal: 24, paddingVertical: 12, backgroundColor: '#3B82F6', borderRadius: 8 }}
        >
          <Text style={{ color: '#FFF', fontWeight: '600' }}>重试</Text>
        </TouchableOpacity>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaProvider>
      <ErrorBoundary>
        <StatusBar barStyle={isDark ? 'light-content' : 'dark-content'} />
        <NavigationContainer>
          <MainTabs />
        </NavigationContainer>
      </ErrorBoundary>
    </SafeAreaProvider>
  );
}
