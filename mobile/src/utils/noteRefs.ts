/**
 * Chat-Note 双向引用 (#2035)
 *
 * Auto-detect note titles in AI response text and enable
 * click-to-open / long-press-to-copy-wikilink behavior.
 *
 * Detection strategy: greedy longest-match with boundary checks
 * to avoid substring false positives (e.g. "React" ≠ "React Native").
 */

import { getNotes, invalidateNoteTitleCache, setOnInvalidateNoteTitleCache } from '../db';

export interface NoteRef {
  title: string;
  noteId: string;
  start: number;
  end: number;
}

// Module-level cache: title → noteId
let noteTitleCache: Map<string, string> | null = null;
/** Promise for in-flight loadNoteTitleMap() — dedup concurrent callers (#2526). */
let noteTitleMapPromise: Promise<Map<string, string>> | null = null;
/** True if invalidation happened while loading — prevents restoring stale data. */
let noteTitleMapDirty = false;
let cacheTimestamp = 0;
const CACHE_TTL_MS = 30_000;

// Register a callback so db.ts invalidations also clear this cache (#2527).
setOnInvalidateNoteTitleCache(() => {
  noteTitleCache = null;
  if (noteTitleMapPromise) {
    noteTitleMapDirty = true;
  }
  cacheTimestamp = 0;
});

/**
 * Load all non-template note titles from DB into a cached map (title → id).
 * Titles sorted by length descending for greedy longest-match.
 *
 * Dedups concurrent callers: if another call is already in-flight, subsequent
 * callers await the same promise instead of firing duplicate DB queries (#2526).
 */
export async function loadNoteTitleMap(): Promise<Map<string, string>> {
  const now = Date.now();
  if (noteTitleCache && now - cacheTimestamp < CACHE_TTL_MS) {
    return noteTitleCache;
  }
  if (noteTitleMapPromise) return noteTitleMapPromise;
  noteTitleMapPromise = _doLoadNoteTitleMap(now);
  return noteTitleMapPromise;
}

async function _doLoadNoteTitleMap(startTime: number): Promise<Map<string, string>> {
  try {
    const notes = await getNotes();
    const sorted = [...notes].sort((a, b) => b.title.length - a.title.length);
    const map = new Map<string, string>();
    for (const n of sorted) {
      if (n.title.trim()) {
        map.set(n.title, n.id);
      }
    }
    // Only cache if not invalidated while we were loading (#2526 read race)
    if (!noteTitleMapDirty) {
      noteTitleCache = map;
      cacheTimestamp = startTime;
    }
    return map;
  } finally {
    noteTitleMapPromise = null;
    noteTitleMapDirty = false;
  }
}

/** Force cache refresh on next load (e.g. after note create/delete). Also
 *  invalidates the db.ts cache to keep both caches in sync (#2527). */
export function clearNoteTitleCache(): void {
  noteTitleCache = null;
  cacheTimestamp = 0;
  if (noteTitleMapPromise) {
    noteTitleMapDirty = true;
  }
  // Invalidate the db.ts cache too, so both note-title caches stay in sync (#2527).
  invalidateNoteTitleCache();
}

/**
 * Find note references in text using greedy longest-match.
 *
 * @param text   Raw text to scan
 * @param titleMap  title → noteId map (from loadNoteTitleMap)
 * @returns Sorted array of NoteRef, empty if none
 */
export function findNoteReferences(
  text: string,
  titleMap: Map<string, string>,
): NoteRef[] {
  if (!titleMap.size || !text) return [];

  const refs: NoteRef[] = [];
  const lowerText = text.toLowerCase();  // hoisted — O(n²) → O(n) (#2396)

  for (const [title, noteId] of titleMap) {
    if (!title.trim()) continue;
    const lowerTitle = title.toLowerCase();
    let searchFrom = 0;

    while (true) {
      const idx = lowerText.indexOf(lowerTitle, searchFrom);
      if (idx === -1) break;

      // Boundary check: ensure match is not in the middle of a Latin word.
      // For ASCII-only titles, the character after the match must not be
      // a letter/digit/underscore (e.g. "React" in "Reactor" → no match).
      // For non-ASCII (CJK etc.) titles, boundary check is relaxed because
      // CJK characters don't form compound words like Latin does.
      const isAsciiOnly = /^[\x00-\x7F]+$/.test(title);
      const afterIdx = idx + title.length;
      const afterChar = afterIdx < text.length ? text[afterIdx] : ' ';
      const validAfter = !isAsciiOnly || !/[a-zA-Z0-9_]/.test(afterChar);
      if (!validAfter) {
        searchFrom = idx + 1;
        continue;
      }
      const beforeIdx = idx - 1;
      const beforeChar = beforeIdx >= 0 ? text[beforeIdx] : ' ';
      const validBefore = !isAsciiOnly || !/[a-zA-Z0-9_]/.test(beforeChar);
      if (!validBefore) {
        searchFrom = idx + 1;
        continue;
      }

      // Avoid overlapping matches (greedy longest-match already ensures
      // longer titles are checked first, but overlapping can still happen
      // with different-length titles at different positions).
      const overlapping = refs.some(
        r =>
          (idx >= r.start && idx < r.end) ||
          (r.start >= idx && r.start < idx + title.length),
      );
      if (!overlapping) {
        refs.push({ title, noteId, start: idx, end: idx + title.length });
      }
      searchFrom = idx + 1;
    }
  }

  return refs.sort((a, b) => a.start - b.start);
}

/**
 * Split a line of text into segments around note references.
 * Non-ref segments are returned with `isNoteRef: false`,
 * ref segments with `isNoteRef: true`.
 */
export function splitLineByNoteRefs(
  line: string,
  refs: NoteRef[],
): Array<{ text: string; isNoteRef: boolean; noteId?: string; title?: string }> {
  if (!refs.length) return [{ text: line, isNoteRef: false }];

  const parts: Array<{
    text: string;
    isNoteRef: boolean;
    noteId?: string;
    title?: string;
  }> = [];
  let cursor = 0;

  for (const ref of refs) {
    if (ref.start > cursor) {
      parts.push({
        text: line.slice(cursor, ref.start),
        isNoteRef: false,
      });
    }
    parts.push({
      text: ref.title,
      isNoteRef: true,
      noteId: ref.noteId,
      title: ref.title,
    });
    cursor = ref.end;
  }

  if (cursor < line.length) {
    parts.push({ text: line.slice(cursor), isNoteRef: false });
  }

  return parts;
}
