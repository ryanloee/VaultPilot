// Regression test for issue #2879: isNoteRelatedQuery regex too broad —
// false-positive note context injection for everyday English words.
//
// Bug: the regex /笔记|记录|保存|记了|记过|知识库|notes?|save|record/i
// matches "save", "record", "notes", "noted" etc. in completely unrelated
// conversations like "Can you save me some time?" or "Do you have a record
// of this phone call?" — triggering full-note-dump context injection.
//
// Fix: replace generic English matches with note-specific phrases:
//   \bnotes?\b  — standalone "note"/"notes" (word boundary prevents "notebook", "noted")
//   save (a )?note — "save note" or "save a note"
//   \bnote (this|that) down\b — "note this down" / "note that down"
//   my notes / my records — possessive note references
// Chinese terms kept as-is (already self-delimiting).

import { isNoteRelatedQuery, looksLikeSmallTalk } from '../../services/rag';

describe('Issue #2879 — isNoteRelatedQuery regex too broad', () => {
  describe('isNoteRelatedQuery — true positives (should match)', () => {
    it('matches 笔记 term', () => {
      expect(isNoteRelatedQuery('查找我的笔记')).toBe(true);
    });
    it('matches "my notes"', () => {
      expect(isNoteRelatedQuery('search my notes for password')).toBe(true);
    });
    it('matches "save a note"', () => {
      expect(isNoteRelatedQuery('save a note about today')).toBe(true);
    });
    it('matches "save note" without article', () => {
      expect(isNoteRelatedQuery('save note: meeting at 3pm')).toBe(true);
    });
    it('matches "note this down"', () => {
      expect(isNoteRelatedQuery('note this down please')).toBe(true);
    });
    it('matches "note that down"', () => {
      expect(isNoteRelatedQuery('note that down for later')).toBe(true);
    });
    it('matches Chinese 保存', () => {
      expect(isNoteRelatedQuery('保存这个信息')).toBe(true);
    });
    it('matches 知识库', () => {
      expect(isNoteRelatedQuery('我的知识库有什么')).toBe(true);
    });
    it('matches "my records"', () => {
      expect(isNoteRelatedQuery('show my records')).toBe(true);
    });
    it('matches standalone "notes"', () => {
      expect(isNoteRelatedQuery('what are my notes about')).toBe(true);
    });
  });

  describe('isNoteRelatedQuery — false positives (should NOT match after fix)', () => {
    it('does NOT match "save me some time" (generic save)', () => {
      expect(isNoteRelatedQuery('Can you save me some time?')).toBe(false);
    });
    it('does NOT match "saving money"', () => {
      expect(isNoteRelatedQuery('tips for saving money')).toBe(false);
    });
    it('does NOT match "record a phone call" (generic record)', () => {
      expect(isNoteRelatedQuery('Do you have a record of this phone call?')).toBe(false);
    });
    it('does NOT match "recording studio"', () => {
      expect(isNoteRelatedQuery('best recording studio equipment')).toBe(false);
    });
    it('does NOT match "noted the address"', () => {
      expect(isNoteRelatedQuery('I noted the address down')).toBe(false);
    });
    it('does NOT match "notebook computer"', () => {
      expect(isNoteRelatedQuery('buy a notebook computer')).toBe(false);
    });
    it('does NOT match "saver" (derived word)', () => {
      expect(isNoteRelatedQuery('screen saver settings')).toBe(false);
    });
    it('does NOT match generic English without note context', () => {
      expect(isNoteRelatedQuery('how is the weather today')).toBe(false);
    });
  });

  describe('looksLikeSmallTalk — note terms override small-talk', () => {
    it('"谢谢你的笔记" is NOT small talk', () => {
      expect(looksLikeSmallTalk('谢谢你的笔记')).toBe(false);
    });
    it('"thanks" alone IS small talk', () => {
      expect(looksLikeSmallTalk('thanks')).toBe(true);
    });
    it('"my notes" prevents small-talk classification', () => {
      expect(looksLikeSmallTalk('hey check my notes')).toBe(false);
    });
    it('"save a note" prevents small-talk', () => {
      expect(looksLikeSmallTalk('save a note please')).toBe(false);
    });
    it('greeting without note terms is small talk', () => {
      expect(looksLikeSmallTalk('hi')).toBe(true);
    });
  });
});