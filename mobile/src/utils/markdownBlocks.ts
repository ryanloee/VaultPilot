/**
 * Parse markdown content into structured blocks for testing.
 *
 * Extracted from MarkdownPreview.tsx for testability (#2805).
 *
 * Supported block types:
 * - 'paragraph': normal text
 * - 'code': fenced code block (non-mermaid languages)
 * - 'mermaid': fenced code block with ```mermaid language hint
 * - 'heading': #-style heading
 * - 'hr': horizontal rule
 * - 'list': unordered list item
 * - 'olist': ordered list item
 * - 'empty': blank line
 */
export type BlockType = 'paragraph' | 'code' | 'mermaid' | 'heading' | 'hr' | 'list' | 'olist' | 'empty';

export interface MarkdownBlock {
  type: BlockType;
  content: string;
  /** Language hint extracted from ``` fence (e.g., 'rust', 'python', 'mermaid') */
  lang?: string;
  /** Heading level (1-3) */
  level?: number;
}

/**
 * Parse markdown content into an array of structured blocks.
 *
 * This mirrors the block-level parsing logic in MarkdownPreview.tsx
 * but returns structured data instead of React nodes, making it
 * testable without @testing-library/react-native.
 */
export function parseMarkdownBlocks(content: string): MarkdownBlock[] {
  const lines = content.split('\n');
  const blocks: MarkdownBlock[] = [];
  let inCodeBlock = false;
  let codeLang: string | null = null;
  let codeLines: string[] = [];

  for (const line of lines) {
    // Fenced code block
    if (line.trimStart().startsWith('```')) {
      if (inCodeBlock) {
        const joined = codeLines.join('\n');
        const isMermaid = codeLang === 'mermaid';
        blocks.push({
          type: isMermaid ? 'mermaid' : 'code',
          content: joined,
          lang: codeLang || undefined,
        });
        codeLines = [];
        codeLang = null;
        inCodeBlock = false;
      } else {
        inCodeBlock = true;
        const langHint = line.trimStart().slice(3).trim();
        codeLang = langHint || null;
      }
      continue;
    }

    if (inCodeBlock) {
      codeLines.push(line);
      continue;
    }

    // Heading
    const headingMatch = line.match(/^(#{1,3})\s+(.*)/);
    if (headingMatch) {
      const level = headingMatch[1].length;
      blocks.push({ type: 'heading', content: headingMatch[2], level });
      continue;
    }

    // Horizontal rule
    if (/^(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      blocks.push({ type: 'hr', content: '' });
      continue;
    }

    // Unordered list
    const listMatch = line.match(/^(\s*)[-*+]\s+(.*)/);
    if (listMatch) {
      blocks.push({ type: 'list', content: listMatch[2] });
      continue;
    }

    // Ordered list
    const olMatch = line.match(/^(\s*)\d+\.\s+(.*)/);
    if (olMatch) {
      blocks.push({ type: 'olist', content: olMatch[2] });
      continue;
    }

    // Empty line
    if (!line.trim()) {
      blocks.push({ type: 'empty', content: '' });
      continue;
    }

    // Normal paragraph
    blocks.push({ type: 'paragraph', content: line });
  }

  // Handle unclosed code block
  if (inCodeBlock && codeLines.length > 0) {
    const joined = codeLines.join('\n');
    const isMermaid = codeLang === 'mermaid';
    blocks.push({
      type: isMermaid ? 'mermaid' : 'code',
      content: joined,
      lang: codeLang || undefined,
    });
  }

  return blocks;
}