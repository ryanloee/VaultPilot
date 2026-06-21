/**
 * Jest mock for expo-file-system (Expo SDK 56).
 */

export const File = jest.fn().mockImplementation(() => ({}));
export const Paths = { cache: '/tmp/cache', document: '/tmp/doc' };
export const createDownloadTask = jest.fn();
