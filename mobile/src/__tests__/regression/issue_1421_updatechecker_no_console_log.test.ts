/**
 * Regression test for #1421 — updateChecker.ts must use console.warn, not console.log.
 */
import { checkForUpdate, downloadAndInstall } from '../../utils/updateChecker';

// Mock expo-file-system and expo-intent-launcher
jest.mock('expo-file-system', () => ({
  File: jest.fn(),
  Paths: { cache: '/tmp' },
  getContentUriAsync: jest.fn().mockResolvedValue('content://test'),
}));
jest.mock('expo-intent-launcher', () => ({
  startActivityAsync: jest.fn(),
}));

describe('issue #1421 — updateChecker uses console.warn, not console.log', () => {
  let logSpy: jest.SpyInstance;

  beforeEach(() => {
    logSpy = jest.spyOn(console, 'log').mockImplementation(() => {});
    jest.spyOn(console, 'warn').mockImplementation(() => {});
    // Mock fetch to reject → triggers catch path with console.warn
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).fetch = jest.fn().mockRejectedValue(new Error('network'));
  });

  afterEach(() => {
    logSpy.mockRestore();
    jest.restoreAllMocks();
  });

  it('checkForUpdate does not call console.log', async () => {
    await checkForUpdate('1.0.0');
    expect(logSpy).not.toHaveBeenCalled();
  });

  it('downloadAndInstall does not call console.log', async () => {
    const result = await downloadAndInstall('https://example.com/fake.apk', '1.0.0');
    expect(result).toBe(false);
    expect(logSpy).not.toHaveBeenCalled();
  });
});
