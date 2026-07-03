import React, { useState, useRef, useEffect, useCallback, memo } from 'react';
import {
  View, Text, TextInput, FlatList, TouchableOpacity,
  KeyboardAvoidingView, Platform, ActivityIndicator, StyleSheet, Alert,
  NativeSyntheticEvent, NativeScrollEvent,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Clipboard from 'expo-clipboard';
import * as Haptics from 'expo-haptics';
import Ionicons from '@expo/vector-icons/Ionicons';
import { useAppStore, getColors } from '../store';
import { chatWithReconnect, ChatMessage } from '../api/client';
import { buildNoteContext, buildSystemPrompt, executeToolCalls, ResponseStyle, RESPONSE_STYLE_LABELS } from '../services/rag';
import { getMessages, addMessage, updateMessage, deleteMessage, createSession, getLatestSession, getNoteTitleMap } from '../db';
import { loadNoteTitleMap, clearNoteTitleCache } from '../utils/noteRefs';
import { MessageBubble } from '../components/chat';

interface Msg { id: string; role: 'user' | 'assistant'; content: string; streaming?: boolean; isError?: boolean; streamStatus?: string; attachments?: { name: string; type: 'image' | 'file' }[]; }

/** Max messages sent to API to avoid exceeding model context window */
const MAX_HISTORY_MESSAGES = 50;

export default function ChatScreen({ navigation, route }: any) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [input, setInput] = useState('');
  const [inputHeight, setInputHeight] = useState(0);
  const [streaming, setStreaming] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [title, setTitle] = useState('新对话');
  const [loading, setLoading] = useState(true);
  const [noteTitleMap, setNoteTitleMap] = useState<Map<string, string> | undefined>(undefined);
  const noteTitleMapRef = useRef(false);
  const abortRef = useRef<AbortController | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const listRef = useRef<FlatList>(null);
  const msgsRef = useRef<Msg[]>([]);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const nearBottomRef = useRef(true);
  const [showScrollBtn, setShowScrollBtn] = useState(false);
  const [responseStyle, setResponseStyle] = useState<ResponseStyle>('standard');
  const activeLoadRef = useRef<string | null>(null);
  const isSendingRef = useRef(false);
  const routeHandledRef = useRef(false);
  const sessionIdRef = useRef<string | null>(null);

  // Load a specific session by ID
  const loadSession = useCallback(async (sid: string, sessionTitle: string) => {
    abortRef.current?.abort();
    activeLoadRef.current = sid;
    setSessionId(sid);
    setTitle(sessionTitle);
    setMsgs([]);
    setShowScrollBtn(false);
    setInput('');
    setInputHeight(0);
    try {
      const history = await getMessages(sid);
      // Guard: if a newer loadSession call has started, discard stale result
      if (activeLoadRef.current !== sid) return;
      // Merge DB history with any messages send() may have added during the
      // await window. Using an updater (instead of overwriting) preserves
      // in-flight user/AI messages that are not yet in the DB (#2101).
      const dbMsgs = history.map(m => ({
        id: m.id, role: m.role as 'user' | 'assistant', content: m.content,
      }));
      const dbIds = new Set(dbMsgs.map(m => m.id));
      setMsgs(prev => {
        // Messages present in state but absent from DB = pending (sending) msgs
        const pending = prev.filter(m => !dbIds.has(m.id));
        return [...dbMsgs, ...pending];
      });
    } catch (e) {
      if (activeLoadRef.current !== sid) return;
      console.warn('[Chat] loadSession failed:', e);
      Alert.alert('加载失败', '无法加载对话记录，请重试');
    }
  }, []);

  // #2035: Resolve [[wikilink]] note title → navigate to NoteEditor
  const handleNoteLinkPress = useCallback(async (title: string) => {
    try {
      const titleMap = await getNoteTitleMap();
      const noteId = titleMap.get(title.toLowerCase());
      if (noteId) {
        navigation.navigate('Notes', { screen: 'NoteEdit', params: { noteId } });
      } else {
        // Note not found — show a brief alert so user knows the link isn't broken
        Alert.alert('未找到笔记', `"${title}" 在 vault 中不存在。`);
      }
    } catch (e) {
      console.warn('[Chat] handleNoteLinkPress failed:', e);
    }
  }, [navigation]);

  // Init session — from route params or latest active session
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // If navigated from SessionsScreen with specific session
        if (route.params?.sessionId) {
          routeHandledRef.current = true;
          await loadSession(route.params.sessionId, route.params.title || '对话');
          return;
        }
        const existing = await getLatestSession();
        if (cancelled) return;
        if (existing) {
          setSessionId(existing.id);
          setTitle(existing.title);
          const history = await getMessages(existing.id);
          if (cancelled) return;
          setMsgs(history.map(m => ({
            id: m.id, role: m.role as 'user' | 'assistant', content: m.content,
          })));
        } else {
          const id = await createSession('新对话');
          if (cancelled) return;
          setSessionId(id);
        }
      } catch (e) {
        if (cancelled) return;
        console.warn('[Chat] session init failed:', e);
        Alert.alert('初始化失败', String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  // #2035: Load note title map for auto-detection of note references
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const map = await loadNoteTitleMap();
        if (!cancelled) {
          setNoteTitleMap(map);
          noteTitleMapRef.current = true;
        }
      } catch (e) {
        console.warn('[Chat] loadNoteTitleMap failed:', e);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  // Keep ref in sync with state so send() reads latest messages
  useEffect(() => { msgsRef.current = msgs; }, [msgs]);

  // Keep a ref of the latest sessionId to avoid stale closure in effect
  sessionIdRef.current = sessionId;
  useEffect(() => { sessionIdRef.current = sessionId; }, [sessionId]);

  // Handle navigation params when returning from SessionsScreen
  // NOTE: sessionId intentionally NOT in dependency array — its change would
  // cause the effect to re-fire with stale route.params, re-loading the old session.
  // Instead we use sessionIdRef for comparison inside the effect.
  useEffect(() => {
    if (route.params?.sessionId && route.params.sessionId !== sessionIdRef.current && !routeHandledRef.current) {
      loadSession(route.params.sessionId, route.params.title || '对话');
    }
    routeHandledRef.current = false;
  }, [route.params?.sessionId, loadSession]);

  // Abort any in-flight stream on unmount
  useEffect(() => {
    return () => {
      abortRef.current?.abort();
      if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, []);

  const send = useCallback(async () => {
    if (!input.trim() || streaming || !sessionId || isSendingRef.current) return;
    isSendingRef.current = true;
    try {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium).catch(e => console.warn('[Haptics] error:', e));
    const userText = input.trim();

    // Check session consistency after await — if user navigated away, abort
    const checkSessionAlive = (expectedSessionId: string): boolean => {
      const current = sessionIdRef.current;
      if (current !== expectedSessionId) {
        console.warn('[Chat] session changed from', expectedSessionId, 'to', current, '— aborting send');
        return false;
      }
      return true;
    };

    // Add user message — only clear input after persistence succeeds
    let userId: string;
    let activeSessionId = sessionId;
    try {
      userId = await addMessage(activeSessionId, 'user', userText);
      if (!checkSessionAlive(activeSessionId)) return;
    } catch (e: any) {
      // FOREIGN KEY = session was deleted/reset; recreate and retry
      if (String(e).includes('FOREIGN KEY') || String(e).includes('constraint')) {
        try {
          const newId = await createSession('新对话');
          activeSessionId = newId;
          userId = await addMessage(newId, 'user', userText);
          if (!checkSessionAlive(newId)) return;
          // Only update UI state after message persistence succeeds (#2053)
          setSessionId(newId);
          setTitle('新对话');
          setMsgs([]);
          msgsRef.current = [];
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

    // --- GUARD: session was switched while user message was being persisted? ---
    if (sessionIdRef.current !== activeSessionId) {
      // User navigated to another session mid-send; delete orphaned user message and bail
      try { await deleteMessage(userId); } catch (_) { /* best-effort cleanup */ }
      return;
    }

    setInput('');
    setInputHeight(0);
    const userMsg: Msg = { id: userId, role: 'user', content: userText };
    // Snapshot before state update — msgsRef may or may not be flushed by React
    // before we build the API history, so pin it here.
    const prevMsgs = [...msgsRef.current];
    setMsgs(prev => [...prev, userMsg]);

    // Save AI placeholder to DB upfront — stable id, no key change later
    let aiId: string;
    try {
      aiId = await addMessage(activeSessionId, 'assistant', '');
      if (!checkSessionAlive(activeSessionId)) {
        // Clean up: remove user message and AI placeholder from old session
        try { await deleteMessage(aiId); } catch (_) {}
        try { await deleteMessage(userId); } catch (_) {}
        setInput(userText);
        setMsgs(prev => prev.filter(m => m.id !== userId && m.id !== aiId));
        return;
      }
    } catch (e) {
      console.warn('[Chat] addMessage (assistant placeholder) failed:', e);
      // Roll back UI: restore user input and remove the user message bubble
      setInput(userText);
      setMsgs(prev => prev.filter(m => m.id !== userId));
      Alert.alert('发送失败', '无法创建 AI 回复记录');
      return;
    }

    // --- GUARD: session was switched while AI placeholder was being created? ---
    if (sessionIdRef.current !== activeSessionId) {
      // Session switched; clean up both orphaned messages
      try { await deleteMessage(userId); } catch (_) { /* best-effort cleanup */ }
      try { await deleteMessage(aiId); } catch (_) { /* best-effort cleanup */ }
      return;
    }

    const aiMsg: Msg = { id: aiId, role: 'assistant', content: '', streaming: true };
    setMsgs(prev => [...prev, aiMsg]);
    setStreaming(true);
    abortRef.current?.abort();
    abortRef.current = new AbortController();

    let full = '';
    try {
      // RAG: search notes for relevant context before sending
      let noteContext: string | null = null;
      try {
        noteContext = await buildNoteContext(userText);
      } catch (ragErr) {
        console.warn('[Chat] buildNoteContext failed, continuing without RAG:', ragErr);
      }
      const systemPrompt = buildSystemPrompt(noteContext, responseStyle);

      const history: ChatMessage[] = [
        { role: 'system', content: systemPrompt },
        ...prevMsgs.filter(m => (m.role !== 'assistant' || !m.streaming) && !m.isError).slice(-MAX_HISTORY_MESSAGES).map(m => ({ role: m.role as any, content: m.content })),
        { role: 'user', content: userText },
      ];
      // #1900: 60s timeout to prevent UI freeze
      const TIMEOUT_MS = 60_000;
      timeoutRef.current = setTimeout(() => {
        abortRef.current?.abort();
      }, TIMEOUT_MS);

      // Check session again before streaming — user may have navigated during the async gap
      if (!checkSessionAlive(activeSessionId)) {
        try { await deleteMessage(aiId); } catch (_) {}
        try { await deleteMessage(userId); } catch (_) {}
        setInput(userText);
        setMsgs(prev => prev.filter(m => m.id !== userId && m.id !== aiId));
        return;
      }

      await chatWithReconnect(history, (chunk) => {
        if (chunk.done) return;
        if (chunk.content) {
          full += chunk.content;
          setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, content: full } : m));
        }
      }, abortRef.current.signal);

      // Execute tool calls (save notes etc.) and clean up markers
      const { cleaned, actions, savedNoteIds } = await executeToolCalls(full);
      // #2223 / #2446: Clear note title cache if any notes were created/updated
      // so that newly saved notes are immediately detected as clickable links
      // and the Notes tab list refreshes on focus.
      if (savedNoteIds.length > 0) {
        clearNoteTitleCache();
        // Reload the ChatScreen's own note-title map so wikilinks for the
        // freshly saved note become clickable in this same response (#2446).
        // Errors are non-fatal — wikilinks simply won't render until next mount.
        loadNoteTitleMap()
          .then(map => { if (!checkSessionAlive(activeSessionId)) return; setNoteTitleMap(map); })
          .catch(e => console.warn('[Chat] post-save loadNoteTitleMap failed:', e));
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
        // If session changed during streaming, skip DB write to avoid data leak
        if (!checkSessionAlive(activeSessionId)) {
          console.warn('[Chat] session changed during streaming — skipping updateMessage');
          setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
          return;
        }
        await updateMessage(aiId, full);
      } catch (persistErr) {
        console.warn('[Chat] updateMessage persist failed:', persistErr);
        Alert.alert('保存失败', 'AI 回复未能保存到数据库，内容仍在当前会话中显示。');
      }
      setMsgs(prev => prev.map(m => m.id === aiId ? { ...m, streaming: false } : m));
    } catch (err: any) {
      // Save whatever partial content was received before the error
      const partial = full || (msgsRef.current.find(m => m.id === aiId)?.content ?? '');
      if (err.name === 'AbortError') {
        // Check if session changed during abort — skip DB writes to avoid leaking data
        if (!checkSessionAlive(activeSessionId)) {
          console.warn('[Chat] session changed during abort — skipping DB writes');
          return;
        }
        if (partial) {
          try { await updateMessage(aiId, partial + '\n\n_[响应被中止]_'); } catch (dbErr) {
            console.error('[Chat] Failed to persist aborted response to DB:', dbErr);
            Alert.alert('保存警告', '中止后的内容未能保存到数据库，重新加载后可能消失。');
          }
          setMsgs(prev => prev.map(m => m.id === aiId
            ? { ...m, content: partial + '\n\n_[响应被中止]_', streaming: false } : m));
        } else {
          // No content received — remove the empty placeholder
          try { await deleteMessage(aiId); } catch (dbErr) {
            console.error('[Chat] Failed to delete empty placeholder:', dbErr);
            Alert.alert('清理警告', '空白回复占位未能从数据库删除，重新加载后可能出现空消息。');
          }
          setMsgs(prev => prev.filter(m => m.id !== aiId));
        }
      } else {
        // Check if session changed during error — skip DB writes to avoid leaking data
        if (!checkSessionAlive(activeSessionId)) {
          console.warn('[Chat] session changed during error — skipping DB writes');
          return;
        }
        // Persist partial content + error marker to database before updating UI
        const errorContent = partial
          ? `${partial}\n\n[错误] ${err.message}`
          : `[错误] ${err.message}`;
        try { await updateMessage(aiId, errorContent); } catch (dbErr) {
          console.error('[Chat] Failed to persist error content to DB:', dbErr);
          Alert.alert('保存警告', '错误信息未能保存到数据库，重新加载后可能消失。');
        }
        setMsgs(prev => prev.map(m => m.id === aiId
          ? { ...m, content: errorContent, streaming: false, isError: true }
          : m));
      }
    }
    } finally {
      setStreaming(false);
      isSendingRef.current = false;
      abortRef.current = null;
      if (timeoutRef.current) { clearTimeout(timeoutRef.current); timeoutRef.current = null; }
    }
  }, [input, streaming, sessionId, responseStyle]);

  // Create a new conversation
  const newChat = useCallback(async () => {
    try {
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e));
      abortRef.current?.abort();
      const id = await createSession('新对话');
      setSessionId(id);
      setTitle('新对话');
      setMsgs([]);
      setInput('');
      setInputHeight(0);
    } catch (e) {
      console.warn('[Chat] newChat failed:', e);
      Alert.alert('新建对话失败', '无法创建新对话，请重试');
    }
  }, []);

  const stop = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy).catch(e => console.warn('[Haptics] error:', e));
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

  // Throttled scroll-to-end with trailing guarantee — fires immediately on
  // the first call in each 100ms window, and also schedules a trailing
  // scroll 150ms after the last rapid call so the final update is never lost.
  const lastScrollRef = useRef(0);
  const scrollToEndThrottled = useCallback((force = false) => {
    if (!force && !nearBottomRef.current) return;
    const now = Date.now();

    // Clear any pending trailing scroll
    if (scrollTimerRef.current) {
      clearTimeout(scrollTimerRef.current);
      scrollTimerRef.current = null;
    }

    if (now - lastScrollRef.current >= 100) {
      // Leading edge: scroll immediately
      lastScrollRef.current = now;
      listRef.current?.scrollToEnd({ animated: true });
    } else {
      // Within throttle window — schedule a trailing scroll so the last
      // rapid update never goes unseen (#2137).
      scrollTimerRef.current = setTimeout(() => {
        scrollTimerRef.current = null;
        if (!nearBottomRef.current) return;
        lastScrollRef.current = Date.now();
        listRef.current?.scrollToEnd({ animated: true });
      }, 150);
    }
  }, []);

  // Wrapper for onContentSizeChange (ignores width/height params)
  const onContentSizeChange = useCallback(() => { scrollToEndThrottled(); }, [scrollToEndThrottled]);

  // Track whether user is near the bottom
  // Use msgsRef to avoid stale closure — ref always has latest messages
  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    const distanceFromBottom = contentSize.height - layoutMeasurement.height - contentOffset.y;
    const near = distanceFromBottom < 120;
    nearBottomRef.current = near;
    setShowScrollBtn(!near && msgsRef.current.length > 0);
  }, []);

  // Scroll when new messages arrive (user sends → force scroll)
  useEffect(() => { scrollToEndThrottled(true); }, [msgs.length]);

  // Stable renderItem callback to preserve React.memo on MessageBubble (#2136)
  const renderItem = useCallback(
    ({ item }: { item: Msg }) => (
      <MessageBubble
        item={item}
        isDark={isDark}
        accentColor={accentColor}
        onDelete={handleDeleteMsg}
        onResend={item.role === 'user' ? handleResend : undefined}
        onNoteLinkPress={handleNoteLinkPress}
        noteTitleMap={noteTitleMap}
      />
    ),
    [isDark, accentColor, handleDeleteMsg, handleResend, handleNoteLinkPress, noteTitleMap]
  );

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
        <TouchableOpacity onPress={() => navigation.navigate('Sessions')} style={s.sessionsBtn}>
          <Ionicons name="menu-outline" size={20} color={accentColor} />
        </TouchableOpacity>
        <Text style={[s.titleText, { color: c.text }]} numberOfLines={1}>{title}</Text>
        <TouchableOpacity onPress={newChat} style={[s.newChatBtn, { borderColor: c.border }]}>
          <Ionicons name="add-outline" size={20} color={accentColor} />
        </TouchableOpacity>
      </View>

      {/* Message list */}
      <FlatList
        ref={listRef}
        data={msgs}
        renderItem={renderItem}
        keyExtractor={item => item.id}
        contentContainerStyle={{ padding: 16, paddingBottom: 8 }}
        onContentSizeChange={onContentSizeChange}
        onScroll={onScroll}
        scrollEventThrottle={16}
        removeClippedSubviews={Platform.OS === 'android'}
        initialNumToRender={15}
        maxToRenderPerBatch={10}
        windowSize={11}
        ListEmptyComponent={
          <View style={s.emptyContainer}>
            <Ionicons name="hand-left-outline" size={28} color={accentColor} style={{ marginBottom: 6 }} />
            <Text style={[s.emptyTitle, { color: c.text }]}>你好，我是 VaultPilot AI</Text>
            <Text style={[s.emptySubtitle, { color: c.textSecondary }]}>有什么可以帮你的？试试这些问题：</Text>
            {['帮我总结一篇笔记', '解释一下这个概念', '写一段代码'].map((q) => (
              <TouchableOpacity key={q} style={[s.suggestionBtn, { borderColor: c.border }]} onPress={() => setInput(q)}>
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
        >
          <Ionicons name="chevron-down-outline" size={18} color={accentColor} />
        </TouchableOpacity>
      )}

      {/* Response style quick-switch */}
      <View style={[s.styleRow, { borderTopColor: c.border, backgroundColor: c.bg }]}>
        {(Object.keys(RESPONSE_STYLE_LABELS) as ResponseStyle[]).map((key) => (
          <TouchableOpacity
            key={key}
            style={[
              s.stylePill,
              {
                borderColor: responseStyle === key ? accentColor : c.border,
                backgroundColor: responseStyle === key ? accentColor + '15' : 'transparent',
              },
            ]}
            onPress={() => { setResponseStyle(key); }}
            disabled={streaming}
            accessibilityRole="button"
            accessibilityLabel={RESPONSE_STYLE_LABELS[key]}
            accessibilityState={{ selected: responseStyle === key }}
          >
            <Text style={[s.styleText, { color: responseStyle === key ? accentColor : c.textSecondary }]}>
              {RESPONSE_STYLE_LABELS[key]}
            </Text>
          </TouchableOpacity>
        ))}
      </View>

      {/* Input bar */}
      <View style={[s.inputBar, { backgroundColor: c.bg }]}>
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
        />
        {streaming ? (
          <TouchableOpacity onPress={stop} style={[s.sendBtn, { backgroundColor: '#EF4444' }]}>
            <Ionicons name="stop-circle-outline" size={22} color="#FFF" />
          </TouchableOpacity>
        ) : (
          <TouchableOpacity
            onPress={send}
            style={[s.sendBtn, { backgroundColor: input.trim() ? accentColor : c.border }]}
            disabled={!input.trim()}
          >
            <Ionicons name="send" size={18} color="#FFF" />
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
  styleRow: {
    flexDirection: 'row',
    paddingHorizontal: 12,
    paddingVertical: 6,
    gap: 8,
    borderTopWidth: 1,
  },
  stylePill: {
    paddingVertical: 4,
    paddingHorizontal: 12,
    borderRadius: 12,
    borderWidth: 1,
  },
  styleText: {
    fontSize: 12,
    fontWeight: '500',
  },
  inputBar: {
    flexDirection: 'row', alignItems: 'flex-end',
    padding: 8,
  },
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
})
