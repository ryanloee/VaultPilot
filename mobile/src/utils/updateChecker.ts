/**
 * APK auto-update — check, download, and install.
 */

import { File, Paths } from 'expo-file-system';
import * as FileSystem from 'expo-file-system/legacy';
import * as IntentLauncher from 'expo-intent-launcher';
import { Platform } from 'react-native';

const GITHUB_API = 'https://api.github.com/repos/ryanloee/VaultPilot/releases/latest';
const GITHUB_RELEASES = 'https://api.github.com/repos/ryanloee/VaultPilot/releases?per_page=5';

export interface UpdateInfo {
  latestVersion: string;
  currentVersion: string;
  releaseUrl: string;
  body: string;
  apkUrl: string | null;
  publishedAt: string;
}

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
      return null;
    }

    const apkAsset = (release.assets ?? []).find(
      (a: { name: string }) => a.name.endsWith('.apk')
    );

    // If the latest release has no APK, search recent releases for one that does
    let finalRelease = release;
    let finalVersion = latestVersion;
    let finalApkAsset = apkAsset;

    if (!apkAsset) {
      try {
        const listRes = await fetch(GITHUB_RELEASES, {
          headers: { Accept: 'application/vnd.github+json' },
          signal: AbortSignal.timeout(8000),
        });
        if (listRes.ok) {
          const releases: Array<{
            tag_name?: string;
            assets?: Array<{ name: string; browser_download_url: string }>;
            html_url?: string;
            body?: string;
            published_at?: string;
          }> = await listRes.json();

          for (const r of releases) {
            const v = (r.tag_name ?? '').replace(/^v/, '');
            if (!v || compareSemver(v, currentVersion) <= 0) continue;
            const asset = (r.assets ?? []).find(
              (a: { name: string }) => a.name.endsWith('.apk'),
            );
            if (asset) {
              finalRelease = r;
              finalVersion = v;
              finalApkAsset = asset;
              break;
            }
          }
        }
      } catch (listErr) {
        console.warn('[UpdateChecker] fallback releases fetch failed:', listErr);
      }
    }

    return {
      latestVersion: finalVersion,
      currentVersion,
      releaseUrl: finalRelease.html_url ?? '',
      body: finalRelease.body ?? '',
      apkUrl: finalApkAsset?.browser_download_url ?? null,
      publishedAt: finalRelease.published_at ?? '',
    };
  } catch (e) {
    console.warn('[UpdateChecker] fetchLatestRelease failed:', e);
    return null;
  }
}

export async function downloadAndInstall(
  apkUrl: string,
  version: string,
  onProgress?: (percent: number) => void,
): Promise<boolean> {
  if (Platform.OS !== 'android') return false;

  try {
    console.warn('[UpdateChecker] Downloading APK from:', apkUrl);

    // Use downloadFileAsync (recommended in SDK 56) with progress tracking via DownloadTask
    const dest = new File(Paths.cache, `VaultPilot-v${version}.apk`);

    // Try downloadFileAsync first (simpler, more reliable)
    const output = await File.downloadFileAsync(apkUrl, dest);

    if (!output?.uri) {
      console.warn('[UpdateChecker] Download returned no URI');
      return false;
    }

    console.warn('[UpdateChecker] Downloaded to:', output.uri);

    // Convert file:// URI to content:// URI (required for Android install intent)
    const contentUri = await FileSystem.getContentUriAsync(output.uri);
    console.warn('[UpdateChecker] Content URI:', contentUri);

    // FLAG_GRANT_READ_URI_PERMISSION (1) + FLAG_ACTIVITY_NEW_TASK (0x10000000)
    await IntentLauncher.startActivityAsync('android.intent.action.INSTALL_PACKAGE', {
      data: contentUri,
      flags: 0x10000001,
      type: 'application/vnd.android.package-archive',
    });

    return true;
  } catch (e) {
    console.warn('[UpdateChecker] Download/install failed:', e);
    return false;
  }
}
