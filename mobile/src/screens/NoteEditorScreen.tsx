import React, { useState, useEffect, useRef } from 'react';
import {
  View, Text, TextInput, TouchableOpacity, StyleSheet, ScrollView, Alert, ActivityIndicator,
  KeyboardAvoidingView, Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Haptics from 'expo-haptics';
import * as Clipboard from 'expo-clipboard';
import { useAppStore, getColors } from '../store';
import MarkdownPreview from '../components/MarkdownPreview';
import { getNote, updateNote, deleteNote, moveToFolder, getFolders, getNoteTags, addTag, removeTag } from '../db';
import { extractAutoTags } from '../utils/autoTag';
import { queuePendingSync } from '../db';
import { useNetworkState } from '../utils/networkState';
import type { NoteEditorScreenProps, RootTabParamList } from '../navigation/types';
import { useNavigation } from '@react-navigation/native';
import type { NavigationProp } from '@react-navigation/native';

export default function NoteEditorScreen({ route, navigation }: NoteEditorScreenProps) {
  const { noteId } = route.params;
  const rootNav = useNavigation<NavigationProp<RootTabParamList>>();
  const { isDark, accentColor } = useAppStore();
  const { isOnline } = useNetworkState();
  const c = getColors(isDark, accentColor);
  const [title, setTitle] = useState('');
  const [content, setContent] = useState('');
  const [folder, setFolder] = useState('');
  const [showFolderPicker, setShowFolderPicker] = useState(false);
  const [tags, setTags] = useState<string[]>([]);
  const [newTag, setNewTag] = useState('');
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(true);
  const [previewMode, setPreviewMode] = useState(false);
  const titleRef = useRef('');
  const contentRef = useRef('');
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);
  const currentFolderRef = useRef('');
  const originalFolderRef = useRef('');
  const pendingRef = useRef<{ title: string; content: string } | null>(null);
  const selectionRef = useRef<{ start: number; end: number }>({ start: 0, end: 0 });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const note = await getNote(noteId);
        if (cancelled) return;
        if (note) {
          titleRef.current = note.title;
          contentRef.current = note.content;
          setTitle(note.title);
          setContent(note.content);
          setFolder(note.folder || '');
          currentFolderRef.current = note.folder || '';
          originalFolderRef.current = note.folder || '';
          const noteTags = await getNoteTags(noteId);
          if (!cancelled) setTags(noteTags);
        } else {
          Alert.alert('笔记不存在', '该笔记可能已被删除', [
            { text: '返回', onPress: () => navigation.goBack() },
          ]);
        }
      } catch (e: unknown) {
        if (cancelled) return;
        Alert.alert('加载失败', e instanceof Error ? e.message : '请重试', [
          { text: '返回', onPress: () => navigation.goBack() },
        ]);
        return;
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [noteId]);

  const save = async (t?: string, ct?: string) => {
    if (!mountedRef.current) return;
    setSaving(true);
    try {
      await updateNote(noteId, t ?? title, ct ?? content);
      // Auto-tag if note has no tags yet
      if (tags.length === 0 && (t ?? title).trim()) {
        const suggestions = extractAutoTags(t ?? title, ct ?? content);
        for (const tag of suggestions) {
          if (!tags.includes(tag)) {
            await addTag(noteId, tag);
          }
        }
        if (suggestions.length > 0 && mountedRef.current) {
          const freshTags = await getNoteTags(noteId);
          if (mountedRef.current) setTags(freshTags);
        }
      }
      // Queue for backend sync if offline
      if (!isOnline) {
        queuePendingSync(noteId).catch(e => console.warn('[NoteEditor] queuePendingSync failed:', e));
      }
    } catch (e) {
      console.warn('[NoteEditor] Save failed:', e);
      if (mountedRef.current) Alert.alert('保存失败', String(e));
    } finally {
      if (mountedRef.current) setSaving(false);
    }
  };

  const autoSave = (newTitle: string, newContent: string) => {
    clearTimeout(timerRef.current);
    pendingRef.current = { title: newTitle, content: newContent };
    timerRef.current = setTimeout(async () => {
      pendingRef.current = null;
      await save(newTitle, newContent);
    }, 1000);
  };

  // Cleanup: flush pending save on unmount
  useEffect(() => {
    return () => {
      mountedRef.current = false;
      clearTimeout(timerRef.current);
      if (pendingRef.current) {
        // Fire-and-forget save on unmount — component state is already gone
        updateNote(noteId, pendingRef.current.title, pendingRef.current.content);
        pendingRef.current = null;
      }
      // Save folder on unmount only if actually changed
      if (currentFolderRef.current !== originalFolderRef.current) {
        moveToFolder(noteId, currentFolderRef.current).catch(e => console.warn('[NoteEditor] moveToFolder on unmount failed:', e));
      }
    };
  }, [noteId]);

  const handleDelete = () => {
    Alert.alert('删除笔记', '确定要删除吗？', [
      { text: '取消', style: 'cancel' },
      { text: '删除', style: 'destructive', onPress: async () => {
        try {
          await deleteNote(noteId);
          navigation.goBack();
        } catch (e) {
          Alert.alert('删除失败', String(e));
        }
      } },
    ]);
  };

  const TOOLBAR = [
    { label: 'B', insert: '**', desc: '加粗' },
    { label: 'I', insert: '*', desc: '斜体' },
    { label: '`', insert: '`', desc: '代码' },
    { label: '#', insert: '# ', desc: '标题' },
    { label: '-', insert: '- ', desc: '列表' },
    { label: '🔗', insert: '[]()', desc: '链接' },
  ];

  const AI_ACTIONS = [
    { key: 'polish', label: '✨ 润色', prompt: '请帮我润色以下笔记内容，改善措辞和表达：' },
    { key: 'summarize', label: '📋 总结', prompt: '请用简洁的语言总结以下笔记的要点：' },
    { key: 'translate', label: '🌐 翻译成英文', prompt: '请将以下笔记翻译成英文：' },
    { key: 'continue', label: '✍️ 续写', prompt: '请根据以下笔记内容，帮我继续写下去：' },
  ];

  const handleAiAction = (action: typeof AI_ACTIONS[0]) => {
    const noteText = content || title || '';
    if (!noteText.trim()) {
      Alert.alert('提示', '笔记内容为空，无法使用 AI 助手');
      return;
    }
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    const prefill = `${action.prompt}\n\n${noteText.slice(0, 2000)}`;
    // Navigate to Chat tab with pre-filled text
    rootNav.navigate('Chat', { screen: 'ChatMain', params: { prefillText: prefill } });
  };

  const showAiActions = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    Alert.alert('AI 笔记助手', '选择操作', [
      ...AI_ACTIONS.map(a => ({ text: a.label, onPress: () => handleAiAction(a) })),
      { text: '取消', style: 'cancel' as const },
    ]);
  };

  const insertFormat = (syntax: string) => {
    const { start, end } = selectionRef.current;
    const isPrefix = syntax.endsWith(' ');
    setContent(prev => {
      const before = prev.slice(0, start);
      const selected = prev.slice(start, end);
      const after = prev.slice(end);
      const next = isPrefix
        ? before + syntax + selected + after
        : before + syntax + selected + syntax + after;
      const newPos = isPrefix ? start + syntax.length + selected.length : start + syntax.length + selected.length + syntax.length;
      selectionRef.current = { start: newPos, end: newPos };
      contentRef.current = next;
      autoSave(titleRef.current, next);
      return next;
    });
  };

  if (loading) {
    return (
      <SafeAreaView style={[s.container, { backgroundColor: c.bg, justifyContent: 'center', alignItems: 'center' }]}>
        <ActivityIndicator color={accentColor} size="large" />
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView style={{ flex: 1, backgroundColor: c.bg }}>
    <KeyboardAvoidingView
      style={[s.container, { backgroundColor: c.bg }]}
      behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
      keyboardVerticalOffset={0}
    >
      {/* Header */}
      <View style={[s.header, { borderBottomColor: c.border }]}>
        <TouchableOpacity onPress={() => navigation.goBack()}>
          <Text style={[s.headerBtn, { color: accentColor }]}>← 返回</Text>
        </TouchableOpacity>
        <Text style={[s.headerTitle, { color: c.textSecondary }]}>
          {saving ? '保存中...' : '已保存'}
        </Text>
        <View style={s.headerActions}>
          <TouchableOpacity onPress={() => {
            const text = content ? (title ? `${title}\n\n${content}` : content) : '';
            if (text) { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); Clipboard.setStringAsync(text); }
          }}>
            <Text style={[s.headerBtn, { color: accentColor }]}>复制</Text>
          </TouchableOpacity>
          <TouchableOpacity onPress={handleDelete}>
            <Text style={[s.headerBtn, { color: '#EF4444', marginLeft: 16 }]}>删除</Text>
          </TouchableOpacity>
        </View>
      </View>

      {/* Folder bar */}
      <TouchableOpacity
        style={[s.folderBar, { borderBottomColor: c.border }]}
        onPress={() => setShowFolderPicker(!showFolderPicker)}
      >
        <Text style={[s.folderLabel, { color: c.textSecondary }]}>
          📁 {folder || '未分类'}
        </Text>
        <Text style={[s.folderLabel, { color: accentColor }]}>
          {showFolderPicker ? '收起' : '编辑'}
        </Text>
      </TouchableOpacity>
      {showFolderPicker && (
        <View style={[s.folderPicker, { backgroundColor: c.card, borderBottomColor: c.border }]}>
          <TextInput
            style={[s.folderInput, { color: c.text, borderColor: c.border }]}
            value={folder}
            onChangeText={(f) => { setFolder(f); currentFolderRef.current = f; }}
            onBlur={() => { moveToFolder(noteId, currentFolderRef.current).catch(e => console.warn('[NoteEditor] moveToFolder failed:', e)); }}
            onSubmitEditing={() => { moveToFolder(noteId, currentFolderRef.current).catch(e => console.warn('[NoteEditor] moveToFolder failed:', e)); }}
            placeholder="输入文件夹名称"
            placeholderTextColor={c.textSecondary}
          />
        </View>
      )}

      {/* Tags bar */}
      <View style={[s.tagsBar, { borderBottomColor: c.border }]}>
        <View style={s.tagsRow}>
          {tags.map(t => (
            <TouchableOpacity
              key={t}
              style={[s.tagChip, { backgroundColor: accentColor + '20', borderColor: accentColor }]}
              onLongPress={async () => {
                await removeTag(noteId, t);
                setTags(prev => prev.filter(x => x !== t));
              }}
            >
              <Text style={[s.tagText, { color: accentColor }]}>#{t}</Text>
            </TouchableOpacity>
          ))}
          <View style={s.tagInputRow}>
            <TextInput
              style={[s.tagInput, { color: c.text, borderColor: c.border }]}
              value={newTag}
              onChangeText={setNewTag}
              placeholder="+ 标签"
              placeholderTextColor={c.textSecondary}
              onSubmitEditing={async () => {
                const tag = newTag.trim();
                if (tag && !tags.includes(tag)) {
                  try {
                    await addTag(noteId, tag);
                    setTags(prev => [...prev, tag]);
                    setNewTag('');
                  } catch (e) {
                    console.warn('[NoteEditor] addTag failed:', e);
                  }
                }
              }}
              returnKeyType="done"
            />
          </View>
        </View>
      </View>

      {/* Title */}
      <TextInput
        style={[s.titleInput, { color: c.text }]}
        value={title}
        onChangeText={(t) => { titleRef.current = t; setTitle(t); autoSave(t, contentRef.current); }}
        placeholder="笔记标题"
        placeholderTextColor={c.textSecondary}
      />

      {/* Content — edit or preview */}
      {previewMode ? (
        <ScrollView style={s.previewContainer} contentContainerStyle={{ padding: 16 }}>
          <MarkdownPreview content={content || '*空白笔记*'} textColor={c.text} accentColor={accentColor} isDark={isDark} />
        </ScrollView>
      ) : (
        <TextInput
          style={[s.contentInput, { color: c.text }]}
          value={content}
          onChangeText={(t) => { contentRef.current = t; setContent(t); autoSave(titleRef.current, t); }}
          onSelectionChange={(e) => { selectionRef.current = e.nativeEvent.selection; }}
          placeholder="开始写作..."
          placeholderTextColor={c.textSecondary}
          multiline
          textAlignVertical="top"
        />
      )}

      {/* Toolbar */}
      <View style={[s.toolbar, { borderTopColor: c.border, backgroundColor: c.bgSecondary }]}>
        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
          <TouchableOpacity
            style={[s.toolBtn, { borderColor: previewMode ? accentColor : c.border, backgroundColor: previewMode ? accentColor + '20' : 'transparent' }]}
            onPress={() => setPreviewMode(v => !v)}
          >
            <Text style={[s.toolLabel, { color: previewMode ? accentColor : c.text }]}>{previewMode ? '✏️' : '👁'}</Text>
          </TouchableOpacity>
          {!previewMode && TOOLBAR.map((t) => (
            <TouchableOpacity
              key={t.label}
              style={[s.toolBtn, { borderColor: c.border }]}
              onPress={() => insertFormat(t.insert)}
            >
              <Text style={[s.toolLabel, { color: c.text }]}>{t.label}</Text>
            </TouchableOpacity>
          ))}
          <TouchableOpacity
            style={[s.toolBtn, { borderColor: accentColor, backgroundColor: accentColor + '15' }]}
            onPress={showAiActions}
          >
            <Text style={[s.toolLabel, { color: accentColor }]}>🤖</Text>
          </TouchableOpacity>
        </ScrollView>
      </View>
    </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1 },
  header: {
    flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center',
    paddingHorizontal: 16, paddingVertical: 12, borderBottomWidth: 1,
  },
  headerBtn: { fontSize: 16, fontWeight: '500' },
  headerActions: { flexDirection: 'row', alignItems: 'center' },
  headerTitle: { fontSize: 13 },
  folderBar: {
    flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center',
    paddingHorizontal: 16, paddingVertical: 10, borderBottomWidth: 1,
  },
  folderLabel: { fontSize: 14 },
  folderPicker: { paddingHorizontal: 16, paddingVertical: 8, borderBottomWidth: 1 },
  folderInput: { fontSize: 14, borderWidth: 1, borderRadius: 8, paddingHorizontal: 12, paddingVertical: 8 },
  tagsBar: { paddingHorizontal: 16, paddingVertical: 8, borderBottomWidth: 1 },
  tagsRow: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 6 },
  tagChip: { paddingHorizontal: 10, paddingVertical: 4, borderRadius: 12, borderWidth: 1 },
  tagText: { fontSize: 13 },
  tagInputRow: { flexDirection: 'row', alignItems: 'center' },
  tagInput: { fontSize: 13, borderWidth: 1, borderRadius: 8, paddingHorizontal: 8, paddingVertical: 4, minWidth: 60 },
  titleInput: {
    fontSize: 22, fontWeight: '700', paddingHorizontal: 16,
    paddingTop: 16, paddingBottom: 8,
  },
  contentInput: {
    flex: 1, fontSize: 16, lineHeight: 24, paddingHorizontal: 16, paddingTop: 8,
  },
  previewContainer: {
    flex: 1,
  },
  toolbar: {
    flexDirection: 'row', paddingVertical: 8, paddingHorizontal: 12, borderTopWidth: 1,
  },
  toolBtn: {
    paddingHorizontal: 14, paddingVertical: 8, marginHorizontal: 4,
    borderWidth: 1, borderRadius: 8,
  },
  toolLabel: { fontSize: 16, fontWeight: '600' },
});
