/**
 * #2154 — Template Snippets (Phase 1)
 * Covers: template CRUD, variable substitution, field extraction, query filtering,
 * instantiation, and idempotent default-template seeding.
 */
import * as sqliteMock from 'expo-sqlite';
import AsyncStorage from '@react-native-async-storage/async-storage';

const mockDb = (sqliteMock as any).__getMockDb();

/** Get a fresh db module and initialize it (runs CREATE TABLE etc), then clear mock state. */
async function freshDb() {
  jest.resetModules();
  // Re-import AsyncStorage in the fresh module registry so the flag persists within a test.
  const freshAsync = require('@react-native-async-storage/async-storage').default;
  // Reset the seeded flag between freshDb() calls unless a test sets it explicitly.
  await freshAsync.removeItem('templates_seeded_v1');
  const freshSqlite = require('expo-sqlite');
  freshSqlite.openDatabaseAsync.mockResolvedValue(mockDb);
  const db = require('../../db');
  await db.getDb();
  jest.clearAllMocks();
  return db;
}

describe('#2154 applyTemplateVariables (pure)', () => {
  const { applyTemplateVariables } = require('../../db');

  it('substitutes built-in variables', () => {
    const out = applyTemplateVariables(
      '# {{title}}\n日期 {{date}} 时间 {{time}} 周{{week}} 库名 {{vault_name}}',
      { title: '我的笔记', date: '2026-06-28', time: '09:30', week: '六' },
    );
    expect(out).toBe('# 我的笔记\n日期 2026-06-28 时间 09:30 周六 库名 VaultPilot');
  });

  it('defaults vault_name when not provided', () => {
    expect(applyTemplateVariables('{{vault_name}}')).toBe('VaultPilot');
  });

  it('fills custom {{field:label}} placeholders from the fields map', () => {
    const out = applyTemplateVariables(
      '书名：{{field:书名}}\n作者：{{field:作者}}',
      {},
      { 书名: '深入理解计算机系统', 作者: 'CSAPP' },
    );
    expect(out).toBe('书名：深入理解计算机系统\n作者：CSAPP');
  });

  it('replaces unfilled custom fields with empty string (no raw {{}} markers)', () => {
    const out = applyTemplateVariables('负责人：{{field:负责人}}：', {}, {});
    expect(out).toBe('负责人：：');
    expect(out).not.toContain('{{');
  });

  it('replaces all occurrences of a variable', () => {
    const out = applyTemplateVariables('{{date}} 和 {{date}}', { date: '2026-06-28' });
    expect(out).toBe('2026-06-28 和 2026-06-28');
  });
});

describe('#2154 extractTemplateFields (pure)', () => {
  const { extractTemplateFields } = require('../../db');

  it('extracts field labels in order of appearance', () => {
    const fields = extractTemplateFields('{{field:书名}} {{field:作者}} {{field:书名}}');
    expect(fields).toEqual(['书名', '作者']);
  });

  it('deduplicates labels', () => {
    const fields = extractTemplateFields('{{field:负责人}}\n{{field:负责人}}');
    expect(fields).toEqual(['负责人']);
  });

  it('returns empty array when no fields present', () => {
    expect(extractTemplateFields('# {{title}}\n{{date}}')).toEqual([]);
  });

  it('trims whitespace in labels', () => {
    const fields = extractTemplateFields('{{field:  日期  }}');
    expect(fields).toEqual(['日期']);
  });
});

describe('#2154 buildTemplateVars (pure)', () => {
  const { buildTemplateVars } = require('../../db');

  it('returns title, date, time, week and vault_name', () => {
    const vars = buildTemplateVars('周报');
    expect(vars.title).toBe('周报');
    expect(vars.vault_name).toBe('VaultPilot');
    expect(vars.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(vars.time).toMatch(/^\d{2}:\d{2}$/);
    expect(['日', '一', '二', '三', '四', '五', '六']).toContain(vars.week);
  });
});

describe('#2154 template DB operations', () => {
  it('createTemplate inserts with is_template = 1', async () => {
    const db = await freshDb();
    const id = await db.createTemplate('会议纪要', '# 会议');
    expect(id).toMatch(/^[0-9a-f-]{36}$/);
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('INSERT INTO notes');
    expect(sql).toContain('is_template');
    // is_template is a SQL literal (1), not a bound param — params carry id/title/content only
    expect(sql).toContain('?, ?, ?, 1');
    expect(params).toHaveLength(3);
    expect(params[0]).toBe(id);
    expect(params[1]).toBe('会议纪要');
    expect(params[2]).toBe('# 会议');
  });

  it('getTemplates queries is_template = 1', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([
      { id: 't1', title: '会议纪要', content: '...', is_template: 1 },
    ]);
    const tpls = await db.getTemplates();
    expect(tpls).toHaveLength(1);
    const [sql] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).toContain('is_template = 1');
  });

  it('setTemplateFlag updates is_template column', async () => {
    const db = await freshDb();
    await db.setTemplateFlag('note1', true);
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('UPDATE notes SET is_template');
    expect(params[0]).toBe(1);

    await db.setTemplateFlag('note1', false);
    const [, params2] = mockDb.runAsync.mock.calls[1];
    expect(params2[0]).toBe(0);
  });

  it('saveAsTemplate copies title+content into a new template', async () => {
    const db = await freshDb();
    mockDb.getFirstAsync.mockResolvedValueOnce({ title: '原始笔记', content: '正文' });
    const id = await db.saveAsTemplate('note1');
    expect(id).toMatch(/^[0-9a-f-]{36}$/);
    // Source query
    const [srcSql, srcParams] = mockDb.getFirstAsync.mock.calls[0];
    expect(srcSql).toContain('SELECT title, content FROM notes');
    expect(srcParams).toEqual(['note1']);
    // Template insert
    const [insSql, insParams] = mockDb.runAsync.mock.calls[0];
    expect(insSql).toContain('is_template');
    expect(insParams[1]).toBe('原始笔记');
    expect(insParams[2]).toBe('正文');
  });

  it('saveAsTemplate returns null when source note missing', async () => {
    const db = await freshDb();
    mockDb.getFirstAsync.mockResolvedValueOnce(null);
    const id = await db.saveAsTemplate('nope');
    expect(id).toBeNull();
  });

  it('instantiateTemplate loads template, substitutes vars, and creates a new note', async () => {
    const db = await freshDb();
    mockDb.getFirstAsync.mockResolvedValueOnce({
      id: 'tpl1', title: '周报', content: '# {{title}}\n日期 {{date}}', is_template: 1,
    });
    const newId = await db.instantiateTemplate('tpl1', {});
    expect(newId).toMatch(/^[0-9a-f-]{36}$/);
    // The new note's content should have variables substituted (no raw {{}})
    const [insSql, insParams] = mockDb.runAsync.mock.calls[0];
    expect(insSql).toContain('INSERT INTO notes');
    expect(insParams[1]).toBe('周报'); // title carried over
    expect(insParams[2]).toContain('# 周报');
    expect(insParams[2]).toContain('日期 20'); // date substituted (starts with year)
    expect(insParams[2]).not.toContain('{{');
  });

  it('instantiateTemplate fills custom fields from fieldValues', async () => {
    const db = await freshDb();
    mockDb.getFirstAsync.mockResolvedValueOnce({
      id: 'tpl2', title: '读书笔记', content: '书名：{{field:书名}}', is_template: 1,
    });
    await db.instantiateTemplate('tpl2', { 书名: 'CSAPP' });
    const [, params] = mockDb.runAsync.mock.calls[0];
    expect(params[2]).toBe('书名：CSAPP');
  });

  it('instantiateTemplate throws when template does not exist', async () => {
    const db = await freshDb();
    mockDb.getFirstAsync.mockResolvedValueOnce(null);
    await expect(db.instantiateTemplate('missing')).rejects.toThrow('模板不存在');
  });
});

describe('#2154 regular queries exclude templates', () => {
  it('getNotes filters is_template = 0 (all variants)', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValue([]);
    await db.getNotes();
    expect(mockDb.getAllAsync.mock.calls[0][0]).toContain('is_template = 0');

    await db.getNotes('work');
    expect(mockDb.getAllAsync.mock.calls[1][0]).toContain('is_template = 0');

    await db.getNotes(undefined, 10);
    expect(mockDb.getAllAsync.mock.calls[2][0]).toContain('is_template = 0');
  });

  it('getNoteCount counts only non-template notes', async () => {
    const db = await freshDb();
    mockDb.getFirstAsync.mockResolvedValueOnce({ count: 5 });
    await db.getNoteCount();
    const [sql] = mockDb.getFirstAsync.mock.calls[0];
    expect(sql).toContain('is_template = 0');
  });

  it('searchNotes filters is_template = 0', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValue([]);
    await db.searchNotes('keyword');
    const sqls = mockDb.getAllAsync.mock.calls.map((c: any[]) => c[0]);
    expect(sqls.some((s: string) => s.includes('is_template = 0'))).toBe(true);
  });
});

describe('#2154 ensureDefaultTemplates seeding', () => {
  it('seeds built-in templates when none exist and flag unset', async () => {
    const db = await freshDb();
    const freshAsync = require('@react-native-async-storage/async-storage').default;
    await freshAsync.removeItem('templates_seeded_v1');
    mockDb.getAllAsync.mockResolvedValue([]); // getTemplates() -> []
    await db.ensureDefaultTemplates();
    // 3 default templates inserted
    const inserts = mockDb.runAsync.mock.calls.filter(
      (c: any[]) => typeof c[0] === 'string' && c[0].includes('INSERT INTO notes'),
    );
    expect(inserts).toHaveLength(db.DEFAULT_TEMPLATES.length);
    // Flag persisted
    expect(await freshAsync.getItem('templates_seeded_v1')).toBe('1');
  });

  it('is idempotent — does not re-seed when flag already set', async () => {
    const db = await freshDb();
    const freshAsync = require('@react-native-async-storage/async-storage').default;
    await freshAsync.setItem('templates_seeded_v1', '1');
    jest.clearAllMocks();
    await db.ensureDefaultTemplates();
    expect(mockDb.runAsync).not.toHaveBeenCalled();
  });

  it('marks flag without seeding when templates already exist', async () => {
    const db = await freshDb();
    const freshAsync = require('@react-native-async-storage/async-storage').default;
    await freshAsync.removeItem('templates_seeded_v1');
    mockDb.getAllAsync.mockResolvedValueOnce([{ id: 'existing', is_template: 1 }]);
    await db.ensureDefaultTemplates();
    const inserts = mockDb.runAsync.mock.calls.filter(
      (c: any[]) => typeof c[0] === 'string' && c[0].includes('INSERT INTO notes'),
    );
    expect(inserts).toHaveLength(0);
    expect(await freshAsync.getItem('templates_seeded_v1')).toBe('1');
  });

  it('DEFAULT_TEMPLATES contains 会议纪要 / 读书笔记 / 周报', () => {
    const { DEFAULT_TEMPLATES } = require('../../db');
    const titles = DEFAULT_TEMPLATES.map((t: any) => t.title);
    expect(titles).toEqual(expect.arrayContaining(['会议纪要', '读书笔记', '周报']));
    // Each template uses at least one variable
    for (const t of DEFAULT_TEMPLATES) {
      expect(t.content).toMatch(/\{\{/);
    }
  });
});
