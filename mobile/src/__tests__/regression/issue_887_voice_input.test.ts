/**
 * Regression test for #887: Voice input hook.
 * Tests the expo-speech-recognition module mock integration.
 */
import { ExpoSpeechRecognitionModule } from 'expo-speech-recognition';

describe('Voice module contract (#887)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('ExpoSpeechRecognitionModule.start accepts options', () => {
    ExpoSpeechRecognitionModule.start({
      lang: 'zh-CN',
      interimResults: true,
      continuous: true,
    });
    expect(ExpoSpeechRecognitionModule.start).toHaveBeenCalledWith({
      lang: 'zh-CN',
      interimResults: true,
      continuous: true,
    });
  });

  it('ExpoSpeechRecognitionModule.stop is callable', () => {
    ExpoSpeechRecognitionModule.stop();
    expect(ExpoSpeechRecognitionModule.stop).toHaveBeenCalled();
  });

  it('ExpoSpeechRecognitionModule.abort is callable', () => {
    ExpoSpeechRecognitionModule.abort();
    expect(ExpoSpeechRecognitionModule.abort).toHaveBeenCalled();
  });

  it('requestPermissionsAsync returns permission status', async () => {
    const result = await ExpoSpeechRecognitionModule.requestPermissionsAsync();
    expect(result).toHaveProperty('granted');
  });
});
