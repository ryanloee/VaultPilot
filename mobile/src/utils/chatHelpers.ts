/**
 * Pure logic helpers extracted from ChatScreen.tsx for testability (#1214).
 *
 * These functions have zero React/RN dependencies and can be unit-tested
 * in a plain Jest environment.
 */

import type { ChatMessage, ContentPart } from '../api/client';

/** Internal message representation used by ChatScreen */
export interface Msg {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  streaming?: boolean;
  isError?: boolean;
}

/** Max messages sent to API to avoid exceeding model context window */
export const MAX_HISTORY_MESSAGES = 50;

/**
 * Build the ChatMessage[] array sent to the API from the local message list.
 *
 * Filters out streaming placeholders and error messages, then takes the last
 * `maxMessages` entries (default MAX_HISTORY_MESSAGES).
 */
export function buildHistory(
  prevMsgs: Msg[],
  systemPrompt: string,
  userContent: string | ContentPart[],
  maxMessages: number = MAX_HISTORY_MESSAGES,
): ChatMessage[] {
  const filtered = prevMsgs
    .filter(m => (m.role !== 'assistant' || !m.streaming) && !m.isError)
    .slice(-maxMessages)
    .map(m => ({ role: m.role as 'user' | 'assistant', content: m.content }));

  return [
    { role: 'system', content: systemPrompt },
    ...filtered,
    { role: 'user', content: userContent },
  ];
}

/**
 * Build the content payload for a user message that may include attachments.
 *
 * If there are no attachments (or only text), returns a plain string.
 * Otherwise returns a ContentPart[] array with text + image_url parts.
 */
export function buildUserContent(
  text: string,
  attachments: { base64: string; mime: string }[],
): string | ContentPart[] {
  const parts: ContentPart[] = [];

  for (const att of attachments) {
    parts.push({ type: 'image_url', image_url: { url: `data:${att.mime};base64,${att.base64}` } });
  }

  if (text) {
    parts.unshift({ type: 'text', text });
  }

  // If there's only text (no image parts), return plain string for efficiency
  if (parts.length > 0 && parts.some(p => p.type !== 'text')) {
    return parts;
  }
  return text;
}

/**
 * Format the final message content after tool call processing.
 * Appends action summaries (e.g. "saved note X") as italic text.
 */
export function formatToolCallResult(cleaned: string, actions: string[]): string {
  if (actions.length === 0) return cleaned;
  return cleaned + '\n\n_' + actions.join('；') + '_';
}

/**
 * Build the preview text shown in the save-confirmation alert.
 * Truncates to 200 chars with ellipsis.
 */
export function buildSavePreview(content: string, maxLen: number = 200): string {
  if (content.length <= maxLen) return content;
  return content.slice(0, maxLen) + '...';
}

/**
 * Infer MIME type from file name/extension.
 * Returns the fallback if extension is unknown.
 */
export function inferMime(name: string, fallback: string): string {
  const ext = name.split('.').pop()?.toLowerCase();
  const map: Record<string, string> = {
    png: 'image/png', gif: 'image/gif', webp: 'image/webp', heic: 'image/heic',
    jpg: 'image/jpeg', jpeg: 'image/jpeg',
    pdf: 'application/pdf', doc: 'application/msword',
    txt: 'text/plain', md: 'text/markdown',
  };
  return ext && map[ext] ? map[ext] : fallback;
}
