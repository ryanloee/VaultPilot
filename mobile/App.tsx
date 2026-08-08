import React, { useCallback, useEffect, useRef, useState } from 'react';
import { StatusBar, useColorScheme, Text, View, ActivityIndicator, TouchableOpacity, StyleSheet, Alert, Linking, type AlertButton } from 'react-native';
import Ionicons from '@expo/vector-icons/Ionicons';
import * as SplashScreen from 'expo-splash-screen';
import { SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';
import { NavigationContainer, createNavigationContainerRef } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { useAppStore, ThemeMode, isValidThemeMode } from './src/store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { getDb, ensureDefaultTemplates } from './src/db';
import { getSettings } from './src/api/client';
import ErrorBoundary from './src/components/ErrorBoundary';
import { autoSyncOnStartup } from './src/services/sync';
import { applyBackgroundSyncFromConfig } from './src/services/backgroundSync';
import { initLocale, t } from './src/i18n';
import { parseVaultPilotUri, evaluateUriSafety, extractSource, type ParsedVaultUri } from './src/utils/uriSafety';
import { getTrustedSources, addTrustedSource } from './src/utils/uriTrustStore';

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
 *
 * NOTE (#3964): 'vaultpilot://' is intentionally NOT in `prefixes` anymore.
 * vaultpilot:// URLs are intercepted by the safety gate below (installed at
 * module scope) and only dispatched to React Navigation after risk evaluation
 * + confirmation, so an external app can never auto-trigger a risky action.
 * expo-sharing:// and normal in-app navigation are unaffected.
 */
const linking: any = {
  prefixes: ['expo-sharing://'],
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

/**
 * #3964 — vaultpilot:// URI risk-confirmation gate.
 *
 * Every vaultpilot:// URL is intercepted HERE, before React Navigation can
 * execute it:
 *   - low risk      → dispatch immediately (open note / search / settings)
 *   - medium risk   → Alert confirmation, unless the x-source is trusted
 *                     (vaultpilot://note/new — creates a note)
 *   - high risk     → ALWAYS Alert confirmation, even for trusted sources
 *                     (vaultpilot://chat/new, vaultpilot://note/new?overwrite)
 *
 * Dispatch goes through navigationRef (nested navigate calls mirroring the
 * linking config above). If the NavigationContainer is not mounted yet the
 * URI is queued and flushed in onReady, covering the cold-start race where
 * the initial URL arrives before the container exists.
 */
const navigationRef = createNavigationContainerRef<{
  Main: { screen?: string; params?: object };
  ShareReceive: object | undefined;
}>();

/** vaultpilot:// URIs captured before the NavigationContainer mounts. */
const pendingVaultUris: string[] = [];

/**
 * Android can deliver the cold-start URL through BOTH getInitialURL() and a
 * subsequent 'url' event; dedupe identical URIs within a short window so the
 * action is not gated/executed twice.
 */
let lastSubmittedVaultUri: { uri: string; at: number } | null = null;

let vaultUriListenersInstalled = false;

function submitVaultUri(uri: string): void {
  if (typeof uri !== 'string' || !uri.startsWith('vaultpilot://')) return;
  const now = Date.now();
  if (lastSubmittedVaultUri && lastSubmittedVaultUri.uri === uri && now - lastSubmittedVaultUri.at < 2000) {
    return;
  }
  lastSubmittedVaultUri = { uri, at: now };
  void gateAndDispatchVaultUri(uri);
}

function flushPendingVaultUris(): void {
  while (pendingVaultUris.length > 0) {
    const uri = pendingVaultUris.shift();
    if (uri) void gateAndDispatchVaultUri(uri);
  }
}

async function gateAndDispatchVaultUri(uri: string): Promise<void> {
  let trustedSources: string[] = [];
  try {
    trustedSources = await getTrustedSources();
  } catch (e) {
    console.warn('[App] getTrustedSources failed:', e);
  }
  const evaluation = evaluateUriSafety(uri, trustedSources);
  if (!evaluation.needsConfirmation) {
    dispatchVaultUri(uri);
    return;
  }

  const source = extractSource(uri);
  const buttons: AlertButton[] = [{ text: '取消', style: 'cancel' }];
  if (evaluation.risk === 'medium' && source) {
    buttons.push({
      text: '信任此来源',
      onPress: () => {
        addTrustedSource(source).catch((e) => console.warn('[App] addTrustedSource failed:', e));
        dispatchVaultUri(uri);
      },
    });
  }
  buttons.push({ text: '确认', onPress: () => dispatchVaultUri(uri) });
  Alert.alert('安全确认', `${evaluation.reason}\n来源：${source || '未知'}`, buttons);
}

function dispatchVaultUri(uri: string): void {
  if (!navigationRef.isReady()) {
    pendingVaultUris.push(uri);
    return;
  }
  const parsed = parseVaultPilotUri(uri);
  if (!navigateForVaultUri(parsed)) {
    // Unknown route — the link simply fails navigation (same as before).
    console.warn('[App] Unhandled vaultpilot URI:', uri);
  }
}

/** Map a parsed vaultpilot:// URI onto the React Navigation routes from the linking config. */
function navigateForVaultUri(parsed: ParsedVaultUri): boolean {
  switch (parsed.route) {
    case 'chat/new':
      navigationRef.navigate('Main', { screen: 'Chat', params: { screen: 'ChatNew' } });
      return true;
    case 'chat':
      navigationRef.navigate('Main', { screen: 'Chat', params: { screen: 'ChatMain' } });
      return true;
    case 'chat/sessions':
      navigationRef.navigate('Main', { screen: 'Chat', params: { screen: 'Sessions' } });
      return true;
    case 'note/new':
      navigationRef.navigate('Main', {
        screen: 'Notes',
        params: {
          screen: 'NoteEdit',
          params: { noteId: 'new', ...(parsed.overwrite ? { overwrite: true } : {}) },
        },
      });
      return true;
    case 'note/:id':
      navigationRef.navigate('Main', {
        screen: 'Notes',
        params: { screen: 'NoteEdit', params: { noteId: parsed.noteId ?? '' } },
      });
      return true;
    case 'note':
      navigationRef.navigate('Main', { screen: 'Notes', params: { screen: 'NotesList' } });
      return true;
    case 'search':
      navigationRef.navigate('Main', { screen: 'Search' });
      return true;
    case 'settings':
      navigationRef.navigate('Main', { screen: 'Settings' });
      return true;
    default:
      return false;
  }
}

/** Install the vaultpilot:// listeners once, at module scope (covers pre-mount window). */
function installVaultUriListeners(): void {
  if (vaultUriListenersInstalled) return;
  vaultUriListenersInstalled = true;

  // Cold start — URL is delivered before any listener exists.
  Linking.getInitialURL()
    .then((url) => {
      if (url) submitVaultUri(url);
    })
    .catch((e) => console.warn('[App] getInitialURL failed:', e));

  // Warm start / app already running.
  Linking.addEventListener('url', ({ url }) => {
    if (url) submitVaultUri(url);
  });
}
installVaultUriListeners();

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
        tabBarLabel: t('nav.chat'),
        tabBarIcon: ({ color }) => <TabIcon name="chatbubble-outline" color={color} />,
      }} />
      <Tab.Screen name="Notes" component={NotesStack} options={{
        tabBarLabel: t('nav.notes'),
        tabBarIcon: ({ color }) => <TabIcon name="document-text-outline" color={color} />,
      }} />
      <Tab.Screen name="Search" component={SearchScreen} options={{
        tabBarLabel: t('nav.search'),
        tabBarIcon: ({ color }) => <TabIcon name="search-outline" color={color} />,
      }} />
      <Tab.Screen name="Settings" component={SettingsScreen} options={{
        tabBarLabel: t('nav.settings'),
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
      useAppStore.getState().setApiSettings(apiSettings).catch((e) => {
        console.warn('[App] Failed to set API settings:', e);
      });

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
    // #3692/#3696 — Initialize i18n locale BEFORE marking ready so the first
    // UI render uses the correct locale. initLocale() never throws (internal
    // try/catch), so awaiting it cannot fail the app boot.
    await initLocale();
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
        <NavigationContainer ref={navigationRef} linking={linking} onReady={() => flushPendingVaultUris()}>
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
