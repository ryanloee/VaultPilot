import React, { useState, useRef, useEffect, useCallback } from 'react';
import {
  View, Text, TouchableOpacity, ActivityIndicator, StyleSheet, Alert,
  KeyboardAvoidingView, Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Haptics from 'expo-haptics';
import * as ImagePicker from 'expo-image-picker';
import * as DocumentPicker from 'expo-document-picker';
import * as FileSystem from 'expo-file-system';
import { useAppStore, getColors } from '../store';
import { chatWithReconnect } from '../api/client';
import { buildNoteContext, buildSystemPrompt, parseToolCalls, executeSave } from '../services/rag';
import { getMessages, addMessage, updateMessage, deleteMessage, createSession, getLatestSession } from '../db';
import type { ChatScreenProps } from '../navigation/types';
import { buildHistory, buildUserContent, formatToolCallResult, buildSavePreview, inferMime } from '../utils/chatHelpers';
import { useVoiceInput } from '../utils/useVoiceInput';
import { useNetworkState } from '../utils/networkState';
import { ChatHeader, MessageList, InputBar, OfflineBanner, ScrollToBottomButton } from '../components/chat';

interface Msg { id: string; role: 'user' | 'assistant'; content: string; streaming?: boolean; isError?: boolean; attachments?: { name: string; type: 'image' | 'file' }[]; }

/** Safe JSON.parse for message attachments — returns undefined on corrupt data. */
function safeParseAttachments(raw: string | null | undefined): { name: string; type: 'image' | 'file' }[] | undefined {
  if (!raw) return undefined;
  try { return JSON.parse(raw); } catch { return undefined; }
}
interface Attachment { name: string; uri: string; type: 'image' | 'file'; }


export default function ChatScreen({ navigation, route }: ChatScreenProps) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [input, setInput] = useState('');
  const [inputHeight, setInputHeight] = useState(0);
  const [streaming, setStreaming] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [title, setTitle] = useState('新对话');
  const [loading, setLoading] = useState(true);
  const abortRef = useRef<AbortController | null>(null);
  const msgsRef = useRef<Msg[]>([]);
  const loadSeqRef = useRef(0); // Track load sequence to prevent race conditions (#1576)
  const initialMountRef = useRef(true); // Skip nav-params effect on first mount (#1619)
  const voice = useVoiceInput();
  const { isOnline } = useNetworkState();
  const [showScrollBtn, setShowScrollBtn] = useState(false);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [scrollTrigger, setScrollTrigger] = useState(0);
  const [initError, setInitError] = useState<string | null>(null);

  // Append voice transcript to input when recognition completes
  useEffect(() => {
    if (voice.transcript && !voice.isListening) {
      setInput(prev => prev ? `${prev} ${voice.transcript}` : voice.transcript);
      voice.setTranscript('');
    }
  }, [voice.transcript, voice.isListening]);

  const pickImage = async () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const result = await ImagePicker.launchImageLibraryAsync({
      mediaTypes: ['images'],
      quality: 0.8,
      allowsEditing: false,
    });
    if (!result.canceled && result.assets[0]) {
      const asset = result.assets[0];
      setAttachments(prev => [...prev, { name: asset.fileName || 'photo.jpg', uri: asset.uri, type: 'image' }]);
    }
  };

  const takePhoto = async () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const perm = await ImagePicker.requestCameraPermissionsAsync();
    if (!perm.granted) { Alert.alert('权限不足', '需要相机权限才能拍照'); return; }
    const result = await ImagePicker.launchCameraAsync({ quality: 0.8 });
    if (!result.canceled && result.assets[0]) {
      const asset = result.assets[0];
      setAttachments(prev => [...prev, { name: asset.fileName || 'photo.jpg', uri: asset.uri, type: 'image' }]);
    }
  };

  const pickDocument = async () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const result = await DocumentPicker.getDocumentAsync({ multiple: false });
    if (!result.canceled && result.assets[0]) {
      const asset = result.assets[0];
      setAttachments(prev => [...prev, { name: asset.name, uri: asset.uri, type: 'file' }]);
    }
  };

  // Load a specific session by ID
  const loadSession = useCallback(async (sid: string, sessionTitle: string) => {
    abortRef.current?.abort();
    setInitError(null);
    const seq = ++loadSeqRef.current; // Increment sequence for race condition protection (#1576)
    const prevMsgs = msgsRef.current;
    const prevSessionId = sessionId;
    const prevTitle = title;
    setSessionId(sid);
    setTitle(sessionTitle);
    setMsgs([]);
    setInput('');
    try {
      const history = await getMessages(sid);
      if (seq !== loadSeqRef.current) return; // Stale load, discard result (#1576)
      setMsgs(history.map(m => ({
        id: m.id, role: m.role as 'user' | 'assistant', content: m.content,
        attachments: safeParseAttachments(m.attachments),
      })));
    } catch (e) {
      if (seq !== loadSeqRef.current) return; // Stale load, discard result (#1682)
      console.warn('[Chat] loadSession failed:', e);
      setSessionId(prevSessionId);
      setTitle(prevTitle);
      Alert.alert('加载会话失败', '无法读取消息记录，请重试', [
        { text: '确定', onPress: () => setMsgs(prevMsgs) },
      ]);
    }
  }, [sessionId, title]);

  // Init session — from route params or latest active session
  useEffect(() => {
    (async () => {
      const seq = ++loadSeqRef.current; // Race condition protection for direct path (#1630)
      try {
        // If navigated from SessionsScreen with specific session
        if (route.params?.sessionId) {
          await loadSession(route.params.sessionId, route.params.title || '对话');
          return;
        }
        const existing = await getLatestSession();
        if (seq !== loadSeqRef.current) return; // Stale init, discard result (#1630)
        if (existing) {
          setSessionId(existing.id);
          setTitle(existing.title);
          const history = await getMessages(existing.id);
          if (seq !== loadSeqRef.current) return; // Stale after second await (#1630)
          setMsgs(history.map(m => ({
            id: m.id, role: m.role as 'user' | 'assistant', content: m.content,
            attachments: safeParseAttachments(m.attachments),
          })));
        } else {
          const id = await createSession('新对话');
          if (seq !== loadSeqRef.current) return; // Stale after createSession (#1630)
          setSessionId(id);
        }
      } catch (e) {
        console.warn('[Chat] session init failed:', e);
        setInitError(String(e));
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  // Retry initialization after failure
  const retryInit = useCallback(async () => {
    setInitError(null);
    setLoading(true);
    const seq = ++loadSeqRef.current; // Race condition protection for retry (#1725)
    try {
      if (route.params?.sessionId) {
        await loadSession(route.params.sessionId, route.params.title || '对话');
      } else {
        const existing = await getLatestSession();
        if (seq !== loadSeqRef.current) return; // Stale retry, discard result (#1725)
        if (existing) {
          setSessionId(existing.id);
          setTitle(existing.title);
          const history = await getMessages(existing.id);
          if (seq !== loadSeqRef.current) return; // Stale after second await (#1725)
          setMsgs(history.map(m => ({
            id: m.id, role: m.role as 'user' | 'assistant', content: m.content,
            attachments: safeParseAttachments(m.attachments),
          })));
        } else {
          const id = await createSession('新对话');
          if (seq !== loadSeqRef.current) return; // Stale after createSession (#1725)
          setSessionId(id);
        }
      }
    } catch (e) {
      if (seq !== loadSeqRef.current) return; // Stale error, discard (#1725)
      console.warn('[Chat] session init retry failed:', e);
      setInitError(String(e));
    } finally {
      if (seq === loadSeqRef.current) setLoading(false); // Only clear loading if still current (#1725)
    }
  }, [route.params?.sessionId, route.params?.title, loadSession]);

  // Keep ref in sync with state so send() reads latest messages
  useEffect(() => { msgsRef.current = msgs; }, [msgs]);

  // Handle navigation params when returning from SessionsScreen
  useEffect(() => {
    // Skip on first mount — init effect already handles route.params.sessionId (#1619)
    if (initialMountRef.current) {
      initialMountRef.current = false;
      return;
    }
    if (route.params?.sessionId && route.params.sessionId !== sessionId) {
      loadSession(route.params.sessionId, route.params.title || '对话');
      // Clear params to prevent infinite retry loop on failure (#1683)
      navigation.setParams({ sessionId: undefined, title: undefined });
    }
  }, [route.params?.sessionId, sessionId, loadSession]);

  // Handle prefillText from NoteEditor AI assistant
  useEffect(() => {
    if (route.params?.prefillText) {
      setInput(route.params.prefillText);
      // Clear the param so it doesn't re-trigger
      navigation.setParams({ prefillText: undefined });
    }
  }, [route.params?.prefillText]);

  // Abort any in-flight stream on unmount
  useEffect(() => {
    return () => {
      abortRef.current?.abort();
    };
  }, []);

  const send = useCallback(async () => {
    if ((!input.trim() && attachments.length === 0) || streaming || !sessionId) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    const userText = input.trim();
    const currentAttachments = [...attachments];

    // Read attachments as base64 before clearing
    const attData: { base64: string; mime: string }[] = [];
    for (const att of currentAttachments) {
      try {
        const base64 = await FileSystem.readAsStringAsync(att.uri, { encoding: FileSystem.EncodingType.Base64 });
        const mime = inferMime(att.name, att.type === 'image' ? 'image/jpeg' : 'application/octet-stream');
        attData.push({ base64, mime });
      } catch (e) {
        console.warn('[Chat] Failed to read attachment:', att.name, e);
        Alert.alert('附件读取失败', `无法读取「${att.name}」，请重新选择`);
        return;
      }
    }
    const userContent = buildUserContent(userText, attData);

    // Add user message — only clear input after persistence succeeds
    let userId: string;
    let activeSessionId = sessionId;
    const attMeta = currentAttachments.map(a => ({ name: a.name, type: a.type }));
    try {
      userId = await addMessage(activeSessionId, 'user', userText, attMeta);
    } catch (e: unknown) {
      // FOREIGN KEY = session was deleted/reset; recreate and retry
      if (String(e).includes('FOREIGN KEY') || String(e).includes('constraint')) {
        try {
          const newId = await createSession('新对话');
          setSessionId(newId);
          activeSessionId = newId;
          userId = await addMessage(newId, 'user', userText, attMeta);
        } catch (e2) {
          console.warn('[Chat] addMessage retry failed:', e2);
          Alert.alert('发送失败', '无法创建对话，请重试');
          return;
        }
      } else {
        console.warn('[Chat] addMessage failed:', e);
        Alert.alert('发送失败', String(e));
        return;
      }
    }
    setInput('');
    setInputHeight(0);
    setAttachments([]);
    const userMsg: Msg = { id: userId, role: 'user', content: userText, attachments: attMeta.length > 0 ? attMeta : undefined };
    // Snapshot before state update — msgsRef may or may not be flushed by React
    // before we build the API history, so pin it here.
    const prevMsgs = [...msgsRef.current];
    setMsgs(prev => [...prev, userMsg]);

    // Save AI placeholder to DB upfront — stable id, no key change later
    let aiId: string;
    try {
      aiId = await addMessage(activeSessionId, 'assistant', '');
    } catch (e) {
      console.warn('[Chat] addMessage (assistant placeholder) failed:', e);
      Alert.alert('发送失败', '无法创建 AI 回复记录');
      return;
    }
    const aiMsg: Msg = { id: aiId, role: 'assistant', content: '', streaming: true };
    setMsgs(prev => [...prev, aiMsg]);
    setStreaming(true);

    let full = '';
    try {
      // RAG: search notes for relevant context, considering recent conversation
      const recentTexts = prevMsgs.slice(-6).map(m => m.content).filter(Boolean);
      let noteContext: string | null = null;
      try {
        noteContext = await buildNoteContext(userText, recentTexts);
      } catch (e) {
        console.warn('[Chat] buildNoteContext failed, continuing without note context:', e);
      }
      const systemPrompt = buildSystemPrompt(noteContext);

      const history = buildHistory(prevMsgs, systemPrompt, userContent);

      abortRef.current = new AbortController();

      await chatWithReconnect(history, (chunk) => {
        if (chunk.done) return;
        if (chunk.content) {
          full += chunk.content;
          setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, content: full } : m));
        }
      }, abortRef.current.signal);

      // Parse tool calls (save notes etc.) — don't execute yet
      const { cleaned, pendingSaves } = parseToolCalls(full);

      // Ask user to confirm each pending save
      const actions: string[] = [];
      for (const save of pendingSaves) {
        const confirmed = await new Promise<boolean>((resolve) => {
          Alert.alert(
            '保存笔记？',
            `AI 想要保存笔记「${save.title}」\n\n${buildSavePreview(save.content)}`,
            [
              { text: '拒绝', style: 'cancel', onPress: () => resolve(false) },
              { text: '保存', onPress: () => resolve(true) },
            ],
            { onDismiss: () => resolve(false) },
          );
        });
        if (confirmed) {
          try {
            const action = await executeSave(save);
            actions.push(action);
          } catch (e) {
            console.warn('[Chat] executeSave failed:', e);
            actions.push(`保存笔记「${save.title}」失败`);
          }
        }
      }

      const finalContent = formatToolCallResult(cleaned, actions);

      if (finalContent !== full) {
        full = finalContent;
        setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, content: full } : m));
      }

      // Persist streamed content — separate try-catch so UI content is preserved on failure
      try {
        await updateMessage(aiId, full);
      } catch (e) {
        console.warn('[Chat] Failed to persist streamed message:', e);
        Alert.alert(
          '保存失败',
          'AI 回复未能保存到本地，切换会话后将丢失。请检查存储空间后重试。',
          [{
            text: '重试',
            onPress: async () => {
              try {
                await updateMessage(aiId, full);
                // Retry succeeded: clear error marker (#1726)
                setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, isError: false } : m));
              } catch (retryErr) {
                // Retry failed: inform user instead of silently swallowing (#1726)
                console.warn('[Chat] Retry persist failed:', retryErr);
                Alert.alert('重试失败', '请检查存储空间后再次重试');
              }
            },
          }],
        );
        setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, isError: true } : m));
      }
      setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
    } catch (err: unknown) {
      const partial = full || msgsRef.current.find(m => m.id === aiId)?.content || '';
      const errMsg = err instanceof Error ? err.message : String(err);
      const errName = err instanceof Error ? err.name : '';

      if (errName === 'AbortError') {
        if (partial) {
          try { await updateMessage(aiId, partial + '\n\n_[响应被中止]_'); } catch (e) { console.warn('[Chat] Failed to save aborted message:', e); }
        } else {
          try { await deleteMessage(aiId); } catch (e) { console.warn('[Chat] Failed to delete empty aborted message:', e); }
          setMsgs(prev => prev.filter(m => m.id !== aiId));
        }
        setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
      } else {
        if (partial) {
          try { await updateMessage(aiId, partial); } catch (e) { console.warn('[Chat] Failed to save partial content:', e); }
        }
        setMsgs(prev => prev.map(m => m.id === aiId
          ? { ...m, content: m.content ? `${m.content}\n\n❌ ${errMsg}` : `❌ ${errMsg}`, streaming: false, isError: true }
          : m));
      }
    } finally {
      setStreaming(false);
      abortRef.current = null;
    }
  }, [input, streaming, sessionId, attachments]);

  // Create a new conversation
  const newChat = useCallback(async () => {
    try {
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      abortRef.current?.abort();
      const id = await createSession('新对话');
      setSessionId(id);
      setTitle('新对话');
      setMsgs([]);
    } catch (e) {
      console.warn('[Chat] newChat failed:', e);
      Alert.alert('创建失败', '无法创建新对话，请重试');
    }
  }, []);

  const stop = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy);
    abortRef.current?.abort();
  };

  const handleDeleteMsg = useCallback(async (msgId: string) => {
    try {
      await deleteMessage(msgId);
      setMsgs(prev => prev.filter(m => m.id !== msgId));
    } catch (e) {
      Alert.alert('删除失败', String(e));
    }
  }, []);

  const handleResend = useCallback(async (msgId: string) => {
    const msg = msgsRef.current.find(m => m.id === msgId);
    if (!msg || !sessionId) return;
    setInput(msg.content);
  }, [sessionId]);

  const handleScrollToEnd = useCallback(() => {
    setShowScrollBtn(false);
    setScrollTrigger(t => t + 1);
  }, []);

  if (loading) {
    return (
      <SafeAreaView style={[styles.center, { backgroundColor: c.bg }]}>
        <ActivityIndicator color={accentColor} size="large" />
      </SafeAreaView>
    );
  }

  if (initError) {
    return (
      <SafeAreaView style={[styles.center, { backgroundColor: c.bg }]}>
        <Text style={{ color: c.text, fontSize: 16, textAlign: 'center', marginBottom: 8 }}>
          初始化会话失败
        </Text>
        <Text style={{ color: c.textSecondary, fontSize: 13, textAlign: 'center', marginBottom: 20, paddingHorizontal: 32 }}>
          {initError}
        </Text>
        <TouchableOpacity
          style={[styles.retryBtn, { backgroundColor: accentColor }]}
          onPress={retryInit}
        >
          <Text style={{ color: '#FFF', fontSize: 15, fontWeight: '600' }}>重试</Text>
        </TouchableOpacity>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView style={{ flex: 1, backgroundColor: c.bg }}>
      <KeyboardAvoidingView
        style={{ flex: 1 }}
        behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
        keyboardVerticalOffset={0}
      >
      <ChatHeader
        title={title}
        accentColor={accentColor}
        borderColor={c.border}
        textColor={c.text}
        onSessionsPress={() => navigation.navigate('Sessions')}
        onNewChatPress={newChat}
      />

      <OfflineBanner visible={!isOnline} isDark={isDark} />

      <MessageList
        messages={msgs}
        isDark={isDark}
        accentColor={accentColor}
        textColor={c.text}
        textColorSecondary={c.textSecondary}
        borderColor={c.border}
        onDeleteMessage={handleDeleteMsg}
        onResendMessage={handleResend}
        onScrollToEnd={handleScrollToEnd}
        onNearBottomChange={(near) => setShowScrollBtn(!near)}
        scrollTrigger={scrollTrigger}
        onSuggestion={(text) => setInput(text)}
      />

      <ScrollToBottomButton
        visible={showScrollBtn}
        accentColor={accentColor}
        bgColor={c.inputBg}
        borderColor={c.border}
        onPress={handleScrollToEnd}
      />

      <InputBar
        input={input}
        inputHeight={inputHeight}
        streaming={streaming}
        attachments={attachments}
        accentColor={accentColor}
        bgColor={c.bg}
        inputBgColor={c.inputBg}
        textColor={c.text}
        textColorSecondary={c.textSecondary}
        borderColor={c.border}
        voiceAvailable={voice.isAvailable}
        voiceListening={voice.isListening}
        voiceVolume={voice.volumeLevel}
        onInputChange={setInput}
        onInputHeightChange={setInputHeight}
        onSend={send}
        onStop={stop}
        onTakePhoto={takePhoto}
        onPickImage={pickImage}
        onPickDocument={pickDocument}
        onRemoveAttachment={(index) => setAttachments(prev => prev.filter((_, i) => i !== index))}
        onVoiceToggle={() => voice.isListening ? voice.stopListening() : voice.startListening()}
        onEmojiSelect={(emoji) => setInput(prev => prev + emoji)}
      />
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  center: { flex: 1, justifyContent: 'center', alignItems: 'center' },
  retryBtn: { paddingHorizontal: 24, paddingVertical: 10, borderRadius: 8 },
});