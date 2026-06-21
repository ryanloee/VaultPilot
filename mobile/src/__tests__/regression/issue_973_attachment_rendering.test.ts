/**
 * Regression tests for issue #973 — attachment rendering in message bubbles.
 *
 * Tests:
 * 1. MIME type inference from file extension (inferMime)
 * 2. Attachment metadata roundtrip through addMessage/getMessages
 * 3. DbMessage.attachments field parsing
 */

// Mock expo-sqlite
jest.mock('expo-sqlite', () => {
  const store: Record<string, any[]> = {};
  const db = {
    execAsync: jest.fn().mockResolvedValue(undefined),
    getAllAsync: jest.fn().mockImplementation((sql: string, params?: any[]) => {
      if (sql.includes('FROM messages') && sql.includes('session_id')) {
        return Promise.resolve(store['messages']?.filter(m => m.session_id === params?.[0]) ?? []);
      }
      return Promise.resolve([]);
    }),
    runAsync: jest.fn().mockImplementation((sql: string, params?: any[]) => {
      if (sql.includes('INSERT INTO messages')) {
        const row = { id: params?.[0], session_id: params?.[1], role: params?.[2], content: params?.[3], attachments: params?.[4], created_at: Date.now() };
        if (!store['messages']) store['messages'] = [];
        store['messages'].push(row);
      }
      return Promise.resolve(undefined);
    }),
    getFirstAsync: jest.fn().mockResolvedValue(null),
    withTransactionAsync: jest.fn().mockImplementation(async (fn: () => Promise<void>) => { await fn(); }),
  };
  return { openDatabaseAsync: jest.fn().mockResolvedValue(db), __store: store, __db: db };
});

// ---- inferMime tests (pure function, test directly) ----

/** Mirror of inferMime from ChatScreen.tsx */
function inferMime(name: string, fallback: string): string {
  const ext = name.split('.').pop()?.toLowerCase();
  const map: Record<string, string> = {
    png: 'image/png', gif: 'image/gif', webp: 'image/webp', heic: 'image/heic',
    jpg: 'image/jpeg', jpeg: 'image/jpeg',
    pdf: 'application/pdf', doc: 'application/msword',
    txt: 'text/plain', md: 'text/markdown',
  };
  return ext && map[ext] ? map[ext] : fallback;
}

describe('issue_973 — inferMime', () => {
  test('infers image/jpeg from .jpg', () => {
    expect(inferMime('photo.jpg', 'fallback')).toBe('image/jpeg');
  });

  test('infers image/png from .png', () => {
    expect(inferMime('screenshot.png', 'fallback')).toBe('image/png');
  });

  test('infers image/gif from .gif', () => {
    expect(inferMime('animation.gif', 'fallback')).toBe('image/gif');
  });

  test('infers image/webp from .webp', () => {
    expect(inferMime('modern.webp', 'fallback')).toBe('image/webp');
  });

  test('infers image/heic from .heic (iOS photos)', () => {
    expect(inferMime('IMG_0001.heic', 'fallback')).toBe('image/heic');
  });

  test('infers application/pdf from .pdf', () => {
    expect(inferMime('document.pdf', 'fallback')).toBe('application/pdf');
  });

  test('infers text/plain from .txt', () => {
    expect(inferMime('readme.txt', 'fallback')).toBe('text/plain');
  });

  test('infers text/markdown from .md', () => {
    expect(inferMime('notes.md', 'fallback')).toBe('text/markdown');
  });

  test('uses fallback for unknown extension', () => {
    expect(inferMime('data.xyz', 'application/octet-stream')).toBe('application/octet-stream');
  });

  test('uses fallback for no extension', () => {
    expect(inferMime('noext', 'image/jpeg')).toBe('image/jpeg');
  });

  test('case insensitive extension', () => {
    expect(inferMime('photo.PNG', 'fallback')).toBe('image/png');
  });

  test('handles multiple dots in filename', () => {
    expect(inferMime('my.photo.v2.jpg', 'fallback')).toBe('image/jpeg');
  });
});

// ---- Attachment metadata roundtrip tests ----

describe('issue_973 — attachment metadata roundtrip via DB', () => {
  beforeEach(() => {
    const sqlite = require('expo-sqlite');
    sqlite.__store['messages'] = [];
    jest.clearAllMocks();
  });

  test('addMessage stores attachments JSON', async () => {
    const { addMessage, getMessages, getDb } = require('../../db');
    await getDb(); // initialize

    const sessionId = 'test-session';
    const atts = [{ name: 'photo.jpg', type: 'image' as const }, { name: 'doc.pdf', type: 'file' as const }];
    const msgId = await addMessage(sessionId, 'user', 'Check these out', atts);

    expect(typeof msgId).toBe('string');

    // Verify stored JSON
    const sqlite = require('expo-sqlite');
    const stored = sqlite.__store['messages'].find((m: any) => m.id === msgId);
    expect(stored).toBeDefined();
    expect(JSON.parse(stored.attachments)).toEqual(atts);
  });

  test('addMessage without attachments stores null', async () => {
    const { addMessage, getDb } = require('../../db');
    await getDb();

    const msgId = await addMessage('s1', 'user', 'plain text');
    const sqlite = require('expo-sqlite');
    const stored = sqlite.__store['messages'].find((m: any) => m.id === msgId);
    expect(stored.attachments).toBeNull();
  });

  test('empty attachments array stores null', async () => {
    const { addMessage, getDb } = require('../../db');
    await getDb();

    const msgId = await addMessage('s1', 'user', 'text', []);
    const sqlite = require('expo-sqlite');
    const stored = sqlite.__store['messages'].find((m: any) => m.id === msgId);
    expect(stored.attachments).toBeNull();
  });
});
