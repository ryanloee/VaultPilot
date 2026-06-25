/**
 * Voice input hook using expo-speech-recognition (cloud API mode).
 *
 * Manual-stop mode: user must explicitly tap the mic button to stop.
 * Auto-restart: cloud API may disconnect on silence; we restart transparently.
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
  const localeRef = useRef('zh-CN');
  const restartTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    try {
      setIsAvailable(ExpoSpeechRecognitionModule.isRecognitionAvailable());
    } catch {
      setIsAvailable(false);
    }
  }, []);

  // Stop voice recognition on unmount
  useEffect(() => {
    return () => {
      if (restartTimerRef.current) clearTimeout(restartTimerRef.current);
      try { ExpoSpeechRecognitionModule.abort(); } catch {}
    };
  }, []);

  const doStart = useCallback((locale: string) => {
    ExpoSpeechRecognitionModule.start({
      lang: locale,
      interimResults: true,
      continuous: true,
      volumeChangeEventOptions: { enabled: true, intervalMillis: 80 },
    });
  }, []);

  // ── Events ──

  useSpeechRecognitionEvent('start', () => {
    setIsListening(true);
  });

  useSpeechRecognitionEvent('result', (event) => {
    const text = event.results[0]?.transcript;
    if (text) setTranscript(text);
  });

  // Cloud API may end the session on silence — auto-restart if user didn't stop
  useSpeechRecognitionEvent('end', () => {
    if (shouldStopRef.current) {
      shouldStopRef.current = false;
      setIsListening(false);
      setVolumeLevel(0);
    } else {
      // Cloud API dropped — restart transparently after short delay
      restartTimerRef.current = setTimeout(() => {
        try { doStart(localeRef.current); } catch {}
      }, 200);
    }
  });

  useSpeechRecognitionEvent('error', (event) => {
    // Fatal errors: stop completely
    const fatal = ['not-allowed', 'service-not-allowed'];
    if (fatal.includes(event.error)) {
      setError(event.message || '语音识别失败');
      shouldStopRef.current = true; // prevent restart
      setIsListening(false);
      return;
    }
    // Transient errors (network, no-speech, timeout, etc.): auto-restart
    if (!shouldStopRef.current) {
      restartTimerRef.current = setTimeout(() => {
        try { doStart(localeRef.current); } catch {}
      }, 300);
    }
  });

  // Volume change for waveform visualization
  useSpeechRecognitionEvent('volumechange', (event) => {
    const normalized = Math.max(0, Math.min(1, (event.value + 2) / 12));
    setVolumeLevel(normalized);
  });

  // ── Public API ──

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
      localeRef.current = locale;
      doStart(locale);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : '无法启动语音识别';
      setError(msg);
    }
  }, [doStart]);

  const stopListening = useCallback(async () => {
    if (restartTimerRef.current) clearTimeout(restartTimerRef.current);
    shouldStopRef.current = true;
    try {
      await ExpoSpeechRecognitionModule.stop();
    } catch (e) {
      console.warn('[VoiceInput] stop failed:', e);
    }
  }, []);

  const cancelListening = useCallback(async () => {
    if (restartTimerRef.current) clearTimeout(restartTimerRef.current);
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
