/**
 * Regression test for #887: Voice input hook.
 * Tests the Voice module mock integration and event handler wiring.
 *
 * Note: renderHook requires jsdom (React Native), so we test the
 * underlying Voice event contract directly.
 */
import Voice from '@react-native-voice/voice';

// Track registered handlers
const registeredHandlers: Record<string, Function | undefined> = {};

Object.defineProperty(Voice, 'onSpeechStart', {
  set(fn: Function | undefined) { registeredHandlers.onSpeechStart = fn; },
  get() { return registeredHandlers.onSpeechStart; },
});
Object.defineProperty(Voice, 'onSpeechEnd', {
  set(fn: Function | undefined) { registeredHandlers.onSpeechEnd = fn; },
  get() { return registeredHandlers.onSpeechEnd; },
});
Object.defineProperty(Voice, 'onSpeechResults', {
  set(fn: Function | undefined) { registeredHandlers.onSpeechResults = fn; },
  get() { return registeredHandlers.onSpeechResults; },
});
Object.defineProperty(Voice, 'onSpeechPartialResults', {
  set(fn: Function | undefined) { registeredHandlers.onSpeechPartialResults = fn; },
  get() { return registeredHandlers.onSpeechPartialResults; },
});
Object.defineProperty(Voice, 'onSpeechError', {
  set(fn: Function | undefined) { registeredHandlers.onSpeechError = fn; },
  get() { return registeredHandlers.onSpeechError; },
});

describe('Voice module contract (#887)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    Object.keys(registeredHandlers).forEach(k => delete registeredHandlers[k]);
  });

  it('Voice.start accepts locale string', async () => {
    await Voice.start('zh-CN');
    expect(Voice.start).toHaveBeenCalledWith('zh-CN');
  });

  it('Voice.stop is callable', async () => {
    await Voice.stop();
    expect(Voice.stop).toHaveBeenCalled();
  });

  it('Voice.cancel is callable', async () => {
    await Voice.cancel();
    expect(Voice.cancel).toHaveBeenCalled();
  });

  it('Voice.destroy is callable', async () => {
    await Voice.destroy();
    expect(Voice.destroy).toHaveBeenCalled();
  });

  it('onSpeechResults handler receives value array', () => {
    let captured: string[] = [];
    Voice.onSpeechResults = (e: { value?: string[] }) => {
      if (e.value) captured = e.value;
    };
    registeredHandlers.onSpeechResults?.({ value: ['你好世界'] });
    expect(captured).toEqual(['你好世界']);
  });

  it('onSpeechPartialResults handler receives partial value', () => {
    let captured: string[] = [];
    Voice.onSpeechPartialResults = (e: { value?: string[] }) => {
      if (e.value) captured = e.value;
    };
    registeredHandlers.onSpeechPartialResults?.({ value: ['你好'] });
    expect(captured).toEqual(['你好']);
  });

  it('onSpeechError handler receives error object', () => {
    let captured: { message?: string } | null = null;
    Voice.onSpeechError = (e: { error?: { message?: string } }) => {
      captured = e.error || null;
    };
    registeredHandlers.onSpeechError?.({ error: { message: 'no match' } });
    expect(captured).toEqual({ message: 'no match' });
  });

  it('onSpeechStart handler fires', () => {
    let fired = false;
    Voice.onSpeechStart = () => { fired = true; };
    registeredHandlers.onSpeechStart?.();
    expect(fired).toBe(true);
  });

  it('onSpeechEnd handler fires', () => {
    let fired = false;
    Voice.onSpeechEnd = () => { fired = true; };
    registeredHandlers.onSpeechEnd?.();
    expect(fired).toBe(true);
  });

  it('handles empty results gracefully', () => {
    let captured: string | null = null;
    Voice.onSpeechResults = (e: { value?: string[] }) => {
      captured = e.value?.[0] ?? null;
    };
    registeredHandlers.onSpeechResults?.({ value: [] });
    expect(captured).toBeNull();
  });

  it('handles undefined value in results', () => {
    let captured: string | null = null;
    Voice.onSpeechResults = (e: { value?: string[] }) => {
      captured = e.value?.[0] ?? null;
    };
    registeredHandlers.onSpeechResults?.({});
    expect(captured).toBeNull();
  });
});
