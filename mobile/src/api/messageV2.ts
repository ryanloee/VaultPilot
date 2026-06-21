/**
 * MessageV2 — Unified cross-platform message schema (#1239).
 *
 * This is the canonical wire format shared by Rust, WinUI, and Mobile.
 * Keep in sync with the Rust definition in src/models.rs and the
 * shared test fixture at tests/fixtures/message_v2_fixtures.json.
 */

export type MessageV2Role = 'user' | 'assistant' | 'system';

export type MessageV2AttachmentType = 'image' | 'file';

export interface MessageV2Attachment {
  type: MessageV2AttachmentType;
  /** Resource locator. Must use the `local://` scheme. */
  url: string;
  /** MIME type (e.g. "image/png", "application/pdf"). */
  mime: string;
}

export interface MessageV2Metadata {
  /** Model that generated this message (empty for user messages). */
  model: string;
  /** Total token count for this message. */
  tokens: number;
  /** Provider-specific extra fields. */
  [key: string]: unknown;
}

export interface MessageV2 {
  id: string;
  role: MessageV2Role;
  content: string;
  attachments: MessageV2Attachment[];
  metadata: MessageV2Metadata;
  /** Reserved extension point for the plugin system. */
  extensions: Record<string, unknown>;
}

/** Create an empty MessageV2 with sensible defaults. */
export function createMessageV2(
  partial: Partial<MessageV2> & { content: string },
): MessageV2 {
  return {
    id: partial.id ?? '',
    role: partial.role ?? 'user',
    content: partial.content,
    attachments: partial.attachments ?? [],
    metadata: partial.metadata ?? { model: '', tokens: 0 },
    extensions: partial.extensions ?? {},
  };
}

/** Validate that all attachment URLs use the `local://` scheme. */
export function validateAttachmentUrls(msg: MessageV2): string[] {
  const errors: string[] = [];
  for (const att of msg.attachments) {
    if (!att.url.startsWith('local://')) {
      errors.push(`attachment url must use local:// scheme, got: ${att.url}`);
    }
  }
  return errors;
}
