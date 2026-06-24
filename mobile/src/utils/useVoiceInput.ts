/**
 * Voice input hook using expo-speech-recognition.
 * Provides speech-to-text for the chat input.
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
}

export function useVoiceInput() {
  const [isListening, setIsListening] = useState(false);
  const [transcript, setTranscript] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isAvailable, setIsAvailable] = useState(false);

  useEffect(() => {
    try {
      setIsAvailable(ExpoSpeechRecognitionModule.isRecognitionAvailable());
    } catch {
      setIsAvailable(false);
    }
  }, []);

  useSpeechRecognitionEvent('start', () => setIsListening(true));
  useSpeechRecognitionEvent('end', () => setIsListening(false));
  useSpeechRecognitionEvent('result', (event) => {
    const text = event.results[0]?.transcript;
    if (text) setTranscript(text);
  });
  useSpeechRecognitionEvent('error', (event) => {
    setError(event.message || '语音识别失败');
    setIsListening(false);
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
      ExpoSpeechRecognitionModule.start({
        lang: locale,
        interimResults: true,
        continuous: true,
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : '无法启动语音识别';
      setError(msg);
    }
  }, []);

  const stopListening = useCallback(async () => {
    try {
      ExpoSpeechRecognitionModule.stop();
    } catch (e) {
      console.warn('[VoiceInput] stop failed:', e);
    }
  }, []);

  const cancelListening = useCallback(async () => {
    try {
      ExpoSpeechRecognitionModule.abort();
    } catch (e) {
      console.warn('[VoiceInput] cancel failed:', e);
    }
    setIsListening(false);
    setTranscript('');
  }, []);

  return {
    isListening,
    transcript,
    error,
    isAvailable,
    startListening,
    stopListening,
    cancelListening,
    setTranscript,
  };
}
