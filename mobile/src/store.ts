import { create } from 'zustand';

export type ThemeMode = 'light' | 'dark' | 'system';

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
  setApiSettings: (s: { apiBase?: string; apiKey?: string; model?: string }) => void;
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
  { name: 'OpenAI', base: 'https://api.openai.com/v1', models: ['gpt-4o', 'gpt-4o-mini', 'o1-mini'] },
  { name: 'DeepSeek', base: 'https://api.deepseek.com/v1', models: ['deepseek-chat', 'deepseek-reasoner'] },
  { name: '通义千问', base: 'https://dashscope.aliyuncs.com/compatible-mode/v1', models: ['qwen-plus', 'qwen-turbo'] },
  { name: '自定义', base: '', models: [] },
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

export function getColors(isDark: boolean, accent: string) {
  return { ...(isDark ? DARK_COLORS : LIGHT_COLORS), accent };
}

export const useAppStore = create<AppState>((set) => ({
  themeMode: 'system',
  isDark: false,
  accentColor: '#3B82F6',
  setThemeMode: (themeMode) => set({ themeMode }),
  setAccentColor: (accentColor) => set({ accentColor }),
  setIsDark: (isDark) => set({ isDark }),

  apiBase: 'https://api.openai.com/v1',
  apiKey: '',
  model: 'gpt-4o-mini',
  setApiSettings: (s) => set((state) => ({
    apiBase: s.apiBase ?? state.apiBase,
    apiKey: s.apiKey ?? state.apiKey,
    model: s.model ?? state.model,
  })),
}));
