/**
 * Regression test for #2983: mobile attachments were silently dropped from the
 * API request body. The API history was built with only `m.content` (plain
 * text), so images/files selected in ChatScreen were never transmitted to the
 * model.
 *
 * This test pins the contract that:
 *   1. `buildUserContent` turns base64 attachments into `image_url` content parts.
 *   2. `buildHistory` embeds that multimodal content as the final user message,
 *      so an attachment-bearing user turn is NOT reduced to plain text.
 */
import { buildUserContent, buildHistory, isTextFile } from '../../utils/chatHelpers';
import type { Msg } from '../../utils/chatHelpers';

describe('issue #2983 — attachments reach the API request body', () => {
  it('buildUserContent produces image_url parts for base64 attachments', () => {
    const content = buildUserContent('describe this', [
      { base64: 'aGVsbG8=', mime: 'image/png' },
    ]) as any[];

    expect(Array.isArray(content)).toBe(true);
    const imagePart = content.find((p) => p.type === 'image_url');
    expect(imagePart).toBeDefined();
    expect(imagePart.image_url.url).toBe('data:image/png;base64,aGVsbG8=');
    // Text must still be present as the first part.
    expect(content[0]).toEqual({ type: 'text', text: 'describe this' });
  });

  it('buildUserContent returns a plain string when there are no attachments', () => {
    expect(buildUserContent('just text', [])).toBe('just text');
  });

  it('buildHistory embeds multimodal user content into the request', () => {
    const prev: Msg[] = [
      { id: '1', role: 'user', content: 'earlier message' },
      { id: '2', role: 'assistant', content: 'earlier reply' },
    ];
    const multimodal = buildUserContent('what is this?', [
      { base64: 'aW1hZ2UtYnl0ZXM=', mime: 'image/jpeg' },
    ]);

    const history = buildHistory(prev, 'system prompt', multimodal);

    const userTurn = history[history.length - 1];
    expect(userTurn.role).toBe('user');
    // MUST NOT be collapsed to a plain string — the image part must survive.
    expect(Array.isArray(userTurn.content)).toBe(true);
    const parts = userTurn.content as any[];
    expect(parts.some((p) => p.type === 'image_url')).toBe(true);
  });

  // ── file (non-image) attachment paths ──

  it('isTextFile recognises text extensions', () => {
    expect(isTextFile('notes.txt')).toBe(true);
    expect(isTextFile('README.md')).toBe(true);
    expect(isTextFile('config.json')).toBe(true);
    expect(isTextFile('photo.jpg')).toBe(false);
    expect(isTextFile('report.pdf')).toBe(false);
    expect(isTextFile('noext')).toBe(false);
  });

  it('buildUserContent injects text file content as a text part', () => {
    const content = buildUserContent('summarise this', [
      {
        base64: '',
        mime: 'text/plain',
        textContent: 'Hello from a .txt file',
      },
    ]) as any[];

    expect(Array.isArray(content)).toBe(true);
    const textParts = content.filter((p) => p.type === 'text');
    expect(textParts.length).toBe(2); // user prompt + file content
    expect(textParts[1].text).toContain('Hello from a .txt file');
  });

  it('buildHistory embeds file-text content in the request', () => {
    const prev: Msg[] = [
      { id: '1', role: 'user', content: 'hi' },
    ];
    const multimodal = buildUserContent('review', [
      { base64: '', mime: 'text/markdown', textContent: '# Title\npara' },
    ]);
    const history = buildHistory(prev, 'system', multimodal);

    const userTurn = history[history.length - 1];
    expect(Array.isArray(userTurn.content)).toBe(true);
    const parts = userTurn.content as any[];
    const fileText = parts.find((p) => p.type === 'text' && (p.text as string).includes('# Title'));
    expect(fileText).toBeDefined();
  });
});
