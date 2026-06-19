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
