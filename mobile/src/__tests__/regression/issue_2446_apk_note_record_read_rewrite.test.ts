/**
 * Regression tests for #2446 — Android note record/read/AI-query rewrite.
 *
 * Covers three concrete user-reported bugs:
 *   1. "笔记会覆盖" — system-prompt format was broken, AI emitted malformed
 *      SAVE_NOTE markers with the same placeholder title.
 *   2. "删除不了" — NotesScreen list was not refreshed after returning from
 *      NoteEditorScreen, so deleted notes still appeared.
 *   3. "点击显示没有该笔记" — clicking a stale (deleted) note in the list
 *      triggered "笔记不存在" because getNote() returned null.
 *
 * The fix redesigns parseToolCalls to accept a closed form
 *   [SAVE_NOTE: title]
 *   content
 *   [/SAVE_NOTE]
 * while remaining backward compatible with the legacy open form. executeSave
 * now returns { noteId, title } so the chat layer can refresh the note-title
 * cache and the Notes tab list.
 */
import { parseToolCalls, executeToolCalls, buildSystemPrompt } from '../../services/rag';

// Mock createNote so executeToolCalls doesn't touch the real DB
jest.mock('../../db', () => ({
  createNote: jest.fn().mockResolvedValue('fake-note-id'),
  searchNotes: jest.fn().mockResolvedValue([]),
  getNotes: jest.fn().mockResolvedValue([]),
  getNoteCount: jest.fn().mockResolvedValue(0),
  updateNote: jest.fn(),
}));

// ── parseToolCalls — new closed-form format ─────────────────

describe('#2446 — parseToolCalls accepts the closed-form format', () => {
  it('parses [SAVE_NOTE: title]\\ncontent\\n[/SAVE_NOTE]', () => {
    const resp = '前文\n[SAVE_NOTE: 我的标题\n第一行内容\n第二行内容\n[/SAVE_NOTE]\n后文';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].title).toBe('我的标题');
    expect(pendingSaves[0].content).toBe('第一行内容\n第二行内容');
    // Both the open and close markers are stripped, leaving surrounding text
    expect(cleaned).toBe('前文\n\n后文');
  });

  it('parses a closing bracket on the title line: [SAVE_NOTE: title]', () => {
    // Many models emit `[SAVE_NOTE: title]` (markdown convention) — the parser
    // must strip the trailing `]` from the title, not store it as part of the title.
    const resp = '[SAVE_NOTE: 标题]\n内容主体';
    const { pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].title).toBe('标题'); // no trailing ]
    expect(pendingSaves[0].content).toBe('内容主体');
  });

  it('parses closed form with title-bracket AND end marker together', () => {
    const resp = '[SAVE_NOTE: 标题]\n多行\n内容\n[/SAVE_NOTE]';
    const { pendingSaves, cleaned } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].title).toBe('标题');
    expect(pendingSaves[0].content).toBe('多行\n内容');
    expect(cleaned).toBe('');
  });

  it('does not capture trailing AI commentary when end marker is used', () => {
    // Without the end marker, "已为你保存" would be captured as note content.
    // With the end marker, it stays in the cleaned response.
    const resp = '[SAVE_NOTE: 周会纪要]\n今天讨论了Roadmap。\n[/SAVE_NOTE]\n已为你保存，还有其他需要吗？';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].content).toBe('今天讨论了Roadmap。');
    expect(cleaned).toContain('已为你保存');
  });

  it('parses multiple closed-form blocks in one response', () => {
    const resp = [
      '好的，分两条记录：',
      '[SAVE_NOTE: 笔记A]',
      'A 的内容',
      '[/SAVE_NOTE]',
      '[SAVE_NOTE: 笔记B]',
      'B 的内容',
      '[/SAVE_NOTE]',
    ].join('\n');
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(2);
    expect(pendingSaves[0].title).toBe('笔记A');
    expect(pendingSaves[0].content).toBe('A 的内容');
    expect(pendingSaves[1].title).toBe('笔记B');
    expect(pendingSaves[1].content).toBe('B 的内容');
    expect(cleaned).toContain('好的，分两条记录');
    expect(cleaned).not.toContain('SAVE_NOTE');
  });
});

// ── Backward compatibility with the legacy open form (#1187) ──

describe('#2446 — parseToolCalls remains backward compatible with the legacy open form', () => {
  it('legacy open form still works: content until next marker or end', () => {
    const resp = '[SAVE_NOTE: 旧格式\n内容直接跟在标题后';
    const { pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].title).toBe('旧格式');
    expect(pendingSaves[0].content).toBe('内容直接跟在标题后');
  });

  it('legacy open form: content runs until next [SAVE_NOTE:', () => {
    const resp = '[SAVE_NOTE: 第一条\n内容一[SAVE_NOTE: 第二条\n内容二';
    const { pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(2);
    expect(pendingSaves[0].content).toBe('内容一');
    expect(pendingSaves[1].content).toBe('内容二');
  });

  it('preserves content containing [ and ] characters (#1187)', () => {
    const content = '查看 [文档](https://example.com) 与 [1, 2, 3]';
    const resp = `[SAVE_NOTE: 链接\n${content}\n[/SAVE_NOTE]`;
    const { pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves[0].content).toBe(content);
  });
});

// ── executeToolCalls returns savedNoteIds (#2446) ────────────

describe('#2446 — executeToolCalls returns the list of created note ids', () => {
  it('returns savedNoteIds alongside cleaned text and actions', async () => {
    const resp = '[SAVE_NOTE: 标题一\n内容一\n[/SAVE_NOTE]\n[SAVE_NOTE: 标题二\n内容二\n[/SAVE_NOTE]';
    const { cleaned, actions, savedNoteIds } = await executeToolCalls(resp);
    expect(savedNoteIds).toHaveLength(2);
    expect(savedNoteIds[0]).toBe('fake-note-id');
    expect(actions).toHaveLength(2);
    expect(actions[0]).toContain('标题一');
    expect(actions[1]).toContain('标题二');
    expect(cleaned).toBe('');
  });

  it('returns empty savedNoteIds when no markers are present', async () => {
    const { cleaned, actions, savedNoteIds } = await executeToolCalls('普通回复');
    expect(savedNoteIds).toEqual([]);
    expect(actions).toEqual([]);
    expect(cleaned).toBe('普通回复');
  });
});

// ── System prompt format is well-formed (#2446) ──────────────

describe('#2446 — buildSystemPrompt documents a well-formed SAVE_NOTE format', () => {
  it('documents the [/SAVE_NOTE] end marker', () => {
    const prompt = buildSystemPrompt(null);
    expect(prompt).toContain('[/SAVE_NOTE]');
  });

  it('explicitly forbids reusing/overwriting existing note titles', () => {
    const prompt = buildSystemPrompt(null);
    // Either Chinese or English instruction is fine; the prompt is locale-aware.
    const forbidsReuse = /不要复用|不要覆盖|never reuse or overwrite/i.test(prompt);
    expect(forbidsReuse).toBe(true);
  });

  it('forbids placeholder titles like "笔记标题" / "Untitled"', () => {
    const prompt = buildSystemPrompt(null);
    const forbidsPlaceholder = /不要使用.*占位|Never use placeholders/i.test(prompt);
    expect(forbidsPlaceholder).toBe(true);
  });
});
