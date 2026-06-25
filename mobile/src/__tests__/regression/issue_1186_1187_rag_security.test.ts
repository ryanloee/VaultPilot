/**
 * Regression tests for #1186 and #1187:
 *
 * #1186 — SECURITY: parseToolCalls must NOT auto-save notes.
 *   It should return pendingSaves for user confirmation.
 *
 * #1187 — BUG: Note content containing '[' was truncated by regex.
 *   parseToolCalls must use indexOf-based parsing to preserve full content.
 */
import { parseToolCalls, PendingSave } from '../../services/rag';

// ─── #1186: Security — no auto-save ────────────────────────────

describe('#1186 — parseToolCalls returns pendingSaves, never auto-saves', () => {
  it('returns pendingSaves array instead of executing saves', () => {
    const resp = '前文\n[SAVE_NOTE: 测试标题\n这是笔记内容\n后文';
    const { cleaned, pendingSaves } = parseToolCalls(resp);

    // Must return pending saves, not execute them
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].title).toBe('测试标题');
    // Content is everything after ] until next marker or end
    expect(pendingSaves[0].content).toBe('这是笔记内容\n后文');
    expect(cleaned).toBe('前文');
  });

  it('returns empty pendingSaves when no markers present', () => {
    const resp = '普通回复没有任何标记';
    const { cleaned, pendingSaves } = parseToolCalls(resp);

    expect(pendingSaves).toHaveLength(0);
    expect(cleaned).toBe(resp);
  });

  it('handles multiple SAVE_NOTE markers — all returned as pending', () => {
    const resp = '[SAVE_NOTE: 第一条\n内容一\n[SAVE_NOTE: 第二条\n内容二';
    const { pendingSaves } = parseToolCalls(resp);

    expect(pendingSaves).toHaveLength(2);
    expect(pendingSaves[0].title).toBe('第一条');
    expect(pendingSaves[1].title).toBe('第二条');
  });

  it('skips markers with empty content', () => {
    const resp = '[SAVE_NOTE: 空标题\n[SAVE_NOTE: 有内容\n实际内容';
    const { pendingSaves } = parseToolCalls(resp);

    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].title).toBe('有内容');
  });
});

// ─── #1187: Content with '[' must not be truncated ─────────────

describe('#1187 — content containing [ is preserved (indexOf parsing)', () => {
  it('preserves content with markdown links [text](url)', () => {
    const content = '查看 [文档](https://example.com) 了解更多';
    const resp = `[SAVE_NOTE: 链接笔记\n${content}`;
    const { pendingSaves } = parseToolCalls(resp);

    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].content).toBe(content);
  });

  it('preserves content with array syntax [1, 2, 3]', () => {
    const content = '数组示例: [1, 2, 3] 和 [[nested]]';
    const resp = `[SAVE_NOTE: 数组笔记\n${content}`;
    const { pendingSaves } = parseToolCalls(resp);

    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].content).toBe(content);
  });

  it('preserves content with multiple [ characters', () => {
    const content = '步骤：\n1. [开始] 做某事\n2. [检查] 结果\n3. [完成] 收工';
    const resp = `[SAVE_NOTE: 步骤笔记\n${content}`;
    const { pendingSaves } = parseToolCalls(resp);

    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].content).toBe(content);
  });

  it('correctly splits two notes when content has [ chars', () => {
    const content1 = '内容含 [方括号]';
    const content2 = '另一篇也含 [brackets]';
    const resp = `[SAVE_NOTE: 笔记A\n${content1}\n[SAVE_NOTE: 笔记B\n${content2}`;
    const { pendingSaves, cleaned } = parseToolCalls(resp);

    expect(pendingSaves).toHaveLength(2);
    expect(pendingSaves[0].content).toBe(content1);
    expect(pendingSaves[1].content).toBe(content2);
    expect(cleaned).toBe('');
  });
});
