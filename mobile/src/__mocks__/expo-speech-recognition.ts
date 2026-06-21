/**
 * Mock for expo-speech-recognition — not available in node test environment.
 */
export const ExpoSpeechRecognitionModule = {
  start: jest.fn(),
  stop: jest.fn(),
  abort: jest.fn(),
  requestPermissionsAsync: jest.fn(async () => ({ granted: true, status: 'granted' })),
  getPermissionsAsync: jest.fn(async () => ({ granted: true, status: 'granted' })),
  addListener: jest.fn(() => ({ remove: jest.fn() })),
};

export function useSpeechRecognitionEvent(_event: string, _handler: Function) {
  // no-op in tests
}
