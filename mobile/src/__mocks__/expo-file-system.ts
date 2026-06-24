/** Jest mock for expo-file-system (Expo SDK 56). */
export const File = {
  downloadFileAsync: jest.fn().mockResolvedValue({ uri: '/tmp/cache/updates/test.apk' }),
  createDownloadTask: jest.fn(),
};
export const Directory = jest.fn().mockImplementation(() => ({
  exists: true,
  create: jest.fn(),
  uri: '/tmp/cache/updates',
}));
export const Paths = { cache: '/tmp/cache', document: '/tmp/doc' };
export const createDownloadTask = jest.fn();
