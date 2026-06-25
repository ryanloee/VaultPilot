/**
 * Voice input hook using expo-speech-recognition.
 * Provides speech-to-text for the chat input.
 *
 * Manual-stop mode: user must explicitly tap the mic button to stop.
 * Exposes volumeLevel (0-1) for waveform visualization.
 */
import { useState, useRef, useCallback, useEffect } from 'react';
import { Alert } from 'react-native';
import {
  ExpoSpeechRecognitionModule,
  useSpeechRecognitionEvent,
} from 'expo-speech-recognition';

export interface VoiceInputState {
  isListening: boolean;
  transcript: string;
  error: string | null;
  isAvailable: boolean;
  volumeLevel: number; // 0-1 normalized
}

export function useVoiceInput() {
  const [isListening, setIsListening] = useState(false);
  const [transcript, setTranscript] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isAvailable, setIsAvailable] = useState(false);
  const [volumeLevel, setVolumeLevel] = useState(0);
  const shouldStopRef = useRef(false); // true only when user explicitly stops

  useEffect(() => {
    try {
      setIsAvailable(ExpoSpeechRecognitionModule.isRecognitionAvailable());
    } catch {
      setIsAvailable(false);
    }
  }, []);

  // Stop voice recognition on unmount to release microphone (#1667)
  useEffect(() => {
    return () => {
      try {
        ExpoSpeechRecognitionModule.abort();
      } catch {
        // ignore cleanup errors
      }
    };
  }, []);

  useSpeechRecognitionEvent('start', () => setIsListening(true));

  useSpeechRecognitionEvent('end', () => {
    if (shouldStopRef.current) {
      shouldStopRef.current = false;
    }
    setIsListening(false);
    setVolumeLevel(0);
  });

  useSpeechRecognitionEvent('result', (event) => {
    const text = event.results[0]?.transcript;
    if (text) setTranscript(text);
  });

  useSpeechRecognitionEvent('error', (event) => {
    // Don't stop on minor errors; only stop on fatal ones
    const fatal = ['not-allowed', 'service-not-allowed', 'audio-capture'];
    if (fatal.includes(event.error)) {
      setError(event.message || '语音识别失败');
      setIsListening(false);
    }
  });

  // Volume change events for waveform visualization
  useSpeechRecognitionEvent('volumechange', (event) => {
    // value ranges from -2 to 10; normalize to 0-1
    const normalized = Math.max(0, Math.min(1, (event.value + 2) / 12));
    setVolumeLevel(normalized);
  });

  const startListening = useCallback(async (locale = 'zh-CN') => {
    try {
      const perm = await ExpoSpeechRecognitionModule.requestPermissionsAsync();
      if (!perm.granted) {
        Alert.alert('提示', '需要麦克风权限才能使用语音输入');
        return;
      }
      setError(null);
      setTranscript('');
      shouldStopRef.current = false;
      ExpoSpeechRecognitionModule.start({
        lang: locale,
        interimResults: true,
        continuous: true,
        volumeChangeEventOptions: {
          enabled: true,
          intervalMillis: 80, // smooth waveform updates
        },
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : '无法启动语音识别';
      setError(msg);
    }
  }, []);

  const stopListening = useCallback(async () => {
    shouldStopRef.current = true;
    try {
      await ExpoSpeechRecognitionModule.stop();
    } catch (e) {
      console.warn('[VoiceInput] stop failed:', e);
    }
  }, []);

  const cancelListening = useCallback(async () => {
    shouldStopRef.current = true;
    try {
      await ExpoSpeechRecognitionModule.abort();
    } catch (e) {
      console.warn('[VoiceInput] cancel failed:', e);
    }
    setIsListening(false);
    setTranscript('');
    setVolumeLevel(0);
  }, []);

  return {
    isListening,
    transcript,
    error,
    isAvailable,
    volumeLevel,
    startListening,
    stopListening,
    cancelListening,
    setTranscript,
  };
}
