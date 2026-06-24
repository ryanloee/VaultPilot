/**
 * Regression test for #1473 — isNoteRelatedQuery exported + tested.
 *
 * isNoteRelatedQuery detects whether the user is asking about notes/knowledge base,
 * triggering RAG to inject all recent notes instead of keyword-based search.
 */

import { isNoteRelatedQuery } from '../../services/rag';

describe('isNoteRelatedQuery (#1473)', () => {
  // Chinese keywords
  it('matches 笔记', () => {
    expect(isNoteRelatedQuery('看看我的笔记')).toBe(true);
  });

  it('matches 记录', () => {
    expect(isNoteRelatedQuery('之前记录了什么')).toBe(true);
  });

  it('matches 保存', () => {
    expect(isNoteRelatedQuery('保存了哪些内容')).toBe(true);
  });

  it('matches 知识库', () => {
    expect(isNoteRelatedQuery('知识库里有什么')).toBe(true);
  });

  it('matches 记了', () => {
    expect(isNoteRelatedQuery('我之前记了什么')).toBe(true);
  });

  it('matches 记过', () => {
    expect(isNoteRelatedQuery('我记过这个话题')).toBe(true);
  });

  // English keywords
  it('matches notes (case-insensitive)', () => {
    expect(isNoteRelatedQuery('show me my notes')).toBe(true);
    expect(isNoteRelatedQuery('Show My Notes')).toBe(true);
  });

  it('matches note (singular)', () => {
    expect(isNoteRelatedQuery('find a note about AI')).toBe(true);
  });

  it('matches save', () => {
    expect(isNoteRelatedQuery('what did I save')).toBe(true);
  });

  it('matches record', () => {
    expect(isNoteRelatedQuery('any record about this')).toBe(true);
  });

  // Negative cases
  it('returns false for unrelated queries', () => {
    expect(isNoteRelatedQuery('今天天气怎么样')).toBe(false);
    expect(isNoteRelatedQuery('what is the meaning of life')).toBe(false);
    expect(isNoteRelatedQuery('帮我写一段代码')).toBe(false);
  });

  it('returns false for empty string', () => {
    expect(isNoteRelatedQuery('')).toBe(false);
  });

  it('returns false for partial matches that do not trigger regex', () => {
    // "notation" does not match notes? (only "note" or "notes")
    expect(isNoteRelatedQuery('musical notation')).toBe(false);
  });
});
