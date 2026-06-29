import React, { memo } from 'react';
import { View, Text, TouchableOpacity, Alert, StyleSheet } from 'react-native';
import * as Clipboard from 'expo-clipboard';
import * as Haptics from 'expo-haptics';
import MarkdownPreview from '../MarkdownPreview';
import Icon from '../Icon';
import { getColors } from '../../store';

interface Message {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  streaming?: boolean;
  streamStatus?: string;
  isError?: boolean;
  attachments?: { name: string; type: 'image' | 'file' }[];
}

interface MessageBubbleProps {
  item: Message;
  isDark: boolean;
  accentColor: string;
  onDelete?: (id: string) => void;
  onResend?: (id: string) => void;
  onNoteLinkPress?: (title: string) => void;
  /** Map of note title (lowercase) → noteId for auto-detection of note refs (#2035). */
  noteTitleMap?: Map<string, string>;
}

const MessageBubble = memo(function MessageBubble({
  item,
  isDark,
  accentColor,
  onDelete,
  onResend,
  onNoteLinkPress,
  noteTitleMap,
}: MessageBubbleProps) {
  const c = getColors(isDark, accentColor);
  const isAssistant = item.role === 'assistant';

  const handleLongPress = () => {
    if (!item.content) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const actions: {
      text: string;
      onPress?: () => void;
      style?: 'default' | 'cancel' | 'destructive';
    }[] = [
      { text: '复制', onPress: () => Clipboard.setStringAsync(item.content) },
    ];
    if (onResend) actions.push({ text: '重新发送', onPress: () => onResend(item.id) });
    if (onDelete)
      actions.push({ text: '删除', style: 'destructive', onPress: () => onDelete(item.id) });
    actions.push({ text: '取消', style: 'cancel' });
    Alert.alert('消息操作', '', actions);
  };

  return (
    <TouchableOpacity
      onLongPress={handleLongPress}
      activeOpacity={0.8}
      accessibilityRole="text"
      accessibilityLabel={`${item.role === 'user' ? '你的消息' : 'AI 回复'}: ${
        item.content?.slice(0, 50) || '思考中'
      }`}
    >
      <View
        style={[
          styles.bubble,
          item.role === 'user'
            ? { backgroundColor: c.userBubble, alignSelf: 'flex-end' }
            : { backgroundColor: c.aiBubble, alignSelf: 'flex-start' },
        ]}
      >
        {/* Attachment indicators for user messages */}
        {item.attachments && item.attachments.length > 0 && (
          <View style={styles.msgAttachRow}>
            {item.attachments.map((att, i) => (
              <View
                key={i}
                style={[styles.msgAttachChip, { borderColor: c.border }]}
              >
                <Text style={{ fontSize: 12, color: c.textSecondary }}>
                  <Icon name={att.type === 'image' ? 'image' : 'document'} size={12} color={c.textSecondary} /> {att.name}
                </Text>
              </View>
            ))}
          </View>
        )}
        {isAssistant && item.content ? (
          <>
            <MarkdownPreview
              content={item.content}
              textColor={c.aiText}
              accentColor={accentColor}
              isDark={isDark}
              onNoteLinkPress={onNoteLinkPress}
              noteTitleMap={noteTitleMap}
            />
            {item.streaming && (
              <Text style={{ color: accentColor, fontSize: 15 }}> ▌</Text>
            )}
          </>
        ) : (
          <Text
            style={{
              color: item.role === 'user' ? c.userText : c.aiText,
              fontSize: 15,
              lineHeight: 22,
            }}
          >
            {item.content || (item.isError ? '⚠️ 发送失败' : (item.streaming ? (item.streamStatus || '思考中...') : ''))}
            {item.streaming && (
              <Text style={{ color: accentColor }}> ▌</Text>
            )}
          </Text>
        )}
      </View>
    </TouchableOpacity>
  );
});

const styles = StyleSheet.create({
  bubble: {
    maxWidth: '80%',
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderRadius: 16,
    marginBottom: 8,
  },
  msgAttachRow: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 4,
    marginBottom: 6,
  },
  msgAttachChip: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 8,
    borderWidth: 1,
    backgroundColor: 'rgba(128,128,128,0.08)',
  },
});

export default MessageBubble;
