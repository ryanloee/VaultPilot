/**
 * Trusted vaultpilot:// URI source store (#3964).
 *
 * Persists the x-source values the user explicitly opted to trust for
 * MEDIUM-risk deep-link actions (see uriSafety.evaluateUriSafety). Backed by
 * AsyncStorage under the key 'vaultpilot_trusted_uri_sources'.
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

/** Persist a trusted source (idempotent; never throws). */
export async function addTrustedSource(src: string): Promise<void> {
  const trimmed = (src ?? '').trim();
  if (!trimmed) return;
  try {
    const current = await getTrustedSources();
    if (current.includes(trimmed)) return;
    await AsyncStorage.setItem(TRUSTED_SOURCES_KEY, JSON.stringify([...current, trimmed]));
  } catch (e) {
    console.warn('[UriTrustStore] addTrustedSource failed:', e);
  }
}
