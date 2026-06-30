/**
 * Unit tests for response style quick-switch (#1965).
 *
 * Tests: ResponseStyle type, RESPONSE_STYLE_LABELS keys,
 * buildSystemPrompt with brief/standard/detailed styles.
 */

import {
  buildSystemPrompt,
  RESPONSE_STYLE_LABELS,
  ResponseStyle,
} from '../../services/rag';

// ── RESPONSE_STYLE_LABELS ──────────────────────────────────

describe('RESPONSE_STYLE_LABELS (#1965)', () => {
  it('has exactly three keys: brief, standard, detailed', () => {
    const keys = Object.keys(RESPONSE_STYLE_LABELS);
    expect(keys).toHaveLength(3);
    expect(keys).toContain('brief');
    expect(keys).toContain('standard');
    expect(keys).toContain('detailed');
  });

  it('every label is a non-empty string', () => {
    for (const key of Object.keys(RESPONSE_STYLE_LABELS) as ResponseStyle[]) {
      expect(typeof RESPONSE_STYLE_LABELS[key]).toBe('string');
      expect(RESPONSE_STYLE_LABELS[key].length).toBeGreaterThan(0);
    }
  });
});

// ── buildSystemPrompt with style ───────────────────────────

describe('buildSystemPrompt response styles (#1965)', () => {
  it('default style is standard (no extra instruction)', () => {
    const prompt = buildSystemPrompt(null);
    const standardExplicit = buildSystemPrompt(null, 'standard');
    // standard should be identical to default
    expect(prompt).toBe(standardExplicit);
  });

  it('brief style injects conciseness instruction', () => {
    const prompt = buildSystemPrompt(null, 'brief');
    // Should contain a brief/concise indicator in either language
    expect(
      prompt.includes('简洁') || prompt.includes('Brief') || prompt.includes('concise'),
    ).toBe(true);
  });

  it('detailed style injects thoroughness instruction', () => {
    const prompt = buildSystemPrompt(null, 'detailed');
    // Should contain a detailed/thorough indicator
    expect(
      prompt.includes('详细') || prompt.includes('Detailed') || prompt.includes('thorough'),
    ).toBe(true);
  });

  it('standard style does NOT inject extra style instruction', () => {
    const prompt = buildSystemPrompt(null, 'standard');
    // The style marker header should not appear in standard mode
    expect(prompt).not.toContain('回答风格');
    expect(prompt).not.toContain('Response Style —');
  });

  it('brief and detailed produce different prompts', () => {
    const brief = buildSystemPrompt(null, 'brief');
    const detailed = buildSystemPrompt(null, 'detailed');
    expect(brief).not.toBe(detailed);
  });

  it('style instruction is appended AFTER base prompt (security rules still present)', () => {
    const prompt = buildSystemPrompt(null, 'detailed');
    // Core security rule must still be present regardless of style
    expect(prompt).toContain('VaultPilot');
  });

  it('works with note context + style together', () => {
    const prompt = buildSystemPrompt('Some note content', 'brief');
    // Both note context and style should be present
    expect(prompt).toContain('Some note content');
    expect(
      prompt.includes('简洁') || prompt.includes('Brief') || prompt.includes('concise'),
    ).toBe(true);
  });
});
