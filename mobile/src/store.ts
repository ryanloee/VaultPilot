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
  } catch { return []; }
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
function syncLegacyFields(set: any, providers: ProviderConfig[], activeProviderIndex: number) {
  if (providers.length === 0) return;
  const idx = Math.min(activeProviderIndex, providers.length - 1);
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
        const newState = {
          apiBase: s.apiBase ?? state.apiBase,
          apiKey: s.apiKey ?? state.apiKey,
          model: s.model ?? state.model,
          apiFormat: s.apiFormat ?? state.apiFormat,
        };
        // Also sync to active provider
        const providers = [...state.providers];
        if (providers.length > 0) {
          const idx = Math.min(state.activeProviderIndex, providers.length - 1);
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
        setTimeout(() => syncLegacyFields(set, providers, activeProviderIndex), 0);
        saveProviderKeysSecure(providers);
        return { providers, activeProviderIndex };
      }),

      removeProvider: (index) => set((state) => {
        const providers = state.providers.filter((_, i) => i !== index);
        let activeProviderIndex = state.activeProviderIndex;
        if (activeProviderIndex >= providers.length) {
          activeProviderIndex = Math.max(0, providers.length - 1);
        }
        if (providers.length > 0) {
          setTimeout(() => syncLegacyFields(set, providers, activeProviderIndex), 0);
        }
        saveProviderKeysSecure(providers);
        return { providers, activeProviderIndex };
      }),

      updateProvider: (index, p) => set((state) => {
        const providers = [...state.providers];
        providers[index] = { ...providers[index], ...p };
        if (index === Math.min(state.activeProviderIndex, providers.length - 1)) {
          setTimeout(() => syncLegacyFields(set, providers, state.activeProviderIndex), 0);
        }
        saveProviderKeysSecure(providers);
        return { providers };
      }),

      setActiveProvider: (index) => set((state) => {
        const activeProviderIndex = Math.min(index, state.providers.length - 1);
        setTimeout(() => syncLegacyFields(set, state.providers, activeProviderIndex), 0);
        return { activeProviderIndex };
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
        providers: state.providers.map(p => ({ ...p, apiKey: '' })),
        activeProviderIndex: state.activeProviderIndex,
      }),
      onRehydrateStorage: () => (state) => {
        if (!state) return;
        // Restore API keys from SecureStore after hydration
        loadProviderKeysSecure().then(keys => {
          if (keys.length === 0) return;
          const providers = state.providers.map((p, i) => ({
            ...p,
            apiKey: keys[i] ?? '',
          }));
          state.providers = providers;
          syncLegacyFields(useAppStore.setState, providers, state.activeProviderIndex);
        });
      },
    }
  )
);
