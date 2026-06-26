import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  View, Text, FlatList, TouchableOpacity, TextInput, StyleSheet, Alert,
  ActivityIndicator, PanResponder, Animated, Dimensions,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import Ionicons from '@expo/vector-icons/Ionicons';
import { useAppStore, getColors } from '../store';
import { getSessions, deleteSession, toggleArchive, togglePin, renameSession, searchSessions, DbSession } from '../db';

const SWIPE_THRESHOLD = -80;
const ACTION_WIDTH = 160;
// Module-level ref to track the currently open row's close callback (mutex)
const openRowCloseRef: { current: (() => void) | null } = { current: null };

function SwipeableRow({ children, onDelete, onArchive }: {
  children: React.ReactNode;
  onDelete: () => void;
  onArchive: () => void;
}) {
  const translateX = useRef(new Animated.Value(0)).current;
  const lastOffset = useRef(0);

  const closeRow = () => {
    Animated.spring(translateX, { toValue: 0, useNativeDriver: true }).start();
    lastOffset.current = 0;
  };

  const panResponder = useRef(
    PanResponder.create({
      onMoveShouldSetPanResponder: (_, g) => Math.abs(g.dx) > 10 && Math.abs(g.dx) > Math.abs(g.dy),
      onPanResponderGrant: () => {
        // Close any other open row before this one starts moving
        if (openRowCloseRef.current && openRowCloseRef.current !== closeRow) {
          openRowCloseRef.current();
        }
      },
      onPanResponderMove: (_, g) => {
        const next = Math.min(0, lastOffset.current + g.dx);
        translateX.setValue(next);
      },
      onPanResponderRelease: (_, g) => {
        const finalVal = lastOffset.current + g.dx;
        if (finalVal < SWIPE_THRESHOLD) {
          Animated.spring(translateX, { toValue: -ACTION_WIDTH, useNativeDriver: true }).start();
          lastOffset.current = -ACTION_WIDTH;
          openRowCloseRef.current = closeRow;
        } else {
          closeRow();
        }
      },
    }),
  ).current;

  return (
    <View style={swipeStyles.rowContainer}>
      <View style={swipeStyles.actions}>
        <TouchableOpacity style={[swipeStyles.actionBtn, { backgroundColor: '#F59E0B' }]} onPress={() => { openRowCloseRef.current = null; onArchive(); }}>
          <Text style={swipeStyles.actionText}>归档</Text>
        </TouchableOpacity>
        <TouchableOpacity style={[swipeStyles.actionBtn, { backgroundColor: '#EF4444' }]} onPress={() => { openRowCloseRef.current = null; onDelete(); }}>
          <Text style={swipeStyles.actionText}>删除</Text>
        </TouchableOpacity>
      </View>
      <Animated.View
        style={[swipeStyles.rowContent, { transform: [{ translateX }] }]}
        {...panResponder.panHandlers}
      >
        {children}
      </Animated.View>
    </View>
  );
}

export default function SessionsScreen({ navigation }: any) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [sessions, setSessions] = useState<DbSession[]>([]);
  const [search, setSearch] = useState('');
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [showArchived, setShowArchived] = useState(false);

  const load = useCallback(async (query?: string) => {
    try {
      const data = query?.trim()
        ? await searchSessions(query.trim())
        : await getSessions(showArchived);
      setSessions(data);
    } catch (e: any) {
      console.warn('[Sessions] load failed:', e);
      Alert.alert('加载失败', e.message || '请重试');
    } finally {
      setLoading(false);
    }
  }, [showArchived]);

  useEffect(() => { load(); }, [load]);

  // Debounce search: wait 300ms after last keystroke
  useEffect(() => {
    const timer = setTimeout(() => load(search), 300);
    return () => clearTimeout(timer);
  }, [search, load]);

  const handleRefresh = async () => {
    setRefreshing(true);
    await load(search);
    setRefreshing(false);
  };

  const handleSelect = (session: DbSession) => {
    navigation.navigate('Chat', { sessionId: session.id, title: session.title });
  };

  const handleDelete = (id: string) => {
    Alert.alert('删除对话', '确定要删除吗？此操作不可撤销。', [
      { text: '取消', style: 'cancel' },
      { text: '删除', style: 'destructive', onPress: async () => {
        try { await deleteSession(id); await load(); } catch (e: any) { Alert.alert('删除失败', e.message); }
      }},
    ]);
  };

  const handleArchive = async (id: string) => {
    try { await toggleArchive(id); await load(); } catch (e: any) { Alert.alert('操作失败', e.message); }
  };

  const handlePin = async (id: string) => {
    try { await togglePin(id); await load(); } catch (e: any) { Alert.alert('操作失败', e.message); }
  };

  const handleLongPress = (item: DbSession) => {
    Alert.alert(item.title || '对话操作', '', [
      { text: item.pinned ? '取消置顶' : '置顶', onPress: () => handlePin(item.id) },
      { text: showArchived ? '取消归档' : '归档', onPress: () => handleArchive(item.id) },
      { text: '删除', style: 'destructive', onPress: () => handleDelete(item.id) },
      { text: '取消', style: 'cancel' },
    ]);
  };

  const fmtTime = (ts: number) => {
    const d = new Date(ts * 1000);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
    return d.toLocaleDateString('zh-CN', { month: 'short', day: 'numeric' });
  };

  if (loading) {
    return (
      <SafeAreaView style={[s.container, { backgroundColor: c.bg, justifyContent: 'center', alignItems: 'center' }]}>
        <ActivityIndicator color={accentColor} size="large" />
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView style={[s.container, { backgroundColor: c.bg }]}>
      <View style={s.topBar}>
        <TouchableOpacity onPress={() => navigation.goBack()}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
          <Ionicons name="arrow-back-outline" size={18} color={accentColor} />
          <Text style={{ color: accentColor, fontSize: 16 }}>返回</Text>
        </View>
        </TouchableOpacity>
        <Text style={[s.screenTitle, { color: c.text }]}>对话列表</Text>
        <TouchableOpacity onPress={() => setShowArchived(v => !v)}>
          <Text style={{ color: accentColor, fontSize: 14 }}>{showArchived ? '活跃' : '归档'}</Text>
        </TouchableOpacity>
      </View>

      <View style={[s.searchBar, { borderColor: c.border }]}>
        <Ionicons name="search-outline" size={18} color={c.textSecondary} style={{ marginRight: 6 }} />
        <TextInput
          style={[s.searchInput, { color: c.text }]}
          placeholder="搜索对话..."
          placeholderTextColor={c.textSecondary}
          value={search}
          onChangeText={setSearch}
        />
      </View>

      <FlatList
        data={sessions}
        keyExtractor={item => item.id}
        contentContainerStyle={{ paddingHorizontal: 16 }}
        refreshing={refreshing}
        onRefresh={handleRefresh}
        renderItem={({ item }) => (
          <SwipeableRow
            onDelete={() => handleDelete(item.id)}
            onArchive={() => handleArchive(item.id)}
          >
            <TouchableOpacity
              style={[s.card, { backgroundColor: c.card, borderColor: c.border }]}
              onPress={() => handleSelect(item)}
              onLongPress={() => handleLongPress(item)}
              activeOpacity={0.7}
            >
              <View style={s.cardHeader}>
                <Text style={[s.cardTitle, { color: c.text }]} numberOfLines={1}>
                  <View style={{ flexDirection: 'row', alignItems: 'center', flex: 1 }}>
                {item.pinned && <Ionicons name="pin" size={14} color={accentColor} style={{ marginRight: 4 }} />}
                <Text style={[s.cardTitle, { color: c.text }]} numberOfLines={1}>{item.title}</Text>
              </View>
                </Text>
                <Text style={[s.cardTime, { color: c.textSecondary }]}>{fmtTime(item.updated_at)}</Text>
              </View>
            </TouchableOpacity>
          </SwipeableRow>
        )}
        ListEmptyComponent={
          <View style={s.empty}>
            <Text style={[s.emptyText, { color: c.textSecondary }]}>
              {showArchived ? '没有归档对话' : search ? '没有找到对话' : '暂无对话'}
            </Text>
          </View>
        }
      />
    </SafeAreaView>
  );
}

const swipeStyles = StyleSheet.create({
  rowContainer: { marginBottom: 10 },
  actions: {
    position: 'absolute', right: 0, top: 0, bottom: 0,
    flexDirection: 'row', width: ACTION_WIDTH,
  },
  actionBtn: {
    flex: 1, justifyContent: 'center', alignItems: 'center',
    borderRadius: 12,
  },
  actionText: { color: '#FFF', fontSize: 14, fontWeight: '600' },
  rowContent: { backgroundColor: 'transparent' },
});

const s = StyleSheet.create({
  container: { flex: 1 },
  topBar: {
    flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center',
    paddingHorizontal: 16, paddingVertical: 10,
  },
  screenTitle: { fontSize: 18, fontWeight: '700' },
  searchBar: {
    flexDirection: 'row', alignItems: 'center',
    marginHorizontal: 16, marginBottom: 12, paddingHorizontal: 12,
    borderWidth: 1, borderRadius: 10, height: 40,
  },
  searchInput: { flex: 1, fontSize: 15, padding: 0 },
  card: { borderWidth: 1, borderRadius: 12, padding: 14 },
  cardHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center' },
  cardTitle: { fontSize: 16, fontWeight: '600', flex: 1 },
  cardTime: { fontSize: 12 },
  empty: { alignItems: 'center', marginTop: 60 },
  emptyText: { fontSize: 15 },
});
