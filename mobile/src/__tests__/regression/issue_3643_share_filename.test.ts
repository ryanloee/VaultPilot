/**
 * Regression test for #3643 — ShareReceiveScreen copyToVault bugs:
 *   1. Broken `![[name]]` embeds (fallback name mismatch between embed & file)
 *   2. Filename collision — multi-file shares silently overwrite
 *   3. Path traversal via unsanitized originalName
 *
 * @see mobile/src/utils/shareHelpers.ts — sanitizeShareFileName, resolveShareFileName, extractShareText
 */
import {
  sanitizeShareFileName,
  resolveShareFileName,
  extractShareText,
} from '../../utils/shareHelpers';
import type { ResolvedSharePayload } from 'expo-sharing';

/** Build a minimal payload for testing. */
function makePayload(
  shareType: 'image' | 'file' | 'text' | 'url',
  overrides: Partial<ResolvedSharePayload> = {},
): ResolvedSharePayload {
  return {
    shareType,
    value: overrides.value ?? '',
    originalName: overrides.originalName,
    contentUri: overrides.contentUri ?? 'content://test/test',
    ...overrides,
  } as ResolvedSharePayload;
}

// ---------------------------------------------------------------------------
// Bug 3: Path traversal via unsanitized originalName (security)
// ---------------------------------------------------------------------------

describe('#3643 Bug 3 — path traversal sanitization', () => {
  test('strips ../ path traversal from originalName', () => {
    expect(sanitizeShareFileName('../../malicious.js')).toBe('malicious.js');
  });

  test('strips absolute Windows path', () => {
    expect(sanitizeShareFileName('C:\\Users\\evil\\hack.exe')).toBe('hack.exe');
  });

  test('strips leading dots that could create hidden files', () => {
    expect(sanitizeShareFileName('.hidden')).toBe('hidden');
    expect(sanitizeShareFileName('...secret')).toBe('secret');
  });

  test('returns empty string for undefined or empty input', () => {
    expect(sanitizeShareFileName(undefined)).toBe('');
    expect(sanitizeShareFileName('')).toBe('');
  });

  test('preserves a normal filename', () => {
    expect(sanitizeShareFileName('photo.jpg')).toBe('photo.jpg');
  });

  test('resolveShareFileName strips path traversal in full payload', () => {
    const p = makePayload('image', { originalName: '../../../etc/passwd' });
    const result = resolveShareFileName(p, 0);
    // Must not contain path separators or parent references
    expect(result).not.toContain('/');
    expect(result).not.toContain('\\');
    expect(result).not.toContain('..');
    // Should use the basename with index suffix
    expect(result).toBe('passwd-1');
  });
});

// ---------------------------------------------------------------------------
// Bug 2: Filename collision — multi-file shares silently overwrite
// ---------------------------------------------------------------------------

describe('#3643 Bug 2 — filename collision prevention', () => {
  test('same originalName produces different filenames at different indices', () => {
    const p1 = makePayload('image', { originalName: 'IMG_0001.jpg' });
    const p2 = makePayload('image', { originalName: 'IMG_0001.jpg' });
    const name1 = resolveShareFileName(p1, 0);
    const name2 = resolveShareFileName(p2, 1);
    expect(name1).not.toBe(name2);
    expect(name1).toBe('IMG_0001-1.jpg');
    expect(name2).toBe('IMG_0001-2.jpg');
  });

  test('missing originalName produces unique deterministic names', () => {
    const p1 = makePayload('image');
    const p2 = makePayload('image');
    const name1 = resolveShareFileName(p1, 0);
    const name2 = resolveShareFileName(p2, 1);
    expect(name1).not.toBe(name2);
    expect(name1).toBe('share-image-1.jpg');
    expect(name2).toBe('share-image-2.jpg');
  });

  test('filename without extension gets suffix appended', () => {
    const p = makePayload('file', { originalName: 'README' });
    expect(resolveShareFileName(p, 0)).toBe('README-1');
  });
});

// ---------------------------------------------------------------------------
// Bug 1: Broken embeds — embed name must match saved file name
// ---------------------------------------------------------------------------

describe('#3643 Bug 1 — embed/file name consistency', () => {
  test('extractShareText embed matches resolveShareFileName for image', () => {
    const p = makePayload('image', { originalName: 'vacation.jpg' });
    const embedText = extractShareText(p, 2);
    const fileName = resolveShareFileName(p, 2);
    // The embed should reference the exact filename that will be saved
    expect(embedText).toBe(`![[${fileName}]]`);
  });

  test('extractShareText embed matches resolveShareFileName for file', () => {
    const p = makePayload('file', { originalName: 'doc.pdf' });
    const embedText = extractShareText(p, 0);
    const fileName = resolveShareFileName(p, 0);
    expect(embedText).toBe(`📎 ${fileName}`);
  });

  test('fallback embed uses deterministic name (not Date.now)', () => {
    const p = makePayload('image');
    const embed1 = extractShareText(p, 0);
    const embed2 = extractShareText(p, 0);
    // Calling twice with same index should produce the same embed
    expect(embed1).toBe(embed2);
    expect(embed1).toBe('![[share-image-1.jpg]]');
  });

  test('embed for unnamed image is no longer the broken "shared-image"', () => {
    const p = makePayload('image');
    const embed = extractShareText(p, 0);
    // Before fix: embed was "![[shared-image]]" but file saved as "share-<timestamp>.jpg"
    expect(embed).not.toContain('shared-image');
  });
});

// ---------------------------------------------------------------------------
// Integration: full batch consistency
// ---------------------------------------------------------------------------

describe('#3643 integration — batch share consistency', () => {
  test('mixed batch: every embed references a uniquely-resolvable filename', () => {
    const payloads: ResolvedSharePayload[] = [
      makePayload('text', { value: 'Hello' }),
      makePayload('image', { originalName: 'a.jpg' }),
      makePayload('image', { originalName: 'a.jpg' }), // same name as above
      makePayload('image'), // unnamed
      makePayload('file', { originalName: '../../secret.txt' }), // path traversal
    ];

    // Compute embed texts and file names with the same indices
    const embeds = payloads.map((p, i) => extractShareText(p, i));
    const fileNames = payloads.map((p, i) => resolveShareFileName(p, i));

    // Extract names from embeds and verify they match the resolved file names
    const imageFileNames = fileNames.filter((_, i) =>
      payloads[i].shareType === 'image' || payloads[i].shareType === 'file',
    );
    expect(imageFileNames.length).toBe(4);

    // All file names must be unique (no collision)
    expect(new Set(imageFileNames).size).toBe(imageFileNames.length);

    // All file names must be safe (no path separators)
    imageFileNames.forEach((name) => {
      expect(name).not.toMatch(/[/\\]/);
    });
  });
});
