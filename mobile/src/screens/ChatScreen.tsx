import React, { useState, useRef, useEffect, useCallback, memo, useMemo } from 'react';
import {
  View, Text, TextInput, FlatList, TouchableOpacity,
  KeyboardAvoidingView, Platform, ActivityIndicator, StyleSheet, Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useAppStore, getColors } from '../store';
import { chat, parseSSEStream, ChatMessage } from '../api/client';
import { getMessages, addMessage, updateMessage, createSession, getLatestSession } from '../db';

interface Msg { id: string; role: 'user' | 'assistant'; content: string; streaming?: boolean; isError?: boolean; }

/** Max messages sent to API to avoid exceeding model context window */
const MAX_HISTORY_MESSAGES = 50;

const MessageBubble = memo(function MessageBubble({ item, isDark, accentColor }: {
  item: Msg; isDark: boolean; accentColor: string;
}) {
  const c = getColors(isDark, accentColor);
  return (
    <View style={[s.bubble, item.role === 'user'
      ? { backgroundColor: c.userBubble, alignSelf: 'flex-end' }
      : { backgroundColor: c.aiBubble, alignSelf: 'flex-start' }]}>
      <Text style={{ color: item.role === 'user' ? c.userText : c.aiText, fontSize: 15, lineHeight: 22 }}>
        {item.content || (item.streaming ? '思考中...' : '')}
        {item.streaming && <Text style={{ color: accentColor }}> ▌</Text>}
      </Text>
    </View>
  );
});

export default function ChatScreen({ navigation }: any) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [input, setInput] = useState('');
  const [streaming, setStreaming] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const abortRef = useRef<AbortController | null>(null);
  const listRef = useRef<FlatList>(null);
  const msgsRef = useRef<Msg[]>([]);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Init session — reuse latest active session or create new one
  useEffect(() => {
    (async () => {
      try {
        const existing = await getLatestSession();
        if (existing) {
          setSessionId(existing.id);
          const history = await getMessages(existing.id);
          setMsgs(history.map(m => ({
            id: m.id, role: m.role as 'user' | 'assistant', content: m.content,
          })));
        } else {
          const id = await createSession('新对话');
          setSessionId(id);
        }
      } catch (e) {
        console.warn('[Chat] session init failed:', e);
        Alert.alert('初始化失败', String(e));
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  // Keep ref in sync with state so send() reads latest messages
  useEffect(() => { msgsRef.current = msgs; }, [msgs]);

  // Abort any in-flight stream on unmount
  useEffect(() => {
    return () => {
      abortRef.current?.abort();
      if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    };
  }, []);

  const send = useCallback(async () => {
    if (!input.trim() || streaming || !sessionId) return;
    const userText = input.trim();

    // Add user message — only clear input after persistence succeeds
    let userId: string;
    try {
      userId = await addMessage(sessionId, 'user', userText);
    } catch (e) {
      console.warn('[Chat] addMessage failed:', e);
      Alert.alert('发送失败', String(e));
      return;
    }
    setInput('');
    const userMsg: Msg = { id: userId, role: 'user', content: userText };
    // Snapshot before state update — msgsRef may or may not be flushed by React
    // before we build the API history, so pin it here.
    const prevMsgs = [...msgsRef.current];
    setMsgs(prev => [...prev, userMsg]);

    // Save AI placeholder to DB upfront — stable id, no key change later
    let aiId: string;
    try {
      aiId = await addMessage(sessionId, 'assistant', '');
    } catch (e) {
      console.warn('[Chat] addMessage (assistant placeholder) failed:', e);
      Alert.alert('发送失败', '无法创建 AI 回复记录');
      return;
    }
    const aiMsg: Msg = { id: aiId, role: 'assistant', content: '', streaming: true };
    setMsgs(prev => [...prev, aiMsg]);
    setStreaming(true);

    try {
      const history: ChatMessage[] = [
        { role: 'system', content: '你是 VaultPilot AI 助手，知识渊博、乐于助人。用中文回答。' },
        ...prevMsgs.filter(m => (m.role !== 'assistant' || !m.streaming) && !m.isError).slice(-MAX_HISTORY_MESSAGES).map(m => ({ role: m.role as any, content: m.content })),
        { role: 'user', content: userText },
      ];

      abortRef.current = new AbortController();
      const stream = await chat(history, abortRef.current.signal);
      let full = '';

      await parseSSEStream(stream, (chunk) => {
        if (chunk.done) return;
        if (chunk.content) {
          full += chunk.content;
          setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, content: full } : m));
        }
      });

      // Persist streamed content — separate try-catch so UI content is preserved on failure
      try {
        await updateMessage(aiId, full);
      } catch {
        // DB save failed; content stays in UI, user still sees the response
      }
      setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
    } catch (err: any) {
      // Save whatever partial content was received before the error
      const partial = msgsRef.current.find(m => m.id === aiId)?.content ?? '';
      if (partial) {
        try { await updateMessage(aiId, partial); } catch { /* best-effort */ }
      }
      if (err.name === 'AbortError') {
        if (partial) {
          try { await updateMessage(aiId, partial + '\n\n_[响应被中止]_'); } catch {}
        }
        setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
      } else {
        // Append error marker without discarding streamed content; mark as error to filter from API history
        setMsgs(prev => prev.map(m => m.id === aiId
          ? { ...m, content: m.content ? `${m.content}\n\n❌ ${err.message}` : `❌ ${err.message}`, streaming: false, isError: true }
          : m));
      }
    } finally {
      setStreaming(false);
      abortRef.current = null;
    }
  }, [input, streaming, sessionId]);

  // Create a new conversation
  const newChat = useCallback(async () => {
    try {
      abortRef.current?.abort();
      const id = await createSession('新对话');
      setSessionId(id);
      setMsgs([]);
    } catch (e) {
      console.warn('[Chat] newChat failed:', e);
    }
  }, []);

  const stop = () => {
    abortRef.current?.abort();
  };

  // Debounced scroll-to-end — avoids per-chunk scroll during streaming
  const scrollToEndDebounced = useCallback(() => {
    if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    scrollTimerRef.current = setTimeout(() => {
      listRef.current?.scrollToEnd({ animated: true });
    }, 100);
  }, []);

  // Also scroll when msgs change (covers new user message + AI start)
  useEffect(() => { scrollToEndDebounced(); }, [msgs.length]);

  if (loading) {
    return (
      <SafeAreaView style={[s.center, { backgroundColor: c.bg }]}>
        <ActivityIndicator color={accentColor} size="large" />
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView style={{ flex: 1, backgroundColor: c.bg }}>
    <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined} keyboardVerticalOffset={0}>
      {/* New chat button */}
      <TouchableOpacity onPress={newChat} style={[s.newChatBtn, { borderColor: c.border }]}>
        <Text style={{ color: accentColor, fontSize: 14 }}>＋ 新对话</Text>
      </TouchableOpacity>

      {/* Message list */}
      <FlatList
        ref={listRef}
        data={msgs}
        renderItem={({ item }) => <MessageBubble item={item} isDark={isDark} accentColor={accentColor} />}
        keyExtractor={item => item.id}
        contentContainerStyle={{ padding: 16, paddingBottom: 8 }}
        onContentSizeChange={scrollToEndDebounced}
        removeClippedSubviews={Platform.OS === 'android'}
        initialNumToRender={15}
        maxToRenderPerBatch={10}
        windowSize={11}
      />

      {/* Input bar */}
      <View style={[s.inputBar, { borderTopColor: c.border, backgroundColor: c.bg }]}>
        <TextInput
          style={[s.textInput, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
          value={input}
          onChangeText={setInput}
          placeholder="输入消息..."
          placeholderTextColor={c.textSecondary}
          multiline
          maxLength={4000}
          editable={!streaming}
        />
        {streaming ? (
          <TouchableOpacity onPress={stop} style={[s.sendBtn, { backgroundColor: '#EF4444' }]}>
            <Text style={s.sendText}>■</Text>
          </TouchableOpacity>
        ) : (
          <TouchableOpacity
            onPress={send}
            style={[s.sendBtn, { backgroundColor: input.trim() ? accentColor : c.border }]}
            disabled={!input.trim()}
          >
            <Text style={s.sendText}>➤</Text>
          </TouchableOpacity>
        )}
      </View>
    </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const s = StyleSheet.create({
  center: { flex: 1, justifyContent: 'center', alignItems: 'center' },
  bubble: {
    maxWidth: '80%', paddingHorizontal: 14, paddingVertical: 10,
    borderRadius: 16, marginBottom: 8,
  },
  inputBar: {
    flexDirection: 'row', alignItems: 'flex-end',
    padding: 8, borderTopWidth: 1,
  },
  textInput: {
    flex: 1, borderWidth: 1, borderRadius: 20,
    paddingHorizontal: 16, paddingVertical: 10,
    fontSize: 15, maxHeight: 100,
  },
  sendBtn: {
    width: 40, height: 40, borderRadius: 20,
    justifyContent: 'center', alignItems: 'center', marginLeft: 8,
  },
  sendText: { color: '#FFF', fontSize: 18 },
  newChatBtn: {
    alignSelf: 'center', paddingVertical: 6, paddingHorizontal: 16,
    borderWidth: 1, borderRadius: 16, marginTop: 8,
  },
});
