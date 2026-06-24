import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

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
const SECURE_KEYS_ID = 'vaultpilot_provider_keys';

async function saveProviderKeysSecure(providers: ProviderConfig[]): Promise<void> {
  const keys = providers.map(p => p.apiKey);
  try {
    await SecureStore.setItemAsync(SECURE_KEYS_ID, JSON.stringify(keys));
  } catch (e) {
    console.warn('[SecureStore] Failed to save provider keys:', e);
  }
}

async function loadProviderKeysSecure(): Promise<string[]> {
  try {
    const raw = await SecureStore.getItemAsync(SECURE_KEYS_ID);
    return raw ? JSON.parse(raw) : [];
  } catch (e) { console.warn('[Store] Failed to load provider keys from SecureStore:', e); return []; }
}

interface AppState {
  themeMode: ThemeMode;
  isDark: boolean;
  accentColor: string;
  setThemeMode: (mode: ThemeMode) => void;
  setAccentColor: (color: string) => void;
  setIsDark: (dark: boolean) => void;

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

/** Clamp provider index to valid range. */
export function clampProviderIndex(index: number, providerCount: number): number {
  if (providerCount === 0) return 0;
  return Math.min(index, providerCount - 1);
}

/** Remove a provider by index, returning a new array. */
export function removeProviderFromList(providers: ProviderConfig[], index: number): ProviderConfig[] {
  return providers.filter((_, i) => i !== index);
}

/** Compute the new active index after removing a provider. */
export function computeActiveIndexAfterRemove(currentIndex: number, newLength: number): number {
  if (newLength === 0) return 0;
  return currentIndex >= newLength ? newLength - 1 : currentIndex;
}

/** Update a provider at index with partial fields, returning a new array. */
export function updateProviderInList(providers: ProviderConfig[], index: number, patch: Partial<ProviderConfig>): ProviderConfig[] {
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

/** Restore API keys into providers from SecureStore keys array. */
export function restoreProviderKeys(providers: ProviderConfig[], keys: string[]): ProviderConfig[] {
  return providers.map((p, i) => ({ ...p, apiKey: keys[i] ?? '' }));
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

      apiBase: 'https://opencode.ai/zen/v1',
      apiKey: '',
      model: 'deepseek-v4-flash-free',
      apiFormat: 'openai' as ApiFormat,
      setApiSettings: (s) => set((state) => {
        const newState = mergeApiSettings(state, s);
        // Also sync to active provider
        const providers = [...state.providers];
        if (providers.length > 0) {
          const idx = clampProviderIndex(state.activeProviderIndex, providers.length);
          providers[idx] = { ...providers[idx], ...newState };
          saveProviderKeysSecure(providers);
        }
        return { ...newState, providers };
      }),

      providers: [],
      activeProviderIndex: 0,

      addProvider: (p) => set((state) => {
        const providers = [...state.providers, p];
        const activeProviderIndex = providers.length - 1;
        saveProviderKeysSecure(providers);
        const active = providers[activeProviderIndex];
        return {
          providers, activeProviderIndex,
          apiBase: active.apiBase, apiKey: active.apiKey,
          model: active.model, apiFormat: active.apiFormat,
        };
      }),

      removeProvider: (index) => set((state) => {
        const providers = removeProviderFromList(state.providers, index);
        const activeProviderIndex = computeActiveIndexAfterRemove(state.activeProviderIndex, providers.length);
        saveProviderKeysSecure(providers);
        const update: Partial<AppState> = { providers, activeProviderIndex };
        if (providers.length > 0) {
          const active = providers[activeProviderIndex];
          update.apiBase = active.apiBase;
          update.apiKey = active.apiKey;
          update.model = active.model;
          update.apiFormat = active.apiFormat;
        }
        return update;
      }),

      updateProvider: (index, p) => set((state) => {
        const providers = updateProviderInList(state.providers, index, p);
        saveProviderKeysSecure(providers);
        const update: Partial<AppState> = { providers };
        if (index === clampProviderIndex(state.activeProviderIndex, providers.length)) {
          const active = providers[index];
          update.apiBase = active.apiBase;
          update.apiKey = active.apiKey;
          update.model = active.model;
          update.apiFormat = active.apiFormat;
        }
        return update;
      }),

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
        accentColor: state.accentColor,
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
          if (keys.length === 0) return;
          const restored = restoreProviderKeys(state.providers, keys);
          useAppStore.setState({ providers: restored });
          syncLegacyFields(useAppStore.setState, restored, state.activeProviderIndex);
        });
      },
    }
  )
);
