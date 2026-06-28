import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  View, Text, FlatList, TouchableOpacity, TextInput, StyleSheet, Alert, ActivityIndicator,
  Animated, Easing, Modal, ScrollView,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Haptics from 'expo-haptics';
import * as Clipboard from 'expo-clipboard';
import Ionicons from '@expo/vector-icons/Ionicons';
import { useAppStore, getColors } from '../store';
import {
  getNotes, createNote, deleteNote, toggleStar, searchNotes, getFolders, DbNote,
  getTemplates, instantiateTemplate, extractTemplateFields,
  searchNotesByIds, buildStudioContext, StudioSourceNote,
} from '../db';
import { chat, ChatMessage, parseSSEStream } from '../api/client';

export default function NotesScreen({ navigation }: any) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [notes, setNotes] = useState<DbNote[]>([]);
  const [search, setSearch] = useState('');
  const [folders, setFolders] = useState<string[]>([]);
  const [activeFolder, setActiveFolder] = useState<string | undefined>(undefined);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const requestIdRef = useRef(0);

  // #2154 — expandable FAB + template picker
  const [fabOpen, setFabOpen] = useState(false);
  const spinRef = useRef(new Animated.Value(0));
  const blankRef = useRef(new Animated.Value(0));
  const tplRef = useRef(new Animated.Value(0));
  const studioRef = useRef(new Animated.Value(0));
  const [showTemplatePicker, setShowTemplatePicker] = useState(false);
  const [templates, setTemplates] = useState<DbNote[]>([]);
  const [fieldTemplate, setFieldTemplate] = useState<DbNote | null>(null);
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});

  // #2166 — Studio delivery types
  const [showStudio, setShowStudio] = useState(false);
  const [studioGenerating, setStudioGenerating] = useState(false);
  const [studioProgress, setStudioProgress] = useState('');

  const STUDIO_TYPES = [
    { key: 'study-guide', label: '学习指南', icon: 'school-outline' as const, desc: '核心概念 + 关键术语 + 自测问题' },
    { key: 'faq', label: 'FAQ', icon: 'help-circle-outline' as const, desc: '5-10 个高频问题与解答' },
    { key: 'quiz', label: '测验', icon: 'checkbox-outline' as const, desc: '选择题 + 答案与解析，可导入闪卡' },
    { key: 'timeline', label: '时间线', icon: 'time-outline' as const, desc: '按时间顺序排列的事件表' },
    { key: 'briefing', label: '简报', icon: 'document-text-outline' as const, desc: '结构化摘要 + 关键要点 + 源笔记引用' },
  ] as const;

  const load = useCallback(async (query: string, folder?: string) => {
    const currentId = ++requestIdRef.current;
    try {
      const data = query ? await searchNotes(query, folder) : await getNotes(folder);
      const folderList = await getFolders();
      if (requestIdRef.current !== currentId) return;
      setNotes(data);
      setFolders(folderList);
    } catch (e: any) {
      if (requestIdRef.current !== currentId) return;
      console.warn('[Notes] load failed:', e);
      Alert.alert('加载失败', e.message || '请重试');
    } finally {
      if (requestIdRef.current === currentId) setLoading(false);
    }
  }, []);

  // Debounce search: wait 300ms after last keystroke before querying
  useEffect(() => {
    const timer = setTimeout(() => load(search, activeFolder), 300);
    return () => clearTimeout(timer);
  }, [search, activeFolder, load]);

  // FAB expand/collapse animation (#2154)
  const animateFab = (open: boolean) => {
    const to = open ? 1 : 0;
    Animated.parallel([
      Animated.timing(spinRef.current, { toValue: to, duration: 200, easing: Easing.out(Easing.quad), useNativeDriver: true }),
      Animated.timing(blankRef.current, { toValue: to, duration: 200, easing: Easing.out(Easing.quad), useNativeDriver: true }),
      Animated.timing(tplRef.current, { toValue: to, duration: 200, easing: Easing.out(Easing.quad), useNativeDriver: true }),
      Animated.timing(studioRef.current, { toValue: to, duration: 200, easing: Easing.out(Easing.quad), useNativeDriver: true }),
    ]).start();
  };

  const toggleFab = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e));
    const next = !fabOpen;
    setFabOpen(next);
    animateFab(next);
  };

  const closeFab = () => {
    if (fabOpen) { setFabOpen(false); animateFab(false); }
  };

  const handleNewBlank = async () => {
    closeFab();
    try {
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e));
      const id = await createNote();
      navigation.navigate('NoteEdit', { noteId: id });
    } catch (e: any) {
      Alert.alert('创建失败', e.message || '请重试');
    }
  };

  const openTemplatePicker = async () => {
    closeFab();
    try {
      const tpls = await getTemplates();
      setTemplates(tpls);
      setShowTemplatePicker(true);
    } catch (e: any) {
      Alert.alert('加载模板失败', e.message || '请重试');
    }
  };

  const selectTemplate = async (tpl: DbNote) => {
    const fields = extractTemplateFields(tpl.content);
    if (fields.length > 0) {
      // Has custom fields — show the field-fill sheet first.
      setFieldValues({});
      setFieldTemplate(tpl);
      setShowTemplatePicker(false);
    } else {
      // No custom fields — instantiate directly.
      try {
        const id = await instantiateTemplate(tpl.id);
        setShowTemplatePicker(false);
        navigation.navigate('NoteEdit', { noteId: id });
      } catch (e: any) {
        Alert.alert('创建失败', e.message || '请重试');
      }
    }
  };

  const confirmFieldTemplate = async () => {
    if (!fieldTemplate) return;
    try {
      const id = await instantiateTemplate(fieldTemplate.id, fieldValues);
      setFieldTemplate(null);
      navigation.navigate('NoteEdit', { noteId: id });
    } catch (e: any) {
      Alert.alert('创建失败', e.message || '请重试');
    }
  };

  const handleDeleteTemplate = (tpl: DbNote) => {
    Alert.alert('删除模板', `确定删除「${tpl.title}」？此操作不影响已用该模板创建的笔记。`, [
      { text: '取消', style: 'cancel' },
      { text: '删除', style: 'destructive', onPress: async () => {
        try {
          await deleteNote(tpl.id);
          setTemplates(prev => prev.filter(t => t.id !== tpl.id));
        } catch (e: any) { Alert.alert('删除失败', e.message || '请重试'); }
      } },
    ]);
  };

  // #2166 — Studio: open panel
  const openStudio = () => {
    closeFab();
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e));
    setShowStudio(true);
  };

  // #2166 — Studio: generate a delivery type from the current note list
  const handleStudioGenerate = async (type: typeof STUDIO_TYPES[number]) => {
    if (studioGenerating) return;
    setStudioGenerating(true);
    setStudioProgress(`正在生成${type.label}...`);

    try {
      // Collect the current visible notes (or current folder if filtered)
      const sourceIds = notes.map(n => n.id);
      if (sourceIds.length === 0) {
        Alert.alert('没有源笔记', '当前视图没有笔记可供生成。请先创建一些笔记。');
        setStudioGenerating(false);
        setStudioProgress('');
        return;
      }

      const sources = await searchNotesByIds(sourceIds);
      const context = buildStudioContext(sources);

      // System prompt per delivery type
      const SYSTEM_PROMPTS: Record<string, string> = {
        'study-guide': 'You are a study guide creator. Based on the source notes below, create a comprehensive study guide including:\n- Core concepts explained\n- Key terminology glossary\n- Self-assessment questions\n\nWrite the output in Chinese as a structured markdown note. Use [[wikilinks]] to reference source notes by their title when citing specific content. Start with an appropriate title as a level-1 heading, then the study guide content.',
        'faq': 'You are an FAQ generator. Based on the source notes below, create 5-10 frequently asked questions with detailed answers extracted from the source material.\n\nWrite the output in Chinese as a structured markdown note. Use [[wikilinks]] to reference source notes by their title. Start with an appropriate title as a level-1 heading.',
        'quiz': 'You are a quiz creator. Based on the source notes below, create multiple-choice quiz questions (with 4 options each) covering the key concepts. Include the correct answer and a brief explanation for each question.\n\nWrite the output in Chinese as a structured markdown note. Use [[wikilinks]] to reference source notes. Start with an appropriate title as a level-1 heading. Use markdown format:\n\n## Question 1\n\nA) ...\nB) ...\nC) ...\nD) ...\n\n**正确答案:** ...\n**解析:** ...',
        'timeline': 'You are a timeline creator. Based on the source notes below, extract all events, dates, and chronological information and organize them into a structured timeline.\n\nWrite the output in Chinese as a structured markdown note. For each event, include: date/time period, event description, and [[wikilinks]] to source notes. Order chronologically. Start with an appropriate title as a level-1 heading.',
        'briefing': 'You are a briefing document creator. Based on the source notes below, create a concise executive briefing including:\n- Executive summary (3-5 sentences)\n- Key takeaways (bullet points)\n- Supporting evidence with [[wikilinks]] to source notes\n- Next steps or recommendations\n\nWrite the output in Chinese as structured markdown. Start with an appropriate title as a level-1 heading.',
      };
      const tags: Record<string, string> = {
        'study-guide': 'studio/study-guide',
        'faq': 'studio/faq',
        'quiz': 'studio/quiz',
        'timeline': 'studio/timeline',
        'briefing': 'studio/briefing',
      };

      const systemPrompt = SYSTEM_PROMPTS[type.key] || SYSTEM_PROMPTS['briefing'];

      const messages: ChatMessage[] = [
        { role: 'system', content: systemPrompt },
        { role: 'user', content: `Generate a ${type.label} from the following source notes:\n\n${context}` },
      ];

      const stream = await chat(messages);
      let full = '';
      await parseSSEStream(stream, (chunk) => {
        if (chunk.done) return;
        if (chunk.content) full += chunk.content;
      });

      if (!full) {
        Alert.alert('生成失败', 'AI 返回结果为空，请重试。');
        setStudioGenerating(false);
        setStudioProgress('');
        return;
      }

      // Save as a new note
      // Extract title from first heading or use default
      const titleMatch = full.match(/^#\s+(.+)/m);
      const noteTitle = titleMatch ? titleMatch[1].trim() : `${type.label} — ${new Date().toLocaleDateString('zh-CN')}`;
      const tagValue = tags[type.key] || 'studio';
      const noteId = await createNote(noteTitle, full);
      // Import addTag dynamically (already imported via db.ts, but addTag isn't imported in this file yet)
      const { addTag } = await import('../db');
      await addTag(noteId, tagValue);

      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success).catch(() => {});
      setShowStudio(false);
      setStudioGenerating(false);
      setStudioProgress('');
      navigation.navigate('NoteEdit', { noteId });
    } catch (e: any) {
      Alert.alert('生成失败', e.message || '请检查 API 设置后重试。');
      setStudioGenerating(false);
      setStudioProgress('');
    }
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await load(search, activeFolder);
    } finally {
      setRefreshing(false);
    }
  };

  const handleLongPress = (item: DbNote) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium).catch(e => console.warn('[Haptics] error:', e));
    Alert.alert(item.title || '笔记操作', '', [
      { text: item.starred ? '取消收藏' : '收藏', onPress: async () => {
        try { await toggleStar(item.id); await load(search, activeFolder); } catch (e: any) { Alert.alert('操作失败', e.message || '请重试'); }
      }},
      { text: '复制内容', onPress: () => {
        const text = item.content ? (item.title ? `${item.title}\n\n${item.content}` : item.content) : '';
        if (text) Clipboard.setStringAsync(text);
      }},
      { text: '删除', style: 'destructive', onPress: () => handleDelete(item.id) },
      { text: '取消', style: 'cancel' },
    ]);
  };

  const handleDelete = (id: string) => {
    Alert.alert('删除笔记', '确定要删除吗？', [
      { text: '取消', style: 'cancel' },
      { text: '删除', style: 'destructive', onPress: async () => {
        try { await deleteNote(id); await load(search, activeFolder); } catch (e: any) { Alert.alert('删除失败', e.message || '请重试'); }
      }},
    ]);
  };

  const fmtTime = (ts: number) => {
    const d = new Date(ts * 1000);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
    return d.toLocaleDateString('zh-CN', { month: 'short', day: 'numeric' });
  };

  const renderItem = ({ item }: { item: DbNote }) => (
    <TouchableOpacity
      style={[s.card, { backgroundColor: c.card, borderColor: c.border }]}
      onPress={() => navigation.navigate('NoteEdit', { noteId: item.id })}
      onLongPress={() => handleLongPress(item)}
    >
      <View style={s.cardHeader}>
        <View style={{ flexDirection: 'row', alignItems: 'center', flex: 1 }}>
          {item.starred && <Ionicons name="star" size={14} color="#F59E0B" style={{ marginRight: 4 }} />}
          <Text style={[s.cardTitle, { color: c.text }]} numberOfLines={1}>{item.title}</Text>
        </View>
        <Text style={[s.cardTime, { color: c.textSecondary }]}>{fmtTime(item.updated_at)}</Text>
      </View>
      <Text style={[s.cardPreview, { color: c.textSecondary }]} numberOfLines={2}>
        {item.folder ? `[${item.folder}] ` : ''}{item.content || '空白笔记'}
      </Text>
    </TouchableOpacity>
  );

  if (loading) {
    return (
      <SafeAreaView style={[s.container, { backgroundColor: c.bg, justifyContent: 'center', alignItems: 'center' }]}>
        <ActivityIndicator color={accentColor} size="large" />
      </SafeAreaView>
    );
  }

  // Interpolations for the expandable FAB actions
  const spin = spinRef.current.interpolate({ inputRange: [0, 1], outputRange: ['0deg', '45deg'] });
  const blankScale = blankRef.current.interpolate({ inputRange: [0, 1], outputRange: [0, 1] });
  const blankY = blankRef.current.interpolate({ inputRange: [0, 1], outputRange: [0, -64] });
  const tplScale = tplRef.current.interpolate({ inputRange: [0, 1], outputRange: [0, 1] });
  const tplY = tplRef.current.interpolate({ inputRange: [0, 1], outputRange: [0, -124] });
  const studioScale = studioRef.current.interpolate({ inputRange: [0, 1], outputRange: [0, 1] });
  const studioY = studioRef.current.interpolate({ inputRange: [0, 1], outputRange: [0, -184] });

  return (
    <SafeAreaView style={[s.container, { backgroundColor: c.bg }]}>
      {/* Search bar */}
      <View style={[s.searchBar, { borderColor: c.border }]}>
        <Ionicons name="search-outline" size={18} color={c.textSecondary} style={{ marginRight: 6 }} />
        <TextInput
          style={[s.searchInput, { color: c.text }]}
          placeholder="搜索笔记..."
          placeholderTextColor={c.textSecondary}
          value={search}
          onChangeText={setSearch}
        />
      </View>

      {/* Folder chips */}
      {folders.length > 0 && (
        <View style={s.chipRow}>
          <TouchableOpacity
            style={[s.chip, { backgroundColor: activeFolder === undefined ? accentColor : c.card, borderColor: c.border }]}
            onPress={() => setActiveFolder(undefined)}
          >
            <Text style={[s.chipText, { color: activeFolder === undefined ? '#FFF' : c.text }]}>全部</Text>
          </TouchableOpacity>
          {folders.map(f => (
            <TouchableOpacity
              key={f}
              style={[s.chip, { backgroundColor: activeFolder === f ? accentColor : c.card, borderColor: c.border }]}
              onPress={() => setActiveFolder(activeFolder === f ? undefined : f)}
            >
              <Text style={[s.chipText, { color: activeFolder === f ? '#FFF' : c.text }]}>{f}</Text>
            </TouchableOpacity>
          ))}
        </View>
      )}

      {/* Notes list */}
      <FlatList
        data={notes}
        renderItem={renderItem}
        keyExtractor={item => item.id}
        contentContainerStyle={{ padding: 16 }}
        refreshing={refreshing}
        onRefresh={handleRefresh}
        ListEmptyComponent={
          <View style={s.empty}>
            <Text style={[s.emptyText, { color: c.textSecondary }]}>
              {search ? '没有找到笔记' : '点击右下角新建笔记'}
            </Text>
          </View>
        }
      />

      {/* Expandable FAB (#2154) — single "+" expands into three actions */}
      {/* Action: Studio (#2166) */}
      <Animated.View
        pointerEvents={fabOpen ? 'auto' : 'none'}
        style={[s.fabAction, { backgroundColor: c.card, borderColor: c.border, right: 28, transform: [{ translateY: studioY }, { scale: studioScale }], opacity: studioRef.current }]}
      >
        <TouchableOpacity style={s.fabActionBtn} onPress={openStudio}>
          <Ionicons name="flask-outline" size={22} color={accentColor} />
          <Text style={[s.fabActionLabel, { color: c.text }]}>Studio</Text>
        </TouchableOpacity>
      </Animated.View>
      {/* Action: from template */}
      <Animated.View
        pointerEvents={fabOpen ? 'auto' : 'none'}
        style={[s.fabAction, { backgroundColor: c.card, borderColor: c.border, right: 28, transform: [{ translateY: tplY }, { scale: tplScale }], opacity: tplRef.current }]}
      >
        <TouchableOpacity style={s.fabActionBtn} onPress={openTemplatePicker}>
          <Ionicons name="layers-outline" size={22} color={accentColor} />
          <Text style={[s.fabActionLabel, { color: c.text }]}>模板</Text>
        </TouchableOpacity>
      </Animated.View>
      {/* Action: blank note */}
      <Animated.View
        pointerEvents={fabOpen ? 'auto' : 'none'}
        style={[s.fabAction, { backgroundColor: c.card, borderColor: c.border, right: 28, transform: [{ translateY: blankY }, { scale: blankScale }], opacity: blankRef.current }]}
      >
        <TouchableOpacity style={s.fabActionBtn} onPress={handleNewBlank}>
          <Ionicons name="document-text-outline" size={22} color={accentColor} />
          <Text style={[s.fabActionLabel, { color: c.text }]}>空白</Text>
        </TouchableOpacity>
      </Animated.View>
      {/* Main toggle button */}
      <TouchableOpacity
        style={[s.fab, { backgroundColor: accentColor }]}
        onPress={toggleFab}
      >
        <Animated.View style={{ transform: [{ rotate: spin }] }}>
          <Text style={s.fabText}>+</Text>
        </Animated.View>
      </TouchableOpacity>

      {/* Template picker — bottom sheet (not a full-screen mask) */}
      <Modal
        visible={showTemplatePicker}
        transparent
        animationType="slide"
        onRequestClose={() => setShowTemplatePicker(false)}
      >
        <TouchableOpacity style={s.sheetBackdrop} activeOpacity={1} onPress={() => setShowTemplatePicker(false)}>
          <TouchableOpacity style={[s.sheet, { backgroundColor: c.bg }]} activeOpacity={1} onPress={() => {}}>
            <View style={[s.sheetHandle, { backgroundColor: c.border }]} />
            <Text style={[s.sheetTitle, { color: c.text }]}>从模板新建</Text>
            <ScrollView style={{ maxHeight: 420 }}>
              {templates.length === 0 ? (
                <Text style={[s.sheetEmpty, { color: c.textSecondary }]}>
                  暂无模板。可在笔记编辑页用「存为模板」创建。
                </Text>
              ) : (
                templates.map(tpl => (
                  <TouchableOpacity
                    key={tpl.id}
                    style={[s.tplItem, { borderColor: c.border, backgroundColor: c.card }]}
                    onPress={() => selectTemplate(tpl)}
                    onLongPress={() => handleDeleteTemplate(tpl)}
                  >
                    <View style={s.tplItemHeader}>
                      <Ionicons name="document-text-outline" size={16} color={accentColor} />
                      <Text style={[s.tplTitle, { color: c.text }]} numberOfLines={1}>{tpl.title}</Text>
                    </View>
                    <Text style={[s.tplPreview, { color: c.textSecondary }]} numberOfLines={2}>
                      {tpl.content || '空白模板'}
                    </Text>
                  </TouchableOpacity>
                ))
              )}
            </ScrollView>
            <Text style={[s.sheetHint, { color: c.textSecondary }]}>长按模板可删除</Text>
          </TouchableOpacity>
        </TouchableOpacity>
      </Modal>

      {/* Field-fill sheet — for templates with custom {{field:...}} placeholders */}
      <Modal
        visible={!!fieldTemplate}
        transparent
        animationType="slide"
        onRequestClose={() => setFieldTemplate(null)}
      >
        <TouchableOpacity style={s.sheetBackdrop} activeOpacity={1} onPress={() => setFieldTemplate(null)}>
          <TouchableOpacity style={[s.sheet, { backgroundColor: c.bg }]} activeOpacity={1} onPress={() => {}}>
            <View style={[s.sheetHandle, { backgroundColor: c.border }]} />
            <Text style={[s.sheetTitle, { color: c.text }]}>填充：{fieldTemplate?.title}</Text>
            <ScrollView style={{ maxHeight: 360 }}>
              {fieldTemplate && extractTemplateFields(fieldTemplate.content).map(label => (
                <View key={label} style={s.fieldRow}>
                  <Text style={[s.fieldLabel, { color: c.textSecondary }]}>{label}</Text>
                  <TextInput
                    style={[s.fieldInput, { color: c.text, borderColor: c.border, backgroundColor: c.card }]}
                    value={fieldValues[label] ?? ''}
                    onChangeText={(v) => setFieldValues(prev => ({ ...prev, [label]: v }))}
                    placeholder={`输入${label}（可留空）`}
                    placeholderTextColor={c.textSecondary}
                  />
                </View>
              ))}
            </ScrollView>
            <TouchableOpacity
              style={[s.fieldConfirm, { backgroundColor: accentColor }]}
              onPress={confirmFieldTemplate}
            >
              <Text style={s.fieldConfirmText}>创建笔记</Text>
            </TouchableOpacity>
          </TouchableOpacity>
        </TouchableOpacity>
      </Modal>

      {/* #2166 — Studio panel bottom sheet */}
      <Modal
        visible={showStudio}
        transparent
        animationType="slide"
        onRequestClose={() => { if (!studioGenerating) setShowStudio(false); }}
      >
        <TouchableOpacity
          style={s.sheetBackdrop}
          activeOpacity={1}
          onPress={() => { if (!studioGenerating) setShowStudio(false); }}
        >
          <TouchableOpacity style={[s.sheet, { backgroundColor: c.bg }]} activeOpacity={1} onPress={() => {}}>
            <View style={[s.sheetHandle, { backgroundColor: c.border }]} />

            <View style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', marginBottom: 12 }}>
              <Text style={[s.sheetTitle, { color: c.text, marginBottom: 0 }]}>Studio — 一键生成</Text>
              <Text style={[s.sheetSubtitle, { color: c.textSecondary }]}>
                来源：{notes.length} 条笔记
              </Text>
            </View>

            {studioGenerating ? (
              <View style={{ paddingVertical: 24, alignItems: 'center' }}>
                <ActivityIndicator color={accentColor} size="large" />
                <Text style={{ color: c.textSecondary, marginTop: 12, fontSize: 14 }}>{studioProgress}</Text>
              </View>
            ) : (
              <>
                <ScrollView style={{ maxHeight: 380 }}>
                  {STUDIO_TYPES.map(t => (
                    <TouchableOpacity
                      key={t.key}
                      style={[s.studioItem, { borderColor: c.border, backgroundColor: c.card }]}
                      onPress={() => handleStudioGenerate(t)}
                    >
                      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
                        <View style={[s.studioIcon, { backgroundColor: accentColor + '18' }]}>
                          <Ionicons name={t.icon} size={20} color={accentColor} />
                        </View>
                        <View style={{ flex: 1 }}>
                          <Text style={[s.studioItemTitle, { color: c.text }]}>{t.label}</Text>
                          <Text style={[s.studioItemDesc, { color: c.textSecondary }]}>{t.desc}</Text>
                        </View>
                        <Ionicons name="chevron-forward-outline" size={16} color={c.textSecondary} />
                      </View>
                    </TouchableOpacity>
                  ))}
                </ScrollView>
                <Text style={[s.sheetHint, { color: c.textSecondary }]}>
                  将使用当前筛选视图中的 {notes.length} 条笔记作为源材料生成{'\n'}生成后自动保存为新笔记
                </Text>
              </>
            )}
          </TouchableOpacity>
        </TouchableOpacity>
      </Modal>
    </SafeAreaView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1 },
  searchBar: {
    flexDirection: 'row', alignItems: 'center',
    margin: 16, marginBottom: 0, paddingHorizontal: 12,
    borderWidth: 1, borderRadius: 10, height: 40,
  },
  searchInput: { flex: 1, fontSize: 15, padding: 0 },
  chipRow: { flexDirection: 'row', paddingHorizontal: 16, paddingVertical: 8, gap: 8 },
  chip: { paddingHorizontal: 12, paddingVertical: 6, borderRadius: 16, borderWidth: 1 },
  chipText: { fontSize: 13 },
  card: { borderWidth: 1, borderRadius: 12, padding: 14, marginBottom: 10 },
  cardHeader: { flexDirection: 'row', justifyContent: 'space-between', marginBottom: 6 },
  cardTitle: { fontSize: 16, fontWeight: '600', flex: 1 },
  cardTime: { fontSize: 12 },
  cardPreview: { fontSize: 14, lineHeight: 20 },
  empty: { alignItems: 'center', marginTop: 60 },
  emptyText: { fontSize: 15 },
  fab: {
    position: 'absolute', right: 20, bottom: 20,
    width: 56, height: 56, borderRadius: 28,
    justifyContent: 'center', alignItems: 'center',
    elevation: 4, shadowColor: '#000', shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.25, shadowRadius: 4,
  },
  fabText: { color: '#FFF', fontSize: 28, lineHeight: 30 },
  // #2154 expandable FAB actions
  fabAction: {
    position: 'absolute', bottom: 20,
    width: 110, height: 46, borderRadius: 23, borderWidth: 1,
    justifyContent: 'center',
    elevation: 3, shadowColor: '#000', shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.2, shadowRadius: 3,
  },
  fabActionBtn: { flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 6 },
  fabActionLabel: { fontSize: 14, fontWeight: '500' },
  // Bottom sheet
  sheetBackdrop: { flex: 1, justifyContent: 'flex-end', backgroundColor: 'rgba(0,0,0,0.35)' },
  sheet: {
    borderTopLeftRadius: 18, borderTopRightRadius: 18,
    paddingHorizontal: 16, paddingBottom: 28, paddingTop: 8,
  },
  sheetHandle: { width: 40, height: 4, borderRadius: 2, alignSelf: 'center', marginBottom: 12 },
  sheetTitle: { fontSize: 17, fontWeight: '600', marginBottom: 12 },
  sheetEmpty: { fontSize: 14, paddingVertical: 24, textAlign: 'center' },
  sheetHint: { fontSize: 12, textAlign: 'center', marginTop: 12 },
  tplItem: { borderWidth: 1, borderRadius: 10, padding: 12, marginBottom: 10 },
  tplItemHeader: { flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: 4 },
  tplTitle: { fontSize: 15, fontWeight: '600', flex: 1 },
  tplPreview: { fontSize: 13, lineHeight: 18 },
  // Field fill
  fieldRow: { marginBottom: 12 },
  fieldLabel: { fontSize: 13, marginBottom: 6 },
  fieldInput: { fontSize: 15, borderWidth: 1, borderRadius: 8, paddingHorizontal: 12, paddingVertical: 10 },
  fieldConfirm: { borderRadius: 10, paddingVertical: 13, alignItems: 'center', marginTop: 8 },
  fieldConfirmText: { color: '#FFF', fontSize: 16, fontWeight: '600' },
  // #2166 Studio
  studioItem: { borderWidth: 1, borderRadius: 12, padding: 14, marginBottom: 10 },
  studioIcon: { width: 36, height: 36, borderRadius: 18, justifyContent: 'center', alignItems: 'center' },
  studioItemTitle: { fontSize: 15, fontWeight: '600', marginBottom: 2 },
  studioItemDesc: { fontSize: 12, lineHeight: 16 },
  sheetSubtitle: { fontSize: 12 },
});
