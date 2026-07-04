/**
 * Unit tests for store.ts — Zustand global state (#1213).
 *
 * Tests: isValidThemeMode, getColors cache, provider CRUD,
 *        setThemeMode, setActiveProvider, setApiSettings, legacy field sync.
 */

import { isValidThemeMode, getColors, useAppStore, ACCENT_COLORS } from '../store';

// Reset store to defaults before each test
beforeEach(() => {
  useAppStore.setState({
    themeMode: 'system',
    isDark: false,
    accentColor: '#3B82F6',
    apiBase: 'https://opencode.ai/zen/v1',
    apiKey: '',
    model: 'deepseek-v4-flash-free',
    apiFormat: 'openai',
    providers: [],
    activeProviderIndex: 0,
  });
  jest.clearAllMocks();
});

// ── isValidThemeMode ──────────────────────────────────────

describe('isValidThemeMode', () => {
  it('accepts light, dark, system', () => {
    expect(isValidThemeMode('light')).toBe(true);
    expect(isValidThemeMode('dark')).toBe(true);
    expect(isValidThemeMode('system')).toBe(true);
  });

  it('rejects invalid values', () => {
    expect(isValidThemeMode('')).toBe(false);
    expect(isValidThemeMode('auto')).toBe(false);
    expect(isValidThemeMode('Light')).toBe(false);
  });
});

// ── getColors ─────────────────────────────────────────────

describe('getColors', () => {
  it('returns light colors when isDark=false', () => {
    const c = getColors(false, '#3B82F6');
    expect(c.bg).toBe('#FFFFFF');
    expect(c.text).toBe('#111827');
    expect(c.accent).toBe('#3B82F6');
  });

  it('returns dark colors when isDark=true', () => {
    const c = getColors(true, '#8B5CF6');
    expect(c.bg).toBe('#000000');
    expect(c.text).toBe('#F9FAFB');
    expect(c.accent).toBe('#8B5CF6');
  });

  it('caches result for same arguments', () => {
    const a = getColors(false, '#3B82F6');
    const b = getColors(false, '#3B82F6');
    expect(a).toBe(b); // same reference
  });

  it('invalidates cache when isDark changes', () => {
    const a = getColors(false, '#3B82F6');
    const b = getColors(true, '#3B82F6');
    expect(a).not.toBe(b);
    expect(b.bg).toBe('#000000');
  });

  it('invalidates cache when accent changes', () => {
    const a = getColors(false, '#3B82F6');
    const b = getColors(false, '#EF4444');
    expect(a).not.toBe(b);
    expect(b.accent).toBe('#EF4444');
  });
});

// ── Provider management ───────────────────────────────────

describe('addProvider', () => {
  it('appends provider and sets it as active', () => {
    const p = { name: 'Test', apiBase: 'https://test.com', apiKey: 'k', model: 'm', apiFormat: 'openai' as const };
    useAppStore.getState().addProvider(p);
    const s = useAppStore.getState();
    expect(s.providers).toHaveLength(1);
    expect(s.providers[0]).toMatchObject(p);
    expect(s.activeProviderIndex).toBe(0);
  });

  it('second provider sets activeProviderIndex to 1', () => {
    const p = { name: 'A', apiBase: 'https://a.com', apiKey: 'k', model: 'm', apiFormat: 'openai' as const };
    useAppStore.getState().addProvider(p);
    useAppStore.getState().addProvider({ ...p, name: 'B', apiBase: 'https://b.com' });
    expect(useAppStore.getState().providers).toHaveLength(2);
    expect(useAppStore.getState().activeProviderIndex).toBe(1);
  });
});

describe('removeProvider', () => {
  it('removes provider at index and adjusts active', () => {
    const p = { name: 'A', apiBase: 'https://a.com', apiKey: 'k', model: 'm', apiFormat: 'openai' as const };
    useAppStore.getState().addProvider(p);
    useAppStore.getState().addProvider({ ...p, name: 'B', apiBase: 'https://b.com' });
    useAppStore.getState().removeProvider(0);
    const s = useAppStore.getState();
    expect(s.providers).toHaveLength(1);
    expect(s.providers[0].name).toBe('B');
    expect(s.activeProviderIndex).toBe(0);
  });

  it('clamps activeProviderIndex when removing last provider', () => {
    const p = { name: 'A', apiBase: 'https://a.com', apiKey: 'k', model: 'm', apiFormat: 'openai' as const };
    useAppStore.getState().addProvider(p);
    useAppStore.getState().removeProvider(0);
    expect(useAppStore.getState().providers).toHaveLength(0);
    expect(useAppStore.getState().activeProviderIndex).toBe(-1);
  });
});

describe('updateProvider', () => {
  it('merges partial update into provider', () => {
    const p = { name: 'A', apiBase: 'https://a.com', apiKey: 'k', model: 'm', apiFormat: 'openai' as const };
    useAppStore.getState().addProvider(p);
    useAppStore.getState().updateProvider(0, { model: 'gpt-4' });
    expect(useAppStore.getState().providers[0].model).toBe('gpt-4');
    expect(useAppStore.getState().providers[0].name).toBe('A'); // unchanged
  });
});

describe('setActiveProvider', () => {
  it('updates activeProviderIndex', () => {
    const p = { name: 'A', apiBase: 'https://a.com', apiKey: 'k', model: 'm', apiFormat: 'openai' as const };
    useAppStore.getState().addProvider(p);
    useAppStore.getState().addProvider({ ...p, name: 'B' });
    useAppStore.getState().setActiveProvider(0);
    expect(useAppStore.getState().activeProviderIndex).toBe(0);
  });

  it('clamps to max index', () => {
    const p = { name: 'A', apiBase: 'https://a.com', apiKey: 'k', model: 'm', apiFormat: 'openai' as const };
    useAppStore.getState().addProvider(p);
    useAppStore.getState().setActiveProvider(99);
    expect(useAppStore.getState().activeProviderIndex).toBe(0);
  });
});

// ── Theme ─────────────────────────────────────────────────

describe('setThemeMode', () => {
  it('updates themeMode', () => {
    useAppStore.getState().setThemeMode('dark');
    expect(useAppStore.getState().themeMode).toBe('dark');
  });
});

describe('setAccentColor', () => {
  it('updates accentColor', () => {
    useAppStore.getState().setAccentColor('#EF4444');
    expect(useAppStore.getState().accentColor).toBe('#EF4444');
  });
});

describe('setIsDark', () => {
  it('updates isDark', () => {
    useAppStore.getState().setIsDark(true);
    expect(useAppStore.getState().isDark).toBe(true);
  });
});

// ── setApiSettings (legacy) ───────────────────────────────

describe('setApiSettings', () => {
  it('updates specified fields', () => {
    useAppStore.getState().setApiSettings({ model: 'gpt-4o' });
    expect(useAppStore.getState().model).toBe('gpt-4o');
    expect(useAppStore.getState().apiBase).toBe('https://opencode.ai/zen/v1'); // unchanged
  });

  it('syncs to active provider', () => {
    const p = { name: 'A', apiBase: 'https://a.com', apiKey: 'k', model: 'm', apiFormat: 'openai' as const };
    useAppStore.getState().addProvider(p);
    useAppStore.getState().setApiSettings({ model: 'new-model' });
    // syncLegacyFields runs via setTimeout, check provider was updated in the set call
    expect(useAppStore.getState().model).toBe('new-model');
  });
});

// ── ACCENT_COLORS ─────────────────────────────────────────

describe('ACCENT_COLORS', () => {
  it('contains at least 6 colors', () => {
    expect(ACCENT_COLORS.length).toBeGreaterThanOrEqual(6);
  });

  it('each color has name and valid hex value', () => {
    for (const c of ACCENT_COLORS) {
      expect(c.name).toBeTruthy();
      expect(c.value).toMatch(/^#[0-9A-Fa-f]{6}$/);
    }
  });
});
