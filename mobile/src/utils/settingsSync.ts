/**
 * Settings export/import for cross-platform sync (#1222).
 *
 * Exports app settings (theme, providers, model) as a portable JSON string.
 * API keys are included in exports for seamless cross-device setup,
 * but excluded by default for security (opt-in via includeKeys flag).
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';
import { saveSettings } from '../api/client';
import { ApiFormat, ThemeMode, ProviderConfig } from '../store';

const SECURE_KEYS_ID = 'vaultpilot_provider_keys';

/** Exportable settings shape (without sensitive keys by default). */
export interface ExportedSettings {
  version: 1;
  exportedAt: string;
  themeMode: string;
  accentColor: string;
  providers: Array<{
    name: string;
    apiBase: string;
    apiKey?: string;
    model: string;
    apiFormat: string;
  }>;
  activeProviderIndex: number;
}

/**
 * Export current settings as a JSON string.
 * @param includeKeys Whether to include API keys (default: false for security)
 */
export async function exportSettings(includeKeys = false): Promise<string> {
  const raw = await AsyncStorage.getItem('vaultpilot-store');
  if (!raw) throw new Error('No settings found');

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let stored: Record<string, any>;
  try {
    stored = JSON.parse(raw);
  } catch {
    throw new Error('设置数据已损坏，无法导出');
  }
  const state = stored?.state ?? stored;

  let providers = (state.providers ?? []) as Array<{
    name: string; apiBase: string; apiKey?: string; model: string; apiFormat: string;
  }>;

  if (includeKeys) {
    // Restore keys from SecureStore (stored as Record<string, string>)
    try {
      const keysRaw = await SecureStore.getItemAsync(SECURE_KEYS_ID);
      const keysRecord: Record<string, string> = keysRaw ? JSON.parse(keysRaw) : {};
      providers = providers.map(p => ({ ...p, apiKey: (keysRecord as Record<string, string>)[p.name] ?? '' }));
    } catch (e) {
      console.warn('[SettingsSync] Failed to read SecureStore keys:', e);
    }
  } else {
    // Strip keys
    providers = providers.map(p => ({ ...p, apiKey: undefined }));
  }

  const exported: ExportedSettings = {
    version: 1,
    exportedAt: new Date().toISOString(),
    themeMode: state.themeMode ?? 'system',
    accentColor: state.accentColor ?? '#3B82F6',
    providers,
    activeProviderIndex: state.activeProviderIndex ?? 0,
  };

  return JSON.stringify(exported, null, 2);
}

/**
 * Import settings from a JSON string.
 * Merges with existing settings — providers are replaced, theme is updated.
 */
export async function importSettings(json: string): Promise<{ providersImported: number }> {
  let data: ExportedSettings;
  try {
    data = JSON.parse(json);
  } catch {
    throw new Error('导入数据格式无效，请检查粘贴内容');
  }

  if (data.version !== 1) {
    throw new Error(`Unsupported settings version: ${data.version}`);
  }

  // Update AsyncStorage store
  const raw = await AsyncStorage.getItem('vaultpilot-store');
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let stored: Record<string, any>;
  try {
    stored = raw ? JSON.parse(raw) : {};
  } catch {
    stored = {};
  }
  const state = stored?.state ?? stored;

  state.themeMode = data.themeMode;
  state.accentColor = data.accentColor;
  state.activeProviderIndex = data.providers.length > 0
    ? Math.min(data.activeProviderIndex, data.providers.length - 1)
    : 0;

  // Save API keys to SecureStore FIRST — if this fails we must not overwrite
  // the existing provider list with empty keys, otherwise keys are lost.
  // Store as Record<string, string> matching what saveProviderKeysSecure uses.
  //
  // #2713: Merge extant keys before overwriting SecureStore.
  // SecureStore.setItemAsync is a full replace — if the device already
  // has keys for providers A,B,C and the import only contains A,B,
  // a straight replace would permanently erase C's key.
  let existingKeys: Record<string, string> = {};
  try {
    const raw = await SecureStore.getItemAsync(SECURE_KEYS_ID);
    existingKeys = raw ? JSON.parse(raw) : {};
  } catch {
    // corrupt / missing — start fresh
  }
  const keys: Record<string, string> = { ...existingKeys };
  for (const p of data.providers) {
    if (p.apiKey) keys[p.name] = p.apiKey;
  }
  if (Object.keys(keys).length > 0) {
    await SecureStore.setItemAsync(SECURE_KEYS_ID, JSON.stringify(keys));
  }
  // If all keys are empty, preserve existing SecureStore content

  // Only after SecureStore succeeded, write providers with keys stripped to AsyncStorage
  state.providers = data.providers.map(p => ({ ...p, apiKey: '' })); // keys go to SecureStore
  if (stored?.state) {
    stored.state = state;
    await AsyncStorage.setItem('vaultpilot-store', JSON.stringify(stored));
  } else {
    await AsyncStorage.setItem('vaultpilot-store', JSON.stringify({ state }));
  }

  // Sync active provider config to cfg_* keys so API client reads the new config
  const activeIndex = state.activeProviderIndex ?? 0;
  const active = data.providers[activeIndex];
  if (active) {
    const settings: { apiBase?: string; apiKey?: string; model?: string; apiFormat?: ApiFormat } = {
      apiBase: active.apiBase,
      model: active.model,
      apiFormat: active.apiFormat as ApiFormat,
    };
    if (active.apiKey) {
      settings.apiKey = active.apiKey;
    }
    await saveSettings(settings);
  }

  // Sync Zustand in-memory state so changes take effect immediately without restart
  const { useAppStore } = await import('../store');
  const fresh = useAppStore.getState();
  useAppStore.setState({
    themeMode: data.themeMode as ThemeMode,
    accentColor: data.accentColor,
    providers: data.providers.map(p => ({
      ...p,
      apiFormat: p.apiFormat as ApiFormat,
    })) as ProviderConfig[],
    activeProviderIndex: data.providers.length > 0
      ? Math.min(data.activeProviderIndex, data.providers.length - 1)
      : 0,
    apiBase: active?.apiBase ?? fresh.apiBase,
    apiKey: active?.apiKey ?? fresh.apiKey,
    model: active?.model ?? fresh.model,
    apiFormat: (active?.apiFormat as ApiFormat) ?? fresh.apiFormat,
  });

  return { providersImported: data.providers.length };
}
