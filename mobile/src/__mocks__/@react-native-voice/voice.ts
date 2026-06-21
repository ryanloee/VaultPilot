/**
 * Mock for @react-native-voice/voice — not available in node test environment.
 */
const noop = () => {};
const asyncNoop = async () => {};

export default {
  start: jest.fn(asyncNoop),
  stop: jest.fn(asyncNoop),
  cancel: jest.fn(asyncNoop),
  destroy: jest.fn(asyncNoop),
  isAvailable: jest.fn(async () => true),
  onSpeechStart: noop as unknown as (() => void) | undefined,
  onSpeechEnd: noop as unknown as (() => void) | undefined,
  onSpeechResults: noop as unknown as ((e: { value?: string[] }) => void) | undefined,
  onSpeechPartialResults: noop as unknown as ((e: { value?: string[] }) => void) | undefined,
  onSpeechError: noop as unknown as ((e: { error?: { message?: string } }) => void) | undefined,
};
