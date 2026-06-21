import React, { useEffect, useRef, useState } from 'react';
import { StatusBar, useColorScheme, Text, View, ActivityIndicator, TouchableOpacity, StyleSheet, Linking } from 'react-native';
import * as SplashScreen from 'expo-splash-screen';
import { SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';
import { NavigationContainer, LinkingOptions } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { useAppStore, ThemeMode, isValidThemeMode } from './src/store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { getDb } from './src/db';
import { getSettings } from './src/api/client';
import ErrorBoundary from './src/components/ErrorBoundary';
import { autoSyncOnStartup } from './src/services/sync';
import appJson from './app.json';
import { checkForUpdate, downloadAndInstall, type UpdateInfo } from './src/utils/updateChecker';

import ChatScreen from './src/screens/ChatScreen';
import SessionsScreen from './src/screens/SessionsScreen';
import NotesScreen from './src/screens/NotesScreen';
import NoteEditorScreen from './src/screens/NoteEditorScreen';
import SearchScreen from './src/screens/SearchScreen';
import SettingsScreen from './src/screens/SettingsScreen';
import OnboardingScreen from './src/screens/OnboardingScreen';

import type { ChatStackParamList, NotesStackParamList, RootTabParamList } from './src/navigation/types';

const ONBOARDING_KEY = 'cfg_onboarding_done';

const Tab = createBottomTabNavigator<RootTabParamList>();
const ChatNativeStack = createNativeStackNavigator<ChatStackParamList>();
const NotesNativeStack = createNativeStackNavigator<NotesStackParamList>();

SplashScreen.preventAutoHideAsync();

/** Deep link config for Quick Settings Tile (#893) and Desktop Widget (#892) */
const linking: LinkingOptions<RootTabParamList> = {
  prefixes: ['vaultpilot://'],
  config: {
    screens: {
      Chat: {
        screens: {
          ChatMain: 'chat',
          Sessions: 'chat/sessions',
        },
      },
      Notes: {
        screens: {
          NotesList: 'note',
          NoteEdit: 'note/:noteId',
        },
      },
    },
  },
};

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
  const [onboardingDone, setOnboardingDone] = useState(true);
  const loadedRef = useRef(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [downloadPct, setDownloadPct] = useState<number | null>(null);
  const [dismissed, setDismissed] = useState(false);

  // Load saved settings on startup
  useEffect(() => {
    (async () => {
      try {
        await getDb(); // Initialize database
        // Auto-sync with backend if configured (non-blocking)
        autoSyncOnStartup();
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

        // Check onboarding status
        const onboardingStatus = await AsyncStorage.getItem(ONBOARDING_KEY);
        setOnboardingDone(onboardingStatus === 'true');
      } catch (e) {
        console.warn('[App] Failed to load settings:', e);
      }
      loadedRef.current = true;
      setInitState('ready');
      await SplashScreen.hideAsync();

      // Auto-check for update after app ready (non-blocking)
      try {
        const currentVer = appJson.expo.version;
        const skipVer = await AsyncStorage.getItem('cfg_skip_update');
        const info = await checkForUpdate(currentVer);
        if (info && info.latestVersion !== skipVer) {
          setUpdateInfo(info);
        }
      } catch {}
    })();
  }, []);

  // Follow system theme — only after initial load to avoid overriding saved preference
  useEffect(() => {
    if (loadedRef.current && themeMode === 'system') {
      setIsDark(systemScheme === 'dark');
    }
  }, [systemScheme, themeMode]);

  const handleUpdate = async () => {
    if (!updateInfo?.apkUrl) {
      if (updateInfo?.releaseUrl) {
        Linking.openURL(updateInfo.releaseUrl);
      }
      return;
    }
    setDownloadPct(0);
    const ok = await downloadAndInstall(updateInfo.apkUrl, updateInfo.latestVersion, setDownloadPct);
    if (!ok) setDownloadPct(null);
  };

  const handleSkipUpdate = async () => {
    if (updateInfo) {
      await AsyncStorage.setItem('cfg_skip_update', updateInfo.latestVersion);
    }
    setDismissed(true);
  };

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
        {initState === 'ready' && !onboardingDone ? (
          <OnboardingScreen onComplete={() => setOnboardingDone(true)} />
        ) : (
          <NavigationContainer linking={linking}>
            {updateInfo && !dismissed && (
              <View style={[styles.updateBanner, { backgroundColor: isDark ? '#1E3A5F' : '#EFF6FF' }]}>
                <Text style={[styles.updateText, { color: isDark ? '#93C5FD' : '#1D4ED8' }]}>
                  📦 v{updateInfo.latestVersion} 可用
                </Text>
                {downloadPct !== null ? (
                  <Text style={[styles.updateText, { color: isDark ? '#86EFAC' : '#15803D' }]}>
                    下载中 {downloadPct}%
                  </Text>
                ) : (
                  <View style={styles.updateButtons}>
                    <TouchableOpacity onPress={handleUpdate} style={styles.updateBtn}>
                      <Text style={styles.updateBtnText}>更新</Text>
                    </TouchableOpacity>
                    <TouchableOpacity onPress={handleSkipUpdate} style={[styles.updateBtn, { backgroundColor: 'transparent' }]}>
                      <Text style={[styles.updateBtnText, { color: isDark ? '#9CA3AF' : '#6B7280' }]}>跳过</Text>
                    </TouchableOpacity>
                  </View>
                )}
              </View>
            )}
            <MainTabs />
          </NavigationContainer>
        )}
      </ErrorBoundary>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({
  updateBanner: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: 12,
    paddingVertical: 8,
    marginHorizontal: 8,
    marginTop: 4,
    borderRadius: 8,
  },
  updateText: {
    fontSize: 13,
    fontWeight: '600',
  },
  updateButtons: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  updateBtn: {
    paddingHorizontal: 12,
    paddingVertical: 4,
    backgroundColor: '#3B82F6',
    borderRadius: 6,
  },
  updateBtnText: {
    color: '#FFF',
    fontSize: 13,
    fontWeight: '600',
  },
});
