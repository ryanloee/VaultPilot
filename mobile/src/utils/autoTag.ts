/**
 * Auto-tagging for notes — extracts keywords from content (#1221).
 *
 * Uses term frequency analysis with stop word filtering.
 * Handles both CJK and Latin text.
 */

import { isCJK } from './cjk';

/** Common stop words to exclude from tag extraction. */
const STOP_WORDS = new Set([
  // Chinese
  '的', '了', '在', '是', '我', '有', '和', '就', '不', '人', '都', '一', '一个',
  '上', '也', '很', '到', '说', '要', '去', '你', '会', '着', '没有', '看', '好',
  '自己', '这', '他', '她', '它', '们', '那', '些', '什么', '怎么', '如何', '可以',
  '但是', '因为', '所以', '如果', '虽然', '或者', '以及', '而且', '但', '而', '把',
  '被', '让', '给', '对', '从', '向', '往', '以', '用', '为', '所', '之', '其',
  // English
  'the', 'a', 'an', 'is', 'are', 'was', 'were', 'be', 'been', 'being',
  'have', 'has', 'had', 'do', 'does', 'did', 'will', 'would', 'could',
  'should', 'may', 'might', 'shall', 'can', 'to', 'of', 'in', 'for',
  'on', 'with', 'at', 'by', 'from', 'as', 'into', 'through', 'during',
  'before', 'after', 'above', 'below', 'between', 'out', 'off', 'over',
  'under', 'again', 'further', 'then', 'once', 'here', 'there', 'when',
  'where', 'why', 'how', 'all', 'each', 'every', 'both', 'few', 'more',
  'most', 'other', 'some', 'such', 'no', 'nor', 'not', 'only', 'own',
  'same', 'so', 'than', 'too', 'very', 'just', 'because', 'but', 'and',
  'or', 'if', 'while', 'this', 'that', 'these', 'those', 'it', 'its',
]);

/** Markdown/formatting patterns to strip before extraction. */
export function stripMarkdown(text: string): string {
  return text
    .replace(/```[\s\S]*?```/g, '') // code blocks
    .replace(/`[^`]+`/g, '')        // inline code
    .replace(/!?\[([^\]]*)\]\([^)]+\)/g, '$1') // links/images
    .replace(/[#*_~>|=-]/g, '')     // formatting chars
    .replace(/https?:\/\/\S+/g, '') // URLs
    .replace(/\s+/g, ' ')
    .trim();
}

/**
 * Extract keyword tags from note content.
 *
 * @param title Note title (weighted higher)
 * @param content Note content
 * @param maxTags Maximum tags to return (default 5)
 * @returns Array of extracted tag strings
 */
export function extractAutoTags(title: string, content: string, maxTags = 5): string[] {
  const cleanTitle = stripMarkdown(title);
  const cleanContent = stripMarkdown(content);
  const combined = `${cleanTitle} ${cleanTitle} ${cleanContent}`; // title weighted 2x

  // Tokenize: split into consecutive CJK and non-CJK runs (char by char)
  // Uses isCJK() from cjk.ts for comprehensive CJK range detection
  const tokens: string[] = [];
  let current = '';
  let currentIsCJK: boolean | null = null;

  for (const ch of combined) {
    const cjk = isCJK(ch);
    if (currentIsCJK === null) {
      currentIsCJK = cjk;
      current = ch;
    } else if (cjk === currentIsCJK) {
      current += ch;
    } else {
      if (currentIsCJK) {
        // CJK run: extract bigrams
        for (let i = 0; i <= current.length - 2; i++) {
          tokens.push(current.slice(i, i + 2));
        }
      } else {
        // Non-CJK run: split on whitespace and keep 3+ char Latin words
        for (const word of current.split(/\s+/)) {
          if (/^[a-zA-Z]{3,}$/.test(word)) {
            tokens.push(word);
          }
        }
      }
      current = ch;
      currentIsCJK = cjk;
    }
  }

  // Don't forget the last run
  if (current) {
    if (currentIsCJK) {
      for (let i = 0; i <= current.length - 2; i++) {
        tokens.push(current.slice(i, i + 2));
      }
    } else {
      for (const word of current.split(/\s+/)) {
        if (/^[a-zA-Z]{3,}$/.test(word)) {
          tokens.push(word);
        }
      }
    }
  }

  // If nothing extracted, return empty
  if (tokens.length === 0) return [];

  // Count term frequency
  const freq = new Map<string, number>();
  for (const token of tokens) {
    const lower = token.toLowerCase();
    if (STOP_WORDS.has(lower)) continue;
    freq.set(lower, (freq.get(lower) ?? 0) + 1);
  }

  // Sort by frequency, take top N
  return [...freq.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, maxTags)
    .map(([word]) => word);
}

/**
 * Auto-tag a note: extract keywords and save as tags.
 * Returns the newly added tags (does not remove existing tags).
 */
export async function autoTagNote(
  noteId: string,
  title: string,
  content: string,
  existingTags: string[],
  addTagFn: (noteId: string, tag: string) => Promise<void>,
): Promise<string[]> {
  const suggestions = extractAutoTags(title, content);
  const newTags: string[] = [];

  for (const tag of suggestions) {
    if (!existingTags.includes(tag)) {
      await addTagFn(noteId, tag);
      newTags.push(tag);
    }
  }

  return newTags;
}
