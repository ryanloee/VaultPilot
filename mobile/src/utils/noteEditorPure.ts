/**
 * Pure functions extracted from NoteEditorScreen for testability.
 *
 * Issue #1430: NoteEditorScreen pure function extraction + unit tests.
 */

/** Result of applying a markdown format at a cursor position. */
export interface FormatResult {
  content: string;
  cursorPos: number;
}

/**
 * Apply markdown formatting syntax around the selected text.
 * Prefix syntaxes (ending with space, e.g. "# ") insert before selection.
 * Wrap syntaxes (e.g. "**") wrap the selection on both sides.
 */
export function applyFormat(content: string, selectionStart: number, selectionEnd: number, syntax: string): FormatResult {
  const before = content.slice(0, selectionStart);
  const selected = content.slice(selectionStart, selectionEnd);
  const after = content.slice(selectionEnd);
  const isPrefix = syntax.endsWith(' ');

  const next = isPrefix
    ? before + syntax + selected + after
    : before + syntax + selected + syntax + after;

  const cursorPos = isPrefix
    ? selectionStart + syntax.length + selected.length
    : selectionStart + syntax.length + selected.length + syntax.length;

  return { content: next, cursorPos };
}

/**
 * Build clipboard text from title and content.
 * Returns empty string if both are empty.
 */
export function buildClipboardText(title: string, content: string): string {
  if (!content) return '';
  return title ? `${title}\n\n${content}` : content;
}

/**
 * Build AI action prefill text with note content truncated to 2000 chars.
 * Returns empty string if no usable text.
 */
export function buildAiPrefill(prompt: string, content: string, title: string): string {
  const noteText = content || title || '';
  if (!noteText.trim()) return '';
  return `${prompt}\n\n${noteText.slice(0, 2000)}`;
}

/**
 * Determine if auto-tagging should be attempted.
 * Returns true when the note has no tags yet and has a non-empty title.
 */
export function shouldAutoTag(tags: string[], title: string): boolean {
  return tags.length === 0 && title.trim().length > 0;
}

/**
 * Parse and validate a new tag input.
 * Returns the cleaned tag string, or null if empty or duplicate.
 */
export function parseNewTag(newTag: string, existingTags: string[]): string | null {
  const tag = newTag.trim();
  if (!tag || existingTags.includes(tag)) return null;
  return tag;
}
