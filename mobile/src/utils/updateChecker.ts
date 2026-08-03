/**
 * APK auto-update — check, download, and install.
 */

import { File, Directory, Paths } from 'expo-file-system';
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
    const timeout1 = new AbortController();
    const timer1 = setTimeout(() => timeout1.abort(), 8000);
    try {
      const res = await fetch(GITHUB_API, {
        headers: { Accept: 'application/vnd.github+json' },
        signal: timeout1.signal,
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
          const timeout2 = new AbortController();
          const timer2 = setTimeout(() => timeout2.abort(), 8000);
          try {
            const listRes = await fetch(GITHUB_RELEASES, {
              headers: { Accept: 'application/vnd.github+json' },
              signal: timeout2.signal,
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
          } finally {
            clearTimeout(timer2);
          }
        } catch (e) {
            console.warn('[UpdateChecker] fallback sync error:', e);
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
    } finally {
      clearTimeout(timer1);
    }
  } catch (e) {
    console.warn('[UpdateChecker] fetchLatestRelease failed:', e);
    return null;
  }
}

/** Timeout (ms) if no progress for this duration. */
const STALL_TIMEOUT_MS = 5 * 60 * 1000; // 5 minutes

/**
 * Open system settings for "Install unknown apps" permission (Android 8+).
 * On Android 26+, the app needs REQUEST_INSTALL_PACKAGES permission to
 * launch the system package installer. This redirects the user to the
 * specific settings page where they can enable it for this app.
 */
export async function requestInstallPermission(): Promise<void> {
  if (Platform.OS !== 'android' || (Platform.Version as number) < 26) return;

  try {
    // Opens Android Settings → Apps → Special app access → Install unknown apps
    // with this app pre-selected.
    await IntentLauncher.startActivityAsync(
      'android.settings.MANAGE_UNKNOWN_APP_SOURCES',
      { data: 'package:com.vaultpilot.mobile' },
    );
  } catch (e) {
    console.warn('[UpdateChecker] Failed to launch install permission settings:', e);
    // Fallback: try general application details settings
    try {
      await IntentLauncher.startActivityAsync('android.settings.APPLICATION_DETAILS_SETTINGS', {
        data: 'package:com.vaultpilot.mobile',
      });
    } catch (e2) {
      console.warn('[UpdateChecker] Fallback settings also failed:', e2);
    }
  }
}

export async function downloadAndInstall(
  apkUrl: string,
  version: string,
  onProgress?: (percent: number) => void,
  signal?: AbortSignal,
): Promise<boolean> {
  if (Platform.OS !== 'android') return false;

  // Check if already aborted before starting
  if (signal?.aborted) return false;

  let downloadAborted = false;

  // Build a promise that rejects when the download should be cancelled
  let abortReject: ((reason: Error) => void) | null = null;
  const abortPromise = new Promise<never>((_resolve, reject) => {
    abortReject = reject;
  });

  const abortDownload = (reason: string) => {
    downloadAborted = true;
    abortReject?.(new Error(reason));
  };

  try {
    console.warn('[UpdateChecker] Downloading APK from:', apkUrl);

    // Use Directory as destination (File.downloadFileAsync requires Directory, not File)
    const downloadDir = new Directory(Paths.cache, 'updates');
    if (!downloadDir.exists) downloadDir.create();

    // Track progress for stall timeout
    let lastProgressTime = Date.now();

    // Start download without awaiting — so watchdog can monitor progress during download
    const result = File.downloadFileAsync(apkUrl, downloadDir, {
      idempotent: true,
      onProgress: ({ bytesWritten, totalBytes }: { bytesWritten: number; totalBytes: number }) => {
        lastProgressTime = Date.now();
        if (onProgress && totalBytes > 0) {
          onProgress(Math.round((bytesWritten / totalBytes) * 100));
        }
      },
    }).catch((err) => {
      // Suppress unhandled rejection if abortPromise already won the race
      if (!downloadAborted) throw err;
      return undefined as never;
    });

    // Stall timeout watchdog — if no progress for STALL_TIMEOUT_MS, abort
    const stallWatch = setInterval(() => {
      if (signal?.aborted || downloadAborted) return;
      if (Date.now() - lastProgressTime > STALL_TIMEOUT_MS) {
        console.warn('[UpdateChecker] Download stalled for too long, aborting');
        abortDownload('Download stalled');
      }
    }, 10_000);

    // Listen for abort — if signal fires, reject the race so we can return immediately
    const onAbort = () => {
      clearInterval(stallWatch);
      abortDownload('Download aborted by signal');
    };
    signal?.addEventListener('abort', onAbort, { once: true });

    try {
      // Race the download against the abort/stall promise
      const awaited = await Promise.race([result, abortPromise]);
      clearInterval(stallWatch);

      if (signal?.aborted || downloadAborted) return false;

      if (!awaited?.uri) {
        console.warn('[UpdateChecker] Download returned no URI');
        return false;
      }

      console.warn('[UpdateChecker] Downloaded to:', awaited.uri);

      // Convert file:// URI to content:// URI (required for Android install intent)
      const contentUri = await FileSystem.getContentUriAsync(awaited.uri);
      console.warn('[UpdateChecker] Content URI:', contentUri);

      // Launch system package installer
      await IntentLauncher.startActivityAsync('android.intent.action.INSTALL_PACKAGE', {
        data: contentUri,
        flags: 1, // FLAG_GRANT_READ_URI_PERMISSION
      });

      return true;
    } finally {
      clearInterval(stallWatch);
      signal?.removeEventListener('abort', onAbort);
    }
  } catch (e) {
    if (signal?.aborted || downloadAborted) return false;
    console.warn('[UpdateChecker] Download/install failed:', e);
    return false;
  }
}
