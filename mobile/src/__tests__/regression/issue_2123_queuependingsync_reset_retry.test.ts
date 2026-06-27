/**
 * Regression test for #2123: queuePendingSync 不重置 retry_count，
 * 导致编辑后的笔记同步被过早放弃。
 *
 * 根因：queuePendingSync 使用 INSERT ... ON CONFLICT(note_id) DO UPDATE 去重，
 * 更新了 action 与 created_at 但未重置 retry_count。flushPendingSyncs 在 5xx 错误
 * 时递增 retry_count，达到 MAX_RETRY_ATTEMPTS(=5) 后 clearPendingSync 清除记录。
 * 由于历史失败计数累积到新编辑上，用户重新编辑的笔记可能在尚未真正尝试同步前
 * 就被清除，导致最新编辑永久丢失且无任何提示。
 *
 * 修复：ON CONFLICT DO UPDATE 中同时重置 retry_count = 0。
 */

// Mock expo-sqlite inline
const mockRunAsync = jest.fn().mockResolvedValue(undefined);
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

import {
  queuePendingSync,
  incrementPendingSyncRetry,
} from '../../db';

describe('issue #2123 — queuePendingSync 重置 retry_count', () => {
  it('queuePendingSync 的 UPSERT 应在冲突时重置 retry_count = 0', async () => {
    jest.clearAllMocks();
    await queuePendingSync('note-2123', 'update');

    const upsertCalls = mockRunAsync.mock.calls.filter(
      (c: any[]) => typeof c[0] === 'string' && c[0].includes('INSERT INTO pending_syncs')
    );
    expect(upsertCalls.length).toBeGreaterThan(0);

    const sql = upsertCalls[0][0] as string;
    // 必须包含 ON CONFLICT DO UPDATE
    expect(sql).toMatch(/ON CONFLICT\(note_id\) DO UPDATE/);
    // SET 子句必须显式重置 retry_count = 0
    expect(sql).toMatch(/retry_count\s*=\s*0/);
    // 仍应保留 action 与 created_at 的更新
    expect(sql).toMatch(/action\s*=\s*excluded\.action/);
    expect(sql).toMatch(/created_at\s*=/);
  });

  it('模拟「失败累积 → 重新编辑」场景：重置后的 SQL 与现有逻辑兼容', async () => {
    // 该用例验证修复没有破坏 incrementPendingSyncRetry 这条独立路径
    // （retry_count 递增仍由 offlineSync.ts 的 flushPendingSyncs 负责）。
    jest.clearAllMocks();
    await incrementPendingSyncRetry('note-2123-b');
    expect(mockRunAsync).toHaveBeenCalledWith(
      expect.stringContaining('UPDATE pending_syncs SET retry_count = retry_count + 1'),
      ['note-2123-b']
    );

    // 重新 queue 时 retry_count 应被重置回 0
    jest.clearAllMocks();
    await queuePendingSync('note-2123-b', 'update');
    const sql = mockRunAsync.mock.calls[0][0] as string;
    expect(sql).toMatch(/retry_count\s*=\s*0/);
  });
});
