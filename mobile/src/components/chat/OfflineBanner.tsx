import React from 'react';
import { View, Text, StyleSheet } from 'react-native';
import Icon from '../Icon';

interface OfflineBannerProps {
  visible: boolean;
}

export default function OfflineBanner({ visible }: OfflineBannerProps) {
  if (!visible) return null;

  return (
    <View style={styles.banner}>
      <Icon name="wifi-off" size={13} color="#92400E" />
      <Text style={styles.text}> 离线模式 — 笔记可查看编辑，聊天需联网</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  banner: {
    backgroundColor: '#FEF3C7',
    paddingVertical: 6,
    paddingHorizontal: 16,
    flexDirection: 'row',
    alignItems: 'center',
  },
  text: {
    color: '#92400E',
    fontSize: 13,
    flex: 1,
  },
});