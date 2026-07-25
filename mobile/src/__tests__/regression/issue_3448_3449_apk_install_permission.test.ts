/**
 * Regression tests for:
 *  #3448 — REQUEST_INSTALL_PACKAGES permission must be declared in app.json
 *  #3449 — requestInstallPermission must use the correct intent action string
 *           (MANAGE_UNKNOWN_APP_SOURCES, not the non-existent _SETTINGS suffix)
 */

// Mock expo modules before importing the module under test
jest.mock('expo-file-system', () => ({
  File: jest.fn(),
  Paths: { cache: '/tmp' },
  getContentUriAsync: jest.fn().mockResolvedValue('content://test'),
}));
jest.mock('expo-intent-launcher', () => ({
  startActivityAsync: jest.fn(),
}));

import * as IntentLauncher from 'expo-intent-launcher';
import { Platform } from 'react-native';

// Load app.json (it's static config, safe to require)
// eslint-disable-next-line @typescript-eslint/no-var-requires
const appConfig = require('../../../app.json');

describe('issue #3448 — REQUEST_INSTALL_PACKAGES declared in app.json', () => {
  it('expo.android.permissions includes REQUEST_INSTALL_PACKAGES', () => {
    const perms = appConfig.expo.android.permissions;
    expect(Array.isArray(perms)).toBe(true);
    expect(perms).toContain('REQUEST_INSTALL_PACKAGES');
  });
});

describe('issue #3449 — requestInstallPermission uses correct intent action', () => {
  const startActivityAsyncMock = IntentLauncher.startActivityAsync as jest.MockedFunction<
    typeof IntentLauncher.startActivityAsync
  >;

  beforeEach(() => {
    startActivityAsyncMock.mockClear();
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    startActivityAsyncMock.mockResolvedValue(undefined as any);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('uses MANAGE_UNKNOWN_APP_SOURCES (no _SETTINGS suffix)', async () => {
    // Force Platform to look like Android 26+
    const origOS = Platform.OS;
    const origVer = Platform.Version;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (Platform as any).OS = 'android';
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (Platform as any).Version = 30;

    const { requestInstallPermission } = await import('../../utils/updateChecker');
    await requestInstallPermission();

    // The FIRST call must use the correct action — not the bogus _SETTINGS variant
    expect(startActivityAsyncMock).toHaveBeenCalled();
    const firstCallAction = startActivityAsyncMock.mock.calls[0][0] as string;
    expect(firstCallAction).toBe('android.settings.MANAGE_UNKNOWN_APP_SOURCES');
    expect(firstCallAction).not.toContain('_SETTINGS');

    // Restore
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (Platform as any).OS = origOS;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (Platform as any).Version = origVer;
  });
});
