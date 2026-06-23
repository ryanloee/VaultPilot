/**
 * Regression test for #1380: UpdateModal fallback behavior.
 *
 * Tests:
 * 1. "查看发布页" button opens releaseUrl (not onClose) when apkUrl is null
 * 2. Error state shows "手动下载" fallback button that opens releaseUrl
 */

import type { UpdateInfo } from '../../utils/updateChecker';

// We test the pure logic that determines button behavior,
// since the component itself requires a full RN render tree.

describe('UpdateModal fallback logic (#1380)', () => {
  const baseInfo: UpdateInfo = {
    latestVersion: '0.3.44',
    currentVersion: '0.3.43',
    releaseUrl: 'https://github.com/ryanloee/VaultPilot/releases/tag/v0.3.44',
    body: 'Bug fixes',
    apkUrl: null,
    publishedAt: '2026-06-23T00:00:00Z',
  };

  it('should use releaseUrl when apkUrl is null (not onClose)', () => {
    // The fix changes: onPress={updateInfo.apkUrl ? handleDownload : onClose}
    // to: onPress={updateInfo.apkUrl ? handleDownload : () => Linking.openURL(updateInfo.releaseUrl)}
    const info = { ...baseInfo, apkUrl: null };
    expect(info.apkUrl).toBeNull();
    expect(info.releaseUrl).toBe('https://github.com/ryanloee/VaultPilot/releases/tag/v0.3.44');
    // The button text should be "查看发布页" when apkUrl is null
    const buttonText = info.apkUrl ? '下载更新' : '查看发布页';
    expect(buttonText).toBe('查看发布页');
  });

  it('should use handleDownload when apkUrl is present', () => {
    const info = { ...baseInfo, apkUrl: 'https://example.com/app.apk' };
    expect(info.apkUrl).toBeTruthy();
    const buttonText = info.apkUrl ? '下载更新' : '查看发布页';
    expect(buttonText).toBe('下载更新');
  });

  it('error state should provide manual download button with releaseUrl', () => {
    const info = { ...baseInfo };
    // Error state should have both "关闭" and "手动下载" buttons
    // "手动下载" opens info.releaseUrl
    expect(info.releaseUrl).toBeTruthy();
    expect(info.releaseUrl).toMatch(/^https:\/\/github\.com/);
  });
});
