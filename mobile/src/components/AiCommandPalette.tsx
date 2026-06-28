/**
 * AI Command Palette — searchable inline panel of one-tap AI actions (#2188).
 *
 * Design constraints (project conventions):
 *  - Ionicons glyphs, never emoji.
 *  - Inline expanding panel (NOT a full-screen modal/overlay).
 *  - Entrance animation via Animated (the toolbar AI button "expands" into this).
 *
 * The panel is presentational + orchestration only: it builds messages via the
 * pure aiActions helpers and streams via the existing chat()/parseSSEStream.
 * Actual note mutation is delegated to the parent through onInsert.
 */

import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  View, Text, TextInput, TouchableOpacity, StyleSheet, ScrollView,
  ActivityIndicator, Animated, Easing, Platform, KeyboardAvoidingView,
} from 'react-native';
import Ionicons from '@expo/vector-icons/Ionicons';
import * as Haptics from 'expo-haptics';

import { chat, ChatMessage, parseSSEStream } from '../api/client';
import {
  AI_ACTIONS, AiAction, AiActionId,
  buildActionMessages, filterActions, resolveContext,
} from '../utils/aiActions';

export interface AiCommandPaletteProps {
  visible: boolean;
  /** Current text selection (may be empty). */
  selectionText: string;
  /** Full note content (fallback target when no selection). */
  noteContent: string;
  /** Insert the generated result at the cursor / replace selection. */
  onInsert: (text: string) => void;
  /** Close the panel. */
  onClose: () => void;
  accentColor: string;
  isDark: boolean;
  colors: {
    bg: string; bgSecondary: string; text: string; textSecondary: string;
    border: string; inputBg: string;
  };
}

function haptic() {
  Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(() => {});
}

export default function AiCommandPalette(props: AiCommandPaletteProps) {
  const { visible, selectionText, noteContent, onInsert, onClose, accentColor, colors: c } = props;

  const [query, setQuery] = useState('');
  const [runningId, setRunningId] = useState<AiActionId | null>(null);
  const [customPrompt, setCustomPrompt] = useState('');
  const [customActive, setCustomActive] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const slide = useRef(new Animated.Value(0)).current; // 0 = hidden, 1 = shown
  const abortRef = useRef<AbortController | null>(null);

  // Entrance / exit animation driven by `visible`.
  useEffect(() => {
    Animated.timing(slide, {
      toValue: visible ? 1 : 0,
      duration: 180,
      easing: Easing.out(Easing.ease),
      useNativeDriver: true,
    }).start();
    if (visible) haptic();
    // Reset transient state when reopening.
    if (visible) {
      setQuery('');
      setError(null);
      setCustomActive(false);
    }
    if (!visible) {
      // Cancel any in-flight stream when the panel closes.
      abortRef.current?.abort();
      setRunningId(null);
    }
  }, [visible]); // eslint-disable-line react-hooks/exhaustive-deps

  // Cleanup on unmount.
  useEffect(() => () => abortRef.current?.abort(), []);

  const filtered = useMemo(() => filterActions(AI_ACTIONS, query), [query]);

  const targetLabel = (selectionText && selectionText.trim().length > 0)
    ? '选中文本'
    : (noteContent && noteContent.trim().length > 0 ? '整篇笔记' : '（无内容）');

  const runAction = async (action: AiAction) => {
    const ctx = resolveContext(selectionText, noteContent);
    if (!ctx) {
      setError('请先选中文字或写入笔记内容');
      haptic();
      return;
    }
    if (action.needsUserPrompt) {
      setCustomActive(true);
      haptic();
      return;
    }
    await streamAction(action.id, ctx);
  };

  const runCustom = async () => {
    const action = AI_ACTIONS.find((a) => a.id === 'custom');
    if (!action) return;
    const ctx = resolveContext(selectionText, noteContent);
    if (!ctx) {
      setError('请先选中文字或写入笔记内容');
      return;
    }
    if (!customPrompt.trim()) return;
    await streamAction('custom', ctx, customPrompt.trim());
  };

  const streamAction = async (
    id: AiActionId, ctx: string, userPrompt?: string,
  ) => {
    setError(null);
    setRunningId(id);
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      const messages: ChatMessage[] = buildActionMessages(id, ctx, userPrompt);
      const stream = await chat(messages, controller.signal);
      let full = '';
      await parseSSEStream(stream, (chunk) => {
        if (chunk.done) return;
        if (chunk.content) full += chunk.content;
      });
      if (full.trim()) {
        onInsert(full);
        haptic();
        onClose();
      } else {
        setError('AI 未返回内容，请重试');
      }
    } catch (e: any) {
      if (e?.name === 'AbortError') return; // user closed — silent
      setError(e?.message || 'AI 生成失败，请重试');
    } finally {
      setRunningId(null);
      abortRef.current = null;
    }
  };

  if (!visible) return null;

  const running = runningId !== null;
  const runningAction = AI_ACTIONS.find((a) => a.id === runningId);

  return (
    <Animated.View
      style={[
        styles.panel,
        {
          borderTopColor: c.border,
          backgroundColor: c.bgSecondary,
          opacity: slide,
          transform: [{
            translateY: slide.interpolate({ inputRange: [0, 1], outputRange: [24, 0] }),
          }],
        },
      ]}
    >
      {/* Header: search + close */}
      <View style={styles.header}>
        <Ionicons name="sparkles-outline" size={16} color={accentColor} style={styles.headerIcon} />
        <TextInput
          style={[styles.searchInput, { color: c.text, borderColor: c.border, backgroundColor: c.inputBg }]}
          value={query}
          onChangeText={setQuery}
          placeholder="搜索 AI 动作…"
          placeholderTextColor={c.textSecondary}
          autoCorrect={false}
          autoCapitalize="none"
        />
        <TouchableOpacity onPress={onClose} hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}>
          <Ionicons name="close" size={20} color={c.textSecondary} />
        </TouchableOpacity>
      </View>

      {/* Context hint */}
      <Text style={[styles.hint, { color: c.textSecondary }]}>
        作用于：<Text style={{ color: c.text, fontWeight: '500' }}>{targetLabel}</Text>
      </Text>

      {/* Body */}
      {running ? (
        <View style={styles.running}>
          <ActivityIndicator color={accentColor} size="small" />
          <Text style={[styles.runningText, { color: c.textSecondary }]}>
            正在「{runningAction?.label ?? '生成'}」…
          </Text>
        </View>
      ) : customActive ? (
        <KeyboardAvoidingView
          behavior={Platform.OS === 'ios' ? 'padding' : undefined}
          style={styles.customWrap}
        >
          <TextInput
            style={[styles.customInput, { color: c.text, borderColor: c.border, backgroundColor: c.inputBg }]}
            value={customPrompt}
            onChangeText={setCustomPrompt}
            placeholder="写什么？让 AI 帮你写…"
            placeholderTextColor={c.textSecondary}
            multiline
            textAlignVertical="top"
            autoFocus
          />
          <View style={styles.customActions}>
            <TouchableOpacity
              style={[styles.customBtn, { backgroundColor: accentColor }]}
              onPress={runCustom}
              disabled={!customPrompt.trim()}
            >
              <Ionicons name="sparkles-outline" size={16} color="#FFF" />
              <Text style={styles.customBtnText}>生成</Text>
            </TouchableOpacity>
            <TouchableOpacity
              style={[styles.customBtnGhost, { borderColor: c.border }]}
              onPress={() => { setCustomActive(false); setCustomPrompt(''); }}
            >
              <Text style={[styles.customBtnGhostText, { color: c.textSecondary }]}>返回</Text>
            </TouchableOpacity>
          </View>
        </KeyboardAvoidingView>
      ) : (
        <ScrollView style={styles.list} keyboardShouldPersistTaps="handled">
          {filtered.length === 0 ? (
            <Text style={[styles.empty, { color: c.textSecondary }]}>没有匹配的动作</Text>
          ) : filtered.map((a) => (
            <TouchableOpacity
              key={a.id}
              style={[styles.row, { borderColor: c.border }]}
              onPress={() => runAction(a)}
              disabled={running}
            >
              <View style={[styles.rowIcon, { backgroundColor: accentColor + '22' }]}>
                <Ionicons name={a.icon as any} size={18} color={accentColor} />
              </View>
              <View style={styles.rowText}>
                <Text style={[styles.rowLabel, { color: c.text }]}>{a.label}</Text>
                <Text style={[styles.rowDesc, { color: c.textSecondary }]} numberOfLines={1}>
                  {a.description}
                </Text>
              </View>
              <Ionicons name="chevron-forward" size={16} color={c.textSecondary} />
            </TouchableOpacity>
          ))}
        </ScrollView>
      )}

      {error ? (
        <Text style={styles.error}>{error}</Text>
      ) : null}
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  panel: {
    borderTopWidth: 1,
    paddingHorizontal: 12,
    paddingTop: 8,
    paddingBottom: 10,
    maxHeight: 320,
  },
  header: {
    flexDirection: 'row', alignItems: 'center', marginBottom: 6,
  },
  headerIcon: { marginRight: 8 },
  searchInput: {
    flex: 1, fontSize: 14, borderWidth: 1, borderRadius: 8,
    paddingHorizontal: 10, paddingVertical: 6, marginRight: 8,
  },
  hint: { fontSize: 12, marginBottom: 6 },
  list: { flexGrow: 0 },
  row: {
    flexDirection: 'row', alignItems: 'center',
    paddingVertical: 9, paddingHorizontal: 8, marginBottom: 4,
    borderWidth: 1, borderRadius: 10,
  },
  rowIcon: {
    width: 32, height: 32, borderRadius: 16,
    alignItems: 'center', justifyContent: 'center', marginRight: 10,
  },
  rowText: { flex: 1 },
  rowLabel: { fontSize: 14, fontWeight: '600' },
  rowDesc: { fontSize: 12, marginTop: 1 },
  running: { flexDirection: 'row', alignItems: 'center', paddingVertical: 14 },
  runningText: { fontSize: 13, marginLeft: 10 },
  customWrap: {},
  customInput: {
    fontSize: 14, borderWidth: 1, borderRadius: 8,
    paddingHorizontal: 10, paddingVertical: 8, minHeight: 60, maxHeight: 120,
  },
  customActions: { flexDirection: 'row', alignItems: 'center', marginTop: 8, gap: 8 },
  customBtn: {
    flexDirection: 'row', alignItems: 'center',
    paddingHorizontal: 14, paddingVertical: 8, borderRadius: 8,
  },
  customBtnText: { color: '#FFF', fontSize: 14, fontWeight: '600', marginLeft: 6 },
  customBtnGhost: {
    paddingHorizontal: 14, paddingVertical: 8, borderRadius: 8, borderWidth: 1,
  },
  customBtnGhostText: { fontSize: 14 },
  empty: { fontSize: 13, paddingVertical: 16, textAlign: 'center' },
  error: { color: '#EF4444', fontSize: 12, marginTop: 6 },
});
