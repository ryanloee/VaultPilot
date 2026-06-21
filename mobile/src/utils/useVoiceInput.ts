/**
 * Voice input hook using @react-native-voice/voice.
 * Provides speech-to-text for the chat input.
 */
import { useState, useEffect, useRef, useCallback } from 'react';
import { Platform, Alert } from 'react-native';

// Lazy-load Voice to avoid crash if native module not linked
let Voice: typeof import('@react-native-voice/voice').default | null = null;
try {
  Voice = require('@react-native-voice/voice').default;
} catch {
  // Native module not available (e.g. Expo Go)
}

export interface VoiceInputState {
  /** Whether actively listening */
  isListening: boolean;
  /** Partial/final transcript */
  transcript: string;
  /** Error message if any */
  error: string | null;
  /** Whether voice input is available on this device */
  isAvailable: boolean;
}

export function useVoiceInput() {
  const [isListening, setIsListening] = useState(false);
  const [transcript, setTranscript] = useState('');
  const [error, setError] = useState<string | null>(null);
  const isAvailable = useRef(Voice !== null);

  useEffect(() => {
    if (!Voice) return;

    const onSpeechStart = () => setIsListening(true);
    const onSpeechEnd = () => setIsListening(false);
    const onSpeechResults = (e: { value?: string[] }) => {
      if (e.value?.[0]) setTranscript(e.value[0]);
    };
    const onSpeechPartialResults = (e: { value?: string[] }) => {
      if (e.value?.[0]) setTranscript(e.value[0]);
    };
    const onSpeechError = (e: { error?: { message?: string } }) => {
      const msg = e.error?.message || '语音识别失败';
      setError(msg);
      setIsListening(false);
    };

    Voice.onSpeechStart = onSpeechStart;
    Voice.onSpeechEnd = onSpeechEnd;
    Voice.onSpeechResults = onSpeechResults;
    Voice.onSpeechPartialResults = onSpeechPartialResults;
    Voice.onSpeechError = onSpeechError;

    return () => {
      Voice?.destroy().catch(() => {});
    };
  }, []);

  const startListening = useCallback(async (locale = 'zh-CN') => {
    if (!Voice) {
      Alert.alert('提示', '语音输入需要开发版 App（Expo Go 不支持）');
      return;
    }
    setError(null);
    setTranscript('');
    try {
      await Voice.start(locale);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : '无法启动语音识别';
      setError(msg);
    }
  }, []);

  const stopListening = useCallback(async () => {
    if (!Voice) return;
    try {
      await Voice.stop();
    } catch {
      // ignore
    }
  }, []);

  const cancelListening = useCallback(async () => {
    if (!Voice) return;
    try {
      await Voice.cancel();
    } catch {
      // ignore
    }
    setIsListening(false);
    setTranscript('');
  }, []);

  return {
    isListening,
    transcript,
    error,
    isAvailable: isAvailable.current,
    startListening,
    stopListening,
    cancelListening,
    setTranscript,
  };
}
