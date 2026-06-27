/**
 * Regression tests for issue #2131: normalizeAnthropicBase regex not anchored
 * to end of string.
 *
 * Bug: the old regex `/\/v\d+.*$/` matched the FIRST `/vN`-looking substring and
 * consumed everything after it (`.*$`). When the base URL's hostname or an
 * earlier path segment contained a `/vN`-looking substring, the URL was
 * truncated from that point, producing a malformed/incorrect URL and breaking
 * Anthropic requests.
 *
 *   "https://v2.proxy.com/v1"            → "https:"            (hostname eaten)
 *   "https://proxy.com/v2/anthropic/v1"  → "https://proxy.com" (mid-path eaten)
 *
 * Fix: anchor the regex to the end (`$`) so only a TRAILING version segment is
 * stripped. These tests cover the previously-broken cases plus regressions for
 * the normal stripping behaviour.
 */
import { normalizeAnthropicBase } from '../../api/clientPure';

describe('normalizeAnthropicBase — hostname/path containing /vN (#2131)', () => {
  it('preserves a hostname segment that starts with vN', () => {
    // After normalizeApiBase appends /v1, the result must not have its host eaten.
    expect(normalizeAnthropicBase('https://v2.proxy.com/v1')).toBe('https://v2.proxy.com');
    expect(normalizeAnthropicBase('https://v1.gateway.com/v1')).toBe('https://v1.gateway.com');
  });

  it('preserves a hostname that starts with vN even without a trailing version segment', () => {
    expect(normalizeAnthropicBase('https://v2.proxy.com')).toBe('https://v2.proxy.com');
  });

  it('preserves a mid-path /vN segment and only strips a trailing version', () => {
    expect(normalizeAnthropicBase('https://proxy.com/v2/anthropic/v1')).toBe('https://proxy.com/v2/anthropic');
  });

  it('leaves a mid-path /vN segment untouched when there is no trailing version', () => {
    expect(normalizeAnthropicBase('https://proxy.com/v2/anthropic')).toBe('https://proxy.com/v2/anthropic');
  });

  it('handles a path like /api/v1beta/claude with a trailing version', () => {
    expect(normalizeAnthropicBase('https://host.com/api/v1beta/claude/v1')).toBe('https://host.com/api/v1beta/claude');
  });
});

describe('normalizeAnthropicBase — normal stripping still works (#2131)', () => {
  it('strips a plain /v1 suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v1')).toBe('https://api.anthropic.com');
  });

  it('strips a plain /v2 suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v2')).toBe('https://api.anthropic.com');
  });

  it('strips a /v1-beta suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v1-beta')).toBe('https://api.anthropic.com');
  });

  it('strips a trailing slash after the version segment', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com/v1/')).toBe('https://api.anthropic.com');
  });

  it('preserves a path before a trailing version segment', () => {
    expect(normalizeAnthropicBase('https://proxy.example.com/anthropic/v1')).toBe('https://proxy.example.com/anthropic');
  });

  it('returns a URL unchanged when there is no version suffix', () => {
    expect(normalizeAnthropicBase('https://api.anthropic.com')).toBe('https://api.anthropic.com');
  });

  it('handles an empty string', () => {
    expect(normalizeAnthropicBase('')).toBe('');
  });
});
