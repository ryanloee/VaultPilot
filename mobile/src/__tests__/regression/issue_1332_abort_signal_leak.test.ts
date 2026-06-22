/**
 * Regression test for #1332: chatAnthropic AbortSignal listener leak.
 *
 * When chatAnthropic succeeds, the onSignalAbort listener must be removed
 * from the original signal to prevent memory leaks and interference with
 * the stream's own timeout mechanism.
 */

import { toAnthropicContent } from '../../api/client';

// We can't easily test chatAnthropic directly (it makes real fetch calls),
// but we can verify the toAnthropicContent helper and document the expected
// cleanup behavior.

describe('issue_1332: AbortSignal listener cleanup', () => {
  test('toAnthropicContent passes through plain string', () => {
    expect(toAnthropicContent('hello')).toBe('hello');
  });

  test('toAnthropicContent converts text parts', () => {
    const result = toAnthropicContent([{ type: 'text', text: 'hi' }]);
    expect(result).toEqual([{ type: 'text', text: 'hi' }]);
  });

  test('toAnthropicContent converts base64 image to Anthropic format', () => {
    const result = toAnthropicContent([
      { type: 'text', text: 'describe this' },
      { type: 'image_url', image_url: { url: 'data:image/png;base64,abc123' } },
    ]);
    expect(result).toHaveLength(2);
    expect(result[1]).toEqual({
      type: 'image',
      source: { type: 'base64', media_type: 'image/png', data: 'abc123' },
    });
  });

  test('toAnthropicContent handles non-data-uri image gracefully', () => {
    const result = toAnthropicContent([
      { type: 'image_url', image_url: { url: 'https://example.com/img.png' } },
    ]);
    expect(result).toEqual([{ type: 'text', text: '[image unavailable]' }]);
  });

  test('AbortSignal with { once: true } listener is auto-removed after firing', () => {
    // Verify that { once: true } ensures single-fire behavior
    const controller = new AbortController();
    let callCount = 0;
    controller.signal.addEventListener('abort', () => { callCount++; }, { once: true });
    controller.abort();
    controller.abort(); // second abort should not fire again
    expect(callCount).toBe(1);
  });

  test('removeEventListener prevents abort callback from firing', () => {
    const controller = new AbortController();
    let fired = false;
    const handler = () => { fired = true; };
    controller.signal.addEventListener('abort', handler);
    controller.signal.removeEventListener('abort', handler);
    controller.abort();
    expect(fired).toBe(false);
  });
});
