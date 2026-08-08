/**
 * Image markdown parsing utilities for Image Lightbox (#3030, #3422).
 *
 * Extracted as pure functions for testability — these do NOT depend on React
 * Native and can be tested with plain Jest.
 */

export interface MarkdownImage {
  /** Image URL or data URI */
  uri: string;
  /** Alt text (also used as accessibility label) */
  alt: string;
  /** Optional title attribute from ![alt](url "title") */
  title?: string;
}

/**
 * Match a markdown image syntax: ![alt](url) or ![alt](url "title")
 *
 * Returns null if the string doesn't start with an image.
 */
const IMAGE_RE = /^!\[([^\]]*)\]\(([^)\s]+)(?:\s+"([^"]*)")?\)/;

/**
 * Extract all markdown images from a single line of text.
 *
 * Returns an array of { uri, alt } for each ![alt](url) found.
 * Handles multiple images on the same line and images embedded
 * in other markdown content.
 */
export function extractImagesFromLine(line: string): MarkdownImage[] {
  const images: MarkdownImage[] = [];
  let remaining = line;

  while (remaining) {
    const match = remaining.match(IMAGE_RE);
    if (!match) {
      // Try to find the next '!' that might start an image
      const nextBang = remaining.indexOf('!', 1);
      if (nextBang === -1) break;
      remaining = remaining.slice(nextBang);
      continue;
    }

    images.push({
      alt: match[1],
      uri: match[2],
      title: match[3] || undefined,
    });

    // Move past the matched image
    remaining = remaining.slice(match[0].length);
  }

  return images;
}

/**
 * Extract ALL images from full markdown content (multi-line).
 *
 * Scans every line for ![alt](url) patterns. Images inside fenced code
 * blocks (```) are ignored.
 */
export function extractImagesFromContent(content: string): MarkdownImage[] {
  const lines = content.split('\n');
  const images: MarkdownImage[] = [];
  let inCodeBlock = false;

  for (const line of lines) {
    if (line.trimStart().startsWith('```')) {
      inCodeBlock = !inCodeBlock;
      continue;
    }
    if (inCodeBlock) continue;

    images.push(...extractImagesFromLine(line));
  }

  return images;
}

/**
 * Extract images ONLY from standalone image lines (lines that contain
 * nothing but image markdown and whitespace).
 *
 * This mirrors the population of images actually rendered as tappable
 * `<TouchableOpacity>` in MarkdownPreview, so the resulting array's
 * indices align with the `imageCounter` / `globalIdx` used by tap
 * handlers (#3454).
 *
 * Inline images (images embedded mid-sentence, e.g.
 * `Hello ![emoji](e.png) world`) are excluded — they are never rendered
 * as clickable image blocks, so they must not appear in the Lightbox
 * image list.
 *
 * Images inside fenced code blocks (```) are ignored, same as
 * {@link extractImagesFromContent}.
 */
export function extractStandaloneImages(content: string): MarkdownImage[] {
  const lines = content.split('\n');
  const images: MarkdownImage[] = [];
  let inCodeBlock = false;

  for (const line of lines) {
    if (line.trimStart().startsWith('```')) {
      inCodeBlock = !inCodeBlock;
      continue;
    }
    if (inCodeBlock) continue;

    if (isStandaloneImageLine(line)) {
      images.push(...extractImagesFromLine(line));
    }
  }

  return images;
}

/**
 * Check if a line is a standalone image paragraph (the line contains
 * ONLY one or more images, possibly with surrounding whitespace).
 *
 * Used by MarkdownPreview to decide whether to render an image block
 * (centered, larger) vs an inline image (within text flow).
 */
export function isStandaloneImageLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;

  // Check if the entire trimmed line is images only
  // Remove all image markdown and check if anything remains
  const withoutImages = trimmed.replace(
    /!\[[^\]]*\]\([^)\s]+(?:\s+"[^"]*")?\)/g,
    '',
  );
  return withoutImages.trim() === '';
}

// ── Gesture / zoom logic (pure, testable) ──

/**
 * Default swipe-down dismiss threshold in pixels (#3422).
 * If the user drags the image down by more than this amount, the Lightbox closes.
 */
export const SWIPE_DISMISS_THRESHOLD = 120;

/**
 * Minimum zoom level (fit-to-screen).
 */
export const MIN_ZOOM = 1;

/**
 * Maximum zoom level.
 */
export const MAX_ZOOM = 5;

/**
 * Zoom step per button press or double-tap.
 */
export const ZOOM_STEP = 2;

/**
 * Clamp a value between min and max.
 */
export function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/**
 * Clamp zoom level to [MIN_ZOOM, MAX_ZOOM].
 */
export function clampZoom(zoom: number): number {
  return clamp(zoom, MIN_ZOOM, MAX_ZOOM);
}

/**
 * Determine whether a swipe-down gesture should dismiss the Lightbox.
 *
 * @param dy Vertical displacement in pixels (positive = downward)
 * @param threshold Minimum displacement to trigger dismiss
 * @returns true if the gesture exceeds the dismiss threshold
 */
export function shouldDismissOnSwipe(
  dy: number,
  threshold: number = SWIPE_DISMISS_THRESHOLD,
): boolean {
  return dy > threshold;
}

/**
 * Calculate the next zoom level when double-tapping.
 *
 * Toggle between 1x (fit) and ZOOM_STEP (2x).
 * If already at or above ZOOM_STEP, reset to 1x.
 *
 * @param currentZoom Current zoom level
 * @returns Next zoom level after double-tap
 */
export function nextZoomOnDoubleTap(currentZoom: number): number {
  if (currentZoom > MIN_ZOOM + 0.01) {
    return MIN_ZOOM;
  }
  return ZOOM_STEP;
}

/**
 * Calculate zoom percentage display string.
 *
 * @param zoom Current zoom level (1.0 = 100%)
 * @returns Percentage string like "100%", "200%", "50%"
 */
export function zoomPercentage(zoom: number): string {
  return `${Math.round(zoom * 100)}%`;
}

/**
 * Calculate the next index when navigating images.
 * Wraps around at boundaries.
 *
 * @param current Current index
 * @param delta +1 for next, -1 for previous
 * @param total Total number of images
 * @returns Next index (wrapped)
 */
export function nextImageIndex(
  current: number,
  delta: number,
  total: number,
): number {
  if (total <= 1) return 0;
  return ((current + delta) % total + total) % total;
}

/**
 * Remove a single line from multi-line content by 0-based line index (#3963).
 *
 * Unlike the previous text-equality filter (`filter(l => l !== imageLine)`),
 * this removes ONLY the line at the given index — so duplicate image lines
 * (same markdown text) are preserved and only the user-selected one is deleted.
 *
 * Out-of-range indices are ignored (content returned unchanged).
 *
 * @param content Full multi-line markdown content
 * @param lineIndex 0-based line index to remove
 * @returns Content with exactly one line removed (or unchanged if invalid index)
 */
export function removeLineByIndex(content: string, lineIndex: number): string {
  if (lineIndex < 0) return content;
  const lines = content.split('\n');
  if (lineIndex >= lines.length) return content;
  lines.splice(lineIndex, 1);
  return lines.join('\n');
}

/**
 * Extract the file name of an image for the Lightbox caption (#3927).
 *
 * Mirrors Obsidian 1.13.4: the lightbox shows the current image's file name.
 * - For http(s)/file URIs: last path segment, query/hash stripped, percent-decoded.
 * - For data URIs (base64 inline images): no file name → falls back to alt text
 *   (or the raw uri when alt is empty), so the caption never looks broken.
 *
 * @param image MarkdownImage to derive the name from
 * @returns Display name for the lightbox caption.
 */
export function imageFileName(image: MarkdownImage): string {
  if (!image.uri) return image.alt || '';
  if (image.uri.startsWith('data:')) {
    // Inline base64 image — no real file name. Use alt text as the caption.
    return image.alt || '';
  }
  try {
    const withoutQuery = image.uri.split(/[?#]/)[0];
    const segments = withoutQuery.split('/');
    const raw = segments[segments.length - 1] || '';
    const name = decodeURIComponent(raw);
    return name || image.alt || '';
  } catch {
    // Malformed percent-encoding — fall back to the raw last segment.
    const withoutQuery = image.uri.split(/[?#]/)[0];
    const segments = withoutQuery.split('/');
    return segments[segments.length - 1] || image.alt || '';
  }
}
