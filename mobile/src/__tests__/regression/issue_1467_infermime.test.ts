/**
 * Regression test for #1467 — inferMime extracted from ChatScreen to chatHelpers.
 *
 * Verifies MIME type inference for common file extensions.
 */

import { inferMime } from '../../utils/chatHelpers';

describe('issue #1467 — inferMime pure function tests', () => {
  test('infers JPEG from .jpg', () => {
    expect(inferMime('photo.jpg', 'fallback')).toBe('image/jpeg');
  });

  test('infers JPEG from .jpeg', () => {
    expect(inferMime('photo.jpeg', 'fallback')).toBe('image/jpeg');
  });

  test('infers PNG from .png', () => {
    expect(inferMime('screenshot.png', 'fallback')).toBe('image/png');
  });

  test('infers GIF from .gif', () => {
    expect(inferMime('animation.gif', 'fallback')).toBe('image/gif');
  });

  test('infers WebP from .webp', () => {
    expect(inferMime('image.webp', 'fallback')).toBe('image/webp');
  });

  test('infers HEIC from .heic', () => {
    expect(inferMime('iphone-photo.heic', 'fallback')).toBe('image/heic');
  });

  test('infers AVIF from .avif', () => {
    expect(inferMime('photo.avif', 'fallback')).toBe('image/avif');
  });

  test('infers SVG from .svg', () => {
    expect(inferMime('icon.svg', 'fallback')).toBe('image/svg+xml');
  });

  test('infers TIFF from .tiff', () => {
    expect(inferMime('scan.tiff', 'fallback')).toBe('image/tiff');
  });

  test('infers TIFF from .tif', () => {
    expect(inferMime('scan.tif', 'fallback')).toBe('image/tiff');
  });

  test('infers PDF from .pdf', () => {
    expect(inferMime('document.pdf', 'fallback')).toBe('application/pdf');
  });

  test('infers plain text from .txt', () => {
    expect(inferMime('notes.txt', 'fallback')).toBe('text/plain');
  });

  test('infers markdown from .md', () => {
    expect(inferMime('readme.md', 'fallback')).toBe('text/markdown');
  });

  test('infers Word from .doc', () => {
    expect(inferMime('report.doc', 'fallback')).toBe('application/msword');
  });

  test('returns fallback for unknown extension', () => {
    expect(inferMime('file.xyz', 'application/octet-stream')).toBe('application/octet-stream');
  });

  test('returns fallback for no extension', () => {
    expect(inferMime('noextension', 'application/octet-stream')).toBe('application/octet-stream');
  });

  test('handles case-insensitive extensions', () => {
    expect(inferMime('PHOTO.JPG', 'fallback')).toBe('image/jpeg');
    expect(inferMime('doc.PDF', 'fallback')).toBe('application/pdf');
  });

  test('handles multiple dots in filename', () => {
    expect(inferMime('my.photo.2024.jpg', 'fallback')).toBe('image/jpeg');
  });

  test('handles empty filename', () => {
    expect(inferMime('', 'fallback')).toBe('fallback');
  });
});
