/**
 * Regression tests for auto-tagging (#1221).
 *
 * Verifies:
 * 1. extractAutoTags extracts meaningful keywords
 * 2. Stop words are filtered out
 * 3. CJK text is handled correctly
 * 4. Markdown is stripped before extraction
 * 5. Title is weighted higher than content
 * 6. maxTags limit is respected
 */

import { extractAutoTags, autoTagNote } from '../../utils/autoTag';

describe('Auto-tagging (#1221)', () => {
  describe('extractAutoTags', () => {
    it('should extract keywords from English text', () => {
      const tags = extractAutoTags(
        'Machine Learning Fundamentals',
        'Machine learning is a subset of artificial intelligence that focuses on building systems that learn from data.'
      );
      expect(tags.length).toBeGreaterThan(0);
      expect(tags).toContain('machine');
      expect(tags).toContain('learning');
    });

    it('should filter stop words', () => {
      const tags = extractAutoTags(
        'The quick brown fox',
        'The quick brown fox jumps over the lazy dog'
      );
      expect(tags).not.toContain('the');
      expect(tags).not.toContain('over');
    });

    it('should handle CJK text via bigrams', () => {
      const tags = extractAutoTags(
        '机器学习基础',
        '机器学习是人工智能的一个子领域，专注于构建从数据中学习的系统'
      );
      expect(tags.length).toBeGreaterThan(0);
      // "机器" and "学习" should be high-frequency bigrams
      expect(tags).toContain('机器');
    });

    it('should strip markdown formatting', () => {
      const tags = extractAutoTags(
        '# Heading',
        '**bold** and *italic* and `code` and [link](http://example.com)'
      );
      // Should not contain markdown chars
      expect(tags.every(t => !t.includes('#') && !t.includes('`'))).toBe(true);
    });

    it('should weight title higher than content', () => {
      const tags = extractAutoTags(
        'quantum computing',
        'classical computing has been the standard for decades'
      );
      // 'quantum' appears in title (weighted 2x) but not in content
      expect(tags).toContain('quantum');
    });

    it('should respect maxTags limit', () => {
      const longText = 'alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau';
      const tags = extractAutoTags('title', longText, 3);
      expect(tags.length).toBeLessThanOrEqual(3);
    });

    it('should return empty for empty input', () => {
      const tags = extractAutoTags('', '');
      expect(tags).toEqual([]);
    });

    it('should ignore short tokens (< 3 chars for Latin)', () => {
      const tags = extractAutoTags('AI ML', 'AI and ML are abbreviations');
      // 'ai' and 'ml' are too short
      expect(tags.every(t => t.length >= 3)).toBe(true);
    });
  });

  describe('autoTagNote', () => {
    it('should add new tags and return them', async () => {
      const mockAddTag = jest.fn().mockResolvedValue(undefined);
      const newTags = await autoTagNote(
        'note-1',
        'Rust Programming',
        'Rust is a systems programming language focused on safety and performance',
        [], // no existing tags
        mockAddTag
      );
      expect(newTags.length).toBeGreaterThan(0);
      expect(mockAddTag).toHaveBeenCalled();
    });

    it('should not add tags that already exist', async () => {
      const mockAddTag = jest.fn().mockResolvedValue(undefined);
      const newTags = await autoTagNote(
        'note-1',
        'Rust Programming',
        'Rust is a systems programming language',
        ['rust', 'programming'], // already exist
        mockAddTag
      );
      // Should not re-add existing tags
      for (const tag of newTags) {
        expect(['rust', 'programming']).not.toContain(tag);
      }
    });
  });
});
