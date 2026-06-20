import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  View, Text, FlatList, TouchableOpacity, TextInput, StyleSheet, Alert, ActivityIndicator,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Haptics from 'expo-haptics';
import { useAppStore, getColors } from '../store';
import { getNotes, createNote, deleteNote, toggleStar, searchNotes, DbNote } from '../db';

export default function NotesScreen({ navigation }: any) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [notes, setNotes] = useState<DbNote[]>([]);
  const [search, setSearch] = useState('');
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const requestIdRef = useRef(0);

  const load = useCallback(async (query: string) => {
    const currentId = ++requestIdRef.current;
    try {
      const data = query ? await searchNotes(query) : await getNotes();
      if (requestIdRef.current !== currentId) return; // stale, discard
      setNotes(data);
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
    const timer = setTimeout(() => load(search), 300);
    return () => clearTimeout(timer);
  }, [search, load]);

  const handleNew = async () => {
    try {
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      const id = await createNote();
      navigation.navigate('NoteEdit', { noteId: id });
    } catch (e: any) {
      Alert.alert('创建失败', e.message || '请重试');
    }
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    await load(search);
    setRefreshing(false);
  };

  const handleLongPress = (item: DbNote) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    Alert.alert(item.title || '笔记操作', '', [
      { text: item.starred ? '取消收藏' : '收藏', onPress: async () => {
        try { await toggleStar(item.id); await load(search); } catch (e: any) { Alert.alert('操作失败', e.message || '请重试'); }
      }},
      { text: '删除', style: 'destructive', onPress: () => handleDelete(item.id) },
      { text: '取消', style: 'cancel' },
    ]);
  };

  const handleDelete = (id: string) => {
    Alert.alert('删除笔记', '确定要删除吗？', [
      { text: '取消', style: 'cancel' },
      { text: '删除', style: 'destructive', onPress: async () => {
        try { await deleteNote(id); await load(search); } catch (e: any) { Alert.alert('删除失败', e.message || '请重试'); }
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
        <Text style={[s.cardTitle, { color: c.text }]} numberOfLines={1}>
          {item.starred ? '⭐ ' : ''}{item.title}
        </Text>
        <Text style={[s.cardTime, { color: c.textSecondary }]}>{fmtTime(item.updated_at)}</Text>
      </View>
      <Text style={[s.cardPreview, { color: c.textSecondary }]} numberOfLines={2}>
        {item.content || '空白笔记'}
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

  return (
    <SafeAreaView style={[s.container, { backgroundColor: c.bg }]}>
      {/* Search bar */}
      <View style={[s.searchBar, { borderColor: c.border }]}>
        <Text style={{ color: c.textSecondary, fontSize: 16 }}>🔍 </Text>
        <TextInput
          style={[s.searchInput, { color: c.text }]}
          placeholder="搜索笔记..."
          placeholderTextColor={c.textSecondary}
          value={search}
          onChangeText={setSearch}
        />
      </View>

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

      {/* FAB */}
      <TouchableOpacity
        style={[s.fab, { backgroundColor: accentColor }]}
        onPress={handleNew}
      >
        <Text style={s.fabText}>+</Text>
      </TouchableOpacity>
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
});
