import React, { useEffect, useRef, useState } from 'react';
import { StatusBar, useColorScheme, Text, View, ActivityIndicator, TouchableOpacity } from 'react-native';
import { NavigationContainer } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { useAppStore, ThemeMode, isValidThemeMode } from './src/store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { getDb } from './src/db';
import { getSettings } from './src/api/client';
import ErrorBoundary from './src/components/ErrorBoundary';

import ChatScreen from './src/screens/ChatScreen';
import NotesScreen from './src/screens/NotesScreen';
import NoteEditorScreen from './src/screens/NoteEditorScreen';
import SettingsScreen from './src/screens/SettingsScreen';

const Tab = createBottomTabNavigator();
const Stack = createNativeStackNavigator();

function NotesStack() {
  return (
    <Stack.Navigator screenOptions={{ headerShown: false }}>
      <Stack.Screen name="NotesList" component={NotesScreen} />
      <Stack.Screen name="NoteEdit" component={NoteEditorScreen} />
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
      <Tab.Screen name="Chat" component={ChatScreen} options={{
        tabBarLabel: '对话',
        tabBarIcon: ({ color }) => <TabIcon label="💬" color={color} />,
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
    })();
  }, []);

  // Follow system theme — only after initial load to avoid overriding saved preference
  useEffect(() => {
    if (loadedRef.current && themeMode === 'system') {
      setIsDark(systemScheme === 'dark');
    }
  }, [systemScheme, themeMode]);

  if (initState === 'loading') {
    return (
      <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center', backgroundColor: isDark ? '#000' : '#FFF' }}>
        <ActivityIndicator size="large" />
      </View>
    );
  }

  if (initState === 'error') {
    return (
      <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center', backgroundColor: isDark ? '#000' : '#FFF', padding: 24 }}>
        <Text style={{ fontSize: 18, fontWeight: '600', color: isDark ? '#F87171' : '#DC2626', marginBottom: 8 }}>数据库初始化失败</Text>
        <Text style={{ fontSize: 14, color: isDark ? '#9CA3AF' : '#6B7280', textAlign: 'center', marginBottom: 16 }}>{errorMsg}</Text>
        <TouchableOpacity
          onPress={() => { setInitState('loading'); setErrorMsg(''); }}
          style={{ paddingHorizontal: 24, paddingVertical: 12, backgroundColor: '#3B82F6', borderRadius: 8 }}
        >
          <Text style={{ color: '#FFF', fontWeight: '600' }}>重试</Text>
        </TouchableOpacity>
      </View>
    );
  }

  return (
    <ErrorBoundary>
      <StatusBar barStyle={isDark ? 'light-content' : 'dark-content'} />
      <NavigationContainer>
        <MainTabs />
      </NavigationContainer>
    </ErrorBoundary>
  );
}
