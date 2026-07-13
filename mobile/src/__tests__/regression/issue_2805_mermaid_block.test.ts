/**
 * Regression test for #2805: Mermaid diagram support.
 *
 * Feature: MarkdownPreview now detects ```mermaid fenced code blocks
 * and renders them as a labelled diagram container instead of a plain
 * code block.
 *
 * Tests the parseMarkdownBlocks() function which mirrors the block-level
 * parsing logic from MarkdownPreview.tsx.
 */

import { parseMarkdownBlocks, MarkdownBlock } from '../../utils/markdownBlocks';

describe('parseMarkdownBlocks — mermaid detection (#2805)', () => {
  // ── Basic mermaid fence detection ──

  it('detects ```mermaid block as type "mermaid"', () => {
    const blocks = parseMarkdownBlocks([
      '```mermaid',
      'graph TD',
      '  A-->B',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('mermaid');
    expect(blocks[0].lang).toBe('mermaid');
    expect(blocks[0].content).toBe('graph TD\n  A-->B');
  });

  it('detects ```mermaid with leading whitespace on opening fence', () => {
    const blocks = parseMarkdownBlocks([
      '  ```mermaid',
      'flowchart LR',
      '  Start --> End',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('mermaid');
    expect(blocks[0].lang).toBe('mermaid');
    expect(blocks[0].content).toBe('flowchart LR\n  Start --> End');
  });

  // ── Non-mermaid code blocks stay as "code" ──

  it('does not treat ```rust as mermaid', () => {
    const blocks = parseMarkdownBlocks([
      '```rust',
      'fn main() {}',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('code');
    expect(blocks[0].lang).toBe('rust');
    expect(blocks[0].content).toBe('fn main() {}');
  });

  it('does not treat ``` (no lang) as mermaid', () => {
    const blocks = parseMarkdownBlocks([
      '```',
      'plain text',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('code');
    expect(blocks[0].lang).toBeUndefined();
    expect(blocks[0].content).toBe('plain text');
  });

  it('does not treat ```python as mermaid', () => {
    const blocks = parseMarkdownBlocks([
      '```python',
      'print("hello")',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('code');
    expect(blocks[0].lang).toBe('python');
  });

  // ── Mixed content: mermaid alongside other blocks ──

  it('correctly parses mermaid block within markdown paragraph content', () => {
    const blocks = parseMarkdownBlocks([
      'Here is a system flow:',
      '',
      '```mermaid',
      'sequenceDiagram',
      '  User->>App: Open',
      '  App->>Server: Request',
      '```',
      '',
      'The diagram above shows the sequence.',
    ].join('\n'));

    // Should have: paragraph, empty, mermaid, empty, paragraph
    expect(blocks).toHaveLength(5);
    expect(blocks[0].type).toBe('paragraph');
    expect(blocks[0].content).toBe('Here is a system flow:');
    expect(blocks[1].type).toBe('empty');
    expect(blocks[2].type).toBe('mermaid');
    expect(blocks[2].content).toBe('sequenceDiagram\n  User->>App: Open\n  App->>Server: Request');
    expect(blocks[3].type).toBe('empty');
    expect(blocks[4].type).toBe('paragraph');
    expect(blocks[4].content).toBe('The diagram above shows the sequence.');
  });

  it('handles multiple code blocks with different languages including mermaid', () => {
    const blocks = parseMarkdownBlocks([
      '```mermaid',
      'gantt',
      '  title Plan',
      '```',
      '',
      '```rust',
      'let x = 1;',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(3); // mermaid, empty, code
    expect(blocks[0].type).toBe('mermaid');
    expect(blocks[0].lang).toBe('mermaid');
    expect(blocks[0].content).toBe('gantt\n  title Plan');
    expect(blocks[2].type).toBe('code');
    expect(blocks[2].lang).toBe('rust');
    expect(blocks[2].content).toBe('let x = 1;');
  });

  // ── Edge cases ──

  it('handles empty mermaid block', () => {
    const blocks = parseMarkdownBlocks([
      '```mermaid',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('mermaid');
    expect(blocks[0].lang).toBe('mermaid');
    expect(blocks[0].content).toBe('');
  });

  it('handles mermaid with single content line', () => {
    const blocks = parseMarkdownBlocks([
      '```mermaid',
      'pie title Pets',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('mermaid');
    expect(blocks[0].content).toBe('pie title Pets');
  });

  // ── Unclosed mermaid block ──

  it('handles unclosed mermaid block (stream interrupted)', () => {
    const blocks = parseMarkdownBlocks([
      '```mermaid',
      'graph TD',
      '  A-->B',
      // no closing ```
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('mermaid');
    expect(blocks[0].lang).toBe('mermaid');
    expect(blocks[0].content).toBe('graph TD\n  A-->B');
  });

  it('handles unclosed non-mermaid code block', () => {
    const blocks = parseMarkdownBlocks([
      '```rust',
      'fn main() {}',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('code');
    expect(blocks[0].lang).toBe('rust');
  });

  // ── Fence with trailing spaces ──

  it('trims trailing spaces from language hint (```mermaid  )', () => {
    const blocks = parseMarkdownBlocks([
      '```mermaid   ',
      'graph TD',
      '  A-->B',
      '```',
    ].join('\n'));

    expect(blocks).toHaveLength(1);
    expect(blocks[0].type).toBe('mermaid');
    expect(blocks[0].lang).toBe('mermaid');
  });

  // ── Case sensitivity: mermaid is typically lowercase ──

  it('treats ```Mermaid (uppercase) as non-mermaid (exact match)', () => {
    // Design choice: exact lowercase 'mermaid' match only.
    // Common practice in markdown is lowercase language hints.
    const blocks = parseMarkdownBlocks([
      '```Mermaid',
      'graph TD',
      '  A-->B',
      '```',
    ].join('\n'));

    expect(blocks[0].type).toBe('code');
    expect(blocks[0].lang).toBe('Mermaid');
  });
});