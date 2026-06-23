/**
 * Pure utility functions extracted from client.ts for testability.
 * These handle Anthropic ↔ OpenAI format conversion.
 *
 * Zero React/RN/IO dependencies — can be unit-tested in plain Jest.
 */

import type { ContentPart } from './client';

// ── Anthropic content conversion ─────────────────────────────

/** Convert OpenAI-style content parts to Anthropic format. */
export function toAnthropicContent(content: string | ContentPart[]): string | Record<string, unknown>[] {
  if (typeof content === 'string') return content;
  return content.map(p => {
    if (p.type === 'text') return p;
    const match = p.image_url.url.match(/^data:(image\/\w+);base64,(.+)$/);
    if (match) return { type: 'image', source: { type: 'base64', media_type: match[1], data: match[2] } };
    return { type: 'text', text: '[image unavailable]' };
  });
}

// ── Anthropic SSE → OpenAI SSE conversion ────────────────────

export interface AnthropicSSEEvent {
  event: string;
  data: string;
}

/**
 * Convert a single Anthropic SSE event into OpenAI-compatible SSE data lines.
 * Returns null if the event should be ignored.
 * Returns '[DONE]' sentinel for message_stop.
 */
export function convertAnthropicEvent(event: string, data: string): string | null {
  try {
    const parsed = JSON.parse(data);
    if (parsed.type === 'content_block_delta' && parsed.delta?.text) {
      const openai = JSON.stringify({ choices: [{ delta: { content: parsed.delta.text } }] });
      return `data: ${openai}\n\n`;
    }
    if (parsed.type === 'message_stop') {
      return 'data: [DONE]\n\n';
    }
  } catch { /* skip unparseable SSE line — expected for binary/data frames */ }
  return null;
}

// ── Non-streaming response wrapping ──────────────────────────

interface NonStreamingChoice {
  message?: {
    content?: string;
    tool_calls?: unknown;
    function_call?: unknown;
  };
  finish_reason?: string;
}

interface NonStreamingResponse {
  choices?: NonStreamingChoice[];
}

/**
 * Wrap a non-streaming JSON response into an OpenAI-compatible SSE stream payload.
 * Returns the encoded bytes that would be enqueued into a ReadableStream.
 */
export function wrapNonStreamingResponse(json: NonStreamingResponse): Uint8Array {
  const message = json.choices?.[0]?.message ?? {};
  const delta: Record<string, unknown> = {};
  if (message.content) delta.content = message.content;
  if (message.tool_calls) delta.tool_calls = message.tool_calls;
  if (message.function_call) delta.function_call = message.function_call;
  const finish_reason = json.choices?.[0]?.finish_reason;
  const encoded = new TextEncoder().encode(
    `data: ${JSON.stringify({ choices: [{ delta, finish_reason }] })}\n\ndata: [DONE]\n\n`
  );
  return encoded;
}

// ── Content text extraction ─────────────────────────────────

/**
 * Extract plain text from content that may be a string or ContentPart[].
 * Returns the concatenated text portions; ignores image_url parts.
 * Fixes #1396: .join('\n') on ContentPart[] produces [object Object].
 */
export function extractTextContent(content: string | ContentPart[]): string {
  if (typeof content === 'string') return content;
  return content
    .filter((p): p is { type: 'text'; text: string } => p.type === 'text')
    .map(p => p.text)
    .join('\n');
}

// ── Anthropic base URL normalization ─────────────────────────

/**
 * Strip /v1 suffix from Anthropic base URL to avoid double /v1 path
 * when appending /v1/messages.
 */
export function normalizeAnthropicBase(apiBase: string): string {
  return apiBase.replace(/\/v\d+.*$/, '');
}
