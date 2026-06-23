/**
 * Regression tests for #1410: messageV2.ts + clientUtils.ts pure function unit tests.
 *
 * Covers createMessageV2, validateAttachmentUrls, isRetryable, sanitizeApiError, normalizeApiBase.
 */

import { createMessageV2, validateAttachmentUrls } from '../../api/messageV2';
import { isRetryable, sanitizeApiError, normalizeApiBase, DEFAULTS } from '../../api/clientUtils';

// ── messageV2.ts ─────────────────────────────────────────────

describe('createMessageV2', () => {
  test('creates message with required content only', () => {
    const msg = createMessageV2({ content: 'hello' });
    expect(msg.content).toBe('hello');
    expect(msg.id).toBe('');
    expect(msg.role).toBe('user');
    expect(msg.attachments).toEqual([]);
    expect(msg.metadata).toEqual({ model: '', tokens: 0 });
    expect(msg.extensions).toEqual({});
  });

  test('creates message with all partial fields', () => {
    const msg = createMessageV2({
      id: 'msg-1',
      role: 'assistant',
      content: 'response',
      attachments: [{ type: 'image', url: 'local://img.png', mime: 'image/png' }],
      metadata: { model: 'gpt-4', tokens: 100 },
      extensions: { foo: 'bar' },
    });
    expect(msg.id).toBe('msg-1');
    expect(msg.role).toBe('assistant');
    expect(msg.content).toBe('response');
    expect(msg.attachments).toHaveLength(1);
    expect(msg.metadata.model).toBe('gpt-4');
    expect(msg.extensions.foo).toBe('bar');
  });

  test('creates system role message', () => {
    const msg = createMessageV2({ content: 'system prompt', role: 'system' });
    expect(msg.role).toBe('system');
  });
});

describe('validateAttachmentUrls', () => {
  test('returns empty errors for valid local:// URLs', () => {
    const msg = createMessageV2({
      content: 'test',
      attachments: [
        { type: 'image', url: 'local://img.png', mime: 'image/png' },
        { type: 'file', url: 'local://doc.pdf', mime: 'application/pdf' },
      ],
    });
    expect(validateAttachmentUrls(msg)).toEqual([]);
  });

  test('returns error for non-local:// URL', () => {
    const msg = createMessageV2({
      content: 'test',
      attachments: [{ type: 'image', url: 'https://example.com/img.png', mime: 'image/png' }],
    });
    const errors = validateAttachmentUrls(msg);
    expect(errors).toHaveLength(1);
    expect(errors[0]).toContain('local://');
    expect(errors[0]).toContain('https://example.com/img.png');
  });

  test('returns empty errors for message with no attachments', () => {
    const msg = createMessageV2({ content: 'test' });
    expect(validateAttachmentUrls(msg)).toEqual([]);
  });

  test('returns multiple errors for multiple invalid URLs', () => {
    const msg = createMessageV2({
      content: 'test',
      attachments: [
        { type: 'image', url: 'http://a.com/1.png', mime: 'image/png' },
        { type: 'file', url: 'ftp://b.com/2.pdf', mime: 'application/pdf' },
      ],
    });
    expect(validateAttachmentUrls(msg)).toHaveLength(2);
  });
});

// ── clientUtils.ts ───────────────────────────────────────────

describe('isRetryable', () => {
  test('returns true for retryable status codes', () => {
    expect(isRetryable(429)).toBe(true);  // Rate limit
    expect(isRetryable(502)).toBe(true);  // Bad gateway
    expect(isRetryable(503)).toBe(true);  // Service unavailable
    expect(isRetryable(504)).toBe(true);  // Gateway timeout
  });

  test('returns false for non-retryable status codes', () => {
    expect(isRetryable(200)).toBe(false); // OK
    expect(isRetryable(400)).toBe(false); // Bad request
    expect(isRetryable(401)).toBe(false); // Unauthorized
    expect(isRetryable(403)).toBe(false); // Forbidden
    expect(isRetryable(404)).toBe(false); // Not found
    expect(isRetryable(500)).toBe(false); // Internal server error
  });
});

describe('sanitizeApiError', () => {
  test('returns friendly message for known status codes', () => {
    expect(sanitizeApiError(401)).toContain('API Key 无效');
    expect(sanitizeApiError(429)).toContain('请求过于频繁');
    expect(sanitizeApiError(500)).toContain('服务器内部错误');
    expect(sanitizeApiError(502)).toContain('服务暂时不可用');
  });

  test('returns generic message for unknown 4xx', () => {
    const msg = sanitizeApiError(418);
    expect(msg).toContain('418');
    expect(msg).toContain('请求有误');
  });

  test('returns generic message for unknown 5xx', () => {
    const msg = sanitizeApiError(599);
    expect(msg).toContain('599');
    expect(msg).toContain('服务端异常');
  });

  test('returns status-only message for other codes', () => {
    const msg = sanitizeApiError(200);
    expect(msg).toContain('200');
  });
});

describe('normalizeApiBase', () => {
  test('returns default for empty string', () => {
    expect(normalizeApiBase('')).toBe(DEFAULTS.apiBase);
  });

  test('returns default for whitespace-only', () => {
    expect(normalizeApiBase('   ')).toBe(DEFAULTS.apiBase);
  });

  test('trims trailing slashes', () => {
    expect(normalizeApiBase('https://api.example.com///')).toBe('https://api.example.com/v1');
  });

  test('trims whitespace', () => {
    expect(normalizeApiBase('  https://api.example.com  ')).toBe('https://api.example.com/v1');
  });

  test('preserves existing versioned path /v1', () => {
    expect(normalizeApiBase('https://api.example.com/v1')).toBe('https://api.example.com/v1');
  });

  test('preserves existing versioned path /v2', () => {
    expect(normalizeApiBase('https://api.example.com/v2')).toBe('https://api.example.com/v2');
  });

  test('preserves versioned path with suffix /v1-beta', () => {
    expect(normalizeApiBase('https://api.example.com/v1-beta')).toBe('https://api.example.com/v1-beta');
  });

  test('appends /v1 when no version path', () => {
    expect(normalizeApiBase('https://api.example.com')).toBe('https://api.example.com/v1');
  });
});
