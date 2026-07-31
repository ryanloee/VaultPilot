/**
 * Share intent helpers — pure functions for processing shared payloads.
 *
 * These are extracted from ShareReceiveScreen to keep them testable without
 * React Native module resolution in Jest.
 *
 * @feature #3073
 */
import type { ResolvedSharePayload } from 'expo-sharing';

/** Sanitize a shared filename — strip path traversal and return a safe base name. (#3643) */
export function sanitizeShareFileName(name: string | null | undefined): string {
  if (!name) return '';
  // Take only the basename — strip all directory components to prevent path traversal.
  const parts = name.split(/[/\\]/);
  const baseName = parts[parts.length - 1] ?? '';
  // Strip leading dots to prevent hidden-file / relative-path abuse.
  return baseName.replace(/^\.+/, '');
}

/**
 * Resolve a unique, safe vault filename for a shared payload.
 *
 * Must produce identical results when called from extractShareText (embed) and
 * copyToVault (file write) for the same (payload, index) pair so that
 * Obsidian-style `![[name]]` embeds always resolve to the saved file. (#3643)
 *
 * @param index - zero-based index of this payload within the full share batch.
 *                Appended as a suffix to guarantee uniqueness.
 */
export function resolveShareFileName(
  p: ResolvedSharePayload,
  index: number,
): string {
  const sanitized = sanitizeShareFileName(p.originalName);
  if (sanitized) {
    // Append index suffix to prevent collision in multi-file shares.
    // Insert before the extension: "photo.jpg" → "photo-1.jpg"
    const dotIdx = sanitized.lastIndexOf('.');
    if (dotIdx > 0) {
      return `${sanitized.slice(0, dotIdx)}-${index + 1}${sanitized.slice(dotIdx)}`;
    }
    return `${sanitized}-${index + 1}`;
  }
  // Deterministic fallback — no Date.now() so embed and copy stay consistent.
  const ext = p.shareType === 'image' ? '.jpg' : '';
  return `share-${p.shareType}-${index + 1}${ext}`;
}

/** Extract share URLs from resolved payloads for template {{share_url}} usage. */
export function extractShareUrls(payloads: ResolvedSharePayload[]): string[] {
  return payloads
    .filter((p) => p.shareType === 'url')
    .map((p) => p.value)
    .filter(Boolean);
}

/** Suggest a note title from a share payload. */
export function suggestShareTitle(payload: ResolvedSharePayload): string {
  if (payload.shareType === 'text') {
    const t = (payload.value ?? '').trim();
    if (!t) return '分享笔记';
    return t.length > 50 ? t.slice(0, 47) + '...' : t;
  }
  if (payload.shareType === 'url') {
    try {
      const u = new URL(payload.value);
      return u.hostname + (u.pathname.length > 1 ? u.pathname.slice(0, 20) : '');
    } catch {
      return '网页分享';
    }
  }
  if (payload.shareType === 'image') {
    return `图片: ${payload.originalName ?? '分享图片'}`;
  }
  if (payload.shareType === 'file') {
    return `文件: ${payload.originalName ?? '分享文件'}`;
  }
  return '分享笔记';
}

/**
 * Extract text content from a share payload based on shareType.
 *
 * Uses deterministic `resolveShareFileName(p, index)` for image/file embeds so
 * that `![[name]]` always matches the file written by `copyToVault` (#3643).
 *
 * @param index - zero-based index within the share batch (for unique filename).
 * @param actualFileName - If provided, overrides the deterministic name. Used
 *   by handleSave when copyToVault has already resolved the filenames (#3639).
 */
export function extractShareText(
  p: ResolvedSharePayload,
  index = 0,
  actualFileName?: string,
): string {
  switch (p.shareType) {
    case 'text':
      return p.value ?? '';
    case 'url':
      return p.value ?? '';
    case 'image': {
      const name = actualFileName ?? resolveShareFileName(p, index);
      return `![[${name}]]`;
    }
    case 'file': {
      const name = actualFileName ?? resolveShareFileName(p, index);
      return `📎 ${name}`;
    }
    default:
      return p.value ?? '';
  }
}