/**
 * Share intent helpers — pure functions for processing shared payloads.
 *
 * These are extracted from ShareReceiveScreen to keep them testable without
 * React Native module resolution in Jest.
 *
 * @feature #3073
 */
import type { ResolvedSharePayload } from 'expo-sharing';

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

/** Extract text content from a share payload based on shareType. */
export function extractShareText(p: ResolvedSharePayload): string {
  switch (p.shareType) {
    case 'text':
      return p.value ?? '';
    case 'url':
      return p.value ?? '';
    case 'image': {
      const name = p.originalName ?? 'shared-image';
      return `![[${name}]]`;
    }
    case 'file': {
      const name = p.originalName ?? 'shared-file';
      return `📎 ${name}`;
    }
    default:
      return p.value ?? '';
  }
}