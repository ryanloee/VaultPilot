/**
 * #3373 — ensureDefaultTemplates TOCTOU race fix
 * Verifies that concurrent calls to ensureDefaultTemplates() are deduped
 * via a module-level promise, preventing duplicate template seeding.
 * Also verifies check-and-set flag hardening and failure recovery.
 */
import * as sqliteMock from 'expo-sqlite';
import AsyncStorage from '@react-native-async-storage/async-storage';

const mockDb = (sqliteMock as any).__getMockDb();

/** Get a fresh db module and initialize it (runs CREATE TABLE etc), then clear mock state. */
async function freshDb() {
  jest.resetModules();
  const freshAsync = require('@react-native-async-storage/async-storage').default;
  await freshAsync.removeItem('templates_seeded_v1');
  const freshSqlite = require('expo-sqlite');
  freshSqlite.openDatabaseAsync.mockResolvedValue(mockDb);
  const db = require('../../db');
  await db.getDb();
  jest.clearAllMocks();
  return db;
}

function countInserts(): number {
  return mockDb.runAsync.mock.calls.filter(
    (c: any[]) => typeof c[0] === 'string' && c[0].includes('INSERT INTO notes'),
  ).length;
}

describe('#3373 ensureDefaultTemplates TOCTOU dedup', () => {
  it('concurrent calls seed templates only once (promise dedup)', async () => {
    const db = await freshDb();
    const freshAsync = require('@react-native-async-storage/async-storage').default;
    await freshAsync.removeItem('templates_seeded_v1');
    mockDb.getAllAsync.mockResolvedValue([]); // getTemplates() -> []

    // Fire two calls concurrently — before either resolves
    const p1 = db.ensureDefaultTemplates();
    const p2 = db.ensureDefaultTemplates();
    await Promise.all([p1, p2]);

    // Should seed exactly DEFAULT_TEMPLATES.length, NOT 2x
    expect(countInserts()).toBe(db.DEFAULT_TEMPLATES.length);
  });

  it('three concurrent calls still seed only once', async () => {
    const db = await freshDb();
    const freshAsync = require('@react-native-async-storage/async-storage').default;
    await freshAsync.removeItem('templates_seeded_v1');
    mockDb.getAllAsync.mockResolvedValue([]);

    const results = await Promise.all([
      db.ensureDefaultTemplates(),
      db.ensureDefaultTemplates(),
      db.ensureDefaultTemplates(),
    ]);
    // All three resolve successfully
    expect(results).toHaveLength(3);
    expect(countInserts()).toBe(db.DEFAULT_TEMPLATES.length);
  });

  it('sets flag before seeding (check-and-set defense)', async () => {
    const db = await freshDb();
    const freshAsync = require('@react-native-async-storage/async-storage').default;
    await freshAsync.removeItem('templates_seeded_v1');
    mockDb.getAllAsync.mockResolvedValue([]);

    await db.ensureDefaultTemplates();

    // Flag should be set
    expect(await freshAsync.getItem('templates_seeded_v1')).toBe('1');
  });

  it('clears flag on seeding failure so next launch retries', async () => {
    const db = await freshDb();
    const freshAsync = require('@react-native-async-storage/async-storage').default;
    await freshAsync.removeItem('templates_seeded_v1');
    mockDb.getAllAsync.mockResolvedValue([]);
    // Make the first createTemplate INSERT fail
    mockDb.runAsync.mockRejectedValueOnce(new Error('DB locked'));

    await db.ensureDefaultTemplates();

    // Flag should be removed because seeding failed
    expect(await freshAsync.getItem('templates_seeded_v1')).toBeNull();
  });

  it('second call after completion does not re-seed (flag guard)', async () => {
    const db = await freshDb();
    const freshAsync = require('@react-native-async-storage/async-storage').default;
    await freshAsync.removeItem('templates_seeded_v1');
    mockDb.getAllAsync.mockResolvedValue([]);

    await db.ensureDefaultTemplates();
    jest.clearAllMocks();
    // Second call after the first fully completes (promise cleared in finally)
    await db.ensureDefaultTemplates();

    expect(countInserts()).toBe(0);
  });
});
