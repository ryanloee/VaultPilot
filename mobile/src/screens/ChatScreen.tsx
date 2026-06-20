import React, { useState, useRef, useEffect, useCallback, memo } from 'react';
import {
  View, Text, TextInput, FlatList, TouchableOpacity,
  KeyboardAvoidingView, Platform, ActivityIndicator, StyleSheet, Alert,
  NativeSyntheticEvent, NativeScrollEvent,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Clipboard from 'expo-clipboard';
import * as Haptics from 'expo-haptics';
import * as ImagePicker from 'expo-image-picker';
import * as DocumentPicker from 'expo-document-picker';
import { useAppStore, getColors } from '../store';
import MarkdownPreview from '../components/MarkdownPreview';
import { chatWithReconnect, ChatMessage } from '../api/client';
import { buildNoteContext, buildSystemPrompt, parseToolCalls, executeSave } from '../services/rag';
import { getMessages, addMessage, updateMessage, deleteMessage, createSession, getLatestSession } from '../db';
import type { ChatScreenProps } from '../navigation/types';

interface Msg { id: string; role: 'user' | 'assistant'; content: string; streaming?: boolean; isError?: boolean; }
interface Attachment { name: string; uri: string; type: 'image' | 'file'; }

/** Max messages sent to API to avoid exceeding model context window */
const MAX_HISTORY_MESSAGES = 50;

const MessageBubble = memo(function MessageBubble({ item, isDark, accentColor, onDelete, onResend }: {
  item: Msg; isDark: boolean; accentColor: string;
  onDelete?: () => void; onResend?: () => void;
}) {
  const c = getColors(isDark, accentColor);
  const isAssistant = item.role === 'assistant';
  const handleLongPress = () => {
    if (!item.content) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const actions: { text: string; onPress?: () => void; style?: 'default' | 'cancel' | 'destructive' }[] = [
      { text: '复制', onPress: () => Clipboard.setStringAsync(item.content) },
    ];
    if (onResend) actions.push({ text: '重新发送', onPress: onResend });
    if (onDelete) actions.push({ text: '删除', style: 'destructive', onPress: onDelete });
    actions.push({ text: '取消', style: 'cancel' });
    Alert.alert('消息操作', '', actions);
  };
  return (
    <TouchableOpacity
      onLongPress={handleLongPress}
      activeOpacity={0.8}
      accessibilityRole="text"
      accessibilityLabel={`${item.role === 'user' ? '你的消息' : 'AI 回复'}: ${item.content?.slice(0, 50) || '思考中'}`}
    >
      <View style={[s.bubble, item.role === 'user'
        ? { backgroundColor: c.userBubble, alignSelf: 'flex-end' }
        : { backgroundColor: c.aiBubble, alignSelf: 'flex-start' }]}>
        {isAssistant && item.content ? (
          <>
            <MarkdownPreview content={item.content} textColor={c.aiText} accentColor={accentColor} isDark={isDark} />
            {item.streaming && <Text style={{ color: accentColor, fontSize: 15 }}> ▌</Text>}
          </>
        ) : (
          <Text style={{ color: item.role === 'user' ? c.userText : c.aiText, fontSize: 15, lineHeight: 22 }}>
            {item.content || (item.streaming ? '思考中...' : '')}
            {item.streaming && <Text style={{ color: accentColor }}> ▌</Text>}
          </Text>
        )}
      </View>
    </TouchableOpacity>
  );
});

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
  const listRef = useRef<FlatList>(null);
  const msgsRef = useRef<Msg[]>([]);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const nearBottomRef = useRef(true);
  const [showScrollBtn, setShowScrollBtn] = useState(false);
  const [attachments, setAttachments] = useState<Attachment[]>([]);

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
    setSessionId(sid);
    setTitle(sessionTitle);
    setMsgs([]);
    setInput('');
    try {
      const history = await getMessages(sid);
      setMsgs(history.map(m => ({
        id: m.id, role: m.role as 'user' | 'assistant', content: m.content,
      })));
    } catch (e) {
      console.warn('[Chat] loadSession failed:', e);
    }
  }, []);

  // Init session — from route params or latest active session
  useEffect(() => {
    (async () => {
      try {
        // If navigated from SessionsScreen with specific session
        if (route.params?.sessionId) {
          await loadSession(route.params.sessionId, route.params.title || '对话');
          return;
        }
        const existing = await getLatestSession();
        if (existing) {
          setSessionId(existing.id);
          setTitle(existing.title);
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

  // Handle navigation params when returning from SessionsScreen
  useEffect(() => {
    if (route.params?.sessionId && route.params.sessionId !== sessionId) {
      loadSession(route.params.sessionId, route.params.title || '对话');
    }
  }, [route.params?.sessionId]);

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
      if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    };
  }, []);

  const send = useCallback(async () => {
    if (!input.trim() || streaming || !sessionId) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    const userText = input.trim();

    // Add user message — only clear input after persistence succeeds
    let userId: string;
    let activeSessionId = sessionId;
    try {
      userId = await addMessage(activeSessionId, 'user', userText);
    } catch (e: unknown) {
      // FOREIGN KEY = session was deleted/reset; recreate and retry
      if (String(e).includes('FOREIGN KEY') || String(e).includes('constraint')) {
        try {
          const newId = await createSession('新对话');
          setSessionId(newId);
          activeSessionId = newId;
          userId = await addMessage(newId, 'user', userText);
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
    const userMsg: Msg = { id: userId, role: 'user', content: userText };
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

    try {
      // RAG: search notes for relevant context before sending
      const noteContext = await buildNoteContext(userText);
      const systemPrompt = buildSystemPrompt(noteContext);

      const history: ChatMessage[] = [
        { role: 'system', content: systemPrompt },
        ...prevMsgs.filter(m => (m.role !== 'assistant' || !m.streaming) && !m.isError).slice(-MAX_HISTORY_MESSAGES).map(m => ({ role: m.role as 'user' | 'assistant', content: m.content })),
        { role: 'user', content: userText },
      ];

      abortRef.current = new AbortController();
      let full = '';

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
            `AI 想要保存笔记「${save.title}」\n\n${save.content.slice(0, 200)}${save.content.length > 200 ? '...' : ''}`,
            [
              { text: '拒绝', style: 'cancel', onPress: () => resolve(false) },
              { text: '保存', onPress: () => resolve(true) },
            ],
          );
        });
        if (confirmed) {
          try {
            const action = await executeSave(save);
            actions.push(action);
          } catch (e) {
            actions.push(`保存笔记「${save.title}」失败`);
          }
        }
      }

      const finalContent = actions.length > 0
        ? cleaned + '\n\n_' + actions.join('；') + '_'
        : cleaned;

      if (finalContent !== full) {
        full = finalContent;
        setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, content: full } : m));
      }

      // Persist streamed content — separate try-catch so UI content is preserved on failure
      try {
        await updateMessage(aiId, full);
      } catch {
        // DB save failed; content stays in UI, user still sees the response
      }
      setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
    } catch (err: unknown) {
      // Save whatever partial content was received before the error
      const partial = msgsRef.current.find(m => m.id === aiId)?.content ?? '';
      if (partial) {
        try { await updateMessage(aiId, partial); } catch { /* best-effort */ }
      }
      const errMsg = err instanceof Error ? err.message : String(err);
      const errName = err instanceof Error ? err.name : '';
      if (errName === 'AbortError') {
        if (partial) {
          try { await updateMessage(aiId, partial + '\n\n_[响应被中止]_'); } catch {}
        }
        setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
      } else {
        // Append error marker without discarding streamed content; mark as error to filter from API history
        setMsgs(prev => prev.map(m => m.id === aiId
          ? { ...m, content: m.content ? `${m.content}\n\n❌ ${errMsg}` : `❌ ${errMsg}`, streaming: false, isError: true }
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
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      abortRef.current?.abort();
      const id = await createSession('新对话');
      setSessionId(id);
      setTitle('新对话');
      setMsgs([]);
    } catch (e) {
      console.warn('[Chat] newChat failed:', e);
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

  // Debounced scroll-to-end — only when user is near bottom
  const scrollToEndDebounced = useCallback((force = false) => {
    if (!force && !nearBottomRef.current) return;
    if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    scrollTimerRef.current = setTimeout(() => {
      listRef.current?.scrollToEnd({ animated: true });
    }, 100);
  }, []);

  // Wrapper for onContentSizeChange (ignores width/height params)
  const onContentSizeChange = useCallback(() => { scrollToEndDebounced(); }, [scrollToEndDebounced]);

  // Track whether user is near the bottom
  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    const distanceFromBottom = contentSize.height - layoutMeasurement.height - contentOffset.y;
    const near = distanceFromBottom < 120;
    nearBottomRef.current = near;
    setShowScrollBtn(!near && msgs.length > 0);
  }, [msgs.length]);

  // Scroll when new messages arrive (user sends → force scroll)
  useEffect(() => { scrollToEndDebounced(true); }, [msgs.length]);

  if (loading) {
    return (
      <SafeAreaView style={[s.center, { backgroundColor: c.bg }]}>
        <ActivityIndicator color={accentColor} size="large" />
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView style={{ flex: 1, backgroundColor: c.bg }}>
    <KeyboardAvoidingView style={{ flex: 1 }} behavior="height" keyboardVerticalOffset={0}>
      {/* Header */}
      <View style={[s.header, { borderBottomColor: c.border }]}>
        <TouchableOpacity onPress={() => navigation.navigate('Sessions')} style={s.sessionsBtn}
          accessibilityRole="button" accessibilityLabel="打开对话列表">
          <Text style={{ color: accentColor, fontSize: 14 }}>☰ 对话</Text>
        </TouchableOpacity>
        <Text style={[s.titleText, { color: c.text }]} numberOfLines={1}
          accessibilityRole="header">{title}</Text>
        <TouchableOpacity onPress={newChat} style={[s.newChatBtn, { borderColor: c.border }]}
          accessibilityRole="button" accessibilityLabel="新建对话">
          <Text style={{ color: accentColor, fontSize: 14 }}>＋</Text>
        </TouchableOpacity>
      </View>

      {/* Message list */}
      <FlatList
        ref={listRef}
        data={msgs}
        renderItem={({ item }) => (
          <MessageBubble
            item={item}
            isDark={isDark}
            accentColor={accentColor}
            onDelete={() => handleDeleteMsg(item.id)}
            onResend={item.role === 'user' ? () => handleResend(item.id) : undefined}
          />
        )}
        keyExtractor={item => item.id}
        contentContainerStyle={{ padding: 16, paddingBottom: 8 }}
        onContentSizeChange={() => scrollToEndDebounced()}
        onScroll={onScroll}
        scrollEventThrottle={16}
        removeClippedSubviews={Platform.OS === 'android'}
        initialNumToRender={15}
        maxToRenderPerBatch={10}
        windowSize={11}
        ListEmptyComponent={
          <View style={s.emptyContainer}>
            <Text style={[s.emptyTitle, { color: c.text }]}>👋 你好，我是 VaultPilot AI</Text>
            <Text style={[s.emptySubtitle, { color: c.textSecondary }]}>有什么可以帮你的？试试这些问题：</Text>
            {['帮我总结一篇笔记', '解释一下这个概念', '写一段代码'].map((q) => (
              <TouchableOpacity key={q} style={[s.suggestionBtn, { borderColor: c.border }]} onPress={() => setInput(q)}
                accessibilityRole="button" accessibilityLabel={`使用建议: ${q}`}>
                <Text style={[s.suggestionText, { color: accentColor }]}>{q}</Text>
              </TouchableOpacity>
            ))}
          </View>
        }
      />

      {showScrollBtn && (
        <TouchableOpacity
          onPress={() => { nearBottomRef.current = true; setShowScrollBtn(false); listRef.current?.scrollToEnd({ animated: true }); }}
          style={[s.scrollBtn, { backgroundColor: c.inputBg, borderColor: c.border }]}
          accessibilityRole="button" accessibilityLabel="滚动到底部"
        >
          <Text style={{ color: accentColor, fontSize: 16 }}>↓</Text>
        </TouchableOpacity>
      )}

      {/* Attachment preview chips */}
      {attachments.length > 0 && (
        <View style={[s.attachRow, { backgroundColor: c.bgSecondary }]}>
          {attachments.map((att, i) => (
            <View key={i} style={[s.attachChip, { borderColor: c.border }]}>
              <Text style={[s.attachName, { color: c.text }]} numberOfLines={1}>
                {att.type === 'image' ? '🖼' : '📄'} {att.name}
              </Text>
              <TouchableOpacity onPress={() => setAttachments(prev => prev.filter((_, j) => j !== i))}>
                <Text style={{ color: '#EF4444', fontSize: 14 }}>✕</Text>
              </TouchableOpacity>
            </View>
          ))}
        </View>
      )}

      {/* Input bar */}
      <View style={[s.inputBar, { borderTopColor: c.border, backgroundColor: c.bg }]}>
        <TouchableOpacity
          style={[s.attachBtn, { borderColor: c.border }]}
          onPress={() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            Alert.alert('添加附件', '', [
              { text: '📷 拍照', onPress: takePhoto },
              { text: '🖼 相册', onPress: pickImage },
              { text: '📄 文件', onPress: pickDocument },
              { text: '取消', style: 'cancel' },
            ]);
          }}
        >
          <Text style={{ color: c.textSecondary, fontSize: 18 }}>📎</Text>
        </TouchableOpacity>
        <TextInput
          style={[s.textInput, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border, height: Math.max(40, Math.min(inputHeight, 120)) }]}
          value={input}
          onChangeText={setInput}
          onContentSizeChange={e => setInputHeight(e.nativeEvent.contentSize.height)}
          placeholder="输入消息..."
          placeholderTextColor={c.textSecondary}
          maxLength={4000}
          editable={!streaming}
          returnKeyType="send"
          blurOnSubmit
          onSubmitEditing={send}
          accessibilityLabel="消息输入框"
        />
        {streaming ? (
          <TouchableOpacity onPress={stop} style={[s.sendBtn, { backgroundColor: '#EF4444' }]}
            accessibilityRole="button" accessibilityLabel="停止生成">
            <Text style={s.sendText}>■</Text>
          </TouchableOpacity>
        ) : (
          <TouchableOpacity
            onPress={send}
            style={[s.sendBtn, { backgroundColor: (input.trim() || attachments.length > 0) ? accentColor : c.border }]}
            disabled={!input.trim() && attachments.length === 0}
            accessibilityRole="button" accessibilityLabel="发送消息"
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
  attachBtn: {
    width: 40, height: 40, borderRadius: 20,
    justifyContent: 'center', alignItems: 'center',
    marginRight: 6,
  },
  attachRow: {
    flexDirection: 'row', flexWrap: 'wrap', gap: 6,
    paddingHorizontal: 12, paddingVertical: 6,
  },
  attachChip: {
    flexDirection: 'row', alignItems: 'center', gap: 6,
    paddingHorizontal: 10, paddingVertical: 4,
    borderRadius: 12, borderWidth: 1,
  },
  attachName: { fontSize: 12, maxWidth: 150 },
  textInput: {
    flex: 1, borderWidth: 1, borderRadius: 20,
    paddingHorizontal: 16, paddingVertical: 10,
    fontSize: 15,
  },
  sendBtn: {
    width: 40, height: 40, borderRadius: 20,
    justifyContent: 'center', alignItems: 'center', marginLeft: 8,
  },
  sendText: { color: '#FFF', fontSize: 18 },
  newChatBtn: {
    paddingVertical: 6, paddingHorizontal: 12,
    borderWidth: 1, borderRadius: 16,
  },
  header: {
    flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between',
    paddingHorizontal: 16, paddingVertical: 8, borderBottomWidth: 1,
  },
  sessionsBtn: {
    paddingVertical: 6, paddingHorizontal: 10,
  },
  titleText: {
    fontSize: 17, fontWeight: '600', flex: 1, textAlign: 'center',
  },
  emptyContainer: {
    flex: 1, justifyContent: 'center', alignItems: 'center',
    paddingTop: 80, paddingHorizontal: 32,
  },
  emptyTitle: { fontSize: 22, fontWeight: '700', marginBottom: 8 },
  emptySubtitle: { fontSize: 15, marginBottom: 20, textAlign: 'center' },
  suggestionBtn: {
    borderWidth: 1, borderRadius: 12, paddingVertical: 10, paddingHorizontal: 16,
    marginBottom: 8, alignSelf: 'stretch',
  },
  suggestionText: { fontSize: 15 },
  scrollBtn: {
    position: 'absolute', bottom: 70, alignSelf: 'center',
    width: 36, height: 36, borderRadius: 18,
    justifyContent: 'center', alignItems: 'center',
    borderWidth: 1, elevation: 2,
    shadowColor: '#000', shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.15, shadowRadius: 2,
  },
});
