/**
 * Jest mock for react-native.
 */

export const Platform = { OS: 'android' };
export const Linking = { openURL: jest.fn().mockResolvedValue(undefined) };
export const Alert = { alert: jest.fn() };
