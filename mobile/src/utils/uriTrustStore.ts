/**
 * Trusted vaultpilot:// URI source store (#3964).
 *
 * Persists the x-source values the user explicitly opted to trust for
 * MEDIUM-risk deep-link actions (see uriSafety.evaluateUriSafety). Backed by
 * AsyncStorage under the key 'vaultpilot_trusted_uri_sources'.
 *
 * Stored values are lowercased at write time and compared case-insensitively
 * at read time, mirroring the Rust `TrustedAppRegistry::is_trusted` (#3995).
 */

import AsyncStorage from '@react-native-async-storage/async-storage';

const TRUSTED_SOURCES_KEY = 'vaultpilot_trusted_uri_sources';

/** Read the persisted list of trusted sources (never throws; [] on error). */
export async function getTrustedSources(): Promise<string[]> {
  try {
    const raw = await AsyncStorage.getItem(TRUSTED_SOURCES_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((s): s is string => typeof s === 'string' && s.length > 0);
  } catch (e) {
    console.warn('[UriTrustStore] getTrustedSources failed:', e);
    return [];
  }
}

/** Persist a trusted source (idempotent; never throws). Lowercased to match
 * Rust's `TrustedAppRegistry.is_trusted` case-insensitive comparison. */
export async function addTrustedSource(src: string): Promise<void> {
  const lowercased = (src ?? '').trim().toLowerCase();
  if (!lowercased) return;
  try {
    const current = await getTrustedSources();
    if (current.includes(lowercased)) return;
    await AsyncStorage.setItem(TRUSTED_SOURCES_KEY, JSON.stringify([...current, lowercased]));
  } catch (e) {
    console.warn('[UriTrustStore] addTrustedSource failed:', e);
  }
}
