/**
 * Regression tests for #1429: store.ts pure function extraction + unit tests.
 *
 * Covers clampProviderIndex, removeProviderFromList, computeActiveIndexAfterRemove,
 * updateProviderInList, mergeApiSettings, sanitizeForPersistence, restoreProviderKeys.
 */

import {
  clampProviderIndex,
  removeProviderFromList,
  computeActiveIndexAfterRemove,
  updateProviderInList,
  mergeApiSettings,
  sanitizeForPersistence,
  restoreProviderKeys,
  isValidThemeMode,
  getColors,
  type ProviderConfig,
} from '../../store';

const makeProvider = (name: string, overrides?: Partial<ProviderConfig>): ProviderConfig => ({
  name,
  apiBase: `https://${name}.com/v1`,
  apiKey: `key-${name}`,
  model: 'gpt-4',
  apiFormat: 'openai',
  ...overrides,
});

// ── clampProviderIndex ────────────────────────────────────

describe('clampProviderIndex', () => {
  test('returns 0 for empty providers', () => {
    expect(clampProviderIndex(5, 0)).toBe(0);
  });

  test('returns index when within range', () => {
    expect(clampProviderIndex(1, 3)).toBe(1);
    expect(clampProviderIndex(0, 3)).toBe(0);
    expect(clampProviderIndex(2, 3)).toBe(2);
  });

  test('clamps to last index when out of range', () => {
    expect(clampProviderIndex(5, 3)).toBe(2);
    expect(clampProviderIndex(100, 1)).toBe(0);
  });

  test('handles single provider', () => {
    expect(clampProviderIndex(0, 1)).toBe(0);
    expect(clampProviderIndex(1, 1)).toBe(0);
  });
});

// ── removeProviderFromList ────────────────────────────────

describe('removeProviderFromList', () => {
  test('removes provider at index', () => {
    const providers = [makeProvider('a'), makeProvider('b'), makeProvider('c')];
    const result = removeProviderFromList(providers, 1);
    expect(result).toHaveLength(2);
    expect(result[0].name).toBe('a');
    expect(result[1].name).toBe('c');
  });

  test('removes first provider', () => {
    const providers = [makeProvider('a'), makeProvider('b')];
    const result = removeProviderFromList(providers, 0);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe('b');
  });

  test('removes last provider', () => {
    const providers = [makeProvider('a'), makeProvider('b')];
    const result = removeProviderFromList(providers, 1);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe('a');
  });

  test('returns empty array when removing from single-element list', () => {
    const result = removeProviderFromList([makeProvider('only')], 0);
    expect(result).toHaveLength(0);
  });

  test('does not mutate original array', () => {
    const providers = [makeProvider('a'), makeProvider('b')];
    const result = removeProviderFromList(providers, 0);
    expect(providers).toHaveLength(2);
    expect(result).toHaveLength(1);
  });
});

// ── computeActiveIndexAfterRemove ─────────────────────────

describe('computeActiveIndexAfterRemove', () => {
  test('returns 0 for empty list', () => {
    expect(computeActiveIndexAfterRemove(2, 0, 0)).toBe(0);
  });

  test('keeps index when removed item is after active', () => {
    expect(computeActiveIndexAfterRemove(0, 2, 3)).toBe(0);
    expect(computeActiveIndexAfterRemove(1, 2, 3)).toBe(1);
  });

  test('shifts index left when removed item is before active', () => {
    // providers: A(0), B(1), C(2), active=B(1), remove A(0) → B becomes 0
    expect(computeActiveIndexAfterRemove(1, 0, 2)).toBe(0);
    // providers: A(0), B(1), C(2), active=C(2), remove A(0) → C becomes 1
    expect(computeActiveIndexAfterRemove(2, 0, 2)).toBe(1);
  });

  test('clamps when active item itself is deleted', () => {
    expect(computeActiveIndexAfterRemove(2, 2, 2)).toBe(1);
    expect(computeActiveIndexAfterRemove(5, 5, 1)).toBe(0);
  });

  test('edge: index equals new length after deletion', () => {
    expect(computeActiveIndexAfterRemove(2, 2, 2)).toBe(1);
  });
});

// ── updateProviderInList ──────────────────────────────────

describe('updateProviderInList', () => {
  test('updates provider fields', () => {
    const providers = [makeProvider('a'), makeProvider('b')];
    const result = updateProviderInList(providers, 1, { model: 'claude-3' });
    expect(result[1].model).toBe('claude-3');
    expect(result[1].name).toBe('b'); // unchanged
    expect(result[0].model).toBe('gpt-4'); // other provider unchanged
  });

  test('does not mutate original array', () => {
    const providers = [makeProvider('a')];
    const result = updateProviderInList(providers, 0, { model: 'new' });
    expect(providers[0].model).toBe('gpt-4');
    expect(result[0].model).toBe('new');
  });

  test('updates multiple fields', () => {
    const providers = [makeProvider('a')];
    const result = updateProviderInList(providers, 0, { model: 'new', apiKey: 'new-key' });
    expect(result[0].model).toBe('new');
    expect(result[0].apiKey).toBe('new-key');
  });
});

// ── mergeApiSettings ──────────────────────────────────────

describe('mergeApiSettings', () => {
  const current = { apiBase: 'https://old.com', apiKey: 'old-key', model: 'gpt-4', apiFormat: 'openai' as const };

  test('returns current when patch is empty', () => {
    expect(mergeApiSettings(current, {})).toEqual(current);
  });

  test('overrides specified fields', () => {
    const result = mergeApiSettings(current, { model: 'claude-3', apiKey: 'new-key' });
    expect(result.model).toBe('claude-3');
    expect(result.apiKey).toBe('new-key');
    expect(result.apiBase).toBe('https://old.com'); // unchanged
  });

  test('overrides all fields', () => {
    const result = mergeApiSettings(current, {
      apiBase: 'https://new.com', apiKey: 'k', model: 'm', apiFormat: 'anthropic',
    });
    expect(result).toEqual({ apiBase: 'https://new.com', apiKey: 'k', model: 'm', apiFormat: 'anthropic' });
  });
});

// ── sanitizeForPersistence ────────────────────────────────

describe('sanitizeForPersistence', () => {
  test('strips API keys from all providers', () => {
    const providers = [makeProvider('a'), makeProvider('b')];
    const result = sanitizeForPersistence(providers);
    expect(result[0].apiKey).toBe('');
    expect(result[1].apiKey).toBe('');
    expect(result[0].name).toBe('a'); // other fields preserved
  });

  test('returns empty array for empty input', () => {
    expect(sanitizeForPersistence([])).toEqual([]);
  });

  test('does not mutate original', () => {
    const providers = [makeProvider('a')];
    sanitizeForPersistence(providers);
    expect(providers[0].apiKey).toBe('key-a');
  });
});

// ── restoreProviderKeys ───────────────────────────────────

describe('restoreProviderKeys', () => {
  test('restores keys by index', () => {
    const providers = [makeProvider('a', { apiKey: '' }), makeProvider('b', { apiKey: '' })];
    const result = restoreProviderKeys(providers, ['key-a', 'key-b']);
    expect(result[0].apiKey).toBe('key-a');
    expect(result[1].apiKey).toBe('key-b');
  });

  test('uses empty string for missing keys', () => {
    const providers = [makeProvider('a', { apiKey: '' }), makeProvider('b', { apiKey: '' })];
    const result = restoreProviderKeys(providers, ['key-a']);
    expect(result[0].apiKey).toBe('key-a');
    expect(result[1].apiKey).toBe('');
  });

  test('handles empty keys array', () => {
    const providers = [makeProvider('a', { apiKey: '' })];
    const result = restoreProviderKeys(providers, []);
    expect(result[0].apiKey).toBe('');
  });
});

// ── isValidThemeMode (existing, verify still works) ───────

describe('isValidThemeMode', () => {
  test('accepts valid modes', () => {
    expect(isValidThemeMode('light')).toBe(true);
    expect(isValidThemeMode('dark')).toBe(true);
    expect(isValidThemeMode('system')).toBe(true);
  });

  test('rejects invalid modes', () => {
    expect(isValidThemeMode('auto')).toBe(false);
    expect(isValidThemeMode('')).toBe(false);
    expect(isValidThemeMode('Light')).toBe(false);
  });
});

// ── getColors (existing, verify caching) ──────────────────

describe('getColors', () => {
  test('returns light colors when not dark', () => {
    const c = getColors(false, '#3B82F6');
    expect(c.bg).toBe('#FFFFFF');
    expect(c.accent).toBe('#3B82F6');
  });

  test('returns dark colors when dark', () => {
    const c = getColors(true, '#EF4444');
    expect(c.bg).toBe('#000000');
    expect(c.accent).toBe('#EF4444');
  });

  test('returns cached result for same params', () => {
    const c1 = getColors(false, '#3B82F6');
    const c2 = getColors(false, '#3B82F6');
    expect(c1).toBe(c2); // same reference = cached
  });

  test('returns new result when params change', () => {
    const c1 = getColors(false, '#3B82F6');
    const c2 = getColors(true, '#3B82F6');
    expect(c1).not.toBe(c2);
  });
});
