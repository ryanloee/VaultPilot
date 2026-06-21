/**
 * APK auto-update checker — compares local app version against GitHub Releases.
 *
 * Design:
 * - Fetches latest release from GitHub API
 * - Compares semver (major.minor.patch) with current app.json version
 * - Returns release info so UI can show update prompt
 * - Download via browser (Linking.openURL) — no expo-intent-launcher dependency
 */

const GITHUB_API = 'https://api.github.com/repos/ryanloee/VaultPilot/releases/latest';

export interface UpdateInfo {
  latestVersion: string;
  currentVersion: string;
  releaseUrl: string;
  body: string; // release notes
  apkUrl: string | null;
  publishedAt: string;
}

/** Compare two semver strings. Returns 1 if a > b, -1 if a < b, 0 if equal. */
export function compareSemver(a: string, b: string): number {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    const da = pa[i] ?? 0;
    const db = pb[i] ?? 0;
    if (da > db) return 1;
    if (da < db) return -1;
  }
  return 0;
}

/** Check GitHub Releases for a newer version. Returns null if up-to-date. */
export async function checkForUpdate(currentVersion: string): Promise<UpdateInfo | null> {
  try {
    const res = await fetch(GITHUB_API, {
      headers: { Accept: 'application/vnd.github+json' },
      signal: AbortSignal.timeout(8000),
    });
    if (!res.ok) return null;

    const release = await res.json();
    const tag: string = release.tag_name ?? '';
    const latestVersion = tag.replace(/^v/, '');

    if (!latestVersion || compareSemver(latestVersion, currentVersion) <= 0) {
      return null; // up-to-date
    }

    const apkAsset = (release.assets ?? []).find(
      (a: { name: string }) => a.name.endsWith('.apk')
    );

    return {
      latestVersion,
      currentVersion,
      releaseUrl: release.html_url ?? '',
      body: release.body ?? '',
      apkUrl: apkAsset?.browser_download_url ?? null,
      publishedAt: release.published_at ?? '',
    };
  } catch {
    // Network error or timeout — silently skip
    return null;
  }
}
