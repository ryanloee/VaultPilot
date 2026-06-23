/**
 * Settings export/import for cross-platform sync (#1222).
 *
 * Exports app settings (theme, providers, model) as a portable JSON string.
 * API keys are included in exports for seamless cross-device setup,
 * but excluded by default for security (opt-in via includeKeys flag).
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

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

  const stored = JSON.parse(raw);
  const state = stored?.state ?? stored;

  let providers = (state.providers ?? []) as Array<{
    name: string; apiBase: string; apiKey?: string; model: string; apiFormat: string;
  }>;

  if (includeKeys) {
    // Restore keys from SecureStore
    try {
      const keysRaw = await SecureStore.getItemAsync(SECURE_KEYS_ID);
      const keys: string[] = keysRaw ? JSON.parse(keysRaw) : [];
      providers = providers.map((p, i) => ({ ...p, apiKey: keys[i] ?? '' }));
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
  const data: ExportedSettings = JSON.parse(json);

  if (data.version !== 1) {
    throw new Error(`Unsupported settings version: ${data.version}`);
  }

  // Update AsyncStorage store
  const raw = await AsyncStorage.getItem('vaultpilot-store');
  const stored = raw ? JSON.parse(raw) : {};
  const state = stored?.state ?? stored;

  state.themeMode = data.themeMode;
  state.accentColor = data.accentColor;
  state.providers = data.providers.map(p => ({ ...p, apiKey: '' })); // keys go to SecureStore
  state.activeProviderIndex = Math.min(data.activeProviderIndex, data.providers.length - 1);

  // Save back
  if (stored?.state) {
    stored.state = state;
    await AsyncStorage.setItem('vaultpilot-store', JSON.stringify(stored));
  } else {
    await AsyncStorage.setItem('vaultpilot-store', JSON.stringify({ state }));
  }

  // Save API keys to SecureStore
  const keys = data.providers.map(p => p.apiKey ?? '');
  if (keys.some(k => k)) {
    try {
      await SecureStore.setItemAsync(SECURE_KEYS_ID, JSON.stringify(keys));
    } catch (e) {
      console.warn('[SettingsSync] Failed to save keys to SecureStore:', e);
    }
  }

  return { providersImported: data.providers.length };
}
