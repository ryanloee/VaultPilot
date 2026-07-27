/**
 * Regression test for #3502: createNote 报"保存失败"但笔记实际已写入。
 *
 * 根因：createNote 的 INSERT（关键写）与 invalidateNoteTitleCache /
 * queuePendingSync（副作用）不在同一事务。INSERT 成功后笔记已提交入库，
 * 但 queuePendingSync 抛错会让 createNote 整体 reject，调用方
 * (rag.ts executeToolCalls) 捕获后向用户输出 `保存失败「标题」`，
 * 尽管笔记其实已经保存成功。
 *
 * 修复：INSERT 后的副作用降级为 best-effort（try/catch + warn），
 * createNote 在 INSERT 成功后始终 resolve 并返回 noteId。
 *
 * 本测试模拟 runAsync 对 notes 表 INSERT 成功、对 pending_syncs 表 INSERT
 * 抛错，断言 createNote 仍 resolve 且返回 noteId。
 */

// Inline mock so we can control runAsync per-SQL.
const mockRunAsync = jest.fn().mockImplementation(async (sql: string) => {
  if (typeof sql === 'string' && sql.includes('INSERT INTO pending_syncs')) {
    throw new Error('simulated pending_syncs write failure');
  }
  return undefined;
});
const mockGetFirstAsync = jest.fn().mockResolvedValue(null);
const mockGetAllAsync = jest.fn().mockResolvedValue([]);
const mockExecAsync = jest.fn().mockResolvedValue(undefined);

jest.mock('expo-sqlite', () => ({
  openDatabaseAsync: jest.fn().mockResolvedValue({
    execAsync: mockExecAsync,
    getAllAsync: mockGetAllAsync,
    getFirstAsync: mockGetFirstAsync,
    runAsync: mockRunAsync,
    withTransactionAsync: jest.fn().mockImplementation(async (fn: () => Promise<void>) => fn()),
  }),
}));

import { createNote } from '../../db';

describe('issue #3502 — createNote 不因 queuePendingSync 失败而误报保存失败', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('notes INSERT 成功 + pending_syncs INSERT 抛错 → createNote 仍 resolve 并返回 noteId', async () => {
    const noteId = await createNote('内存小参 18-22-38-38', '用户提供的内存主时序...');

    // 返回了有效的 noteId（非空字符串）
    expect(typeof noteId).toBe('string');
    expect(noteId.length).toBeGreaterThan(0);

    // 关键写（INSERT INTO notes）确实执行了
    const notesInserts = mockRunAsync.mock.calls.filter(
      (c: unknown[]) => typeof c[0] === 'string' && c[0].includes('INSERT INTO notes')
    );
    expect(notesInserts.length).toBe(1);

    // 同步队列写入也尝试了（只是它抛错了）
    const syncInserts = mockRunAsync.mock.calls.filter(
      (c: unknown[]) => typeof c[0] === 'string' && c[0].includes('INSERT INTO pending_syncs')
    );
    expect(syncInserts.length).toBe(1);
  });

  it('updateNote 路径同样不应被 queuePendingSync 失败拖垮（对称保护）', async () => {
    // 复用同一 mock：UPDATE notes 成功，pending_syncs 抛错
    mockRunAsync.mockImplementation(async (sql: string) => {
      if (typeof sql === 'string' && sql.includes('INSERT INTO pending_syncs')) {
        throw new Error('simulated pending_syncs write failure');
      }
      return undefined;
    });

    const { updateNote } = await import('../../db');
    // 不抛错即通过
    await expect(updateNote('note-x', '新标题', '新内容')).resolves.toBeUndefined();

    const updates = mockRunAsync.mock.calls.filter(
      (c: unknown[]) => typeof c[0] === 'string' && c[0].includes('UPDATE notes')
    );
    expect(updates.length).toBe(1);
  });
});
