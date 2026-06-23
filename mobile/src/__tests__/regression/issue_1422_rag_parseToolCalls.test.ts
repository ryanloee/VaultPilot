/**
 * Unit tests for rag.ts parseToolCalls + buildSystemPrompt (#1422).
 *
 * parseToolCalls: pure parser for [SAVE_NOTE: title] markers.
 * buildSystemPrompt: prompt builder with security rules + note instructions.
 */

import { parseToolCalls, buildSystemPrompt } from '../../services/rag';

// ── parseToolCalls ────────────────────────────────────────

describe('parseToolCalls', () => {
  it('returns empty result for plain text without markers', () => {
    const result = parseToolCalls('Hello, how can I help?');
    expect(result.cleaned).toBe('Hello, how can I help?');
    expect(result.pendingSaves).toEqual([]);
  });

  it('parses a single SAVE_NOTE marker', () => {
    const input = 'Here is the note:\n[SAVE_NOTE: My Title]\nSome content here.';
    const result = parseToolCalls(input);
    expect(result.pendingSaves).toEqual([
      { title: 'My Title', content: 'Some content here.' },
    ]);
    expect(result.cleaned).toBe('Here is the note:');
  });

  it('parses multiple SAVE_NOTE markers', () => {
    const input = 'Notes:\n[SAVE_NOTE: Title A]\nContent A\n[SAVE_NOTE: Title B]\nContent B';
    const result = parseToolCalls(input);
    expect(result.pendingSaves).toHaveLength(2);
    expect(result.pendingSaves[0]).toEqual({ title: 'Title A', content: 'Content A' });
    expect(result.pendingSaves[1]).toEqual({ title: 'Title B', content: 'Content B' });
    expect(result.cleaned).toBe('Notes:');
  });

  it('trims whitespace from title and content', () => {
    const input = '[SAVE_NOTE:  Spaced Title  ]\n  Spaced Content  ';
    const result = parseToolCalls(input);
    expect(result.pendingSaves[0].title).toBe('Spaced Title');
    expect(result.pendingSaves[0].content).toBe('Spaced Content');
  });

  it('skips marker with empty title', () => {
    const input = 'Before [SAVE_NOTE: ]\nContent after empty title';
    const result = parseToolCalls(input);
    // Empty title → skip, content after ] is treated as plain text
    expect(result.pendingSaves).toEqual([]);
  });

  it('skips marker with empty (whitespace-only) content', () => {
    const input = 'Before [SAVE_NOTE: Title]\n   ';
    const result = parseToolCalls(input);
    // Whitespace-only content trims to empty → not added to pendingSaves
    expect(result.pendingSaves).toEqual([]);
    // Marker text remains in cleaned since it wasn't added to saves
    expect(result.cleaned).toContain('Before');
  });

  it('handles unclosed bracket gracefully', () => {
    const input = 'Text [SAVE_NOTE: unclosed bracket no close';
    const result = parseToolCalls(input);
    // No closing ] → break, no saves
    expect(result.pendingSaves).toEqual([]);
    expect(result.cleaned).toBe(input);
  });

  it('handles marker at start of response', () => {
    const input = '[SAVE_NOTE: First]\nFirst content';
    const result = parseToolCalls(input);
    expect(result.pendingSaves).toHaveLength(1);
    expect(result.cleaned).toBe('');
  });

  it('handles content with special characters', () => {
    const input = '[SAVE_NOTE: Code Snippet]\n```js\nconsole.log("hello");\n```';
    const result = parseToolCalls(input);
    expect(result.pendingSaves[0].content).toContain('```js');
    expect(result.pendingSaves[0].content).toContain('console.log');
  });

  it('strips marker and content, preserves surrounding text', () => {
    const input = 'Intro text.\n[SAVE_NOTE: Note]\nNote body.\nClosing text.';
    const result = parseToolCalls(input);
    // Content = "Note body.\nClosing text." — all stripped with marker
    expect(result.cleaned).toBe('Intro text.');
    expect(result.pendingSaves).toHaveLength(1);
    expect(result.pendingSaves[0].content).toContain('Note body.');
    expect(result.pendingSaves[0].content).toContain('Closing text.');
  });

  it('handles empty input', () => {
    const result = parseToolCalls('');
    expect(result.cleaned).toBe('');
    expect(result.pendingSaves).toEqual([]);
  });
});

// ── buildSystemPrompt ─────────────────────────────────────

describe('buildSystemPrompt', () => {
  it('returns a non-empty string', () => {
    const prompt = buildSystemPrompt(null);
    expect(prompt.length).toBeGreaterThan(0);
  });

  it('includes security rules (language-independent check)', () => {
    const prompt = buildSystemPrompt(null);
    // Security rules contain "SAVE_NOTE" in note instructions
    expect(prompt).toContain('SAVE_NOTE');
    // Security rules mention system prompt confidentiality
    expect(prompt).toMatch(/系统提示词|system prompt/i);
  });

  it('includes note instructions with SAVE_NOTE format', () => {
    const prompt = buildSystemPrompt(null);
    expect(prompt).toContain('[SAVE_NOTE:');
  });

  it('includes note context when provided', () => {
    const context = '## My Note\nSome relevant content from the vault.';
    const prompt = buildSystemPrompt(context);
    expect(prompt).toContain(context);
    // Context section header varies by locale
    expect(prompt).toMatch(/知识库检索结果|Knowledge Base Results/);
  });

  it('does not include context section when noteContext is null', () => {
    const prompt = buildSystemPrompt(null);
    expect(prompt).not.toMatch(/知识库检索结果|Knowledge Base Results/);
  });

  it('does not include context section when noteContext is empty string', () => {
    const prompt = buildSystemPrompt('');
    // Empty string is falsy, should not add context
    expect(prompt).not.toMatch(/知识库检索结果|Knowledge Base Results/);
  });

  it('prompt structure: security rules come before note instructions', () => {
    const prompt = buildSystemPrompt(null);
    const securityIdx = prompt.search(/安全规则|Security Rules/);
    const noteIdx = prompt.search(/笔记能力|note abilities/i);
    expect(securityIdx).toBeLessThan(noteIdx);
  });

  it('context is injected between security rules and note instructions', () => {
    const prompt = buildSystemPrompt('test context');
    const contextIdx = prompt.indexOf('test context');
    const noteIdx = prompt.search(/笔记能力|note abilities/i);
    expect(contextIdx).toBeGreaterThan(0);
    expect(contextIdx).toBeLessThan(noteIdx);
  });
});
