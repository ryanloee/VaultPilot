/**
 * ToolPanel — @ 工具选择面板
 *
 * 当用户在 Chat 输入框中输入 "@" 时弹出，展示可用的 AI 工具。
 * 支持按输入内容过滤、动画展开、非全屏。
 */
import React, { useEffect, useRef, useMemo } from 'react';
import {
  View, Text, TouchableOpacity, FlatList, StyleSheet, Animated,
} from 'react-native';
import Ionicons from '@expo/vector-icons/Ionicons';
import * as Haptics from 'expo-haptics';

/* ── 工具定义 ── */

export interface ToolDef {
  id: string;
  iconName: keyof typeof Ionicons.glyphMap;
  label: string;
  description: string;
  commandPrefix: string;  // e.g. "@web: "
  placeholder: string;    // Placeholder after selection
}

export const BUILTIN_TOOLS: ToolDef[] = [
  {
    id: 'vault',
    iconName: 'search-outline',
    label: '@vault',
    description: '搜索 vault 笔记',
    commandPrefix: '@vault: ',
    placeholder: '@vault: 搜索关键词…',
  },
  {
    id: 'web',
    iconName: 'globe-outline',
    label: '@web',
    description: '实时 Web 搜索',
    commandPrefix: '@web: ',
    placeholder: '@web: 搜索内容…',
  },
  {
    id: 'url',
    iconName: 'link-outline',
    label: '@url',
    description: '提取网页内容并总结',
    commandPrefix: '@url: ',
    placeholder: '@url: https://…',
  },
  {
    id: 'youtube',
    iconName: 'logo-youtube',
    label: '@youtube',
    description: '提取 YouTube 视频内容',
    commandPrefix: '@youtube: ',
    placeholder: '@youtube: https://youtube.com/…',
  },
];

/* ── Props ── */

interface ToolPanelProps {
  visible: boolean;
  filter: string;           // Text typed after @ (for filtering)
  onSelect: (tool: ToolDef) => void;
  onDismiss: () => void;
  accentColor: string;
  bgColor: string;
  textColor: string;
  textSecondaryColor: string;
  borderColor: string;
  inputBgColor: string;
}

/* ── Component ── */

export default function ToolPanel({
  visible,
  filter,
  onSelect,
  onDismiss,
  accentColor,
  bgColor,
  textColor,
  textSecondaryColor,
  borderColor,
  inputBgColor,
}: ToolPanelProps) {
  const slideAnim = useRef(new Animated.Value(0)).current;

  useEffect(() => {
    if (visible) {
      Animated.timing(slideAnim, { toValue: 1, duration: 150, useNativeDriver: true }).start();
    } else {
      Animated.timing(slideAnim, { toValue: 0, duration: 100, useNativeDriver: true }).start();
    }
  }, [visible]);

  const filtered = useMemo(() => {
    if (!filter) return BUILTIN_TOOLS;
    const lower = filter.toLowerCase();
    return BUILTIN_TOOLS.filter(
      t => t.id.includes(lower) || t.label.toLowerCase().includes(lower) || t.description.toLowerCase().includes(lower),
    );
  }, [filter]);

  const handleSelect = (tool: ToolDef) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(() => {});
    onSelect(tool);
  };

  if (!visible && (slideAnim as any).__value === 0) return null;

  return (
    <Animated.View
      style={[
        styles.container,
        {
          backgroundColor: inputBgColor,
          borderColor,
          opacity: slideAnim,
          transform: [{
            translateY: slideAnim.interpolate({ inputRange: [0, 1], outputRange: [40, 0] }),
          }],
        },
      ]}
    >
      {/* Header */}
      <View style={styles.header}>
        <Text style={[styles.headerTitle, { color: textSecondaryColor }]}>选择工具</Text>
        <TouchableOpacity onPress={onDismiss} hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}>
          <Ionicons name="close" size={18} color={textSecondaryColor} />
        </TouchableOpacity>
      </View>

      {/* Tool list */}
      {filtered.length === 0 ? (
        <View style={styles.emptyContainer}>
          <Text style={[styles.emptyText, { color: textSecondaryColor }]}>
            无匹配工具
          </Text>
        </View>
      ) : (
        <FlatList
          data={filtered}
          keyExtractor={item => item.id}
          style={styles.list}
          contentContainerStyle={styles.listContent}
          renderItem={({ item }) => (
            <TouchableOpacity
              style={[styles.toolItem, { borderBottomColor: borderColor }]}
              onPress={() => handleSelect(item)}
              activeOpacity={0.6}
            >
              <View style={[styles.iconWrap, { backgroundColor: accentColor + '18' }]}>
                <Ionicons name={item.iconName} size={20} color={accentColor} />
              </View>
              <View style={styles.toolInfo}>
                <Text style={[styles.toolLabel, { color: textColor }]}>{item.label}</Text>
                <Text style={[styles.toolDesc, { color: textSecondaryColor }]}>{item.description}</Text>
              </View>
              <Ionicons name="chevron-forward" size={16} color={textSecondaryColor} />
            </TouchableOpacity>
          )}
        />
      )}
    </Animated.View>
  );
}

/* ── Styles ── */

const styles = StyleSheet.create({
  container: {
    borderTopWidth: 1,
    borderBottomWidth: 1,
    maxHeight: 240,
  },
  header: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    paddingHorizontal: 14,
    paddingVertical: 8,
  },
  headerTitle: {
    fontSize: 13,
    fontWeight: '600',
    letterSpacing: 0.3,
  },
  list: {
    maxHeight: 190,
  },
  listContent: {
    paddingBottom: 6,
  },
  toolItem: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingVertical: 10,
    paddingHorizontal: 14,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  iconWrap: {
    width: 36,
    height: 36,
    borderRadius: 10,
    justifyContent: 'center',
    alignItems: 'center',
    marginRight: 10,
  },
  toolInfo: {
    flex: 1,
  },
  toolLabel: {
    fontSize: 15,
    fontWeight: '600',
  },
  toolDesc: {
    fontSize: 12,
    marginTop: 2,
  },
  emptyContainer: {
    paddingVertical: 24,
    alignItems: 'center',
  },
  emptyText: {
    fontSize: 13,
  },
});
