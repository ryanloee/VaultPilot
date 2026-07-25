/**
 * Image Lightbox feature tests (#3030, #3422).
 *
 * Tests pure utility functions for image markdown parsing and gesture logic.
 * No React Native dependency — fully unit-testable.
 */
import {
  extractImagesFromLine,
  extractImagesFromContent,
  extractStandaloneImages,
  isStandaloneImageLine,
  shouldDismissOnSwipe,
  nextZoomOnDoubleTap,
  zoomPercentage,
  clampZoom,
  nextImageIndex,
  clamp,
  SWIPE_DISMISS_THRESHOLD,
  MIN_ZOOM,
  MAX_ZOOM,
  ZOOM_STEP,
} from '../../utils/imageMarkdown';

describe('imageMarkdown — image parsing (#3030)', () => {
  // ── extractImagesFromLine ──

  it('extracts a single ![alt](url) image', () => {
    const result = extractImagesFromLine('![screenshot](https://example.com/screenshot.png)');
    expect(result).toHaveLength(1);
    expect(result[0].alt).toBe('screenshot');
    expect(result[0].uri).toBe('https://example.com/screenshot.png');
  });

  it('extracts image with title attribute', () => {
    const result = extractImagesFromLine('![logo](https://example.com/logo.png "My Logo")');
    expect(result).toHaveLength(1);
    expect(result[0].alt).toBe('logo');
    expect(result[0].uri).toBe('https://example.com/logo.png');
    expect(result[0].title).toBe('My Logo');
  });

  it('extracts empty alt text', () => {
    const result = extractImagesFromLine('![](https://example.com/no-alt.png)');
    expect(result).toHaveLength(1);
    expect(result[0].alt).toBe('');
    expect(result[0].uri).toBe('https://example.com/no-alt.png');
  });

  it('extracts multiple images from same line', () => {
    const line = '![first](https://example.com/a.png) ![second](https://example.com/b.png)';
    const result = extractImagesFromLine(line);
    expect(result).toHaveLength(2);
    expect(result[0].alt).toBe('first');
    expect(result[1].alt).toBe('second');
  });

  it('extracts image embedded in text', () => {
    const line = 'Here is a photo: ![photo](https://example.com/p.jpg) nice!';
    const result = extractImagesFromLine(line);
    expect(result).toHaveLength(1);
    expect(result[0].alt).toBe('photo');
  });

  it('returns empty for non-image text', () => {
    expect(extractImagesFromLine('just text')).toEqual([]);
    expect(extractImagesFromLine('[link](url)')).toEqual([]);
    expect(extractImagesFromLine('!notanimage')).toEqual([]);
  });

  it('handles data URIs', () => {
    const line = '![img](data:image/png;base64,iVBOR==)';
    const result = extractImagesFromLine(line);
    expect(result).toHaveLength(1);
    expect(result[0].uri).toBe('data:image/png;base64,iVBOR==');
  });

  // ── extractImagesFromContent ──

  it('extracts images from multi-line content', () => {
    const content = [
      '# Title',
      '',
      '![first](https://example.com/a.png)',
      'Some text',
      '![second](https://example.com/b.png)',
    ].join('\n');
    const result = extractImagesFromContent(content);
    expect(result).toHaveLength(2);
    expect(result[0].alt).toBe('first');
    expect(result[1].alt).toBe('second');
  });

  it('ignores images inside code blocks', () => {
    const content = [
      '![real](https://example.com/real.png)',
      '```',
      '![fake](https://example.com/fake.png)',
      '```',
      '![real2](https://example.com/real2.png)',
    ].join('\n');
    const result = extractImagesFromContent(content);
    expect(result).toHaveLength(2);
    expect(result[0].alt).toBe('real');
    expect(result[1].alt).toBe('real2');
  });

  it('handles empty content', () => {
    expect(extractImagesFromContent('')).toEqual([]);
  });

  it('handles content with no images', () => {
    expect(extractImagesFromContent('# Title\n\nSome text')).toEqual([]);
  });

  // ── isStandaloneImageLine ──

  it('returns true for line with only an image', () => {
    expect(isStandaloneImageLine('![alt](url)')).toBe(true);
    expect(isStandaloneImageLine('  ![alt](url)  ')).toBe(true);
  });

  it('returns true for line with multiple images only', () => {
    expect(isStandaloneImageLine('![a](url1) ![b](url2)')).toBe(true);
  });

  it('returns false for image mixed with text', () => {
    expect(isStandaloneImageLine('Look: ![alt](url)')).toBe(false);
  });

  it('returns false for empty line', () => {
    expect(isStandaloneImageLine('')).toBe(false);
    expect(isStandaloneImageLine('   ')).toBe(false);
  });

  it('returns false for non-image content', () => {
    expect(isStandaloneImageLine('just text')).toBe(false);
  });
});

describe('extractStandaloneImages — Lightbox index alignment (#3454)', () => {
  // ── Core regression: inline images must not pollute the Lightbox list ──

  it('excludes inline images, keeping indices aligned with imageCounter', () => {
    // Repro from #3454: a note with an inline image followed by a
    // standalone image. extractImagesFromContent would return both
    // (indices 0, 1), but only the standalone image gets a globalIdx
    // (0), so tapping it would open allImages[0] = the inline image.
    const content = [
      'Hello world ![emoji](https://e.com/emoji.png)',
      '',
      '![screenshot](https://e.com/screenshot.png)',
    ].join('\n');

    const allImages = extractImagesFromContent(content);
    expect(allImages).toHaveLength(2); // both inline + standalone

    const standalone = extractStandaloneImages(content);
    expect(standalone).toHaveLength(1); // only the standalone image
    expect(standalone[0].alt).toBe('screenshot');
    // Index 0 now correctly maps to the screenshot — the only tappable image
  });

  it('returns all images when every image is on its own line', () => {
    const content = [
      '# Title',
      '',
      '![first](https://e.com/a.png)',
      '![second](https://e.com/b.png)',
    ].join('\n');

    expect(extractStandaloneImages(content)).toHaveLength(2);
    expect(extractStandaloneImages(content)[0].alt).toBe('first');
    expect(extractStandaloneImages(content)[1].alt).toBe('second');
  });

  it('handles multiple standalone images on the same line', () => {
    const content = '![a](u1) ![b](u2)';
    const result = extractStandaloneImages(content);
    expect(result).toHaveLength(2);
    expect(result[0].alt).toBe('a');
    expect(result[1].alt).toBe('b');
  });

  it('skips images inside fenced code blocks', () => {
    const content = [
      '![real](https://e.com/real.png)',
      '```',
      '![fake](https://e.com/fake.png)',
      '```',
    ].join('\n');

    expect(extractStandaloneImages(content)).toHaveLength(1);
    expect(extractStandaloneImages(content)[0].alt).toBe('real');
  });

  it('returns empty for content with only inline images', () => {
    const content = 'Text ![inline](u.png) more text';
    expect(extractStandaloneImages(content)).toEqual([]);
  });

  it('returns empty for content with no images', () => {
    expect(extractStandaloneImages('# Title\n\nJust text')).toEqual([]);
    expect(extractStandaloneImages('')).toEqual([]);
  });

  it('handles mixed inline + standalone across multiple paragraphs', () => {
    const content = [
      'See ![inline1](u1.png) here.',
      '',
      '![standalone1](u2.png)',
      '',
      'Another ![inline2](u3.png) mid-sentence.',
      '',
      '![standalone2](u4.png)',
    ].join('\n');

    const result = extractStandaloneImages(content);
    expect(result).toHaveLength(2);
    expect(result[0].alt).toBe('standalone1');
    expect(result[1].alt).toBe('standalone2');
  });

  it('indices match the order images are rendered (top-to-bottom)', () => {
    const content = [
      '![top](u-top.png)',
      'inline ![skip](u-skip.png) text',
      '![bottom](u-bottom.png)',
    ].join('\n');

    const result = extractStandaloneImages(content);
    expect(result).toHaveLength(2);
    expect(result[0].alt).toBe('top');
    expect(result[1].alt).toBe('bottom');
  });
});

describe('imageMarkdown — gesture/zoom logic (#3422)', () => {
  // ── shouldDismissOnSwipe ──

  it('dismisses when drag exceeds threshold', () => {
    expect(shouldDismissOnSwipe(150)).toBe(true);
    expect(shouldDismissOnSwipe(300)).toBe(true);
  });

  it('does not dismiss when drag below threshold', () => {
    expect(shouldDismissOnSwipe(50)).toBe(false);
    expect(shouldDismissOnSwipe(119)).toBe(false);
  });

  it('does not dismiss on upward swipe', () => {
    expect(shouldDismissOnSwipe(-200)).toBe(false);
  });

  it('boundary: exactly at threshold does not dismiss (exclusive)', () => {
    expect(shouldDismissOnSwipe(SWIPE_DISMISS_THRESHOLD)).toBe(false);
  });

  it('supports custom threshold', () => {
    expect(shouldDismissOnSwipe(60, 50)).toBe(true);
    expect(shouldDismissOnSwipe(40, 50)).toBe(false);
  });

  // ── nextZoomOnDoubleTap ──

  it('zooms in from 1x on double-tap', () => {
    expect(nextZoomOnDoubleTap(1)).toBe(ZOOM_STEP);
    expect(nextZoomOnDoubleTap(1.0)).toBe(2);
  });

  it('resets to 1x when already zoomed', () => {
    expect(nextZoomOnDoubleTap(2)).toBe(1);
    expect(nextZoomOnDoubleTap(3)).toBe(1);
  });

  it('resets to 1x at exactly ZOOM_STEP', () => {
    expect(nextZoomOnDoubleTap(ZOOM_STEP)).toBe(1);
  });

  // ── clampZoom ──

  it('clamps zoom to valid range', () => {
    expect(clampZoom(0.5)).toBe(MIN_ZOOM);
    expect(clampZoom(1)).toBe(1);
    expect(clampZoom(2.5)).toBe(2.5);
    expect(clampZoom(10)).toBe(MAX_ZOOM);
  });

  it('clamp general utility', () => {
    expect(clamp(5, 0, 10)).toBe(5);
    expect(clamp(-5, 0, 10)).toBe(0);
    expect(clamp(15, 0, 10)).toBe(10);
  });

  // ── zoomPercentage ──

  it('formats zoom percentage correctly', () => {
    expect(zoomPercentage(1)).toBe('100%');
    expect(zoomPercentage(2)).toBe('200%');
    expect(zoomPercentage(1.5)).toBe('150%');
    expect(zoomPercentage(0.5)).toBe('50%');
  });

  it('rounds non-even percentages', () => {
    expect(zoomPercentage(1.333)).toBe('133%');
    expect(zoomPercentage(2.666)).toBe('267%');
  });

  // ── nextImageIndex ──

  it('navigates forward', () => {
    expect(nextImageIndex(0, 1, 3)).toBe(1);
    expect(nextImageIndex(1, 1, 3)).toBe(2);
  });

  it('wraps forward past last image', () => {
    expect(nextImageIndex(2, 1, 3)).toBe(0);
  });

  it('navigates backward', () => {
    expect(nextImageIndex(2, -1, 3)).toBe(1);
    expect(nextImageIndex(1, -1, 3)).toBe(0);
  });

  it('wraps backward past first image', () => {
    expect(nextImageIndex(0, -1, 3)).toBe(2);
  });

  it('returns 0 for single image', () => {
    expect(nextImageIndex(0, 1, 1)).toBe(0);
    expect(nextImageIndex(0, -1, 1)).toBe(0);
  });
});
