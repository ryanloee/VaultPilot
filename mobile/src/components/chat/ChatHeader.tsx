import React from 'react';
import { View, Text, TouchableOpacity, StyleSheet } from 'react-native';

interface ChatHeaderProps {
  title: string;
  accentColor: string;
  borderColor: string;
  textColor: string;
  onSessionsPress: () => void;
  onNewChatPress: () => void;
}

export default function ChatHeader({
  title,
  accentColor,
  borderColor,
  textColor,
  onSessionsPress,
  onNewChatPress,
}: ChatHeaderProps) {
  return (
    <View style={[styles.header, { borderBottomColor: borderColor }]}>
      <TouchableOpacity
        onPress={onSessionsPress}
        style={styles.sessionsBtn}
        accessibilityRole="button"
        accessibilityLabel="打开对话列表"
      >
        <Text style={{ color: accentColor, fontSize: 14 }}>☰ 对话</Text>
      </TouchableOpacity>
      <Text
        style={[styles.titleText, { color: textColor }]}
        numberOfLines={1}
        accessibilityRole="header"
      >
        {title}
      </Text>
      <TouchableOpacity
        onPress={onNewChatPress}
        style={[styles.newChatBtn, { borderColor }]}
        accessibilityRole="button"
        accessibilityLabel="新建对话"
      >
        <Text style={{ color: accentColor, fontSize: 14 }}>＋</Text>
      </TouchableOpacity>
    </View>
  );
}

const styles = StyleSheet.create({
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: 16,
    paddingVertical: 8,
    borderBottomWidth: 1,
  },
  sessionsBtn: {
    paddingVertical: 6,
    paddingHorizontal: 10,
  },
  titleText: {
    fontSize: 17,
    fontWeight: '600',
    flex: 1,
    textAlign: 'center',
  },
  newChatBtn: {
    paddingVertical: 6,
    paddingHorizontal: 12,
    borderWidth: 1,
    borderRadius: 16,
  },
});