import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';
import { Alert } from 'react-native';

export type ThemeMode = 'light' | 'dark' | 'system';

const VALID_THEME_MODES: ThemeMode[] = ['light', 'dark', 'system'];
export function isValidThemeMode(v: string): v is ThemeMode {
  return (VALID_THEME_MODES as string[]).includes(v);
}

export type ApiFormat = 'openai' | 'anthropic';

/** A saved API provider configuration. */
export interface ProviderConfig {
  name: string;
  apiBase: string;
  apiKey: string;
  model: string;
  apiFormat: ApiFormat;
}

// ── Secure provider key storage ───────────────────────────
// API keys are stored in SecureStore (encrypted), not AsyncStorage (plain text).
// A fallback copy is kept in AsyncStorage (base64 obfuscated) so keys survive
// APK updates where the Android keystore binding changes (#2394).
const SECURE_KEYS_ID = 'vaultpilot_provider_keys';
const ASYNC_FALLBACK_KEYS_ID = 'vaultpilot_provider_keys_backup';

/** Minimal obfuscation for AsyncStorage fallback — not crypto, just prevents casual reading. */
function obfuscate(s: string): string {
  return btoa(unescape(encodeURIComponent(s)));
}
function deobfuscate(s: string): string {
  return decodeURIComponent(escape(atob(s)));
}

// Write queue to serialize SecureStore writes and prevent race conditions (#2519)
let keySavePromise: Promise<void> = Promise.resolve();

async function saveProviderKeysSecure(providers: ProviderConfig[]): Promise<void> {
  const keys: Record<string, string> = {};
  for (const p of providers) {
    keys[p.name] = p.apiKey;
  }
  const prevPromise = keySavePromise;
  let secureStoreFailed = false;
  let asyncStorageFailed = false;
  const savePromise = prevPromise.then(async () => {
    try {
      await SecureStore.setItemAsync(SECURE_KEYS_ID, JSON.stringify(keys));
    } catch (e) {
      console.warn('[SecureStore] Failed to save provider keys:', e);
      secureStoreFailed = true;
    }
    // Always write fallback to AsyncStorage (survives APK updates: #2394)
    try {
      await AsyncStorage.setItem(ASYNC_FALLBACK_KEYS_ID, obfuscate(JSON.stringify(keys)));
    } catch (e) {
      console.error('[AsyncStorage] Failed to save fallback provider keys:', e);
      asyncStorageFailed = true;
    }
    // #2712: If BOTH SecureStore and AsyncStorage fallback fail, throw so callers
    // can detect the failure. If only SecureStore fails but AsyncStorage succeeds,
    // we survive (keys are in fallback, will be re-promoted on next load).
    if (secureStoreFailed && asyncStorageFailed) {
      throw new Error(
        '[VaultPilot] Critical: provider keys could not be saved to either SecureStore or AsyncStorage fallback — ' +
        'API key changes will be lost on next restart.'
      );
    }
    if (secureStoreFailed && !asyncStorageFailed) {
      console.warn('[VaultPilot] SecureStore write failed but AsyncStorage fallback succeeded. ' +
        'Keys will be re-promoted to SecureStore on next load.');
    }
  });
  // Don't swallow errors — let them propagate to callers (#2712)
  // #2771: keySavePromise resolves on error by design — the queue is for
  // serialization, not failure propagation. The error propagates to the
  // direct caller via await savePromise below. Future saves chain onto a
  // resolved keySavePromise and proceed independently (correct behavior).
  keySavePromise = savePromise.then(
    () => {},
    (err) => { console.error('[VaultPilot] Provider key save failed:', err); }
  );
  await savePromise;
}

async function loadProviderKeysSecure(): Promise<Record<string, string> | null> {
  try {
    const raw = await SecureStore.getItemAsync(SECURE_KEYS_ID);
    if (raw) {
      const parsed = JSON.parse(raw);
      // Support legacy string[] format for backward compatibility (#1629)
      if (Array.isArray(parsed)) {
        return {};
      }
      return parsed as Record<string, string>;
    }
    // SecureStore empty — try AsyncStorage fallback (survives APK updates: #2394)
    try {
      const fallback = await AsyncStorage.getItem(ASYNC_FALLBACK_KEYS_ID);
      if (fallback) {
        const keys = JSON.parse(deobfuscate(fallback)) as Record<string, string>;
        // Re-save to SecureStore so next restart reads from primary storage
        await SecureStore.setItemAsync(SECURE_KEYS_ID, JSON.stringify(keys));
        return keys;
      }
    } catch (fb) {
      console.warn('[Store] AsyncStorage fallback read failed:', fb);
    }
    return {};
  } catch (e) {
    console.warn('[Store] Failed to load provider keys from SecureStore:', e);
    return null; // Signal load failure — caller must handle to avoid wiping keys (#1629)
  }
}

interface AppState {
  themeMode: ThemeMode;
  isDark: boolean;
  accentColor: string;
  setThemeMode: (mode: ThemeMode) => void;
  setAccentColor: (color: string) => void;
  setIsDark: (dark: boolean) => void;

  // Focus / reading mode (#2894): when on, hide AI command palette entry,
  // assistant floating button and context suggestion panels for immersive writing.
  focusMode: boolean;
  setFocusMode: (focus: boolean) => void;

  // Legacy flat fields (kept for client.ts backward compat, synced from active provider)
  apiBase: string;
  apiKey: string;
  model: string;
  apiFormat: ApiFormat;
  setApiSettings: (s: { apiBase?: string; apiKey?: string; model?: string; apiFormat?: ApiFormat }) => void;

  // Multi-provider
  providers: ProviderConfig[];
  activeProviderIndex: number;
  addProvider: (p: ProviderConfig) => void;
  removeProvider: (index: number) => void;
  updateProvider: (index: number, p: Partial<ProviderConfig>) => void;
  setActiveProvider: (index: number) => void;
}

export const ACCENT_COLORS = [
  { name: '蓝', value: '#3B82F6' },
  { name: '紫', value: '#8B5CF6' },
  { name: '绿', value: '#10B981' },
  { name: '橙', value: '#F59E0B' },
  { name: '红', value: '#EF4444' },
  { name: '青', value: '#06B6D4' },
];

export const PROVIDERS = [
  { name: 'OpenCode Zen', base: 'https://opencode.ai/zen/v1', format: 'openai' as const, models: ['deepseek-v4-flash-free', 'mimo-v2.5-free', 'qwen3.6-plus-free', 'minimax-m3-free', 'big-pickle'] },
  { name: 'OpenRouter', base: 'https://openrouter.ai/api/v1', format: 'openai' as const, models: ['google/gemma-4-31b-it:free', 'nvidia/nemotron-3-super-120b-a12b:free', 'qwen/qwen3-coder:free'] },
  { name: 'OpenAI', base: 'https://api.openai.com/v1', format: 'openai' as const, models: ['gpt-4o', 'gpt-4o-mini', 'o1-mini'] },
  { name: 'Anthropic', base: 'https://api.anthropic.com', format: 'anthropic' as const, models: ['claude-sonnet-4-20250514', 'claude-3-5-haiku-20241022'] },
];

// ── Pure helper functions (exported for testing) ──────────

/** Clamp provider index to valid range (0 to providerCount-1). */
export function clampProviderIndex(index: number, providerCount: number): number {
  if (providerCount === 0) return -1;
  return Math.max(0, Math.min(index, providerCount - 1));
}

/** Remove a provider by index, returning a new array. */
export function removeProviderFromList(providers: ProviderConfig[], index: number): ProviderConfig[] {
  return providers.filter((_, i) => i !== index);
}

/** Compute the new active index after removing a provider. */
export function computeActiveIndexAfterRemove(currentIndex: number, removedIndex: number, newLength: number): number {
  if (newLength === 0) return -1;
  if (currentIndex === removedIndex) {
    // Active item was deleted — clamp to valid range
    return Math.min(currentIndex, newLength - 1);
  }
  if (removedIndex < currentIndex) {
    // Deleted item was before active — shift left by one
    return currentIndex - 1;
  }
  // Deleted item was after active — index unchanged
  return currentIndex;
}

/** Update a provider at index with partial fields, returning a new array. */
export function updateProviderInList(providers: ProviderConfig[], index: number, patch: Partial<ProviderConfig>): ProviderConfig[] {
  if (index < 0 || index >= providers.length) return providers;
  const next = [...providers];
  next[index] = { ...next[index], ...patch };
  return next;
}

/** Merge partial API settings into current state. */
export function mergeApiSettings(current: { apiBase: string; apiKey: string; model: string; apiFormat: ApiFormat }, patch: { apiBase?: string; apiKey?: string; model?: string; apiFormat?: ApiFormat }) {
  return {
    apiBase: patch.apiBase ?? current.apiBase,
    apiKey: patch.apiKey ?? current.apiKey,
    model: patch.model ?? current.model,
    apiFormat: patch.apiFormat ?? current.apiFormat,
  };
}

/** Strip API keys from providers for AsyncStorage persistence. */
export function sanitizeForPersistence(providers: ProviderConfig[]): ProviderConfig[] {
  return providers.map(p => ({ ...p, apiKey: '' }));
}

/**
 * Filter AI toolbar actions out of a toolbar config when focus mode is on (#2894).
 * Keeps all non-AI formatting actions (bold, italic, code, heading, list, link).
 * Exported as a pure helper so the hiding behavior is unit-testable without
 * rendering the full editor screen.
 */
export function filterFocusModeToolbar<T extends { action?: string }>(items: T[], focusMode: boolean): T[] {
  if (!focusMode) return items;
  return items.filter(it => it.action !== 'aiWrite' && it.action !== 'aiCmd');
}

/** Restore API keys into providers from SecureStore keys map. */
export function restoreProviderKeys(providers: ProviderConfig[], keys: Record<string, string>): ProviderConfig[] {
  return providers.map(p => ({ ...p, apiKey: keys[p.name] ?? '' }));
}

const LIGHT_COLORS = {
  bg: '#FFFFFF', bgSecondary: '#F3F4F6', text: '#111827', textSecondary: '#6B7280',
  border: '#E5E7EB', card: '#FFFFFF', inputBg: '#F9FAFB',
  userBubble: '#3B82F6', userText: '#FFFFFF', aiBubble: '#F3F4F6', aiText: '#111827',
};

const DARK_COLORS = {
  bg: '#000000', bgSecondary: '#111111', text: '#F9FAFB', textSecondary: '#9CA3AF',
  border: '#1F2937', card: '#111111', inputBg: '#1F2937',
  userBubble: '#3B82F6', userText: '#FFFFFF', aiBubble: '#1F2937', aiText: '#F9FAFB',
};

type ColorScheme = typeof LIGHT_COLORS & { accent: string };

let cachedColors: ColorScheme | null = null;
let cachedIsDark = false;
let cachedAccent = '';

export function getColors(isDark: boolean, accent: string): ColorScheme {
  if (isDark === cachedIsDark && accent === cachedAccent && cachedColors) {
    return cachedColors;
  }
  cachedIsDark = isDark;
  cachedAccent = accent;
  cachedColors = { ...(isDark ? DARK_COLORS : LIGHT_COLORS), accent };
  return cachedColors;
}

/** Sync legacy flat fields from the active provider so client.ts keeps working. */
function syncLegacyFields(set: (partial: Partial<AppState>) => void, providers: ProviderConfig[], activeProviderIndex: number) {
  if (providers.length === 0) return;
  const idx = clampProviderIndex(activeProviderIndex, providers.length);
  const p = providers[idx];
  set({
    apiBase: p.apiBase,
    apiKey: p.apiKey,
    model: p.model,
    apiFormat: p.apiFormat,
  });
}

export const useAppStore = create<AppState>()(
  persist(
    (set, get) => ({
      themeMode: 'system',
      isDark: false,
      accentColor: '#3B82F6',
      setThemeMode: (themeMode) => set({ themeMode }),
      setAccentColor: (accentColor) => set({ accentColor }),
      setIsDark: (isDark) => set({ isDark }),
      focusMode: false,
      setFocusMode: (focusMode) => set({ focusMode }),

      apiBase: 'https://opencode.ai/zen/v1',
      apiKey: '',
      model: 'deepseek-v4-flash-free',
      apiFormat: 'openai' as ApiFormat,
      setApiSettings: async (s) => {
        let updatedProviders: ProviderConfig[] | undefined;
        set((state) => {
          const merged = mergeApiSettings(state, s);
          const result: Partial<AppState> = { ...merged };
          // Sync explicitly-provided fields to active provider to prevent
          // data loss on provider switch (#2736). Only fields explicitly
          // passed in `s` are synced, avoiding stale flat-field overwrite (#2551).
          if (state.providers.length > 0) {
            const idx = clampProviderIndex(state.activeProviderIndex, state.providers.length);
            if (idx >= 0) {
              const providerPatch: Partial<ProviderConfig> = {};
              if (s.apiKey !== undefined) providerPatch.apiKey = s.apiKey;
              if (s.apiBase !== undefined) providerPatch.apiBase = s.apiBase;
              if (s.model !== undefined) providerPatch.model = s.model;
              if (s.apiFormat !== undefined) providerPatch.apiFormat = s.apiFormat;
              if (Object.keys(providerPatch).length > 0) {
                const providers = [...state.providers];
                providers[idx] = { ...providers[idx], ...providerPatch };
                result.providers = providers;
                updatedProviders = providers;
              }
            }
          }
          return result;
        });
        if (updatedProviders) await saveProviderKeysSecure(updatedProviders);
      },

      providers: [],
      activeProviderIndex: 0,

      addProvider: async (p) => {
        let updatedProviders: ProviderConfig[] | undefined;
        set((state) => {
          const providers = [...state.providers, p];
          const activeProviderIndex = providers.length - 1;
          updatedProviders = providers;
          const active = providers[activeProviderIndex];
          return {
            providers, activeProviderIndex,
            apiBase: active.apiBase, apiKey: active.apiKey,
            model: active.model, apiFormat: active.apiFormat,
          };
        });
        if (updatedProviders) await saveProviderKeysSecure(updatedProviders);
      },

      removeProvider: async (index) => {
        let updatedProviders: ProviderConfig[] | undefined;
        set((state) => {
          if (state.providers.length === 0) return {}; // Guard: nothing to remove
          // Bounds check: an out-of-range index (e.g. -1) would leave the
          // providers array unchanged but produce an invalid activeProviderIndex
          // → providers[activeProviderIndex] is undefined → TypeError crash (#2549)
          if (index < 0 || index >= state.providers.length) return {};
          const providers = removeProviderFromList(state.providers, index);
          const activeProviderIndex = computeActiveIndexAfterRemove(state.activeProviderIndex, index, providers.length);
          updatedProviders = providers;
          const update: Partial<AppState> = { providers, activeProviderIndex };
          if (providers.length > 0) {
            const active = providers[activeProviderIndex];
            update.apiBase = active.apiBase;
            update.apiKey = active.apiKey;
            update.model = active.model;
            update.apiFormat = active.apiFormat;
          } else {
            update.apiBase = '';
            update.apiKey = '';
            update.model = '';
            update.apiFormat = 'openai';
          }
          return update;
        });
        if (updatedProviders) await saveProviderKeysSecure(updatedProviders);
      },

      updateProvider: async (index, p) => {
        let updatedProviders: ProviderConfig[] | undefined;
        set((state) => {
          const providers = updateProviderInList(state.providers, index, p);
          updatedProviders = providers;
          const update: Partial<AppState> = { providers };
          if (index === clampProviderIndex(state.activeProviderIndex, providers.length)) {
            const active = providers[index];
            update.apiBase = active.apiBase;
            update.apiKey = active.apiKey;
            update.model = active.model;
            update.apiFormat = active.apiFormat;
          }
          return update;
        });
        if (updatedProviders) await saveProviderKeysSecure(updatedProviders);
      },

      setActiveProvider: (index) => set((state) => {
        if (state.providers.length === 0) return {}; // Guard against empty providers (#1578)
        const activeProviderIndex = clampProviderIndex(index, state.providers.length);
        const active = state.providers[activeProviderIndex];
        return {
          activeProviderIndex,
          apiBase: active.apiBase, apiKey: active.apiKey,
          model: active.model, apiFormat: active.apiFormat,
        };
      }),
    }),
    {
      name: 'vaultpilot-store',
      storage: createJSONStorage(() => AsyncStorage),
      partialize: (state) => ({
        themeMode: state.themeMode,
        isDark: state.isDark,
        accentColor: state.accentColor,
        focusMode: state.focusMode,
        apiBase: state.apiBase,
        // apiKey excluded — stored in SecureStore instead
        model: state.model,
        apiFormat: state.apiFormat,
        providers: sanitizeForPersistence(state.providers),
        activeProviderIndex: state.activeProviderIndex,
      }),
      onRehydrateStorage: () => (state) => {
        if (!state) return;
        // Restore API keys from SecureStore after hydration
        loadProviderKeysSecure().then(keys => {
          // Use fresh state snapshot instead of stale closure (#1770)
          const fresh = useAppStore.getState();
          if (keys === null) {
            // SecureStore read failed — preserve existing keys to avoid wiping (#1629)
            Alert.alert(
              '密钥加载失败',
              '无法从安全存储读取 API Key，已保留当前会话中的密钥。重启应用后可能需要重新输入。',
              [{ text: '知道了' }],
            );
            return;
          }
          if (Object.keys(keys).length === 0) {
            // SecureStore returned empty after rehydration. Check if providers
            // existed before — if so, SecureStore was likely wiped by APK update
            // or device restore. Show a warning instead of silently clearing keys.
            const providerCount = fresh.providers.length;
            if (providerCount > 0) {
              Alert.alert(
                'API Key 需要重新输入',
                `APK 更新后安全存储被清空，请重新输入 ${providerCount} 个 API Key。`,
                [{ text: '知道了' }],
              );
            }
            // Still invalidate settings cache so getSettings() reads fresh store
            // values instead of stale pre-hydration cache (#2553).
            import('./api/settingsCache').then(m => m.invalidateSettingsCache()).catch(e => {
              console.warn('[Store] failed to invalidate settings cache after rehydration (empty keys):', e);
            });
            return;
          }
          const restored = restoreProviderKeys(fresh.providers, keys);
          useAppStore.setState({ providers: restored });
          syncLegacyFields(useAppStore.setState, restored, fresh.activeProviderIndex);
          // Invalidate the getSettings module-level cache after hydration so that
          // subsequent getSettings calls read fresh store values instead of a value
          // cached from AsyncStorage during the pre-hydration window (#2102).
          import('./api/settingsCache').then(m => m.invalidateSettingsCache()).catch(e => {
            console.warn('[Store] failed to invalidate settings cache after rehydration:', e);
          });
        }).catch(e => console.warn('[Store] rehydration error:', e));
      },
    }
  )
);
