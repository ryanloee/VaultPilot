/**
 * Unit tests for updateChecker.ts — checkForUpdate pure logic (#1394).
 *
 * Tests: GitHub API response parsing, version comparison, APK asset
 *        detection, error handling, and edge cases.
 */

import { checkForUpdate, compareSemver } from '../../utils/updateChecker';

const mockFetch = jest.fn();
(globalThis as any).fetch = mockFetch;

beforeEach(() => {
  jest.clearAllMocks();
  mockFetch.mockReset();
});

function mockRelease(overrides: Record<string, unknown> = {}) {
  return {
    tag_name: 'v0.4.0',
    html_url: 'https://github.com/ryanloee/VaultPilot/releases/tag/v0.4.0',
    body: 'Release notes here',
    published_at: '2026-06-23T00:00:00Z',
    assets: [
      { name: 'VaultPilot-v0.4.0.apk', browser_download_url: 'https://github.com/.../VaultPilot-v0.4.0.apk' },
    ],
    ...overrides,
  };
}

describe('checkForUpdate', () => {
  it('returns null when no update available (same version)', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockRelease({ tag_name: 'v0.3.44' })),
    });
    const result = await checkForUpdate('0.3.44');
    expect(result).toBeNull();
  });

  it('returns null when local is newer', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockRelease({ tag_name: 'v0.3.44' })),
    });
    const result = await checkForUpdate('0.4.0');
    expect(result).toBeNull();
  });

  it('returns update info when newer version available', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockRelease()),
    });
    const result = await checkForUpdate('0.3.44');
    expect(result).not.toBeNull();
    expect(result!.latestVersion).toBe('0.4.0');
    expect(result!.currentVersion).toBe('0.3.44');
    expect(result!.releaseUrl).toBe('https://github.com/ryanloee/VaultPilot/releases/tag/v0.4.0');
    expect(result!.apkUrl).toBe('https://github.com/.../VaultPilot-v0.4.0.apk');
  });

  it('returns null when API returns error', async () => {
    mockFetch.mockResolvedValueOnce({ ok: false, status: 403 });
    const result = await checkForUpdate('0.3.44');
    expect(result).toBeNull();
  });

  it('returns null on network error', async () => {
    mockFetch.mockRejectedValueOnce(new Error('Network error'));
    const result = await checkForUpdate('0.3.44');
    expect(result).toBeNull();
  });

  it('handles missing APK asset gracefully', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockRelease({ assets: [] })),
    });
    const result = await checkForUpdate('0.3.44');
    expect(result).not.toBeNull();
    expect(result!.apkUrl).toBeNull();
  });

  it('handles missing tag_name gracefully', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockRelease({ tag_name: '' })),
    });
    const result = await checkForUpdate('0.3.44');
    expect(result).toBeNull();
  });

  it('handles missing html_url and body gracefully', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockRelease({ html_url: undefined, body: undefined })),
    });
    const result = await checkForUpdate('0.3.44');
    expect(result).not.toBeNull();
    expect(result!.releaseUrl).toBe('');
    expect(result!.body).toBe('');
  });

  it('handles assets without APK file', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockRelease({
        assets: [{ name: 'source.zip', browser_download_url: 'https://...' }],
      })),
    });
    const result = await checkForUpdate('0.3.44');
    expect(result).not.toBeNull();
    expect(result!.apkUrl).toBeNull();
  });

  it('finds APK among multiple assets', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(mockRelease({
        assets: [
          { name: 'source.zip', browser_download_url: 'https://.../source.zip' },
          { name: 'VaultPilot-v0.4.0.apk', browser_download_url: 'https://.../apk' },
          { name: 'checksums.txt', browser_download_url: 'https://.../checksums' },
        ],
      })),
    });
    const result = await checkForUpdate('0.3.44');
    expect(result!.apkUrl).toBe('https://.../apk');
  });
});
