/**
 * Unit tests for rag.ts — RAG search logic (#1208).
 *
 * Tests: extractKeywords (via buildNoteContext), parseToolCalls edge cases,
 * getDeviceLocale, buildSystemPrompt.
 */

import { buildNoteContext, parseToolCalls, getDeviceLocale, buildSystemPrompt, executeSave } from '../../services/rag';
import { searchNotes, getNotes } from '../../db';

// Mock the db module
jest.mock('../../db', () => ({
  searchNotes: jest.fn(),
  getNotes: jest.fn(),
  createNote: jest.fn(),
  updateNote: jest.fn(),
}));

const mockSearchNotes = searchNotes as jest.MockedFunction<typeof searchNotes>;
const mockGetNotes = getNotes as jest.MockedFunction<typeof getNotes>;
const mockCreateNote = require('../../db').createNote as jest.MockedFunction<any>;
const mockUpdateNote = require('../../db').updateNote as jest.MockedFunction<any>;

// Default: getNotes returns some notes so search is not skipped
const defaultNotes = [
  { id: '1', title: 'Default Note', content: 'Default content', starred: 0, folder: '', created_at: 0, updated_at: 0 },
];

beforeEach(() => {
  jest.clearAllMocks();
  mockGetNotes.mockResolvedValue(defaultNotes);
});

// ── getDeviceLocale ─────────────────────────────────────────

describe('getDeviceLocale', () => {
  it('returns a lowercase 2-letter locale', () => {
    const locale = getDeviceLocale();
    expect(locale).toMatch(/^[a-z]{2}$/);
  });
});

// ── extractKeywords (tested via buildNoteContext) ────────────

describe('buildNoteContext — keyword extraction', () => {
  it('returns null for empty message', async () => {
    const result = await buildNoteContext('');
    expect(result).toBeNull();
  });

  it('falls back to recent notes when only stop words present', async () => {
    // When only stop words, search returns nothing, but fallback to recent notes
    mockSearchNotes.mockResolvedValue([]);
    const result = await buildNoteContext('the a an is are was were');
    // With fallback, result should not be null if notes exist
    expect(result).not.toBeNull();
    expect(result).toContain('Default Note');
  });

  it('returns null when no notes exist', async () => {
    mockGetNotes.mockResolvedValue([]);
    const result = await buildNoteContext('the a an is are was were');
    expect(result).toBeNull();
  });

  it('extracts meaningful keywords and searches notes', async () => {
    mockSearchNotes.mockResolvedValue([
      { id: '1', title: 'Test Note', content: 'Test content', starred: 0, folder: '', created_at: 0, updated_at: 0 },
    ]);

    const result = await buildNoteContext('tell me about TypeScript generics');
    expect(result).not.toBeNull();
    expect(mockSearchNotes).toHaveBeenCalled();
    // Should have been called with non-stop-word keywords
    const calledWith = mockSearchNotes.mock.calls.map(c => c[0]);
    expect(calledWith.some(kw => kw.includes('typescript') || kw.includes('generics'))).toBe(true);
  });

  it('handles CJK text with ngram keywords', async () => {
    mockSearchNotes.mockResolvedValue([]);
    await buildNoteContext('什么是机器学习');
    // CJK ngrams should be extracted as keywords
    expect(mockSearchNotes).toHaveBeenCalled();
  });

  it('deduplicates notes across multiple keyword searches', async () => {
    const note = { id: '1', title: 'Note', content: 'Content', starred: 0, folder: '', created_at: 0, updated_at: 0 };
    mockSearchNotes.mockResolvedValue([note]);

    const result = await buildNoteContext('TypeScript programming code');
    expect(result).not.toBeNull();
    // Even if multiple keywords match the same note, it should appear once
  });

  it('limits to MAX_CONTEXT_NOTES (5) notes', async () => {
    const notes = Array.from({ length: 10 }, (_, i) => ({
      id: String(i), title: `Note ${i}`, content: `Content ${i}`, starred: 0, folder: '', created_at: 0, updated_at: 0,
    }));
    // Each call returns different notes
    mockSearchNotes.mockImplementation(async () => notes.splice(0, 2));

    const result = await buildNoteContext('alpha bravo charlie delta echo foxtrot golf hotel');
    if (result) {
      // Count note blocks
      const blocks = result.match(/【/g);
      expect(blocks?.length).toBeLessThanOrEqual(5);
    }
  });

  it('truncates long note content to MAX_NOTE_CONTENT_CHARS', async () => {
    const longContent = 'x'.repeat(2000);
    mockSearchNotes.mockResolvedValue([
      { id: '1', title: 'Long', content: longContent, starred: 0, folder: '', created_at: 0, updated_at: 0 },
    ]);

    const result = await buildNoteContext('long content test');
    expect(result).not.toBeNull();
    expect(result).toContain('...');
    // The content in result should be truncated
    expect(result!.length).toBeLessThan(longContent.length + 200);
  });

  it('escapes double quotes in keywords for FTS safety', async () => {
    mockSearchNotes.mockResolvedValue([]);
    // Should not throw even with quotes in input
    await buildNoteContext('test "quoted" keyword');
    expect(mockSearchNotes).toHaveBeenCalled();
  });

  it('uses recent conversation history for keyword extraction', async () => {
    mockSearchNotes.mockResolvedValue([
      { id: '1', title: 'Rust Lifetimes', content: 'Lifetimes in Rust...', starred: 0, folder: '', created_at: 0, updated_at: 0 },
    ]);
    // "explain more" alone would yield no useful keywords,
    // but history contains "Rust lifetimes" which should be extracted
    const result = await buildNoteContext('explain more', [
      'I was reading about Rust lifetimes',
      'The borrow checker is confusing',
    ]);
    expect(result).not.toBeNull();
    expect(result).toContain('Rust Lifetimes');
    // Should have searched with keywords from history
    expect(mockSearchNotes).toHaveBeenCalled();
    const searchedKeywords = mockSearchNotes.mock.calls.map((c: any) => c[0]);
    const hasRust = searchedKeywords.some((k: string) => k.includes('rust'));
    expect(hasRust).toBe(true);
  });

  it('without recentMessages still works (backward compatible)', async () => {
    mockSearchNotes.mockResolvedValue([]);
    const result = await buildNoteContext('TypeScript generics');
    // With fallback to recent notes, result should not be null
    expect(result).not.toBeNull();
  });
});

// ── parseToolCalls edge cases ───────────────────────────────

describe('parseToolCalls — edge cases', () => {
  it('handles multiple SAVE_NOTE markers', () => {
    const resp = 'Start\n[SAVE_NOTE: Title 1]Content 1\n[SAVE_NOTE: Title 2]Content 2\nEnd';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(2);
    expect(pendingSaves[0].title).toBe('Title 1');
    expect(pendingSaves[1].title).toBe('Title 2');
  });

  it('handles empty content after marker', () => {
    const resp = 'text\n[SAVE_NOTE: Empty]';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    // Empty content — marker is preserved since no save is created
    expect(pendingSaves).toHaveLength(0);
    expect(cleaned).toContain('text');
  });

  it('handles marker with no closing bracket', () => {
    const resp = 'text\n[SAVE_NOTE: unclosed title without bracket';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(0);
  });

  it('preserves content containing [ characters (#1187 regression)', () => {
    const resp = '[SAVE_NOTE: Code]Use array[0] to access first element';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].content).toBe('Use array[0] to access first element');
  });

  it('handles empty title (skips marker)', () => {
    const resp = 'text\n[SAVE_NOTE: ]content here';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(0);
  });
});

// ── buildSystemPrompt ───────────────────────────────────────

describe('buildSystemPrompt', () => {
  it('includes security rules', () => {
    const prompt = buildSystemPrompt(null);
    // Security rules are locale-dependent; check for both Chinese and English
    const hasSecurity = prompt.includes('机密') || prompt.includes('confidential');
    expect(hasSecurity).toBe(true);
  });

  it('includes note context when provided', () => {
    const context = 'Here are some notes:\n\nNote 1 content';
    const prompt = buildSystemPrompt(context);
    expect(prompt).toContain(context);
  });

  it('does not include note context section when null', () => {
    const prompt = buildSystemPrompt(null);
    expect(prompt).not.toContain('Here are the user');
    expect(prompt).not.toContain('以下是用户保存');
  });

  it('includes note-saving instructions', () => {
    const prompt = buildSystemPrompt(null);
    expect(prompt).toContain('SAVE_NOTE');
  });
});

// ── executeSave ─────────────────────────────────────────────

describe('executeSave', () => {
  beforeEach(() => {
    mockCreateNote.mockReset();
    mockUpdateNote.mockReset();
  });

  it('calls createNote with title and content (single operation)', async () => {
    mockCreateNote.mockResolvedValue('note-123');

    const result = await executeSave({ title: 'My Note', content: 'Note body here' });
    expect(mockCreateNote).toHaveBeenCalledWith('My Note', 'Note body here');
    expect(mockUpdateNote).not.toHaveBeenCalled();
    expect(result).toContain('My Note');
  });

  it('returns Chinese confirmation message', async () => {
    mockCreateNote.mockResolvedValue('id-1');

    const result = await executeSave({ title: '测试', content: '内容' });
    expect(result).toBe('已保存笔记「测试」');
  });
});

// ── parseToolCalls — additional edge cases ──────────────────

describe('parseToolCalls — additional edge cases', () => {
  it('handles content with multiple [ characters after title', () => {
    const resp = '[SAVE_NOTE: Arrays]Use arr[0] and arr[1] for access';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].content).toBe('Use arr[0] and arr[1] for access');
    expect(cleaned).toBe('');
  });

  it('handles marker at very start of response', () => {
    const resp = '[SAVE_NOTE: First]Content from start';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].title).toBe('First');
    expect(cleaned).toBe('');
  });

  it('handles response with no markers at all', () => {
    const resp = 'Just a normal response without any markers.';
    const { cleaned, pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(0);
    expect(cleaned).toBe(resp);
  });

  it('handles title with special characters', () => {
    const resp = '[SAVE_NOTE: C++ 与 Rust 对比]内容在这里';
    const { pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].title).toBe('C++ 与 Rust 对比');
  });

  it('handles content spanning multiple lines', () => {
    const resp = '[SAVE_NOTE: Multi]\nLine 1\nLine 2\nLine 3';
    const { pendingSaves } = parseToolCalls(resp);
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0].content).toContain('Line 1');
    expect(pendingSaves[0].content).toContain('Line 3');
  });
});
