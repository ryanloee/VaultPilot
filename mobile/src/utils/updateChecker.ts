/**
 * APK auto-update — check, download, and install.
 */

import { File, Paths } from 'expo-file-system';
import * as IntentLauncher from 'expo-intent-launcher';
import { Platform, Linking } from 'react-native';

const GITHUB_API = 'https://api.github.com/repos/ryanloee/VaultPilot/releases/latest';

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

    return {
      latestVersion,
      currentVersion,
      releaseUrl: release.html_url ?? '',
      body: release.body ?? '',
      apkUrl: apkAsset?.browser_download_url ?? null,
      publishedAt: release.published_at ?? '',
    };
  } catch {
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
    const dest = new File(Paths.cache, `VaultPilot-v${version}.apk`);
    const task = File.createDownloadTask(apkUrl, dest, {
      onProgress: ({ bytesWritten, totalBytes }: { bytesWritten: number; totalBytes: number }) => {
        if (onProgress && totalBytes > 0) {
          onProgress(Math.round((bytesWritten / totalBytes) * 100));
        }
      },
    });

    const result = await task.downloadAsync();
    if (!result?.uri) return false;

    await IntentLauncher.startActivityAsync('android.intent.action.VIEW', {
      data: result.uri,
      flags: 1,
      type: 'application/vnd.android.package-archive',
    });
    return true;
  } catch (e) {
    console.warn('[UpdateChecker] Download/install failed:', e);
    try {
      const releaseUrl = `https://github.com/ryanloee/VaultPilot/releases/tag/v${version}`;
      await Linking.openURL(releaseUrl);
    } catch {}
    return false;
  }
}
