import React, { useEffect, useRef, useState } from 'react';
import { StatusBar, useColorScheme, View, Text, ActivityIndicator } from 'react-native';
import { NavigationContainer } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { useAppStore, ThemeMode } from './src/store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { getDb } from './src/db';
import { getSettings } from './src/api/client';

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
        tabBarIcon: ({ color }) => <TabIcon label="💬" />,
      }} />
      <Tab.Screen name="Notes" component={NotesStack} options={{
        tabBarLabel: '笔记',
        tabBarIcon: ({ color }) => <TabIcon label="📝" />,
      }} />
      <Tab.Screen name="Settings" component={SettingsScreen} options={{
        tabBarLabel: '设置',
        tabBarIcon: ({ color }) => <TabIcon label="⚙️" />,
      }} />
    </Tab.Navigator>
  );
}

function TabIcon({ label }: { label: string }) {
  return <>{label}</>;
}

export default function App() {
  const { isDark, setIsDark, themeMode, accentColor } = useAppStore();
  const systemScheme = useColorScheme();
  const loadedRef = useRef(false);
  const [dbReady, setDbReady] = useState(false);
  const [dbError, setDbError] = useState<string | null>(null);

  // Load saved settings on startup
  useEffect(() => {
    (async () => {
      try {
        await getDb(); // Initialize database
      } catch (e: any) {
        console.error('[App] DB init failed:', e);
        setDbError(e.message ?? '数据库初始化失败');
        return;
      }
      setDbReady(true);
      try {
        // Load API settings from cfg_* keys (matches SettingsScreen's saveSettings)
        const apiSettings = await getSettings();
        useAppStore.getState().setApiSettings(apiSettings);

        // Load theme settings from cfg_* keys
        const [savedTheme, savedColor] = await Promise.all([
          AsyncStorage.getItem('cfg_theme_mode'),
          AsyncStorage.getItem('cfg_accent_color'),
        ]);
        if (savedTheme) {
          const mode = savedTheme as ThemeMode;
          useAppStore.getState().setThemeMode(mode);
          if (mode === 'system') {
            setIsDark(systemScheme === 'dark');
          } else {
            setIsDark(mode === 'dark');
          }
        }
        if (savedColor) useAppStore.getState().setAccentColor(savedColor);
      } catch (e) {
        console.warn('[App] Failed to load settings:', e);
      }
      loadedRef.current = true;
    })();
  }, []);

  // Follow system theme — only after initial load to avoid overriding saved preference
  useEffect(() => {
    if (loadedRef.current && themeMode === 'system') {
      setIsDark(systemScheme === 'dark');
    }
  }, [systemScheme, themeMode]);

  if (dbError) {
    return (
      <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center', backgroundColor: isDark ? '#000' : '#FFF' }}>
        <Text style={{ color: '#EF4444', fontSize: 16, textAlign: 'center', paddingHorizontal: 32 }}>
          数据库初始化失败：{dbError}
        </Text>
        <Text style={{ color: isDark ? '#9CA3AF' : '#6B7280', marginTop: 8 }}>请重启应用重试</Text>
      </View>
    );
  }

  if (!dbReady) {
    return (
      <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center', backgroundColor: isDark ? '#000' : '#FFF' }}>
        <ActivityIndicator size="large" color={accentColor} />
      </View>
    );
  }

  return (
    <>
      <StatusBar barStyle={isDark ? 'light-content' : 'dark-content'} />
      <NavigationContainer>
        <MainTabs />
      </NavigationContainer>
    </>
  );
}
