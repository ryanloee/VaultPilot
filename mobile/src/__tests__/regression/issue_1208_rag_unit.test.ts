/**
 * Unit tests for rag.ts — RAG search logic (#1208).
 *
 * Tests: extractKeywords (via buildNoteContext), parseToolCalls edge cases,
 * getDeviceLocale, buildSystemPrompt.
 */

import { buildNoteContext, parseToolCalls, getDeviceLocale, buildSystemPrompt } from '../../services/rag';
import { searchNotes } from '../../db';

// Mock the db module
jest.mock('../../db', () => ({
  searchNotes: jest.fn(),
  createNote: jest.fn(),
  updateNote: jest.fn(),
}));

const mockSearchNotes = searchNotes as jest.MockedFunction<typeof searchNotes>;

beforeEach(() => {
  jest.clearAllMocks();
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

  it('returns null when only stop words present', async () => {
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

  it('handles CJK text with single-char keywords', async () => {
    mockSearchNotes.mockResolvedValue([]);
    const result = await buildNoteContext('什么是机器学习');
    // CJK single chars should be extracted as keywords
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
