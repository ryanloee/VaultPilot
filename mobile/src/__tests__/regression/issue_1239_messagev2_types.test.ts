/**
 * Tests for MessageV2 mobile types (#1239).
 */
import {
  createMessageV2,
  validateAttachmentUrls,
  type MessageV2,
} from '../../api/messageV2';

describe('MessageV2', () => {
  describe('createMessageV2', () => {
    it('creates a message with defaults', () => {
      const msg = createMessageV2({ content: 'hello' });
      expect(msg.content).toBe('hello');
      expect(msg.role).toBe('user');
      expect(msg.id).toBe('');
      expect(msg.attachments).toEqual([]);
      expect(msg.extensions).toEqual({});
      expect(msg.metadata).toEqual({ model: '', tokens: 0 });
    });

    it('respects provided values', () => {
      const msg = createMessageV2({
        id: 'abc',
        role: 'assistant',
        content: 'hi',
        attachments: [{ type: 'image', url: 'local://img.png', mime: 'image/png' }],
        metadata: { model: 'gpt-4', tokens: 10 },
      });
      expect(msg.id).toBe('abc');
      expect(msg.role).toBe('assistant');
      expect(msg.attachments).toHaveLength(1);
      expect(msg.metadata.model).toBe('gpt-4');
    });

    it('handles system role', () => {
      const msg = createMessageV2({ content: 'system prompt', role: 'system' });
      expect(msg.role).toBe('system');
    });
  });

  describe('validateAttachmentUrls', () => {
    it('returns no errors for local:// urls', () => {
      const msg = createMessageV2({
        content: 'test',
        attachments: [
          { type: 'image', url: 'local://vault/img.png', mime: 'image/png' },
          { type: 'file', url: 'local://vault/doc.pdf', mime: 'application/pdf' },
        ],
      });
      expect(validateAttachmentUrls(msg)).toEqual([]);
    });

    it('returns errors for non-local:// urls', () => {
      const msg = createMessageV2({
        content: 'test',
        attachments: [
          { type: 'image', url: 'https://evil.com/img.png', mime: 'image/png' },
        ],
      });
      const errors = validateAttachmentUrls(msg);
      expect(errors).toHaveLength(1);
      expect(errors[0]).toContain('local://');
    });

    it('returns no errors for empty attachments', () => {
      const msg = createMessageV2({ content: 'test' });
      expect(validateAttachmentUrls(msg)).toEqual([]);
    });

    it('catches path traversal attempts', () => {
      const msg = createMessageV2({
        content: 'test',
        attachments: [
          { type: 'file', url: '/etc/passwd', mime: 'text/plain' },
        ],
      });
      const errors = validateAttachmentUrls(msg);
      expect(errors).toHaveLength(1);
    });
  });
});
