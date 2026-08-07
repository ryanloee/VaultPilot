import React, { useState, useEffect, useRef } from 'react';
import {
  View, Text, TextInput, TouchableOpacity, StyleSheet, ScrollView, Alert, ActivityIndicator,
  KeyboardAvoidingView, Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Haptics from 'expo-haptics';
import * as Clipboard from 'expo-clipboard';
import Ionicons from '@expo/vector-icons/Ionicons';
import { useAppStore, getColors, filterFocusModeToolbar } from '../store';
import MarkdownPreview from '../components/MarkdownPreview';
import { getNote, createNote, updateNote, deleteNote, moveToFolder, getFolders, getNoteTags, addTag, removeTag, saveAsTemplate } from '../db';
import { chat, ChatMessage, parseSSEStream } from '../api/client';
import AiActionPalette from '../components/ai/AiActionPalette';
import { fetchKnowledgeGraph } from '../services/graphService';
import { extractRelatedNotes, RelatedNote } from '../utils/relatedNotes';

export default function NoteEditorScreen({ route, navigation }: any) {
  const { noteId } = route.params;
  const { isDark, accentColor, focusMode } = useAppStore();
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
  const [showAiWrite, setShowAiWrite] = useState(false);
  const [aiPrompt, setAiPrompt] = useState('');
  const [aiGenerating, setAiGenerating] = useState(false);
  const [showAiPalette, setShowAiPalette] = useState(false);
  // 相关笔记（局部图谱 1 跳邻居，#3832）：null=加载中，[]=已加载但无邻居
  const [relatedNotes, setRelatedNotes] = useState<RelatedNote[] | null>(null);
  const [relatedError, setRelatedError] = useState(false);
  const titleRef = useRef('');
  const contentRef = useRef('');
  const timerRef = useRef<any>(null);
  const mountedRef = useRef(true);
  const currentFolderRef = useRef('');
  const originalFolderRef = useRef('');
  const pendingRef = useRef<{ title: string; content: string } | null>(null);
  const selectionRef = useRef<{ start: number; end: number }>({ start: 0, end: 0 });
  const aiAbortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // #2915 — Handle widget deep link vaultpilot://note/new:
        // create a fresh note instead of trying to load non-existent id "new".
        if (noteId === 'new') {
          const newId = await createNote();
          if (cancelled) return;
          // Navigate replace so back button doesn't return to a broken "new" route
          navigation.setParams({ noteId: newId });
          setLoading(false);
          return;
        }
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
      } catch (e: any) {
        if (cancelled) return;
        Alert.alert('加载失败', e.message || '请重试', [
          { text: '返回', onPress: () => navigation.goBack() },
        ]);
        return;
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [noteId]);

  // 相关笔记（局部图谱 1 跳邻居，#3832）：进屏时拉取 /api/graph，
  // 在客户端用当前 noteId 提取 1 跳邻居。后端不可达/无 token/图为空时优雅降级。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!noteId || noteId === 'new') {
        // 新建笔记没有稳定 id，跳过图谱查询
        if (!cancelled) { setRelatedError(true); setRelatedNotes(null); }
        return;
      }
      const graph = await fetchKnowledgeGraph();
      if (cancelled) return;
      if (!graph) {
        setRelatedError(true);
        setRelatedNotes(null);
        return;
      }
      const neighbors = extractRelatedNotes(graph, noteId);
      if (cancelled) return;
      setRelatedNotes(neighbors);
    })();
    return () => { cancelled = true; };
  }, [noteId]);

  const save = async (t?: string, ct?: string) => {
    if (!mountedRef.current) return;
    setSaving(true);
    try {
      await updateNote(noteId, t ?? title, ct ?? content);
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
      aiAbortRef.current?.abort();
      clearTimeout(timerRef.current);
      if (pendingRef.current) {
        // Fire-and-forget save on unmount — component state is already gone
        updateNote(noteId, pendingRef.current.title, pendingRef.current.content);
        pendingRef.current = null;
      }
      // Save folder on unmount if it was changed
      if (currentFolderRef.current !== originalFolderRef.current) {
        moveToFolder(noteId, currentFolderRef.current).catch(() => {});
      }
    };
  }, [noteId]);

  const handleDelete = () => {
    Alert.alert('删除笔记', '确定要删除吗？', [
      { text: '取消', style: 'cancel' },
      { text: '删除', style: 'destructive', onPress: async () => {
        try {
          await deleteNote(noteId);
          // #2446: clear any pending autosave so we don't re-create the note
          // after the DELETE has been issued.
          pendingRef.current = null;
          clearTimeout(timerRef.current);
          navigation.goBack();
        } catch (e) {
          Alert.alert('删除失败', String(e));
        }
      } },
    ]);
  };

  // #2154 — save current note's title+content as a reusable template (non-destructive copy)
  const handleSaveAsTemplate = async () => {
    try {
      const tplId = await saveAsTemplate(noteId);
      if (tplId) {
        Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e));
        Alert.alert('已存为模板', `「${title || '无标题'}」已保存为模板，可在新建笔记时套用。`);
      } else {
        Alert.alert('操作失败', '笔记不存在');
      }
    } catch (e) {
      Alert.alert('存为模板失败', String(e));
    }
  };

  const TOOLBAR = [
    { label: 'B', insert: '**', desc: '加粗' },
    { label: 'I', insert: '*', desc: '斜体' },
    { label: '`', insert: '`', desc: '代码' },
    { label: '#', insert: '# ', desc: '标题' },
    { label: '-', insert: '- ', desc: '列表' },
    { label: 'link', insert: '[]()', desc: '链接', icon: 'link-outline' },
    { label: 'AI', insert: '', desc: 'AI 写作', icon: 'color-wand-outline', action: 'aiWrite' },
    { label: 'Cmd', insert: '', desc: 'AI 命令面板', icon: 'terminal-outline', action: 'aiCmd' },
  ];

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

  const handleAiWrite = async () => {
    if (!aiPrompt.trim() || aiGenerating) return;
    // Create AbortController for cancellation on unmount
    const ac = new AbortController();
    aiAbortRef.current = ac;
    setAiGenerating(true);
    try {
      const messages: ChatMessage[] = [
        { role: 'system', content: 'You are a professional writing assistant integrated into a note-taking app. Help the user write, improve, expand, or polish their notes. Respond with only the generated content, no extra commentary or formatting instructions.' },
        { role: 'user', content: `Writing instruction: ${aiPrompt}\n\nCurrent note content:\n${content}` },
      ];
      const stream = await chat(messages, ac.signal);
      let full = '';
      await parseSSEStream(stream, (chunk) => {
        if (chunk.done) return;
        if (chunk.content) full += chunk.content;
      }, { signal: ac.signal });
      if (full && mountedRef.current) {
        const { start, end } = selectionRef.current;
        setContent(prev => {
          const before = prev.slice(0, start);
          const selected = prev.slice(start, end);
          const after = prev.slice(end);
          const next = before + full + after;
          const newPos = start + full.length;
          selectionRef.current = { start: newPos, end: newPos };
          contentRef.current = next;
          autoSave(titleRef.current, next);
          return next;
        });
      }
    } catch (e: any) {
      // AbortError: user left the screen → silent return
      if ((e as Error).name === 'AbortError') return;
      if (mountedRef.current) {
        Alert.alert('AI 写作失败', e.message || '请重试');
      }
    } finally {
      aiAbortRef.current = null;
      if (mountedRef.current) {
        setAiGenerating(false);
        setShowAiWrite(false);
        setAiPrompt('');
      }
    }
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
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
          <Ionicons name="arrow-back-outline" size={18} color={accentColor} />
          <Text style={[s.headerBtn, { color: accentColor }]}>返回</Text>
        </View>
        </TouchableOpacity>
        <Text style={[s.headerTitle, { color: c.textSecondary }]}>
          {saving ? '保存中...' : '已保存'}
        </Text>
        <View style={s.headerActions}>
          <TouchableOpacity onPress={handleSaveAsTemplate}>
            <Text style={[s.headerBtn, { color: accentColor }]}>存为模板</Text>
          </TouchableOpacity>
          <TouchableOpacity onPress={() => {
            const text = content ? (title ? `${title}\n\n${content}` : content) : '';
            if (text) { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); Clipboard.setStringAsync(text); }
          }}>
            <Text style={[s.headerBtn, { color: accentColor, marginLeft: 16 }]}>复制</Text>
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
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
          <Ionicons name="folder-outline" size={16} color={c.textSecondary} />
          <Text style={[s.folderLabel, { color: c.textSecondary }]}>{folder || '未分类'}</Text>
        </View>
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
            onBlur={() => {
              if (currentFolderRef.current !== originalFolderRef.current) {
                moveToFolder(noteId, currentFolderRef.current)
                  .then(() => { originalFolderRef.current = currentFolderRef.current; })
                  .catch(e => console.warn('[NoteEditor] moveToFolder failed:', e));
              }
            }}
            onSubmitEditing={() => {
              if (currentFolderRef.current !== originalFolderRef.current) {
                moveToFolder(noteId, currentFolderRef.current)
                  .then(() => { originalFolderRef.current = currentFolderRef.current; })
                  .catch(e => console.warn('[NoteEditor] moveToFolder failed:', e));
              }
            }}
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
                try {
                  await removeTag(noteId, t);
                  setTags(prev => prev.filter(x => x !== t));
                } catch (e) {
                  Alert.alert('删除标签失败', String(e));
                }
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
                    Alert.alert('添加标签失败', String(e));
                  }
                }
              }}
              returnKeyType="done"
            />
          </View>
        </View>
      </View>

      {/* 相关笔记（局部图谱 1 跳邻居，#3832） */}
      <View style={[s.relatedSection, { borderBottomColor: c.border }]} testID="related-notes-section">
        <View style={s.relatedHeader}>
          <Ionicons name="git-network-outline" size={14} color={c.textSecondary} />
          <Text style={[s.relatedHeaderTitle, { color: c.textSecondary }]}>相关笔记</Text>
        </View>
        {relatedError ? (
          <Text style={[s.relatedEmpty, { color: c.textSecondary }]}>未连接后端</Text>
        ) : relatedNotes === null ? (
          <ActivityIndicator size="small" color={accentColor} style={s.relatedLoading} testID="related-notes-loading" />
        ) : relatedNotes.length === 0 ? (
          <Text style={[s.relatedEmpty, { color: c.textSecondary }]}>暂无相关笔记</Text>
        ) : (
          relatedNotes.map(n => (
            <TouchableOpacity
              key={n.id}
              testID={`related-note-${n.id}`}
              style={[s.relatedItem, { backgroundColor: c.card, borderColor: c.border }]}
              onPress={() => navigation.navigate('NoteEdit', { noteId: n.id })}
            >
              <Text style={[s.relatedItemTitle, { color: c.text }]} numberOfLines={1}>
                {n.title || '未命名'}
              </Text>
              <Text style={[s.relatedItemMeta, { color: c.textSecondary }]}>
                {n.in_degree + n.out_degree} 条链接
              </Text>
            </TouchableOpacity>
          ))
        )}
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

      {/* AI Write panel */}
      {showAiWrite && (
        <View style={[s.aiWritePanel, { borderTopColor: c.border, backgroundColor: c.bgSecondary }]}>
          <TextInput
            style={[s.aiWriteInput, { color: c.text, borderColor: c.border }]}
            value={aiPrompt}
            onChangeText={setAiPrompt}
            placeholder="写什么？让 AI 帮你写..."
            placeholderTextColor={c.textSecondary}
            multiline
            textAlignVertical="top"
          />
          <View style={s.aiWriteActions}>
            <TouchableOpacity
              style={[s.aiWriteBtn, { backgroundColor: accentColor }]}
              onPress={handleAiWrite}
              disabled={aiGenerating || !aiPrompt.trim()}
            >
              <Ionicons name="sparkles-outline" size={16} color="#FFF" />
              <Text style={s.aiWriteBtnText}>生成</Text>
            </TouchableOpacity>
            {aiGenerating && <ActivityIndicator color={accentColor} size="small" style={{ marginLeft: 8 }} />}
          </View>
        </View>
      )}

      {/* Toolbar */}
      <View style={[s.toolbar, { borderTopColor: c.border, backgroundColor: c.bgSecondary }]}>
        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
          <TouchableOpacity
            style={[s.toolBtn, { borderColor: previewMode ? accentColor : c.border, backgroundColor: previewMode ? accentColor + '20' : 'transparent' }]}
            onPress={() => setPreviewMode(v => !v)}
            accessibilityLabel={previewMode ? '切换到编辑模式' : '切换到预览模式'}
          >
            <Ionicons name={previewMode ? 'create-outline' : 'eye-outline'} size={16} color={previewMode ? accentColor : c.text} />
          </TouchableOpacity>
          {!previewMode && filterFocusModeToolbar(TOOLBAR, focusMode).map((t) => (
            <TouchableOpacity
              key={t.label}
              style={[s.toolBtn, { borderColor: c.border }]}
              onPress={() => {
                if ((t as any).action === 'aiWrite') {
                  setShowAiWrite(true);
                } else if ((t as any).action === 'aiCmd') {
                  Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(() => {});
                  setShowAiPalette(true);
                } else {
                  insertFormat(t.insert);
                }
              }}
            >
              {(t as any).icon
                ? <Ionicons name={(t as any).icon} size={16} color={c.text} />
                : <Text style={[s.toolLabel, { color: c.text }]}>{t.label}</Text>
              }
            </TouchableOpacity>
          ))}
        </ScrollView>
      </View>

      {/* AI Command Palette — hidden in focus / reading mode (#2894) */}
      {!focusMode && (
      <AiActionPalette
        visible={showAiPalette}
        onClose={() => setShowAiPalette(false)}
        sourceText={content}
        noteId={noteId}
        onInsertResult={(text) => {
          if (!text) return;
          const { start, end } = selectionRef.current;
          setContent(prev => {
            const before = prev.slice(0, start);
            const selected = prev.slice(start, end);
            const after = prev.slice(end);
            const next = before + text + after;
            const newPos = start + text.length;
            selectionRef.current = { start: newPos, end: newPos };
            contentRef.current = next;
            autoSave(titleRef.current, next);
            return next;
          });
        }}
      />
      )}
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
  aiWritePanel: {
    paddingHorizontal: 12, paddingVertical: 8, borderTopWidth: 1,
  },
  aiWriteInput: {
    fontSize: 14, borderWidth: 1, borderRadius: 8,
    paddingHorizontal: 10, paddingVertical: 8, maxHeight: 80,
  },
  aiWriteActions: {
    flexDirection: 'row', alignItems: 'center', marginTop: 8,
  },
  aiWriteBtn: {
    flexDirection: 'row', alignItems: 'center', gap: 4,
    paddingHorizontal: 14, paddingVertical: 7, borderRadius: 8,
  },
  aiWriteBtnText: {
    color: '#FFF', fontSize: 14, fontWeight: '600',
  },
  // 相关笔记（局部图谱，#3832）
  relatedSection: {
    paddingHorizontal: 16, paddingVertical: 10, borderBottomWidth: 1,
  },
  relatedHeader: {
    flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: 8,
  },
  relatedHeaderTitle: { fontSize: 13, fontWeight: '600' },
  relatedLoading: { alignSelf: 'flex-start', marginVertical: 4 },
  relatedEmpty: { fontSize: 13, paddingVertical: 4 },
  relatedItem: {
    flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center',
    borderWidth: 1, borderRadius: 10, paddingHorizontal: 12, paddingVertical: 10,
    marginBottom: 6, gap: 8,
  },
  relatedItemTitle: { flex: 1, fontSize: 15, fontWeight: '500' },
  relatedItemMeta: { fontSize: 12 },
});
