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

export const EncodingType = { UTF8: 'utf8', Base64: 'base64' };
export const readAsStringAsync = jest.fn().mockResolvedValue('ZmFrZS1iYXNlNjQtYnl0ZXM=');
