/**
 * Regression test for #1463 — db.ts uuid() export + format validation.
 *
 * Verifies:
 * 1. uuid() returns valid v4 UUID format
 * 2. Uniqueness across multiple calls
 * 3. Fallback path (crypto.randomUUID unavailable) produces valid format
 */

import { uuid } from '../../db';

describe('uuid()', () => {
  const UUID_V4_REGEX = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

  it('returns a valid v4 UUID format', () => {
    const id = uuid();
    expect(id).toMatch(UUID_V4_REGEX);
  });

  it('returns unique values on consecutive calls', () => {
    const ids = new Set(Array.from({ length: 100 }, () => uuid()));
    expect(ids.size).toBe(100);
  });

  it('returns a 36-character string with dashes', () => {
    const id = uuid();
    expect(id).toHaveLength(36);
    expect(id.split('-')).toHaveLength(5);
  });

  it('has version nibble = 4 at position 14', () => {
    // Position 14 in "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx" is the version digit
    const id = uuid();
    expect(id[14]).toBe('4');
  });

  it('has variant nibble (8/9/a/b) at position 19', () => {
    const id = uuid();
    expect(['8', '9', 'a', 'b']).toContain(id[19]);
  });

  it('fallback path produces valid format when crypto.randomUUID is unavailable', () => {
    // Temporarily mock crypto to be undefined so the fallback runs
    const originalCrypto = globalThis.crypto;
    try {
      // @ts-expect-error — intentionally removing crypto for fallback test
      globalThis.crypto = undefined;
      const id = uuid();
      expect(id).toMatch(UUID_V4_REGEX);
      expect(id).toHaveLength(36);
    } finally {
      globalThis.crypto = originalCrypto;
    }
  });
});
