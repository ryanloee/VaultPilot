/**
 * Regression test for #2742: autoTag emits spurious bigram tags from CJK
 * punctuation (。、！？…).
 *
 * isCJK() covers the CJK Symbols & Punctuation block (U+3000–U+303F), so
 * full-width punctuation was merged into CJK runs, producing bogus bigrams
 * like "好。" / "记。" that were offered as auto-tag suggestions.
 *
 * The fix terminates a CJK word run at punctuation boundaries (isCjkRunChar),
 * so punctuation is dropped instead of joining a bigram. A larger maxTags is
 * used so valid bigrams from later runs are not crowded out by the top-5 limit.
 */

import { extractAutoTags } from '../../utils/autoTag';

describe('issue_2742: autoTag drops CJK punctuation bigrams', () => {
  test('trailing full-width period does not create a punctuation bigram', () => {
    const tags = extractAutoTags('', '今天天气很好。 关于人工智能的笔记内容', 30);
    expect(tags).not.toContain('好。');
    expect(tags).not.toContain('记。');
    // Valid CJK bigrams from later runs are still produced.
    expect(tags).toContain('笔记');
  });

  test('multiple punctuation marks are not merged into CJK runs', () => {
    const tags = extractAutoTags('', '人工智能的笔记，关于机器学习！深度学习？', 30);
    expect(tags).not.toContain('记，');
    expect(tags).not.toContain('习！');
    expect(tags).not.toContain('习？');
    expect(tags).toContain('人工');
  });

  test('enumeration comma punctuation terminates runs', () => {
    const tags = extractAutoTags('', ' project 计划、任务与笔记 ', 30);
    expect(tags).not.toContain('划、');
    expect(tags).not.toContain('、任');
  });

  test('pure punctuation produces no tags', () => {
    const tags = extractAutoTags('', '。、！？…；：', 30);
    expect(tags).toEqual([]);
  });
});
