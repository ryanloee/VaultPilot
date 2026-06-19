import React, { useState, useRef, useEffect, useCallback } from 'react';
import {
  View, Text, TextInput, FlatList, TouchableOpacity,
  KeyboardAvoidingView, Platform, ActivityIndicator, StyleSheet,
} from 'react-native';
import { useAppStore, getColors } from '../store';
import { chat, parseSSEStream, ChatMessage } from '../api/client';
import { getMessages, addMessage, updateMessage, createSession } from '../db';

interface Msg { id: string; role: 'user' | 'assistant'; content: string; streaming?: boolean; }

export default function ChatScreen({ navigation }: any) {
  const { isDark, accentColor, apiBase, apiKey, model } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [input, setInput] = useState('');
  const [streaming, setStreaming] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const abortRef = useRef<AbortController | null>(null);
  const listRef = useRef<FlatList>(null);
  const msgsRef = useRef<Msg[]>([]);

  // Init session
  useEffect(() => {
    (async () => {
      const id = await createSession('新对话');
      setSessionId(id);
      setLoading(false);
    })();
  }, []);

  // Keep ref in sync with state so send() reads latest messages
  useEffect(() => { msgsRef.current = msgs; }, [msgs]);

  const send = useCallback(async () => {
    if (!input.trim() || streaming || !sessionId) return;
    const userText = input.trim();
    setInput('');

    // Add user message
    const userId = await addMessage(sessionId, 'user', userText);
    const userMsg: Msg = { id: userId, role: 'user', content: userText };
    setMsgs(prev => [...prev, userMsg]);

    // Prepare AI message placeholder
    const aiId = `stream-${Date.now()}`;
    const aiMsg: Msg = { id: aiId, role: 'assistant', content: '', streaming: true };
    setMsgs(prev => [...prev, aiMsg]);
    setStreaming(true);

    try {
      const history: ChatMessage[] = [
        { role: 'system', content: '你是 VaultPilot AI 助手，知识渊博、乐于助人。用中文回答。' },
        ...msgsRef.current.filter(m => m.role !== 'assistant' || !m.streaming).map(m => ({ role: m.role as any, content: m.content })),
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

      // Save AI message to DB
      const savedId = await addMessage(sessionId, 'assistant', full);
      setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, id: savedId, streaming: false } : m));
    } catch (err: any) {
      if (err.name === 'AbortError') {
        if (full) {
          try { await addMessage(sessionId, 'assistant', full + '\n\n_[响应被中止]_'); } catch {}
        }
        setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
      } else {
        setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, content: `❌ ${err.message}`, streaming: false } : m));
      }
    } finally {
      setStreaming(false);
      abortRef.current = null;
    }
  }, [input, streaming, sessionId, apiKey, apiBase, model]);

  const stop = () => {
    abortRef.current?.abort();
  };

  if (loading) {
    return (
      <View style={[s.center, { backgroundColor: c.bg }]}>
        <ActivityIndicator color={accentColor} size="large" />
      </View>
    );
  }

  const renderMsg = ({ item }: { item: Msg }) => (
    <View style={[s.bubble, item.role === 'user'
      ? { backgroundColor: c.userBubble, alignSelf: 'flex-end' }
      : { backgroundColor: c.aiBubble, alignSelf: 'flex-start' }]}>
      <Text style={{ color: item.role === 'user' ? c.userText : c.aiText, fontSize: 15, lineHeight: 22 }}>
        {item.content || (item.streaming ? '思考中...' : '')}
        {item.streaming && <Text style={{ color: accentColor }}> ▌</Text>}
      </Text>
    </View>
  );

  return (
    <KeyboardAvoidingView style={{ flex: 1, backgroundColor: c.bg }} behavior={Platform.OS === 'ios' ? 'padding' : undefined} keyboardVerticalOffset={90}>
      {/* Message list */}
      <FlatList
        ref={listRef}
        data={msgs}
        renderItem={renderMsg}
        keyExtractor={item => item.id}
        contentContainerStyle={{ padding: 16, paddingBottom: 8 }}
        onContentSizeChange={() => listRef.current?.scrollToEnd({ animated: true })}
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
});
