import React, { useState, useEffect, useRef } from 'react';
import {
  View, Text, TextInput, TouchableOpacity, StyleSheet, ScrollView, Alert, ActivityIndicator,
} from 'react-native';
import { useAppStore, getColors } from '../store';
import { getNote, updateNote, deleteNote } from '../db';

export default function NoteEditorScreen({ route, navigation }: any) {
  const { noteId } = route.params;
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [title, setTitle] = useState('');
  const [content, setContent] = useState('');
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(true);
  const timerRef = useRef<any>(null);
  const pendingRef = useRef<{ title: string; content: string } | null>(null);
  const selectionRef = useRef<{ start: number; end: number }>({ start: 0, end: 0 });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const note = await getNote(noteId);
        if (cancelled) return;
        if (note) {
          setTitle(note.title);
          setContent(note.content);
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

  const save = async (t?: string, ct?: string) => {
    setSaving(true);
    try {
      await updateNote(noteId, t ?? title, ct ?? content);
    } catch (e) {
      console.warn('[NoteEditor] Save failed:', e);
      Alert.alert('保存失败', String(e));
    } finally {
      setSaving(false);
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
      clearTimeout(timerRef.current);
      if (pendingRef.current) {
        // Fire-and-forget save on unmount — component state is already gone
        updateNote(noteId, pendingRef.current.title, pendingRef.current.content);
        pendingRef.current = null;
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

  const insertFormat = (syntax: string) => {
    const { start, end } = selectionRef.current;
    const isPrefix = syntax.endsWith(' ');
    setContent(prev => {
      const before = prev.slice(0, start);
      const selected = prev.slice(start, end);
      const after = prev.slice(end);
      return isPrefix
        ? before + syntax + selected + after
        : before + syntax + selected + syntax + after;
    });
  };

  if (loading) {
    return (
      <View style={[s.container, { backgroundColor: c.bg, justifyContent: 'center', alignItems: 'center' }]}>
        <ActivityIndicator color={accentColor} size="large" />
      </View>
    );
  }

  return (
    <View style={[s.container, { backgroundColor: c.bg }]}>
      {/* Header */}
      <View style={[s.header, { borderBottomColor: c.border }]}>
        <TouchableOpacity onPress={() => navigation.goBack()}>
          <Text style={[s.headerBtn, { color: accentColor }]}>← 返回</Text>
        </TouchableOpacity>
        <Text style={[s.headerTitle, { color: c.textSecondary }]}>
          {saving ? '保存中...' : '已保存'}
        </Text>
        <TouchableOpacity onPress={handleDelete}>
          <Text style={[s.headerBtn, { color: '#EF4444' }]}>删除</Text>
        </TouchableOpacity>
      </View>

      {/* Title */}
      <TextInput
        style={[s.titleInput, { color: c.text }]}
        value={title}
        onChangeText={(t) => { setTitle(t); autoSave(t, content); }}
        placeholder="笔记标题"
        placeholderTextColor={c.textSecondary}
      />

      {/* Content */}
      <TextInput
        style={[s.contentInput, { color: c.text }]}
        value={content}
        onChangeText={(t) => { setContent(t); autoSave(title, t); }}
        onSelectionChange={(e) => { selectionRef.current = e.nativeEvent.selection; }}
        placeholder="开始写作..."
        placeholderTextColor={c.textSecondary}
        multiline
        textAlignVertical="top"
      />

      {/* Toolbar */}
      <View style={[s.toolbar, { borderTopColor: c.border, backgroundColor: c.bgSecondary }]}>
        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
          {TOOLBAR.map((t) => (
            <TouchableOpacity
              key={t.label}
              style={[s.toolBtn, { borderColor: c.border }]}
              onPress={() => insertFormat(t.insert)}
            >
              <Text style={[s.toolLabel, { color: c.text }]}>{t.label}</Text>
            </TouchableOpacity>
          ))}
        </ScrollView>
      </View>
    </View>
  );
}

const s = StyleSheet.create({
  container: { flex: 1 },
  header: {
    flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center',
    paddingHorizontal: 16, paddingVertical: 12, borderBottomWidth: 1,
  },
  headerBtn: { fontSize: 16, fontWeight: '500' },
  headerTitle: { fontSize: 13 },
  titleInput: {
    fontSize: 22, fontWeight: '700', paddingHorizontal: 16,
    paddingTop: 16, paddingBottom: 8,
  },
  contentInput: {
    flex: 1, fontSize: 16, lineHeight: 24, paddingHorizontal: 16, paddingTop: 8,
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
