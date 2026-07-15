import React, { useState, useCallback, useRef } from 'react';
import {
  View, Text, TextInput, FlatList, TouchableOpacity, StyleSheet, ActivityIndicator,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useAppStore, getColors } from '../store';
import { globalSearch, GlobalSearchResult } from '../db';
import Icon from '../components/Icon';
import { fmtTime } from '../utils/timeFormat';

export default function SearchScreen({ navigation }: any) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<GlobalSearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestIdRef = useRef(0);

  const doSearch = useCallback(async (q: string) => {
    if (!q.trim()) { setResults([]); setSearched(false); return; }
    const currentId = ++requestIdRef.current;
    setLoading(true);
    setError(null);
    try {
      const data = await globalSearch(q.trim());
      if (requestIdRef.current !== currentId) return;
      setResults(data);
      setSearched(true);
    } catch (e) {
      if (requestIdRef.current !== currentId) return;
      console.warn('[Search] failed:', e);
      setError(e instanceof Error ? e.message : '搜索失败，请重试');
      setSearched(true);
      setResults([]);
    } finally {
      if (requestIdRef.current === currentId) setLoading(false);
    }
  }, []);

  const handlePress = (item: GlobalSearchResult) => {
    if (item.type === 'note') {
      navigation.navigate('Notes', { screen: 'NoteEdit', params: { noteId: item.id } });
    } else {
      navigation.navigate('Chat', {
        screen: 'ChatMain',
        params: { sessionId: item.sessionId, title: item.title },
      });
    }
  };

  const renderItem = ({ item }: { item: GlobalSearchResult }) => (
    <TouchableOpacity
      style={[s.card, { backgroundColor: c.card, borderColor: c.border }]}
      onPress={() => handlePress(item)}
      activeOpacity={0.7}
    >
      <View style={s.cardHeader}>
        <Text style={[s.typeTag, { backgroundColor: item.type === 'note' ? '#10B981' : accentColor }]}>
          {item.type === 'note' ? '笔记' : '对话'}
        </Text>
        <Text style={[s.title, { color: c.text }]} numberOfLines={1}>{item.title}</Text>
        <Text style={[s.time, { color: c.textSecondary }]}>{fmtTime(item.updated_at)}</Text>
      </View>
      {item.snippet ? (
        <Text style={[s.snippet, { color: c.textSecondary }]} numberOfLines={2}>{item.snippet}</Text>
      ) : null}
    </TouchableOpacity>
  );

  return (
    <SafeAreaView style={[s.container, { backgroundColor: c.bg }]}>
      <View style={[s.searchBar, { borderColor: c.border }]}>
        <Icon name="search" size={16} color={c.textSecondary} />
        <TextInput
          style={[s.searchInput, { color: c.text }]}
          placeholder="搜索对话和笔记..."
          placeholderTextColor={c.textSecondary}
          value={query}
          onChangeText={setQuery}
          onSubmitEditing={() => doSearch(query)}
          returnKeyType="search"
          autoFocus
        />
        {query.length > 0 && (
          <TouchableOpacity onPress={() => { setQuery(''); setResults([]); setSearched(false); }}>
            <Text style={{ color: c.textSecondary, fontSize: 16 }}>✕</Text>
          </TouchableOpacity>
        )}
      </View>

      {loading ? (
        <View style={s.center}>
          <ActivityIndicator color={accentColor} size="large" />
        </View>
      ) : (
        <FlatList
          data={results}
          renderItem={renderItem}
          keyExtractor={item => `${item.type}-${item.id}`}
          contentContainerStyle={{ padding: 16 }}
          ListEmptyComponent={
            error ? (
              <View style={s.center}>
                <Text style={[s.emptyText, { color: '#EF4444' }]}>{error}</Text>
              </View>
            ) : searched ? (
              <View style={s.center}>
                <Text style={[s.emptyText, { color: c.textSecondary }]}>没有找到匹配内容</Text>
              </View>
            ) : (
              <View style={s.center}>
                <Text style={[s.emptyText, { color: c.textSecondary }]}>输入关键词搜索对话和笔记</Text>
              </View>
            )
          }
        />
      )}
    </SafeAreaView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1 },
  searchBar: {
    flexDirection: 'row', alignItems: 'center',
    marginHorizontal: 16, marginTop: 8, marginBottom: 4,
    paddingHorizontal: 12, borderWidth: 1, borderRadius: 10, height: 40,
  },
  searchInput: { flex: 1, fontSize: 15, padding: 0 },
  center: { flex: 1, justifyContent: 'center', alignItems: 'center', paddingTop: 60 },
  emptyText: { fontSize: 15 },
  card: { borderWidth: 1, borderRadius: 12, padding: 14, marginBottom: 10 },
  cardHeader: { flexDirection: 'row', alignItems: 'center', marginBottom: 4 },
  typeTag: {
    fontSize: 11, color: '#FFF', fontWeight: '600',
    paddingHorizontal: 6, paddingVertical: 2, borderRadius: 4, overflow: 'hidden', marginRight: 8,
  },
  title: { fontSize: 15, fontWeight: '600', flex: 1 },
  time: { fontSize: 12 },
  snippet: { fontSize: 13, lineHeight: 18 },
});
