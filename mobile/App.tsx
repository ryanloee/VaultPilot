import React, { useEffect } from 'react';
import { StatusBar, useColorScheme } from 'react-native';
import { NavigationContainer } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { useAppStore } from './src/store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { getDb } from './src/db';

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

  // Load saved settings on startup
  useEffect(() => {
    (async () => {
      await getDb(); // Initialize database
      const saved = await AsyncStorage.getItem('app_settings');
      if (saved) {
        const s = JSON.parse(saved);
        if (s.themeMode) {
          useAppStore.getState().setThemeMode(s.themeMode);
          if (s.themeMode === 'system') {
            setIsDark(systemScheme === 'dark');
          } else {
            setIsDark(s.themeMode === 'dark');
          }
        }
        if (s.accentColor) useAppStore.getState().setAccentColor(s.accentColor);
        if (s.apiBase) useAppStore.getState().setApiSettings({ apiBase: s.apiBase });
        if (s.apiKey) useAppStore.getState().setApiSettings({ apiKey: s.apiKey });
        if (s.model) useAppStore.getState().setApiSettings({ model: s.model });
      }
    })();
  }, []);

  // Follow system theme
  useEffect(() => {
    if (themeMode === 'system') {
      setIsDark(systemScheme === 'dark');
    }
  }, [systemScheme, themeMode]);

  return (
    <>
      <StatusBar barStyle={isDark ? 'light-content' : 'dark-content'} />
      <NavigationContainer>
        <MainTabs />
      </NavigationContainer>
    </>
  );
}
