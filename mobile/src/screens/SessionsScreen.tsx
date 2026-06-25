import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  View, Text, FlatList, TouchableOpacity, TextInput, StyleSheet, Alert,
  ActivityIndicator, PanResponder, Animated, Dimensions, Modal,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useAppStore, getColors } from '../store';
import { getSessions, deleteSession, toggleArchive, togglePin, renameSession, searchSessions, DbSession } from '../db';
import Icon from '../components/Icon';
import type { SessionsScreenProps } from '../navigation/types';
import { fmtTime } from '../utils/timeFormat';

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
        <TouchableOpacity style={[swipeStyles.actionBtn, { backgroundColor: '#F59E0B' }]} onPress={() => { openRowCloseRef.current = null; onArchive(); }}
          accessibilityRole="button" accessibilityLabel="归档对话">
          <Text style={swipeStyles.actionText}>归档</Text>
        </TouchableOpacity>
        <TouchableOpacity style={[swipeStyles.actionBtn, { backgroundColor: '#EF4444' }]} onPress={() => { openRowCloseRef.current = null; onDelete(); }}
          accessibilityRole="button" accessibilityLabel="删除对话">
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

export default function SessionsScreen({ navigation }: SessionsScreenProps) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const [sessions, setSessions] = useState<DbSession[]>([]);
  const [search, setSearch] = useState('');
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const [renameTarget, setRenameTarget] = useState<DbSession | null>(null);
  const [renameText, setRenameText] = useState('');
  const requestIdRef = useRef(0);

  const load = useCallback(async (query?: string) => {
    const currentId = ++requestIdRef.current;
    try {
      const data = query?.trim()
        ? await searchSessions(query.trim())
        : await getSessions(showArchived);
      if (requestIdRef.current !== currentId) return;
      setSessions(data);
    } catch (e: unknown) {
      if (requestIdRef.current !== currentId) return;
      console.warn('[Sessions] load failed:', e);
      Alert.alert('加载失败', e instanceof Error ? e.message : '请重试');
    } finally {
      if (requestIdRef.current === currentId) setLoading(false);
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
    // Navigate to the ChatMain screen within the ChatStack (not the tab)
    navigation.navigate('ChatMain', { sessionId: session.id, title: session.title });
  };

  const handleDelete = (id: string) => {
    Alert.alert('删除对话', '确定要删除吗？此操作不可撤销。', [
      { text: '取消', style: 'cancel' },
      { text: '删除', style: 'destructive', onPress: async () => {
        try { await deleteSession(id); await load(); } catch (e: unknown) { Alert.alert('删除失败', e instanceof Error ? e.message : '操作失败'); }
      }},
    ]);
  };

  const handleArchive = async (id: string) => {
    try { await toggleArchive(id); await load(); } catch (e: unknown) { Alert.alert('操作失败', e instanceof Error ? e.message : '操作失败'); }
  };

  const handlePin = async (id: string) => {
    try { await togglePin(id); await load(); } catch (e: unknown) { Alert.alert('操作失败', e instanceof Error ? e.message : '操作失败'); }
  };

  const handleRename = async () => {
    if (!renameTarget || !renameText.trim()) return;
    try {
      await renameSession(renameTarget.id, renameText.trim());
      setRenameTarget(null);
      setRenameText('');
      await load(search);
    } catch (e: unknown) {
      Alert.alert('重命名失败', e instanceof Error ? e.message : '操作失败');
    }
  };

  const handleLongPress = (item: DbSession) => {
    Alert.alert(item.title || '对话操作', '', [
      { text: '重命名', onPress: () => { setRenameTarget(item); setRenameText(item.title); } },
      { text: item.pinned ? '取消置顶' : '置顶', onPress: () => handlePin(item.id) },
      { text: showArchived ? '取消归档' : '归档', onPress: () => handleArchive(item.id) },
      { text: '删除', style: 'destructive', onPress: () => handleDelete(item.id) },
      { text: '取消', style: 'cancel' },
    ]);
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
        <TouchableOpacity onPress={() => navigation.goBack()}
          accessibilityRole="button" accessibilityLabel="返回">
          <Text style={{ color: accentColor, fontSize: 16 }}>← 返回</Text>
        </TouchableOpacity>
        <Text style={[s.screenTitle, { color: c.text }]} accessibilityRole="header">对话列表</Text>
        <TouchableOpacity onPress={() => setShowArchived(v => !v)}
          accessibilityRole="button" accessibilityLabel={showArchived ? '显示活跃对话' : '显示归档对话'}>
          <Text style={{ color: accentColor, fontSize: 14 }}>{showArchived ? '活跃' : '归档'}</Text>
        </TouchableOpacity>
      </View>

      <View style={[s.searchBar, { borderColor: c.border }]}>
        <Icon name="search" size={16} color={c.textSecondary} />
        <TextInput
          style={[s.searchInput, { color: c.text }]}
          placeholder="搜索对话..."
          placeholderTextColor={c.textSecondary}
          value={search}
          onChangeText={setSearch}
          accessibilityLabel="搜索对话"
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
              accessibilityRole="button"
              accessibilityLabel={`${item.pinned ? '已置顶 ' : ''}${item.title}，${fmtTime(item.updated_at)}`}
              accessibilityHint="点击打开对话，长按查看操作"
            >
              <View style={s.cardHeader}>
                <Text style={[s.cardTitle, { color: c.text }]} numberOfLines={1}>
                  {item.pinned ? <Icon name="pin" size={14} color={c.textSecondary} style={{ marginRight: 4 }} /> : null}{item.title}
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
      {/* Rename Modal */}
      <Modal visible={!!renameTarget} transparent animationType="fade">
        <View style={renameStyles.overlay}>
          <View style={[renameStyles.content, { backgroundColor: c.card }]}>
            <Text style={[renameStyles.title, { color: c.text }]}>重命名对话</Text>
            <TextInput
              style={[renameStyles.input, { color: c.text, borderColor: c.border }]}
              value={renameText}
              onChangeText={setRenameText}
              placeholder="输入新标题"
              placeholderTextColor={c.textSecondary}
              autoFocus
              selectTextOnFocus
            />
            <View style={renameStyles.buttons}>
              <TouchableOpacity onPress={() => { setRenameTarget(null); setRenameText(''); }} style={renameStyles.btn}>
                <Text style={{ color: c.textSecondary, fontSize: 16 }}>取消</Text>
              </TouchableOpacity>
              <TouchableOpacity onPress={handleRename} style={renameStyles.btn}>
                <Text style={{ color: accentColor, fontSize: 16, fontWeight: '600' }}>确定</Text>
              </TouchableOpacity>
            </View>
          </View>
        </View>
      </Modal>
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

const renameStyles = StyleSheet.create({
  overlay: { flex: 1, justifyContent: 'center', alignItems: 'center', backgroundColor: 'rgba(0,0,0,0.5)' },
  content: { width: '80%', borderRadius: 16, padding: 20 },
  title: { fontSize: 18, fontWeight: '700', marginBottom: 16, textAlign: 'center' },
  input: { borderWidth: 1, borderRadius: 10, padding: 12, fontSize: 16, marginBottom: 16 },
  buttons: { flexDirection: 'row', justifyContent: 'flex-end', gap: 16 },
  btn: { paddingVertical: 8, paddingHorizontal: 16 },
});
