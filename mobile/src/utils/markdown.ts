/**
 * Inline markdown parser — pure logic, no React/RN dependencies.
 *
 * Extracted from MarkdownPreview.tsx for testability.
 * Handles: **bold**, *italic*, `code`, [link](url), [[wikilink]], [[note#^blockid]]
 */

/** Type of inline markdown element */
export type InlineElementType = 'text' | 'bold' | 'italic' | 'code' | 'link' | 'wikilink' | 'blockref';

/** Parsed inline element */
export interface InlineElement {
  type: InlineElementType;
  text: string;
  /** Only present for 'link', 'wikilink', 'blockref' type */
  url?: string;
  /** For blockref: the block anchor ID (e.g. '^abcdef12') */
  blockId?: string;
  /** For blockref: the note name/identifier */
  noteName?: string;
}

/**
 * Parse a line of text into inline markdown elements.
 * Supports: **bold**, *italic*, `code`, [text](url), [[wikilink]], [[note#^blockid]]
 *
 * Parsing order: code > wikilink/blockref > bold > italic > link > plain text.
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

    // Wikilink / Block reference [[note#^blockid]] or [[Note Name]] or [[Note Name|Display]]
    const wikilinkMatch = remaining.match(
      /^(.*?)\[\[([^\[\]]+?)(?:\|([^\[\]]*?))?\]\](.*)$/,
    );
    if (wikilinkMatch) {
      if (wikilinkMatch[1]) elements.push(...parsePlain(wikilinkMatch[1]));
      const raw = wikilinkMatch[2];
      const display = wikilinkMatch[3] || raw;
      const hashPos = raw.indexOf('#');
      const caretPos = raw.indexOf('#^');
      if (caretPos >= 0) {
        // Block reference: [[Note Name#^blockid]] or [[#^blockid]]
        const noteName = caretPos === 0 ? undefined : raw.slice(0, caretPos).trim();
        const blockId = raw.slice(caretPos + 2).trim();
        elements.push({
          type: 'blockref',
          text: display,
          noteName,
          blockId,
          url: raw,
        });
      } else if (hashPos >= 0) {
        // Heading anchor: [[Note Name#Heading]]
        const noteName = hashPos === 0 ? undefined : raw.slice(0, hashPos).trim();
        const heading = raw.slice(hashPos + 1).trim();
        elements.push({
          type: 'blockref',
          text: display,
          noteName,
          blockId: heading,
          url: raw,
        });
      } else {
        // Simple wikilink: [[Note Name]]
        elements.push({
          type: 'wikilink',
          text: display,
          url: raw,
        });
      }
      remaining = wikilinkMatch[4];
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
    const linkMatch = remaining.match(/^(.*?)\[([^\]]+)\]\(([^)]+)\)(.*)$/);
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

/**
 * Parse a block reference string into its components.
 */
export function parseBlockRef(raw: string): {
  noteName?: string;
  blockId?: string;
  display: string;
} {
  const hashPos = raw.indexOf('#');
  const caretPos = raw.indexOf('#^');
  if (caretPos >= 0) {
    return {
      noteName: caretPos === 0 ? undefined : raw.slice(0, caretPos).trim(),
      blockId: raw.slice(caretPos + 2).trim(),
      display: raw,
    };
  }
  if (hashPos >= 0) {
    return {
      noteName: hashPos === 0 ? undefined : raw.slice(0, hashPos).trim(),
      blockId: raw.slice(hashPos + 1).trim(),
      display: raw,
    };
  }
  return { noteName: raw.trim(), display: raw };
}
