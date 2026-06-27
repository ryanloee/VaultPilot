/**
 * Jest mock for react-native.
 */

export const Platform = { OS: 'android' };
export const Linking = { openURL: jest.fn().mockResolvedValue(undefined) };
export const Alert = { alert: jest.fn() };

// Default phone-like dimensions; tests can override via jest.mocked(useWindowDimensions).mockReturnValue(...)
export const useWindowDimensions = jest.fn(() => ({
  width: 360,
  height: 800,
  scale: 1,
  fontScale: 1,
}));
