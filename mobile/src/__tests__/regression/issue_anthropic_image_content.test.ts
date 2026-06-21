/**
 * Regression test: toAnthropicContent converts OpenAI image format to Anthropic format.
 *
 * Before this fix, chatWithReconnect passed raw OpenAI content parts to Anthropic API,
 * causing image attachments to be silently dropped or rejected.
 */
import { toAnthropicContent } from '../../api/client';

describe('toAnthropicContent — image format conversion', () => {
  test('plain string passes through unchanged', () => {
    expect(toAnthropicContent('hello world')).toBe('hello world');
  });

  test('text-only parts return as-is', () => {
    const parts = [{ type: 'text' as const, text: 'hello' }];
    const result = toAnthropicContent(parts);
    expect(result).toEqual([{ type: 'text', text: 'hello' }]);
  });

  test('converts data:image/jpeg base64 to Anthropic image block', () => {
    const parts = [
      { type: 'image_url' as const, image_url: { url: 'data:image/jpeg;base64,/9j/abc123' } },
    ];
    const result = toAnthropicContent(parts) as any[];
    expect(result).toHaveLength(1);
    expect(result[0]).toEqual({
      type: 'image',
      source: { type: 'base64', media_type: 'image/jpeg', data: '/9j/abc123' },
    });
  });

  test('converts data:image/png base64 to Anthropic image block', () => {
    const parts = [
      { type: 'image_url' as const, image_url: { url: 'data:image/png;base64,iVBORw0K' } },
    ];
    const result = toAnthropicContent(parts) as any[];
    expect(result[0].source.media_type).toBe('image/png');
    expect(result[0].source.data).toBe('iVBORw0K');
  });

  test('non-data image URL returns [image unavailable] text', () => {
    const parts = [
      { type: 'image_url' as const, image_url: { url: 'https://example.com/photo.jpg' } },
    ];
    const result = toAnthropicContent(parts) as any[];
    expect(result[0]).toEqual({ type: 'text', text: '[image unavailable]' });
  });

  test('mixed text and image parts are all converted', () => {
    const parts = [
      { type: 'text' as const, text: 'Look at this:' },
      { type: 'image_url' as const, image_url: { url: 'data:image/webp;base64,UklGR' } },
    ];
    const result = toAnthropicContent(parts) as any[];
    expect(result).toHaveLength(2);
    expect(result[0].type).toBe('text');
    expect(result[1].type).toBe('image');
    expect(result[1].source.media_type).toBe('image/webp');
  });

  test('handles multiple images', () => {
    const parts = [
      { type: 'image_url' as const, image_url: { url: 'data:image/jpeg;base64,aaa' } },
      { type: 'image_url' as const, image_url: { url: 'data:image/png;base64,bbb' } },
    ];
    const result = toAnthropicContent(parts) as any[];
    expect(result).toHaveLength(2);
    expect(result[0].source.media_type).toBe('image/jpeg');
    expect(result[1].source.media_type).toBe('image/png');
  });
});
