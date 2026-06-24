import React, { useRef, useCallback, useEffect } from 'react';
import { View, Text, FlatList, TouchableOpacity, Platform, StyleSheet } from 'react-native';
import { NativeSyntheticEvent, NativeScrollEvent } from 'react-native';
import MessageBubble from './MessageBubble';

interface Message {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  streaming?: boolean;
  isError?: boolean;
  attachments?: { name: string; type: 'image' | 'file' }[];
}

interface MessageListProps {
  messages: Message[];
  isDark: boolean;
  accentColor: string;
  textColor: string;
  textColorSecondary: string;
  borderColor: string;
  onDeleteMessage: (msgId: string) => void;
  onResendMessage: (msgId: string) => void;
  onScrollToEnd: () => void;
  onNearBottomChange?: (nearBottom: boolean) => void;
  scrollTrigger?: number;
  onSuggestion?: (text: string) => void;
}

export default function MessageList({
  messages,
  isDark,
  accentColor,
  textColor,
  textColorSecondary,
  borderColor,
  onDeleteMessage,
  onResendMessage,
  onScrollToEnd,
  onNearBottomChange,
  scrollTrigger,
  onSuggestion,
}: MessageListProps) {
  const listRef = useRef<FlatList>(null);
  const nearBottomRef = useRef(true);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    };
  }, []);

  const scrollToEndDebounced = useCallback((force = false) => {
    if (!force && !nearBottomRef.current) return;
    if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    scrollTimerRef.current = setTimeout(() => {
      listRef.current?.scrollToEnd({ animated: true });
    }, 100);
  }, []);

  const onContentSizeChange = useCallback(() => {
    scrollToEndDebounced();
  }, [scrollToEndDebounced]);

  // When parent triggers scroll (button press), scroll to bottom
  useEffect(() => {
    if (scrollTrigger !== undefined && scrollTrigger > 0) {
      nearBottomRef.current = true;
      listRef.current?.scrollToEnd({ animated: true });
    }
  }, [scrollTrigger]);

  const onScroll = useCallback(
    (e: NativeSyntheticEvent<NativeScrollEvent>) => {
      const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
      const distanceFromBottom =
        contentSize.height - layoutMeasurement.height - contentOffset.y;
      const near = distanceFromBottom < 120;
      if (near !== nearBottomRef.current) {
        nearBottomRef.current = near;
        onNearBottomChange?.(near);
      }
    },
    [onNearBottomChange]
  );

  const handleScrollToEnd = useCallback(() => {
    nearBottomRef.current = true;
    listRef.current?.scrollToEnd({ animated: true });
    onScrollToEnd();
  }, [onScrollToEnd]);

  return (
    <>
      <FlatList
        ref={listRef}
        data={messages}
        renderItem={({ item }) => (
          <MessageBubble
            item={item}
            isDark={isDark}
            accentColor={accentColor}
            onDelete={() => onDeleteMessage(item.id)}
            onResend={
              item.role === 'user' ? () => onResendMessage(item.id) : undefined
            }
          />
        )}
        keyExtractor={(item) => item.id}
        contentContainerStyle={{ padding: 16, paddingBottom: 8 }}
        onContentSizeChange={onContentSizeChange}
        onScroll={onScroll}
        scrollEventThrottle={16}
        removeClippedSubviews={Platform.OS === 'android'}
        initialNumToRender={15}
        maxToRenderPerBatch={10}
        windowSize={11}
        ListEmptyComponent={
          <View style={styles.emptyContainer}>
            <Text style={[styles.emptyTitle, { color: textColor }]}>
              👋 你好，我是 VaultPilot AI
            </Text>
            <Text style={[styles.emptySubtitle, { color: textColorSecondary }]}>
              有什么可以帮你的？试试这些问题：
            </Text>
            {['帮我总结一篇笔记', '解释一下这个概念', '写一段代码'].map((q) => (
              <TouchableOpacity
                key={q}
                style={[styles.suggestionBtn, { borderColor }]}
                accessibilityRole="button"
                accessibilityLabel={`使用建议: ${q}`}
                onPress={() => onSuggestion?.(q)}
              >
                <Text style={[styles.suggestionText, { color: accentColor }]}>
                  {q}
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        }
      />
      {/* Scroll to bottom button - handled by parent */}
    </>
  );
}

const styles = StyleSheet.create({
  emptyContainer: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    paddingTop: 80,
    paddingHorizontal: 32,
  },
  emptyTitle: { fontSize: 22, fontWeight: '700', marginBottom: 8 },
  emptySubtitle: {
    fontSize: 15,
    marginBottom: 20,
    textAlign: 'center',
  },
  suggestionBtn: {
    borderWidth: 1,
    borderRadius: 12,
    paddingVertical: 10,
    paddingHorizontal: 16,
    marginBottom: 8,
    alignSelf: 'stretch',
  },
  suggestionText: { fontSize: 15 },
});