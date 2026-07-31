/**
 * Regression test for #3073 — mobile share receive helpers
 * Tests pure functions for processing shared payloads.
 */

import {
  extractShareUrls,
  suggestShareTitle,
  extractShareText,
} from '../../utils/shareHelpers';
import type { ResolvedSharePayload } from 'expo-sharing';

function makePayload(overrides: Partial<ResolvedSharePayload> = {}): ResolvedSharePayload {
  return {
    value: '',
    shareType: 'text',
    mimeType: 'text/plain',
    contentUri: null,
    contentType: 'text',
    contentMimeType: 'text/plain',
    originalName: null,
    contentSize: null,
    ...overrides,
  } as ResolvedSharePayload;
}

describe('extractShareUrls', () => {
  it('returns URLs from URL-type payloads', () => {
    const payloads: ResolvedSharePayload[] = [
      makePayload({ shareType: 'url', value: 'https://example.com/article' }),
      makePayload({ shareType: 'text', value: 'hello world' }),
    ];
    const urls = extractShareUrls(payloads);
    expect(urls).toEqual(['https://example.com/article']);
  });

  it('returns empty array when no URL payloads', () => {
    const payloads: ResolvedSharePayload[] = [
      makePayload({ shareType: 'text', value: 'some text' }),
    ];
    expect(extractShareUrls(payloads)).toEqual([]);
  });

  it('handles empty input', () => {
    expect(extractShareUrls([])).toEqual([]);
  });

  it('filters out falsy values', () => {
    const payloads: ResolvedSharePayload[] = [
      makePayload({ shareType: 'url', value: '' }),
    ];
    expect(extractShareUrls(payloads)).toEqual([]);
  });

  it('extracts multiple URLs', () => {
    const payloads: ResolvedSharePayload[] = [
      makePayload({ shareType: 'url', value: 'https://a.com' }),
      makePayload({ shareType: 'url', value: 'https://b.com' }),
    ];
    expect(extractShareUrls(payloads)).toEqual(['https://a.com', 'https://b.com']);
  });
});

describe('suggestShareTitle', () => {
  it('suggests title from text payload', () => {
    expect(suggestShareTitle(makePayload({ shareType: 'text', value: 'Hello World' }))).toBe('Hello World');
  });

  it('truncates long text titles', () => {
    const long = 'a'.repeat(100);
    const title = suggestShareTitle(makePayload({ shareType: 'text', value: long }));
    expect(title.endsWith('...')).toBe(true);
    expect(title.length).toBe(50);
  });

  it('returns default when text is empty', () => {
    expect(suggestShareTitle(makePayload({ shareType: 'text', value: '' }))).toBe('分享笔记');
  });

  it('suggests title from URL payload', () => {
    const title = suggestShareTitle(
      makePayload({ shareType: 'url', value: 'https://github.com/ryanloee/VaultPilot' }),
    );
    expect(title).toContain('github.com');
  });

  it('falls back for invalid URL', () => {
    const title = suggestShareTitle(
      makePayload({ shareType: 'url', value: 'not-a-url' }),
    );
    expect(title).toBe('网页分享');
  });

  it('suggests title for image payload', () => {
    const title = suggestShareTitle(
      makePayload({ shareType: 'image', originalName: 'photo.jpg', value: '' }),
    );
    expect(title).toBe('图片: photo.jpg');
  });

  it('suggests title for file payload', () => {
    const title = suggestShareTitle(
      makePayload({ shareType: 'file', originalName: 'report.pdf' }),
    );
    expect(title).toBe('文件: report.pdf');
  });

  it('returns default for unknown share type', () => {
    expect(suggestShareTitle(makePayload({ shareType: 'video', value: 'something' }))).toBe('分享笔记');
  });
});

describe('extractShareText', () => {
  it('extracts value from text payload', () => {
    const result = extractShareText(makePayload({ shareType: 'text', value: 'hello' }));
    expect(result).toBe('hello');
  });

  it('extracts URL from url payload', () => {
    const result = extractShareText(makePayload({ shareType: 'url', value: 'https://example.com' }));
    expect(result).toBe('https://example.com');
  });

  it('formats image with markdown', () => {
    const result = extractShareText(makePayload({ shareType: 'image', originalName: 'photo.jpg' }));
    expect(result).toBe('![[photo-1.jpg]]');
  });

  it('falls back for image without name', () => {
    const result = extractShareText(makePayload({ shareType: 'image' }));
    expect(result).toBe('![[share-image-1.jpg]]');
  });

  it('formats file with emoji', () => {
    const result = extractShareText(makePayload({ shareType: 'file', originalName: 'doc.pdf' }));
    expect(result).toBe('📎 doc-1.pdf');
  });

  it('returns empty for empty text', () => {
    const result = extractShareText(makePayload({ shareType: 'text', value: '' }));
    expect(result).toBe('');
  });

  it('returns value for unknown share type', () => {
    const result = extractShareText(makePayload({ shareType: 'video' as any, value: 'test.mp4' }));
    expect(result).toBe('test.mp4');
  });
});

describe('extractShareText — actualFileName override (#3639, #3643)', () => {
  it('uses actualFileName for image embed when provided', () => {
    const p = makePayload({ shareType: 'image', originalName: null });
    const result = extractShareText(p, 0, 'saved-image.jpg');
    expect(result).toBe('![[saved-image.jpg]]');
  });

  it('actualFileName wins over deterministic naming', () => {
    const p = makePayload({ shareType: 'image', originalName: 'original.jpg' });
    const result = extractShareText(p, 0, 'original-2.jpg');
    expect(result).toBe('![[original-2.jpg]]');
  });

  it('uses actualFileName for file reference when provided', () => {
    const p = makePayload({ shareType: 'file', originalName: null });
    const result = extractShareText(p, 0, 'saved-file.bin');
    expect(result).toBe('📎 saved-file.bin');
  });

  it('falls back to deterministic resolveShareFileName when actualFileName omitted', () => {
    const p = makePayload({ shareType: 'image', originalName: 'photo.jpg' });
    // resolveShareFileName(p, 0) = 'photo-1.jpg'
    const result = extractShareText(p, 0);
    expect(result).toBe('![[photo-1.jpg]]');
  });

  it('falls back to deterministic fallback when neither actualFileName nor originalName', () => {
    const p = makePayload({ shareType: 'image', originalName: null });
    const result = extractShareText(p, 0);
    expect(result).toBe('![[share-image-1.jpg]]');
  });

  it('file type falls back to deterministic share-file-N', () => {
    const p = makePayload({ shareType: 'file', originalName: null });
    const result = extractShareText(p, 0);
    expect(result).toBe('📎 share-file-1');
  });

  it('text/url payloads ignore both index and actualFileName', () => {
    const textP = makePayload({ shareType: 'text', value: 'hello' });
    expect(extractShareText(textP, 0, 'ignored.txt')).toBe('hello');

    const urlP = makePayload({ shareType: 'url', value: 'https://x.com' });
    expect(extractShareText(urlP, 0, 'ignored.txt')).toBe('https://x.com');
  });
});