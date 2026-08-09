/**
 * Regression tests for #4006 / #4007 — mobile URI gate mirror drift vs the
 * Rust classifier (src/deep_link.rs).
 *
 * #4006 — TS parseVaultPilotUri disagreed with Rust parse_deep_link on:
 *   - empty path segments (note//delete degraded HIGH→Medium in TS);
 *   - duplicate overwrite params (TS last-wins vs Rust any-truthy);
 *   - note ids not percent-decoded (encoded deep links failed to open).
 *
 * #4007 — the TS route table itself drifted:
 *   - chat / chat/sessions / note are intentional mobile navigation
 *     extensions (App.tsx) but were classified LOW; Rust has no such routes
 *     (Unknown → Medium), so they are now MEDIUM — the mobile gate is never
 *     weaker than the Rust side;
 *   - daily (Rust Daily → LOW) was missing entirely;
 *   - x-source param KEY matching is now case-insensitive (Rust
 *     parse_xcallback uses eq_ignore_ascii_case).
 *
 * All tests are pure-function tests (no React rendering) per convention.
 */

import {
  classifyUriActionRisk,
  evaluateUriSafety,
  extractSource,
  parseVaultPilotUri,
  type UriActionRisk,
} from '../../utils/uriSafety';

describe('#4006 empty path segments are filtered (Rust: split("/").filter(|s| !s.is_empty()))', () => {
  const cases: Array<[string, UriActionRisk]> = [
    // note//delete === note/delete → HIGH (was medium/unknown in TS)
    ['vaultpilot://note//delete', 'high'],
    ['vaultpilot://note//delete?id=abc', 'high'],
    // chat//new === chat/new → HIGH
    ['vaultpilot://chat//new', 'high'],
    // note//abc === note/abc → LOW OpenNote
    ['vaultpilot://note//abc', 'low'],
    // note// → bare note route (mobile extension) → medium
    ['vaultpilot://note//', 'medium'],
  ];

  it.each(cases)('%s → %s', (uri, expected) => {
    expect(classifyUriActionRisk(uri)).toBe(expected);
  });

  it('note//abc opens note "abc"', () => {
    const parsed = parseVaultPilotUri('vaultpilot://note//abc');
    expect(parsed.route).toBe('note/:id');
    expect(parsed.noteId).toBe('abc');
  });
});

describe('#4006 overwrite flag is any-truthy across duplicate params (Rust flag())', () => {
  it('duplicate overwrite params: any truthy value wins, not last-wins', () => {
    // Rust: pairs.iter().any(|(k,v)| k=="overwrite" && is_truthy(v)) → HIGH
    expect(classifyUriActionRisk('vaultpilot://note/new?overwrite=1&overwrite=0')).toBe('high');
    expect(classifyUriActionRisk('vaultpilot://note/new?overwrite=0&overwrite=1')).toBe('high');
    expect(classifyUriActionRisk('vaultpilot://note/new?overwrite=0&overwrite=0')).toBe('medium');
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite=1&overwrite=0').overwrite).toBe(true);
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite=0&overwrite=1').overwrite).toBe(true);
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite=0&overwrite=0').overwrite).toBe(false);
  });

  it('bare overwrite (no "=") is dropped like Rust split_once failure', () => {
    expect(classifyUriActionRisk('vaultpilot://note/new?overwrite')).toBe('medium');
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite').overwrite).toBeUndefined();
  });
});

describe('#4006 note ids are percent-decoded (Rust url_decode, "+" → space)', () => {
  it('decodes %20', () => {
    const parsed = parseVaultPilotUri('vaultpilot://note/hello%20world');
    expect(parsed.route).toBe('note/:id');
    expect(parsed.noteId).toBe('hello world');
  });

  it('decodes "+" to a space like Rust url_decode', () => {
    const parsed = parseVaultPilotUri('vaultpilot://note/hello+world');
    expect(parsed.noteId).toBe('hello world');
  });

  it('decodes in the note/open/<id> alias too', () => {
    const parsed = parseVaultPilotUri('vaultpilot://note/open/hello%20world');
    expect(parsed.route).toBe('note/:id');
    expect(parsed.noteId).toBe('hello world');
  });

  it('keeps malformed percent-encoding literally (Rust is lenient, no throw)', () => {
    const parsed = parseVaultPilotUri('vaultpilot://note/hello%zz');
    expect(parsed.noteId).toBe('hello%zz');
  });
});

describe('#4007 route table parity — mobile extensions are MEDIUM, daily is LOW', () => {
  const cases: Array<[string, UriActionRisk]> = [
    // Rust: these are Unknown → Medium. Mobile keeps them as navigation
    // extensions (App.tsx) but with Rust-equivalent risk.
    ['vaultpilot://chat', 'medium'],
    ['vaultpilot://chat/sessions', 'medium'],
    ['vaultpilot://note', 'medium'],
    // Rust has a dedicated Daily route → Low (#4007).
    ['vaultpilot://daily', 'low'],
    ['vaultpilot://Daily', 'low'], // case-insensitive route keywords (#3734)
  ];

  it.each(cases)('%s → %s', (uri, expected) => {
    expect(classifyUriActionRisk(uri)).toBe(expected);
  });

  it('parses daily as its own route', () => {
    expect(parseVaultPilotUri('vaultpilot://daily').route).toBe('daily');
  });
});

describe('#4007 x-source param key matched case-insensitively (Rust eq_ignore_ascii_case)', () => {
  it('recognizes X-SOURCE / X-Source / x-source', () => {
    expect(extractSource('vaultpilot://note/new?X-SOURCE=com.app')).toBe('com.app');
    expect(extractSource('vaultpilot://note/new?X-Source=com.app')).toBe('com.app');
    expect(extractSource('vaultpilot://note/new?x-source=com.app')).toBe('com.app');
  });

  it('a trusted X-SOURCE variant auto-allows a medium action', () => {
    const r = evaluateUriSafety('vaultpilot://note/new?X-SOURCE=com.app', ['com.app']);
    expect(r.risk).toBe('medium');
    expect(r.needsConfirmation).toBe(false);
  });

  it('an untrusted X-SOURCE still needs confirmation', () => {
    const r = evaluateUriSafety('vaultpilot://note/new?X-SOURCE=com.evil', ['com.app']);
    expect(r.risk).toBe('medium');
    expect(r.needsConfirmation).toBe(true);
  });
});
