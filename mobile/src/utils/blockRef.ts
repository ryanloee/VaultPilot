/**
 * Block-level reference utilities — #2152 (Block-level Transclusion), Phase 1.
 *
 * Pure logic — no React/RN dependencies — so it is fully unit-testable.
 *
 * Implements an Obsidian-compatible subset of block references:
 *   - Block anchor syntax:  `^blockId` appended to the end of a block, OR a
 *     standalone `^blockId` line that anchors the immediately preceding block.
 *   - Reference syntax:     `[[Note Title#^blockId]]` (cross-note),
 *                           `[[#^blockId]]` (same-note reference).
 *
 * Anchors must begin with a letter and contain only `[A-Za-z0-9-]`.
 */

/** A logical markdown block detected during parsing. */
export interface MarkdownBlock {
  /** 0-based ordinal index of the block within the document. */
  index: number;
  /** The block's anchor id, if one was declared; otherwise null. */
  anchor: string | null;
  /** The human-readable text of the block (anchor marker stripped, trimmed). */
  text: string;
  /** 0-based index of the first source line of the block. */
  startLine: number;
}

/** A parsed block reference `[[Note#^id]]`. */
export interface BlockReference {
  /** Note title being referenced; `null` for same-note references `[[#^id]]`. */
  noteTitle: string | null;
  /** Anchor id; `null` for a plain wikilink `[[Note]]`. */
  anchor: string | null;
  /** The inner target text (without surrounding `[[ ]]`), trimmed. */
  target: string;
}

/** Result of inserting an anchor at a caret position. */
export interface InsertAnchorResult {
  content: string;
  anchor: string;
}

/** Matches an anchor token. */
const ANCHOR_TOKEN_RE = /^[A-Za-z][A-Za-z0-9-]*$/;
/** Matches a trailing inline anchor like `text ^myAnchor` (capturing `myAnchor`). */
const TRAILING_ANCHOR_RE = /\s\^([A-Za-z][A-Za-z0-9-]*)\s*$/;
/** Matches a standalone anchor line `^myAnchor`. */
const STANDALONE_ANCHOR_RE = /^\^([A-Za-z][A-Za-z0-9-]*)$/;
/** Matches a full `[[…]]` wikilink/reference token. */
const REF_RE = /^\[\[([^\]]*)\]\]$/;

/** Return the trailing inline anchor id of `text`, or null. */
export function trailingAnchor(text: string): string | null {
  const m = text.match(TRAILING_ANCHOR_RE);
  return m ? m[1] : null;
}

/** Remove a trailing inline anchor marker from `text`. */
export function stripTrailingAnchor(text: string): string {
  return text.replace(TRAILING_ANCHOR_RE, '');
}

/** True if `line` (trimmed) is a standalone `^id` anchor line. */
export function isStandaloneAnchorLine(line: string): boolean {
  return STANDALONE_ANCHOR_RE.test(line.trim());
}

/**
 * Split markdown `content` into logical blocks.
 *
 * A block is a maximal run of consecutive non-blank lines. A heading line
 * (`# …`) always forms its own block. Lines inside fenced code blocks
 * (```…```) are skipped entirely. A standalone `^id` line attaches its anchor
 * to the immediately preceding block (and is not itself emitted as a block).
 */
export function extractBlocks(content: string): MarkdownBlock[] {
  const lines = content.split('\n');
  const blocks: MarkdownBlock[] = [];
  let inCode = false;
  let buf: string[] = [];
  let bufStart = -1;
  let ordinal = 0;

  const flush = () => {
    if (buf.length === 0) return;
    const joined = buf.join('\n');
    const anchor = trailingAnchor(joined);
    const text = anchor ? stripTrailingAnchor(joined).trim() : joined.trim();
    blocks.push({ index: ordinal++, anchor, text, startLine: bufStart });
    buf = [];
    bufStart = -1;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    // fenced code block toggle
    if (trimmed.startsWith('```')) {
      flush();
      inCode = !inCode;
      continue;
    }
    if (inCode) continue;

    // blank line ends current paragraph
    if (trimmed === '') {
      flush();
      continue;
    }

    // standalone anchor line → attach to the preceding block
    const sa = trimmed.match(STANDALONE_ANCHOR_RE);
    if (sa) {
      flush();
      const prev = blocks[blocks.length - 1];
      if (prev && prev.anchor == null) prev.anchor = sa[1];
      // Otherwise the anchor has no block to bind to and is dropped.
      continue;
    }

    // heading → its own block
    if (/^#{1,6}\s/.test(trimmed)) {
      flush();
      const anchor = trailingAnchor(trimmed);
      const text = anchor ? stripTrailingAnchor(trimmed).trim() : trimmed;
      blocks.push({ index: ordinal++, anchor, text, startLine: i });
      continue;
    }

    // normal paragraph line
    if (bufStart === -1) bufStart = i;
    buf.push(line);
  }
  flush();
  return blocks;
}

/**
 * Build a map of `anchorId → blockText` for all anchored blocks in `content`.
 * When duplicate anchors exist, the first occurrence wins (Obsidian behaviour).
 */
export function extractBlockAnchors(content: string): Map<string, string> {
  const map = new Map<string, string>();
  for (const b of extractBlocks(content)) {
    if (b.anchor && !map.has(b.anchor)) map.set(b.anchor, b.text);
  }
  return map;
}

/** Return the (cleaned) text of the block identified by `anchor`, or null. */
export function getBlockByAnchor(content: string, anchor: string): string | null {
  return extractBlockAnchors(content).get(anchor) ?? null;
}

/**
 * Parse a `[[…]]` token into a structured block reference.
 *
 * Supported forms:
 *   - `[[Note#^id]]`  → { noteTitle: 'Note', anchor: 'id' }
 *   - `[[#^id]]`      → { noteTitle: null,  anchor: 'id' }  (same-note)
 *   - `[[Note]]`      → { noteTitle: 'Note', anchor: null } (plain wikilink)
 *
 * Returns null if `link` is not a valid `[[…]]` token or the anchor id
 * (when present) is malformed.
 */
export function parseBlockReference(link: string): BlockReference | null {
  const m = link.match(REF_RE);
  if (!m) return null;
  const inner = m[1].trim();
  if (!inner) return null;

  const hashIdx = inner.indexOf('#^');
  if (hashIdx !== -1) {
    const noteTitle = inner.slice(0, hashIdx).trim();
    const anchor = inner.slice(hashIdx + 2).trim();
    if (!anchor || !ANCHOR_TOKEN_RE.test(anchor)) return null;
    return { noteTitle: noteTitle === '' ? null : noteTitle, anchor, target: inner };
  }

  // plain wikilink [[Note]]
  return { noteTitle: inner, anchor: null, target: inner };
}

/** Truncate `text` to a preview-friendly single-line length. */
export function getBlockPreview(text: string, maxLen = 60): string {
  const clean = text.replace(/\s+/g, ' ').trim();
  if (clean.length <= maxLen) return clean;
  return clean.slice(0, Math.max(0, maxLen - 1)).trimEnd() + '…';
}

/** Generate a short random block id (8-char base36, always starts with a letter). */
export function generateBlockId(): string {
  const letters = 'abcdefghijklmnopqrstuvwxyz';
  let s = letters[Math.floor(Math.random() * letters.length)];
  for (let i = 0; i < 7; i++) {
    s += Math.floor(Math.random() * 36).toString(36);
  }
  return s;
}

/**
 * Insert a block anchor at the end of the paragraph block containing `caretPos`.
 *
 * The anchor is appended inline (` ^id`) to the last non-blank line of the
 * block. If that line already carries an anchor (inline or standalone), no
 * insertion is performed and null is returned. A custom `anchor` may be
 * supplied; otherwise a random id is generated.
 */
export function insertAnchorAt(
  content: string,
  caretPos: number,
  anchor: string = generateBlockId(),
): InsertAnchorResult | null {
  if (caretPos < 0 || caretPos > content.length) return null;
  if (!ANCHOR_TOKEN_RE.test(anchor)) return null;

  const lines = content.split('\n');

  // Resolve the line index that contains caretPos.
  let pos = 0;
  let lineIdx = 0;
  for (let i = 0; i < lines.length; i++) {
    const lineEnd = pos + lines[i].length;
    if (caretPos <= lineEnd) { lineIdx = i; break; }
    lineIdx = i;
    pos = lineEnd + 1; // +1 for '\n'
  }

  // Walk forward to the last non-blank line of the block.
  let endIdx = lineIdx;
  while (endIdx + 1 < lines.length && lines[endIdx + 1].trim() !== '') {
    endIdx++;
  }

  const targetLine = lines[endIdx];
  if (STANDALONE_ANCHOR_RE.test(targetLine.trim())) return null;
  if (TRAILING_ANCHOR_RE.test(targetLine)) return null;

  lines[endIdx] = targetLine.replace(/\s*$/, '') + ` ^${anchor}`;
  return { content: lines.join('\n'), anchor };
}
