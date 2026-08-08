/**
 * Regression tests for #3964 / #3995 — vaultpilot:// URI risk-confirmation
 * gate (TS mirror of Rust classify_uri_action_risk, src/deep_link.rs).
 *
 * External apps could trigger vaultpilot://chat/new (silently starts an AI
 * agent) or vaultpilot://note/new?overwrite=true (irreversible overwrite)
 * with zero confirmation. The gate classifies every vaultpilot:// URL and only
 * dispatches risky ones after an explicit Alert confirmation.
 *
 * #3995 — the mirror drifted from the Rust classifier:
 *   - note/delete|edit|bulk are HIGH in Rust but were typed LOW (fallthrough
 *     to note/:id → "open the note named delete/edit/bulk");
 *   - unknown routes are Medium-conservative in Rust, LOW in TS;
 *   - overwrite truthy set {1,true,yes,on} (Rust), TS used an inverted falsy set;
 *   - route keywords case-insensitive in Rust (#3734), TS was case-sensitive;
 *   - TrustedAppRegistry::is_trusted lowercases both sides, TS did not.
 *
 * All tests here are pure-function tests (no React rendering) per the project
 * convention — the classifier/evaluator must not import react-native internals.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import {
  classifyUriActionRisk,
  evaluateUriSafety,
  extractSource,
  parseVaultPilotUri,
  type UriActionRisk,
} from '../../utils/uriSafety';
import { getTrustedSources, addTrustedSource } from '../../utils/uriTrustStore';

describe('#3964/#3995 classifyUriActionRisk — mirror of Rust risk classification', () => {
  const cases: Array<[string, UriActionRisk]> = [
    // Starting an AI chat may trigger agent tool execution → HIGH
    ['vaultpilot://chat/new', 'high'],
    ['vaultpilot://chat/new?x-source=com.example.app', 'high'],
    ['vaultpilot://Chat/New', 'high'], // #3734 case-insensitive route keywords
    // Irreversible overwrite → HIGH
    ['vaultpilot://note/new?overwrite=true', 'high'],
    ['vaultpilot://note/new?overwrite=1', 'high'],
    ['vaultpilot://note/new?overwrite=TRUE', 'high'],
    ['vaultpilot://note/new?overwrite=yes', 'high'],
    ['vaultpilot://note/new?overwrite=on', 'high'],
    ['vaultpilot://note/new?overwrite', 'high'], // bare flag → "true"
    // #3995: destructive note routes → HIGH (Rust DeleteNote/EditNote/BulkNoteOp)
    ['vaultpilot://note/delete', 'high'],
    ['vaultpilot://note/delete?id=abc', 'high'],
    ['vaultpilot://note/edit', 'high'],
    ['vaultpilot://note/edit?id=abc', 'high'],
    ['vaultpilot://note/bulk?op=delete', 'high'],
    ['vaultpilot://note/bulk?op=move', 'high'],
    // Plain note creation → MEDIUM
    ['vaultpilot://note/new', 'medium'],
    ['vaultpilot://note/new?overwrite=false', 'medium'],
    ['vaultpilot://note/new?overwrite=0', 'medium'],
    ['vaultpilot://note/new?overwrite=no', 'medium'],
    ['vaultpilot://note/new?overwrite=off', 'medium'],
    ['vaultpilot://note/new?overwrite=2', 'medium'], // #3995: 2 is NOT in {1,true,yes,on}
    ['vaultpilot://note/new/', 'medium'], // trailing slash normalized
    // Open existing note (incl. note/open/<id> alias) → LOW
    ['vaultpilot://note/123', 'low'],
    ['vaultpilot://note/abc-def', 'low'],
    ['vaultpilot://note/9f8e7d6c-5b4a-4321-9876-fedcba987654', 'low'],
    ['vaultpilot://note/open/abc', 'low'], // #3995 OpenNote alias
    // Navigation targets → LOW
    ['vaultpilot://search', 'low'],
    ['vaultpilot://settings', 'low'],
    ['vaultpilot://chat', 'low'],
    ['vaultpilot://chat/sessions', 'low'],
    ['vaultpilot://note', 'low'],
    // Unknown / unparseable → MEDIUM (Rust: Unknown → conservative Medium)
    ['vaultpilot://bogus/route', 'medium'],
    ['vaultpilot://', 'medium'],
    ['vaultpilot://note/a/b', 'medium'],
    ['not-a-uri', 'medium'],
    ['', 'medium'],
    ['https://example.com/chat/new', 'medium'],
    // malformed query pair is skipped, path still chat/new → HIGH
    ['vaultpilot://chat/new?x-source=%zz', 'high'],
  ];

  it.each(cases)('%s → %s', (uri, expected) => {
    expect(classifyUriActionRisk(uri)).toBe(expected);
  });
});

describe('#3964 evaluateUriSafety — trust semantics', () => {
  it('medium risk needs confirmation when source is unknown', () => {
    const r = evaluateUriSafety('vaultpilot://note/new', []);
    expect(r.risk).toBe('medium');
    expect(r.needsConfirmation).toBe(true);
    expect(r.reason.length).toBeGreaterThan(0);
  });

  it('medium risk needs confirmation even with a non-empty trust list but no x-source', () => {
    const r = evaluateUriSafety('vaultpilot://note/new', ['com.example.app']);
    expect(r.risk).toBe('medium');
    expect(r.needsConfirmation).toBe(true);
  });

  it('medium risk does NOT need confirmation when x-source is trusted', () => {
    const r = evaluateUriSafety('vaultpilot://note/new?x-source=com.example.app', ['com.example.app']);
    expect(r.risk).toBe('medium');
    expect(r.needsConfirmation).toBe(false);
  });

  it('medium risk trust match is case-insensitive (#3995 mirrors Rust is_trusted)', () => {
    const r = evaluateUriSafety('vaultpilot://note/new?x-source=COM.Example.App', ['com.example.app']);
    expect(r.risk).toBe('medium');
    expect(r.needsConfirmation).toBe(false);
  });

  it('medium risk still needs confirmation when x-source is NOT in the trust list', () => {
    const r = evaluateUriSafety('vaultpilot://note/new?x-source=com.evil.app', ['com.example.app']);
    expect(r.risk).toBe('medium');
    expect(r.needsConfirmation).toBe(true);
  });

  it('high risk (chat/new) ALWAYS confirms even when the source is trusted', () => {
    const r = evaluateUriSafety('vaultpilot://chat/new?x-source=com.example.app', ['com.example.app']);
    expect(r.risk).toBe('high');
    expect(r.needsConfirmation).toBe(true);
  });

  it('high risk (overwrite) ALWAYS confirms even when the source is trusted', () => {
    const r = evaluateUriSafety('vaultpilot://note/new?overwrite=true&x-source=com.example.app', ['com.example.app']);
    expect(r.risk).toBe('high');
    expect(r.needsConfirmation).toBe(true);
  });

  it('high risk (note/delete,#3995) ALWAYS confirms even when trusted', () => {
    const r = evaluateUriSafety('vaultpilot://note/delete?id=abc&x-source=com.example.app', ['com.example.app']);
    expect(r.risk).toBe('high');
    expect(r.needsConfirmation).toBe(true);
  });

  it('low risk never needs confirmation', () => {
    const r = evaluateUriSafety('vaultpilot://note/123?x-source=com.evil.app', []);
    expect(r.risk).toBe('low');
    expect(r.needsConfirmation).toBe(false);
  });
});

describe('#3964 extractSource — x-source parsing', () => {
  it('parses the x-source query param', () => {
    expect(extractSource('vaultpilot://chat/new?x-source=com.example.app')).toBe('com.example.app');
  });

  it('returns empty string when x-source is absent', () => {
    expect(extractSource('vaultpilot://chat/new')).toBe('');
    expect(extractSource('vaultpilot://chat/new?foo=bar')).toBe('');
  });

  it('URL-decodes encoded values', () => {
    expect(extractSource('vaultpilot://note/new?x-source=com.example.app%2Fprod')).toBe('com.example.app/prod');
  });

  it('returns empty string for malformed / non-vaultpilot URIs', () => {
    expect(extractSource('')).toBe('');
    expect(extractSource('garbage')).toBe('');
    expect(extractSource('https://example.com/?x-source=com.example.app')).toBe('');
    expect(extractSource('vaultpilot://')).toBe('');
  });
});

describe('#3964 parseVaultPilotUri — route descriptor', () => {
  it('parses known routes', () => {
    expect(parseVaultPilotUri('vaultpilot://chat/new').route).toBe('chat/new');
    expect(parseVaultPilotUri('vaultpilot://chat').route).toBe('chat');
    expect(parseVaultPilotUri('vaultpilot://chat/sessions').route).toBe('chat/sessions');
    expect(parseVaultPilotUri('vaultpilot://note/new').route).toBe('note/new');
    expect(parseVaultPilotUri('vaultpilot://note').route).toBe('note');
    expect(parseVaultPilotUri('vaultpilot://search').route).toBe('search');
    expect(parseVaultPilotUri('vaultpilot://settings').route).toBe('settings');
  });

  it('case-insensitive route keywords (#3995 / Rust #3734)', () => {
    expect(parseVaultPilotUri('vaultpilot://Chat/New').route).toBe('chat/new');
    expect(parseVaultPilotUri('vaultpilot://NOTE/Delete').route).toBe('note/delete');
    expect(parseVaultPilotUri('vaultpilot://Settings').route).toBe('settings');
  });

  it('destructive routes get their own route, not note/:id (#3995)', () => {
    expect(parseVaultPilotUri('vaultpilot://note/delete').route).toBe('note/delete');
    expect(parseVaultPilotUri('vaultpilot://note/edit').route).toBe('note/edit');
    expect(parseVaultPilotUri('vaultpilot://note/bulk?op=move').route).toBe('note/bulk');
  });

  it('extracts noteId for note/:id', () => {
    const parsed = parseVaultPilotUri('vaultpilot://note/abc-123');
    expect(parsed.route).toBe('note/:id');
    expect(parsed.noteId).toBe('abc-123');
  });

  it('note/open/<id> alias maps to note/:id (#3995)', () => {
    const parsed = parseVaultPilotUri('vaultpilot://note/open/abc-123');
    expect(parsed.route).toBe('note/:id');
    expect(parsed.noteId).toBe('abc-123');
  });

  it('maps unknown/malformed URIs to unknown', () => {
    expect(parseVaultPilotUri('vaultpilot://bogus').route).toBe('unknown');
    expect(parseVaultPilotUri('vaultpilot://').route).toBe('unknown');
    expect(parseVaultPilotUri('not-a-uri').route).toBe('unknown');
    expect(parseVaultPilotUri('vaultpilot://note/a/b').route).toBe('unknown');
  });

  it('captures the overwrite flag with the Rust truthy set (#3995)', () => {
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite=true').overwrite).toBe(true);
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite=1').overwrite).toBe(true);
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite=on').overwrite).toBe(true);
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite=false').overwrite).toBe(false);
    expect(parseVaultPilotUri('vaultpilot://note/new?overwrite=2').overwrite).toBe(false);
    expect(parseVaultPilotUri('vaultpilot://note/new').overwrite).toBeUndefined();
  });
});

describe('#3964 uriTrustStore — AsyncStorage-backed trusted sources (lowercased, #3995)', () => {
  beforeEach(async () => {
    await AsyncStorage.clear();
  });

  it('returns [] when nothing has been stored', async () => {
    expect(await getTrustedSources()).toEqual([]);
  });

  it('adds and persists a lowercased trusted source', async () => {
    await addTrustedSource('Com.Example.App');
    expect(await getTrustedSources()).toEqual(['com.example.app']);
    const raw = await AsyncStorage.getItem('vaultpilot_trusted_uri_sources');
    expect(raw).toBe(JSON.stringify(['com.example.app']));
  });

  it('is idempotent — no duplicates across case variants', async () => {
    await addTrustedSource('a.b.c');
    await addTrustedSource('A.B.C');
    expect(await getTrustedSources()).toEqual(['a.b.c']);
  });

  it('ignores empty sources', async () => {
    await addTrustedSource('');
    await addTrustedSource('   ');
    expect(await getTrustedSources()).toEqual([]);
  });

  it('adds multiple distinct sources', async () => {
    await addTrustedSource('com.a');
    await addTrustedSource('com.b');
    expect(await getTrustedSources()).toEqual(['com.a', 'com.b']);
  });
});