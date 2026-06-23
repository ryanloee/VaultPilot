import React from 'react';
import { View, Text, StyleSheet } from 'react-native';
import Icon from '../Icon';

interface OfflineBannerProps {
  visible: boolean;
  isDark?: boolean;
}

export default function OfflineBanner({ visible, isDark = false }: OfflineBannerProps) {
  if (!visible) return null;

  const bgColor = isDark ? '#451A03' : '#FEF3C7';
  const textColor = isDark ? '#FDE68A' : '#92400E';

  return (
    <View style={[styles.banner, { backgroundColor: bgColor }]}>
      <Icon name="wifi-off" size={13} color={textColor} />
      <Text style={[styles.text, { color: textColor }]}> 离线模式 — 笔记可查看编辑，聊天需联网</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  banner: {
    paddingVertical: 6,
    paddingHorizontal: 16,
    flexDirection: 'row',
    alignItems: 'center',
  },
  text: {
    fontSize: 13,
    flex: 1,
  },
});
