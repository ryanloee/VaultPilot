/**
 * Unit tests for AI Command Palette action registry (#2188 Phase 1).
 *
 * Covers: context resolution, message building per action, filtering, lookup.
 * These are pure helpers with no React Native / streaming dependencies.
 */

import {
  AI_ACTIONS,
  buildActionMessages,
  resolveContext,
  hasContext,
  filterActions,
  getActionById,
} from '../utils/aiActions';
import type { AiActionId } from '../utils/aiActions';

// ── resolveContext / hasContext ───────────────────────────

describe('resolveContext', () => {
  it('prefers a non-empty selection over note content', () => {
    expect(resolveContext('  hello  ', 'full note')).toBe('hello');
  });

  it('falls back to note content when selection is blank', () => {
    expect(resolveContext('   ', 'full note')).toBe('full note');
  });

  it('falls back to note content when selection is empty', () => {
    expect(resolveContext('', 'full note')).toBe('full note');
  });

  it('handles undefined inputs safely', () => {
    expect(resolveContext(undefined as any, 'note')).toBe('note');
    expect(resolveContext('sel', undefined as any)).toBe('sel');
    expect(resolveContext(undefined as any, undefined as any)).toBe('');
  });
});

describe('hasContext', () => {
  it('true when selection present', () => {
    expect(hasContext('sel', '')).toBe(true);
  });
  it('true when only note content present', () => {
    expect(hasContext('', 'note')).toBe(true);
  });
  it('false when both empty', () => {
    expect(hasContext('   ', '   ')).toBe(false);
  });
});

// ── buildActionMessages ───────────────────────────────────

describe('buildActionMessages', () => {
  const CTX = 'Some note text';

  it('always returns a system + user pair', () => {
    for (const a of AI_ACTIONS) {
      const msgs = buildActionMessages(a.id, CTX);
      expect(msgs.length).toBeGreaterThanOrEqual(2);
      expect(msgs[0].role).toBe('system');
      expect(msgs.some((m) => m.role === 'user')).toBe(true);
    }
  });

  it('embeds the context into the user message', () => {
    for (const a of AI_ACTIONS) {
      const userMsg = buildActionMessages(a.id, CTX)
        .filter((m) => m.role === 'user')
        .map((m) => (typeof m.content === 'string' ? m.content : ''))
        .join('\n');
      expect(userMsg).toContain(CTX);
    }
  });

  it('summarize asks for bullet points', () => {
    const [sys, user] = buildActionMessages('summarize', CTX);
    expect((sys.content as string).toLowerCase()).toContain('summar');
    expect((user.content as string).toLowerCase()).toContain('summarize');
  });

  it('translate_en targets English', () => {
    const [, user] = buildActionMessages('translate_en', CTX);
    expect((user.content as string).toLowerCase()).toContain('english');
  });

  it('translate_zh targets Chinese', () => {
    const [sys] = buildActionMessages('translate_zh', CTX);
    expect((sys.content as string)).toContain('简体中文');
  });

  it('continue instructs to output only the continuation', () => {
    const [sys] = buildActionMessages('continue', CTX);
    expect((sys.content as string).toLowerCase()).toContain('continuation');
  });

  it('extract_todos produces a markdown task list', () => {
    const [sys] = buildActionMessages('extract_todos', CTX);
    expect((sys.content as string)).toContain('- [ ]');
  });

  it('custom embeds the user instruction when provided', () => {
    const [, user] = buildActionMessages('custom', CTX, 'Make it funnier');
    expect((user.content as string)).toContain('Make it funnier');
  });

  it('custom still works with a blank instruction', () => {
    const msgs = buildActionMessages('custom', CTX, '   ');
    expect(msgs.length).toBeGreaterThanOrEqual(2);
  });
});

// ── filterActions ─────────────────────────────────────────

describe('filterActions', () => {
  it('returns everything on empty query', () => {
    expect(filterActions(AI_ACTIONS, '').length).toBe(AI_ACTIONS.length);
    expect(filterActions(AI_ACTIONS, '   ').length).toBe(AI_ACTIONS.length);
  });

  it('matches by localized label', () => {
    const r = filterActions(AI_ACTIONS, '总结');
    expect(r.length).toBe(1);
    expect(r[0].id).toBe('summarize');
  });

  it('matches by English keyword (translate → two actions)', () => {
    const r = filterActions(AI_ACTIONS, 'translate');
    expect(r.map((a) => a.id).sort()).toEqual(['translate_en', 'translate_zh']);
  });

  it('matches case-insensitively', () => {
    const r = filterActions(AI_ACTIONS, 'TODO');
    expect(r.length).toBe(1);
    expect(r[0].id).toBe('extract_todos');
  });

  it('returns empty array when nothing matches', () => {
    expect(filterActions(AI_ACTIONS, 'zzz_nomatch')).toEqual([]);
  });
});

// ── getActionById ─────────────────────────────────────────

describe('getActionById', () => {
  it('finds an existing action', () => {
    expect(getActionById(AI_ACTIONS, 'rewrite')?.id).toBe('rewrite');
  });
  it('returns undefined for unknown id', () => {
    expect(getActionById(AI_ACTIONS, 'nope' as AiActionId)).toBeUndefined();
  });
});

// ── registry sanity ───────────────────────────────────────

describe('AI_ACTIONS registry', () => {
  it('every action has a unique id', () => {
    const ids = AI_ACTIONS.map((a) => a.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('every action has label, icon, description, and at least one keyword', () => {
    for (const a of AI_ACTIONS) {
      expect(a.label.length).toBeGreaterThan(0);
      expect(a.icon.length).toBeGreaterThan(0);
      expect(a.description.length).toBeGreaterThan(0);
      expect(a.keywords.length).toBeGreaterThanOrEqual(1);
    }
  });

  it('icons are Ionicons glyph names (no emoji)', () => {
    // Reject any action whose icon contains a non-ASCII (emoji) character.
    for (const a of AI_ACTIONS) {
      expect(/^[\x00-\x7F]+$/.test(a.icon)).toBe(true);
    }
  });

  it('only the custom action requires a user prompt', () => {
    const needing = AI_ACTIONS.filter((a) => a.needsUserPrompt);
    expect(needing.map((a) => a.id)).toEqual(['custom']);
  });
});
