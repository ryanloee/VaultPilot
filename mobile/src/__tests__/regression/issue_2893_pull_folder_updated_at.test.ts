/**
 * Regression tests for #2893 — mobile pull sync must propagate the server
 * `folder` (derived from `path`) and the real server `updated_at` into the
 * local SQLite `notes` row, instead of leaving folder='' and updated_at=now.
 */
import * as sqliteMock from 'expo-sqlite';
import { deriveFolderFromPath, parseServerTimestamp } from '../../services/sync';

const mockDb = (sqliteMock as any).__getMockDb();

/** Get a fresh db module and initialize it (runs CREATE TABLE etc), then clear mock state. */
async function freshDb() {
  jest.resetModules();
  const freshSqlite = require('expo-sqlite');
  freshSqlite.openDatabaseAsync.mockResolvedValue(mockDb);
  const db = require('../../db');
  // Trigger getDb() to initialize the database
  await db.getDb();
  // Clear all mock calls from initialization so tests start clean
  jest.clearAllMocks();
  return db;
}

describe('issue #2893 — folder/updated_at propagation (sync helpers)', () => {
  it('deriveFolderFromPath: nested path yields nested folder', () => {
    expect(deriveFolderFromPath('work/projects/meeting.md')).toBe('work/projects');
  });

  it('deriveFolderFromPath: single-level path yields single folder', () => {
    expect(deriveFolderFromPath('work/meeting.md')).toBe('work');
  });

  it('deriveFolderFromPath: root note (no slash) yields empty folder', () => {
    expect(deriveFolderFromPath('meeting.md')).toBe('');
  });

  it('deriveFolderFromPath: empty/undefined path yields empty folder', () => {
    expect(deriveFolderFromPath('')).toBe('');
    expect(deriveFolderFromPath(undefined)).toBe('');
    expect(deriveFolderFromPath(null)).toBe('');
  });

  it('deriveFolderFromPath: tolerates backslash separators (Windows server)', () => {
    expect(deriveFolderFromPath('work\\sub\\note.md')).toBe('work/sub');
  });

  it('parseServerTimestamp: RFC3339 string -> unix seconds', () => {
    // 2026-01-01T00:00:00Z
    expect(parseServerTimestamp('2026-01-01T00:00:00Z')).toBe(1767225600);
  });

  it('parseServerTimestamp: offset-aware RFC3339 string -> unix seconds', () => {
    // 2026-01-01T08:00:00+08:00 == 2026-01-01T00:00:00Z
    expect(parseServerTimestamp('2026-01-01T08:00:00+08:00')).toBe(1767225600);
  });

  it('parseServerTimestamp: missing/unparseable -> undefined (fallback to now)', () => {
    expect(parseServerTimestamp('')).toBeUndefined();
    expect(parseServerTimestamp(undefined)).toBeUndefined();
    expect(parseServerTimestamp('not-a-date')).toBeUndefined();
  });
});

describe('issue #2893 — db createNote/updateNote write folder + updated_at', () => {
  it('createNote stores provided folder and server updated_at', async () => {
    const db = await freshDb();
    await db.createNote('Meeting', 'body', 'note-1', {
      skipQueue: true,
      folder: 'work/projects',
      updated_at: 1767225600,
    });
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('INSERT INTO notes');
    expect(sql).toContain('folder');
    expect(sql).toContain('updated_at');
    expect(params).toContain('work/projects');
    expect(params).toContain(1767225600);
  });

  it('createNote falls back to now when updated_at omitted (param null -> COALESCE)', async () => {
    const db = await freshDb();
    await db.createNote('Meeting', 'body', 'note-2', { skipQueue: true, folder: 'work' });
    const [, params] = mockDb.runAsync.mock.calls[0];
    // updated_at param is null -> SQL COALESCE(?, strftime('%s','now')) uses now
    expect(params).toContain('work');
    expect(params).toContain(null);
  });

  it('updateNote writes folder and server updated_at', async () => {
    const db = await freshDb();
    await db.updateNote('note-1', 'Meeting', 'body', {
      skipQueue: true,
      folder: 'work',
      updated_at: 1767225600,
    });
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('UPDATE notes');
    expect(sql).toContain('folder = ?');
    expect(sql).toContain('updated_at = COALESCE');
    // params order: title, content, is_template, folder, updated_at, id
    expect(params[3]).toBe('work');
    expect(params[4]).toBe(1767225600);
    expect(params[5]).toBe('note-1');
  });

  it('updateNote keeps root folder (empty string is a valid value, not null)', async () => {
    const db = await freshDb();
    await db.updateNote('note-1', 'Meeting', 'body', { skipQueue: true, folder: '' });
    const [, params] = mockDb.runAsync.mock.calls[0];
    // folder param is '' (empty root), NOT null
    expect(params[3]).toBe('');
  });
});
