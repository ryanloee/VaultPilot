/**
 * Unit tests for api/clientUtils.ts — API utility functions.
 *
 * Tests: isRetryable, sanitizeApiError, normalizeApiBase.
 * All functions are pure with zero external dependencies.
 */

import { isRetryable, sanitizeApiError, normalizeApiBase, DEFAULTS } from '../../api/clientUtils';

// ── isRetryable ──────────────────────────────────────────

describe('isRetryable', () => {
  it('returns true for 429 (rate limit)', () => {
    expect(isRetryable(429)).toBe(true);
  });

  it('returns true for 502, 503, 504 (server errors)', () => {
    expect(isRetryable(502)).toBe(true);
    expect(isRetryable(503)).toBe(true);
    expect(isRetryable(504)).toBe(true);
  });

  it('returns false for non-retryable status codes', () => {
    expect(isRetryable(200)).toBe(false);
    expect(isRetryable(400)).toBe(false);
    expect(isRetryable(401)).toBe(false);
    expect(isRetryable(403)).toBe(false);
    expect(isRetryable(404)).toBe(false);
    expect(isRetryable(500)).toBe(false);
  });
});

// ── sanitizeApiError ─────────────────────────────────────

describe('sanitizeApiError', () => {
  it('returns friendly message for known status codes', () => {
    expect(sanitizeApiError(401)).toContain('API Key 无效');
    expect(sanitizeApiError(429)).toContain('请求过于频繁');
    expect(sanitizeApiError(500)).toContain('服务器内部错误');
    expect(sanitizeApiError(502)).toContain('服务暂时不可用');
    expect(sanitizeApiError(503)).toContain('请稍后重试');
    expect(sanitizeApiError(504)).toContain('服务响应超时');
  });

  it('returns generic server error for unknown 5xx', () => {
    expect(sanitizeApiError(501)).toContain('服务端异常');
    expect(sanitizeApiError(599)).toContain('服务端异常');
  });

  it('returns generic client error for unknown 4xx', () => {
    expect(sanitizeApiError(418)).toContain('请求有误');
    expect(sanitizeApiError(451)).toContain('请求有误');
  });

  it('returns status-only for other codes', () => {
    expect(sanitizeApiError(200)).toBe('API 错误 (200)');
    expect(sanitizeApiError(301)).toBe('API 错误 (301)');
  });

  it('includes the status code in all messages', () => {
    for (const code of [400, 401, 403, 404, 408, 429, 500, 502, 503, 504]) {
      expect(sanitizeApiError(code)).toContain(String(code));
    }
  });
});

// ── normalizeApiBase ─────────────────────────────────────

describe('normalizeApiBase', () => {
  it('returns default for empty string', () => {
    expect(normalizeApiBase('')).toBe(DEFAULTS.apiBase);
  });

  it('returns default for whitespace-only', () => {
    expect(normalizeApiBase('   ')).toBe(DEFAULTS.apiBase);
  });

  it('appends /v1 to bare URL', () => {
    expect(normalizeApiBase('https://api.openai.com')).toBe('https://api.openai.com/v1');
  });

  it('preserves URL already ending in /v1', () => {
    expect(normalizeApiBase('https://api.openai.com/v1')).toBe('https://api.openai.com/v1');
  });

  it('preserves URL with /v2', () => {
    expect(normalizeApiBase('https://api.openai.com/v2')).toBe('https://api.openai.com/v2');
  });

  it('preserves URL with /v1-beta', () => {
    expect(normalizeApiBase('https://api.openai.com/v1-beta')).toBe('https://api.openai.com/v1-beta');
  });

  it('preserves URL with versioned path and subpath', () => {
    expect(normalizeApiBase('https://api.anthropic.com/v1/messages')).toBe('https://api.anthropic.com/v1/messages');
  });

  it('strips trailing slashes', () => {
    expect(normalizeApiBase('https://api.openai.com/')).toBe('https://api.openai.com/v1');
    expect(normalizeApiBase('https://api.openai.com///')).toBe('https://api.openai.com/v1');
  });

  it('trims whitespace', () => {
    expect(normalizeApiBase('  https://api.openai.com  ')).toBe('https://api.openai.com/v1');
  });

  it('handles localhost URLs', () => {
    expect(normalizeApiBase('http://localhost:8080')).toBe('http://localhost:8080/v1');
    expect(normalizeApiBase('http://localhost:8080/v1')).toBe('http://localhost:8080/v1');
  });

  it('does not add /v1 to URLs with non-version paths', () => {
    expect(normalizeApiBase('https://api.openai.com/chat')).toBe('https://api.openai.com/chat/v1');
  });

  it('preserves URLs with version and trailing content', () => {
    expect(normalizeApiBase('https://openrouter.ai/api/v1')).toBe('https://openrouter.ai/api/v1');
  });
});
