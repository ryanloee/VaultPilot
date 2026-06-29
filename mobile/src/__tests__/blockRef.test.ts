/**
 * Unit tests for #2152 — block-level transclusion utilities (utils/blockRef.ts).
 * Pure-logic coverage, no React/RN dependencies.
 */

import {
  extractBlocks,
  extractBlockAnchors,
  getBlockByAnchor,
  parseBlockReference,
  getBlockPreview,
  generateBlockId,
  insertAnchorAt,
  trailingAnchor,
  stripTrailingAnchor,
  isStandaloneAnchorLine,
} from '../utils/blockRef';

// ── trailingAnchor / stripTrailingAnchor ───────────────────────

describe('trailingAnchor', () => {
  it('extracts a trailing inline anchor', () => {
    expect(trailingAnchor('some text ^myAnchor')).toBe('myAnchor');
  });
  it('returns null when no anchor present', () => {
    expect(trailingAnchor('no anchor here')).toBeNull();
  });
  it('requires whitespace before ^', () => {
    // "foo^bar" is not a valid anchor (no separating space)
    expect(trailingAnchor('foo^bar')).toBeNull();
  });
  it('handles trailing whitespace after anchor', () => {
    expect(trailingAnchor('text ^id   ')).toBe('id');
  });
});

describe('stripTrailingAnchor', () => {
  it('removes a trailing inline anchor', () => {
    expect(stripTrailingAnchor('text ^myAnchor')).toBe('text');
  });
  it('leaves text without anchor unchanged', () => {
    expect(stripTrailingAnchor('plain text')).toBe('plain text');
  });
});

describe('isStandaloneAnchorLine', () => {
  it('detects a standalone anchor line', () => {
    expect(isStandaloneAnchorLine('^myAnchor')).toBe(true);
    expect(isStandaloneAnchorLine('  ^myAnchor  ')).toBe(true);
  });
  it('rejects non-anchor lines', () => {
    expect(isStandaloneAnchorLine('text ^myAnchor')).toBe(false);
    expect(isStandaloneAnchorLine('not an anchor')).toBe(false);
    expect(isStandaloneAnchorLine('^1bad')).toBe(false); // must start with a letter
  });
});

// ── extractBlocks ───────────────────────────────────────────────

describe('extractBlocks', () => {
  it('splits paragraphs separated by blank lines', () => {
    const blocks = extractBlocks('First paragraph.\n\nSecond paragraph.');
    expect(blocks).toHaveLength(2);
    expect(blocks[0].text).toBe('First paragraph.');
    expect(blocks[1].text).toBe('Second paragraph.');
  });

  it('treats each heading as its own block', () => {
    const blocks = extractBlocks('# Title\nbody under heading');
    expect(blocks).toHaveLength(2);
    expect(blocks[0].text).toBe('# Title');
    expect(blocks[1].text).toBe('body under heading');
  });

  it('extracts a trailing inline anchor for a paragraph', () => {
    const blocks = extractBlocks('A paragraph with an anchor. ^intro');
    expect(blocks[0].anchor).toBe('intro');
    expect(blocks[0].text).toBe('A paragraph with an anchor.');
  });

  it('extracts a standalone anchor line attaching to the preceding block', () => {
    const blocks = extractBlocks('Some content.\n^standalone');
    expect(blocks).toHaveLength(1);
    expect(blocks[0].anchor).toBe('standalone');
    expect(blocks[0].text).toBe('Some content.');
  });

  it('drops a standalone anchor with no preceding block', () => {
    const blocks = extractBlocks('^orphan');
    expect(blocks).toHaveLength(0);
  });

  it('skips lines inside fenced code blocks', () => {
    const blocks = extractBlocks('```\ncode line ^notAnchor\n```\nreal text');
    expect(blocks).toHaveLength(1);
    expect(blocks[0].text).toBe('real text');
    expect(blocks[0].anchor).toBeNull();
  });

  it('assigns sequential ordinal indices', () => {
    const blocks = extractBlocks('a\n\nb\n\nc');
    expect(blocks.map(b => b.index)).toEqual([0, 1, 2]);
  });

  it('records the start line of each block', () => {
    const blocks = extractBlocks('a\n\nb');
    expect(blocks[0].startLine).toBe(0);
    expect(blocks[1].startLine).toBe(2);
  });

  it('returns an empty array for empty content', () => {
    expect(extractBlocks('')).toEqual([]);
  });
});

// ── extractBlockAnchors / getBlockByAnchor ──────────────────────

describe('extractBlockAnchors', () => {
  it('maps anchor id → block text', () => {
    const map = extractBlockAnchors('Hello world. ^greeting\n\nOther text');
    expect(map.get('greeting')).toBe('Hello world.');
    expect(map.has('greeting')).toBe(true);
  });

  it('first occurrence wins for duplicate anchors', () => {
    const map = extractBlockAnchors('First. ^dup\n\nSecond. ^dup');
    expect(map.get('dup')).toBe('First.');
  });

  it('includes only anchored blocks', () => {
    const map = extractBlockAnchors('No anchor here\n\nWith one. ^x');
    expect(map.size).toBe(1);
    expect(map.get('x')).toBe('With one.');
  });
});

describe('getBlockByAnchor', () => {
  it('returns the block text for a known anchor', () => {
    expect(getBlockByAnchor('Content. ^abc', 'abc')).toBe('Content.');
  });
  it('returns null for an unknown anchor', () => {
    expect(getBlockByAnchor('Content. ^abc', 'missing')).toBeNull();
  });
});

// ── parseBlockReference ─────────────────────────────────────────

describe('parseBlockReference', () => {
  it('parses a cross-note block reference', () => {
    const ref = parseBlockReference('[[Meeting Notes#^decisions]]');
    expect(ref).not.toBeNull();
    expect(ref!.noteTitle).toBe('Meeting Notes');
    expect(ref!.anchor).toBe('decisions');
  });

  it('parses a same-note block reference', () => {
    const ref = parseBlockReference('[[#^intro]]');
    expect(ref).not.toBeNull();
    expect(ref!.noteTitle).toBeNull();
    expect(ref!.anchor).toBe('intro');
  });

  it('parses a plain wikilink without anchor', () => {
    const ref = parseBlockReference('[[Some Note]]');
    expect(ref).not.toBeNull();
    expect(ref!.noteTitle).toBe('Some Note');
    expect(ref!.anchor).toBeNull();
  });

  it('rejects a reference with a malformed anchor id', () => {
    // anchor must start with a letter
    expect(parseBlockReference('[[Note#^1bad]]')).toBeNull();
  });

  it('returns null for non-bracketed text', () => {
    expect(parseBlockReference('not a link')).toBeNull();
  });

  it('returns null for empty brackets', () => {
    expect(parseBlockReference('[[]]')).toBeNull();
  });
});

// ── getBlockPreview ─────────────────────────────────────────────

describe('getBlockPreview', () => {
  it('returns short text unchanged (collapsed whitespace)', () => {
    expect(getBlockPreview('hello   world')).toBe('hello world');
  });
  it('truncates long text with an ellipsis', () => {
    const long = 'a'.repeat(100);
    const preview = getBlockPreview(long, 10);
    expect(preview.length).toBe(10);
    expect(preview.endsWith('…')).toBe(true);
  });
  it('uses a default max length of 60', () => {
    const long = 'a'.repeat(80);
    const preview = getBlockPreview(long);
    expect(preview.length).toBe(60);
    expect(preview.endsWith('…')).toBe(true);
  });
  it('handles empty input', () => {
    expect(getBlockPreview('')).toBe('');
  });
});

// ── generateBlockId ─────────────────────────────────────────────

describe('generateBlockId', () => {
  it('produces an 8-char id', () => {
    expect(generateBlockId()).toHaveLength(8);
  });
  it('produces unique ids (probabilistically)', () => {
    const ids = new Set<string>();
    for (let i = 0; i < 1000; i++) ids.add(generateBlockId());
    expect(ids.size).toBeGreaterThan(990);
  });
  it('only uses base36 characters', () => {
    const id = generateBlockId();
    expect(/^[a-z0-9]+$/.test(id)).toBe(true);
  });
});

// ── insertAnchorAt ──────────────────────────────────────────────

describe('insertAnchorAt', () => {
  it('appends an inline anchor to the end of the paragraph block', () => {
    const res = insertAnchorAt('Hello world', 5);
    expect(res).not.toBeNull();
    expect(res!.content).toBe('Hello world ^' + res!.anchor);
  });

  it('uses a provided custom anchor', () => {
    const res = insertAnchorAt('text', 2, 'custom');
    expect(res!.anchor).toBe('custom');
    expect(res!.content).toBe('text ^custom');
  });

  it('appends to the last line of a multi-line paragraph', () => {
    const content = 'line one\nline two';
    const res = insertAnchorAt(content, 3);
    // anchor lands on the last line of the block
    expect(res!.content).toBe('line one\nline two ^' + res!.anchor);
  });

  it('does not extend past a blank line into the next block', () => {
    const content = 'first\n\nsecond';
    const res = insertAnchorAt(content, 1); // caret in the first block
    expect(res!.content.startsWith('first ^')).toBe(true);
    expect(res!.content.endsWith('\n\nsecond')).toBe(true);
  });

  it('returns null when the block already has an inline anchor', () => {
    expect(insertAnchorAt('text ^exists', 2)).toBeNull();
  });

  it('returns null when the target line is a standalone anchor', () => {
    expect(insertAnchorAt('^id', 1)).toBeNull();
  });

  it('returns null for an out-of-range caret', () => {
    expect(insertAnchorAt('text', -1)).toBeNull();
    expect(insertAnchorAt('text', 999)).toBeNull();
  });

  it('returns null for a malformed custom anchor', () => {
    expect(insertAnchorAt('text', 2, '1bad')).toBeNull();
  });
});
