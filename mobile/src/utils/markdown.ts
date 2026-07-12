/**
 * Inline markdown parser — pure logic, no React/RN dependencies.
 *
 * Extracted from MarkdownPreview.tsx for testability.
 * Handles: **bold**, *italic*, `code`, [link](url)
 */

/** Type of inline markdown element */
export type InlineElementType = 'text' | 'bold' | 'italic' | 'code' | 'link';

/** Parsed inline element */
export interface InlineElement {
  type: InlineElementType;
  text: string;
  /** Only present for 'link' type */
  url?: string;
}

/**
 * Parse a line of text into inline markdown elements.
 * Supports: **bold**, *italic*, `code`, [text](url)
 *
 * Parsing order: code > bold > italic > link > plain text.
 * This prevents `*` inside `**bold**` from being treated as italic.
 */
export function parseInline(text: string): InlineElement[] {
  const elements: InlineElement[] = [];
  let remaining = text;

  while (remaining) {
    // Inline code (highest priority — prevents formatting inside code)
    const codeMatch = remaining.match(/^(.*?)`([^`]+)`(.*)$/);
    if (codeMatch) {
      if (codeMatch[1]) elements.push(...parsePlain(codeMatch[1]));
      elements.push({ type: 'code', text: codeMatch[2] });
      remaining = codeMatch[3];
      continue;
    }

    // Bold **text**
    const boldMatch = remaining.match(/^(.*?)\*\*([^*]+)\*\*(.*)$/);
    if (boldMatch) {
      if (boldMatch[1]) elements.push(...parsePlain(boldMatch[1]));
      elements.push({ type: 'bold', text: boldMatch[2] });
      remaining = boldMatch[3];
      continue;
    }

    // Italic *text*
    const italicMatch = remaining.match(/^(.*?)\*([^*]+)\*(.*)$/);
    if (italicMatch) {
      if (italicMatch[1]) elements.push(...parsePlain(italicMatch[1]));
      elements.push({ type: 'italic', text: italicMatch[2] });
      remaining = italicMatch[3];
      continue;
    }

    // Link [text](url)
    // Handle URLs with balanced parentheses (e.g. Wikipedia disambiguation links)
    // Uses [^()] to stop at both parens, then (?:\([^()]*\)[^()]*)* for balanced single-level parens
    const linkMatch = remaining.match(/^(.*?)\[([^\]]+)\]\(([^()]*(?:\([^()]*\)[^()]*)*)\)(.*)$/);
    if (linkMatch) {
      if (linkMatch[1]) elements.push(...parsePlain(linkMatch[1]));
      elements.push({ type: 'link', text: linkMatch[2], url: linkMatch[3] });
      remaining = linkMatch[4];
      continue;
    }

    // Plain text — find the next special character
    const nextSpecial = remaining.search(/[*`\[]/);
    if (nextSpecial > 0) {
      elements.push({ type: 'text', text: remaining.slice(0, nextSpecial) });
      remaining = remaining.slice(nextSpecial);
    } else if (nextSpecial === -1) {
      elements.push({ type: 'text', text: remaining });
      remaining = '';
    } else {
      // nextSpecial === 0 means a special char we couldn't parse — emit it as text
      elements.push({ type: 'text', text: remaining[0] });
      remaining = remaining.slice(1);
    }
  }

  return elements;
}

/** Helper: return text elements for plain segments (no special parsing needed) */
function parsePlain(text: string): InlineElement[] {
  if (!text) return [];
  return [{ type: 'text', text }];
}
