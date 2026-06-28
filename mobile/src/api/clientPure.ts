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
  let parsed: Record<string, any>;
  try {
    parsed = JSON.parse(data);
  } catch {
    /* skip unparseable SSE line — expected for binary/data frames */
    return null;
  }

  // Handle Anthropic error events (overloaded_error, rate_limit_error, etc.)
  // Match Rust client behavior at src/ai/client.rs:625-636
  if (parsed.type === 'error') {
    const errorType = parsed.error?.type || 'unknown';
    const errorMessage = parsed.error?.message || 'Anthropic API error';
    throw new Error(`Anthropic API error (${errorType}): ${errorMessage}`);
  }

  if (parsed.type === 'content_block_delta' && parsed.delta?.text) {
    const openai = JSON.stringify({ choices: [{ delta: { content: parsed.delta.text } }] });
    return `data: ${openai}\n\n`;
  }
  if (parsed.type === 'message_stop') {
    return 'data: [DONE]\n\n';
  }
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
 * Strip a trailing version segment (e.g. /v1, /v2, /v1-beta) from an Anthropic
 * base URL so the caller can safely append /v1/messages without producing a
 * doubled path (…/v1/v1/messages).
 *
 * The regex is anchored to the END of the string: it only removes a version
 * segment that is the final path component. This prevents a host or an earlier
 * path segment that merely *contains* a /vN-looking substring — e.g.
 * `https://v2.proxy.com` or `https://proxy.com/v2/anthropic` — from being
 * truncated, which previously produced a malformed URL and broke Anthropic
 * requests. See #2131.
 */
export function normalizeAnthropicBase(apiBase: string): string {
  return apiBase.replace(/\/v\d+(?:[-\w]*)?\/?$/, '');
}
