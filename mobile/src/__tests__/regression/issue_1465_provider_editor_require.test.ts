/**
 * Regression test for #1465 — ProviderEditor must import PROVIDERS as a
 * value (not require() in component body).
 *
 * Verifies:
 * 1. PROVIDERS is exported as a runtime value from store
 * 2. PROVIDERS has the expected shape (name, base, format, models)
 */

import { PROVIDERS } from '../../store';

describe('issue #1465 — ProviderEditor require() removal', () => {
  test('PROVIDERS is a non-empty array', () => {
    expect(Array.isArray(PROVIDERS)).toBe(true);
    expect(PROVIDERS.length).toBeGreaterThan(0);
  });

  test('each PROVIDERS entry has required fields', () => {
    for (const p of PROVIDERS) {
      expect(typeof p.name).toBe('string');
      expect(p.name.length).toBeGreaterThan(0);
      expect(typeof p.base).toBe('string');
      expect(p.base).toMatch(/^https?:\/\//);
      expect(['openai', 'anthropic']).toContain(p.format);
      expect(Array.isArray(p.models)).toBe(true);
      expect(p.models.length).toBeGreaterThan(0);
    }
  });

  test('PROVIDERS includes OpenAI and Anthropic presets', () => {
    const names = PROVIDERS.map(p => p.name);
    expect(names).toContain('OpenAI');
    expect(names).toContain('Anthropic');
  });

  test('Anthropic preset uses anthropic format', () => {
    const anthropic = PROVIDERS.find(p => p.name === 'Anthropic');
    expect(anthropic).toBeDefined();
    expect(anthropic!.format).toBe('anthropic');
  });
});
