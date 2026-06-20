import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import AsyncStorage from '@react-native-async-storage/async-storage';

export type ThemeMode = 'light' | 'dark' | 'system';

const VALID_THEME_MODES: ThemeMode[] = ['light', 'dark', 'system'];
export function isValidThemeMode(v: string): v is ThemeMode {
  return (VALID_THEME_MODES as string[]).includes(v);
}

export type ApiFormat = 'openai' | 'anthropic';

interface AppState {
  themeMode: ThemeMode;
  isDark: boolean;
  accentColor: string;
  setThemeMode: (mode: ThemeMode) => void;
  setAccentColor: (color: string) => void;
  setIsDark: (dark: boolean) => void;

  apiBase: string;
  apiKey: string;
  model: string;
  apiFormat: ApiFormat;
  setApiSettings: (s: { apiBase?: string; apiKey?: string; model?: string; apiFormat?: ApiFormat }) => void;
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

export const useAppStore = create<AppState>()(
  persist(
    (set) => ({
      themeMode: 'system',
      isDark: false,
      accentColor: '#3B82F6',
      setThemeMode: (themeMode) => set({ themeMode }),
      setAccentColor: (accentColor) => set({ accentColor }),
      setIsDark: (isDark) => set({ isDark }),

      apiBase: 'https://api.openai.com/v1',
      apiKey: '',
      model: 'gpt-4o-mini',
      apiFormat: 'openai' as ApiFormat,
      setApiSettings: (s) => set((state) => ({
        apiBase: s.apiBase ?? state.apiBase,
        apiKey: s.apiKey ?? state.apiKey,
        model: s.model ?? state.model,
        apiFormat: s.apiFormat ?? state.apiFormat,
      })),
    }),
    {
      name: 'vaultpilot-store',
      storage: createJSONStorage(() => AsyncStorage),
      partialize: (state) => ({
        themeMode: state.themeMode,
        accentColor: state.accentColor,
        apiBase: state.apiBase,
        model: state.model,
        apiFormat: state.apiFormat,
        // apiKey excluded — stored separately in SecureStore
      }),
    }
  )
);
