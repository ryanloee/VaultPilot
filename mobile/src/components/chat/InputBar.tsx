import React, { useState, useRef } from 'react';
import { View, Text, TextInput, TouchableOpacity, StyleSheet, Animated, FlatList } from 'react-native';
import * as Haptics from 'expo-haptics';

interface Attachment {
  name: string;
  uri: string;
  type: 'image' | 'file';
}

interface InputBarProps {
  input: string;
  inputHeight: number;
  streaming: boolean;
  attachments: Attachment[];
  accentColor: string;
  bgColor: string;
  inputBgColor: string;
  textColor: string;
  textColorSecondary: string;
  borderColor: string;
  voiceAvailable: boolean;
  voiceListening: boolean;
  onInputChange: (text: string) => void;
  onInputHeightChange: (height: number) => void;
  onSend: () => void;
  onStop: () => void;
  onTakePhoto: () => void;
  onPickImage: () => void;
  onPickDocument: () => void;
  onRemoveAttachment: (index: number) => void;
  onVoiceToggle: () => void;
  onEmojiSelect?: (emoji: string) => void;
}

/* ── Emoji data (common emoji organized by category) ── */
const EMOJI_CATEGORIES = [
  {
    name: '常用',
    emojis: ['😀','😂','🤣','😍','🥰','😘','😊','😉','🤔','😅','😢','😭','😡','🥳','🤩','😎','🥺','😴','🤯','🥶','🤢','🤮','👍','👎','❤️','🔥','💯','⭐','🎉','🙏','👏','🤝','💪','✌️','👀','💡'],
  },
  {
    name: '表情',
    emojis: ['😀','😁','😂','🤣','😃','😄','😅','😆','😉','😊','😋','😎','😍','🥰','😘','😗','😙','😚','🙂','🤗','🤔','🤨','😐','😑','😶','🙄','😏','😣','😥','😮','🤐','😯','😪','😫','🥱','😴','😌','😛','😜','🤪','😝','🤑','🤗','🤭','🤫','🤥','😬','🙃','😇'],
  },
  {
    name: '手势',
    emojis: ['👋','🤚','🖐️','✋','🖖','👌','🤌','🤏','✌️','🤞','🤟','🤘','🤙','👈','👉','👆','👇','☝️','👍','👎','✊','👊','🤛','🤜','👏','🙌','👐','🤲','🤝','🙏','💪','🦾'],
  },
  {
    name: '物品',
    emojis: ['❤️','🧡','💛','💚','💙','💜','🖤','🤍','💔','❣️','💕','💞','💓','💗','💖','💘','💝','🔥','💯','💥','✨','🌟','⭐','🎉','🎊','🏆','🎯','📌','📎','🔗','💡','🔔','📱','💻','⌨️','📷','📸','🎬','📝','✏️','📚','🔑','🔒'],
  },
];

export default function InputBar({
  input,
  inputHeight,
  streaming,
  attachments,
  accentColor,
  bgColor,
  inputBgColor,
  textColor,
  textColorSecondary,
  borderColor,
  voiceAvailable,
  voiceListening,
  onInputChange,
  onInputHeightChange,
  onSend,
  onStop,
  onTakePhoto,
  onPickImage,
  onPickDocument,
  onRemoveAttachment,
  onVoiceToggle,
  onEmojiSelect,
}: InputBarProps) {
  const [plusExpanded, setPlusExpanded] = useState(false);
  const [showEmojiPicker, setShowEmojiPicker] = useState(false);
  const [activeEmojiCategory, setActiveEmojiCategory] = useState(0);
  const expandAnim = useRef(new Animated.Value(0)).current;

  const togglePlus = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    if (plusExpanded) {
      Animated.timing(expandAnim, { toValue: 0, duration: 150, useNativeDriver: false }).start();
      setPlusExpanded(false);
    } else {
      setShowEmojiPicker(false);
      Animated.timing(expandAnim, { toValue: 1, duration: 150, useNativeDriver: false }).start();
      setPlusExpanded(true);
    }
  };

  const toggleEmojiPicker = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    if (showEmojiPicker) {
      setShowEmojiPicker(false);
    } else {
      setPlusExpanded(false);
      expandAnim.setValue(0);
      setShowEmojiPicker(true);
    }
  };

  const handleEmojiSelect = (emoji: string) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onEmojiSelect?.(emoji);
  };

  const closeAll = () => {
    setPlusExpanded(false);
    setShowEmojiPicker(false);
    expandAnim.setValue(0);
  };

  const quickActions = [
    { icon: '📷', label: '拍照', onPress: () => { closeAll(); onTakePhoto(); } },
    { icon: '🖼️', label: '相册', onPress: () => { closeAll(); onPickImage(); } },
    { icon: '📄', label: '文件', onPress: () => { closeAll(); onPickDocument(); } },
    { icon: '😊', label: '表情', onPress: () => { toggleEmojiPicker(); } },
  ];

  return (
    <View style={[styles.wrapper, { backgroundColor: bgColor, borderTopColor: borderColor }]}>
      {/* Attachment preview chips */}
      {attachments.length > 0 && (
        <View style={[styles.attachRow, { borderBottomColor: borderColor }]}>
          {attachments.map((att, i) => (
            <View key={i} style={[styles.attachChip, { borderColor, backgroundColor: inputBgColor }]}>
              <Text style={[styles.attachName, { color: textColorSecondary }]} numberOfLines={1}>
                {att.type === 'image' ? '🖼 ' : '📄 '}{att.name}
              </Text>
              <TouchableOpacity onPress={() => onRemoveAttachment(i)} hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}>
                <Text style={[styles.attachRemove, { color: textColorSecondary }]}>✕</Text>
              </TouchableOpacity>
            </View>
          ))}
        </View>
      )}

      {/* Expandable action panel */}
      {plusExpanded && (
        <Animated.View
          style={[
            styles.expandedPanel,
            { backgroundColor: inputBgColor, borderBottomColor: borderColor },
            { maxHeight: expandAnim.interpolate({ inputRange: [0, 1], outputRange: [0, 80] }), opacity: expandAnim },
          ]}
        >
          <View style={styles.expandedActions}>
            {quickActions.map((action, i) => (
              <TouchableOpacity
                key={i}
                style={[styles.expandedBtn, { borderColor: accentColor + '40', backgroundColor: accentColor + '10' }]}
                onPress={action.onPress}
              >
                <Text style={styles.expandedIcon}>{action.icon}</Text>
                <Text style={[styles.expandedLabel, { color: textColorSecondary }]}>{action.label}</Text>
              </TouchableOpacity>
            ))}
          </View>
        </Animated.View>
      )}

      {/* Emoji picker panel */}
      {showEmojiPicker && (
        <View style={[styles.emojiPanel, { backgroundColor: inputBgColor, borderTopColor: borderColor }]}>
          {/* Category tabs */}
          <View style={[styles.emojiTabs, { borderBottomColor: borderColor }]}>
            {EMOJI_CATEGORIES.map((cat, i) => (
              <TouchableOpacity
                key={i}
                style={[styles.emojiTab, activeEmojiCategory === i && { borderBottomColor: accentColor, borderBottomWidth: 2 }]}
                onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); setActiveEmojiCategory(i); }}
              >
                <Text style={[styles.emojiTabText, { color: activeEmojiCategory === i ? accentColor : textColorSecondary }]}>
                  {cat.name}
                </Text>
              </TouchableOpacity>
            ))}
            <View style={{ flex: 1 }} />
            <TouchableOpacity
              style={styles.emojiTab}
              onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); setShowEmojiPicker(false); }}
            >
              <Text style={[styles.emojiTabText, { color: textColorSecondary }]}>✕</Text>
            </TouchableOpacity>
          </View>
          {/* Emoji grid */}
          <FlatList
            data={EMOJI_CATEGORIES[activeEmojiCategory].emojis}
            numColumns={8}
            keyExtractor={(item, index) => `${item}-${index}`}
            style={styles.emojiGrid}
            contentContainerStyle={styles.emojiGridContent}
            renderItem={({ item }) => (
              <TouchableOpacity
                style={styles.emojiItem}
                onPress={() => handleEmojiSelect(item)}
              >
                <Text style={styles.emojiText}>{item}</Text>
              </TouchableOpacity>
            )}
          />
        </View>
      )}

      {/* Main input bar */}
      <View style={[styles.inputBar, { borderTopColor: borderColor }]}>
        {/* Plus button */}
        <TouchableOpacity
          style={[styles.plusBtn, plusExpanded && { backgroundColor: accentColor + '20' }]}
          onPress={togglePlus}
        >
          <Text style={[styles.plusIcon, { color: plusExpanded ? accentColor : textColorSecondary }]}>
            {plusExpanded ? '✕' : '＋'}
          </Text>
        </TouchableOpacity>

        {/* Text input */}
        <TextInput
          style={[
            styles.textInput,
            {
              color: textColor,
              backgroundColor: inputBgColor,
              borderColor,
              height: Math.max(40, Math.min(inputHeight, 120)),
            },
          ]}
          value={input}
          onChangeText={onInputChange}
          onContentSizeChange={(e) => onInputHeightChange(e.nativeEvent.contentSize.height)}
          placeholder="输入消息..."
          placeholderTextColor={textColorSecondary}
          multiline
          maxLength={4000}
          editable={!streaming}
          onFocus={closeAll}
        />

        {/* Emoji button */}
        <TouchableOpacity
          style={[styles.emojiBtn, showEmojiPicker && { backgroundColor: accentColor + '20' }]}
          onPress={toggleEmojiPicker}
        >
          <Text style={[styles.emojiBtnText, { color: showEmojiPicker ? accentColor : textColorSecondary }]}>😊</Text>
        </TouchableOpacity>

        {/* Voice button */}
        {voiceAvailable && (
          <TouchableOpacity
            style={[styles.voiceBtn, voiceListening && { backgroundColor: '#FF3B3020' }]}
            onPress={onVoiceToggle}
          >
            <Text style={[styles.voiceIcon, { color: voiceListening ? '#FF3B30' : textColorSecondary }]}>
              {voiceListening ? '⏹' : '🎤'}
            </Text>
          </TouchableOpacity>
        )}

        {/* Send / Stop button */}
        {streaming ? (
          <TouchableOpacity style={[styles.sendBtn, { backgroundColor: '#FF3B30' }]} onPress={onStop}>
            <Text style={styles.sendText}>■</Text>
          </TouchableOpacity>
        ) : (
          <TouchableOpacity
            style={[styles.sendBtn, { backgroundColor: input.trim() ? accentColor : borderColor }]}
            onPress={onSend}
            disabled={!input.trim()}
          >
            <Text style={[styles.sendText, { color: input.trim() ? '#fff' : textColorSecondary }]}>➤</Text>
          </TouchableOpacity>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrapper: { borderTopWidth: 1 },
  attachRow: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    paddingHorizontal: 12,
    paddingVertical: 6,
    gap: 6,
    borderBottomWidth: 1,
  },
  attachChip: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 8,
    paddingVertical: 4,
    borderRadius: 12,
    borderWidth: 1,
    gap: 4,
  },
  attachName: { fontSize: 12, maxWidth: 120 },
  attachRemove: { fontSize: 12, fontWeight: '600' },
  /* Expanded panel */
  expandedPanel: {
    overflow: 'hidden',
    borderBottomWidth: 1,
  },
  expandedActions: {
    flexDirection: 'row',
    justifyContent: 'space-around',
    paddingVertical: 10,
    paddingHorizontal: 16,
  },
  expandedBtn: {
    alignItems: 'center',
    justifyContent: 'center',
    width: 64,
    height: 56,
    borderRadius: 12,
    borderWidth: 1,
  },
  expandedIcon: { fontSize: 22, marginBottom: 2 },
  expandedLabel: { fontSize: 10, fontWeight: '500' },
  /* Emoji picker */
  emojiPanel: {
    borderTopWidth: 1,
    maxHeight: 260,
  },
  emojiTabs: {
    flexDirection: 'row',
    paddingHorizontal: 8,
    borderBottomWidth: 1,
  },
  emojiTab: {
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderBottomWidth: 0,
  },
  emojiTabText: { fontSize: 13, fontWeight: '500' },
  emojiGrid: {
    maxHeight: 210,
  },
  emojiGridContent: {
    paddingHorizontal: 4,
    paddingVertical: 4,
  },
  emojiItem: {
    width: '12.5%',
    aspectRatio: 1,
    alignItems: 'center',
    justifyContent: 'center',
  },
  emojiText: { fontSize: 24 },
  /* Input bar */
  inputBar: {
    flexDirection: 'row',
    alignItems: 'flex-end',
    paddingHorizontal: 8,
    paddingVertical: 6,
    gap: 4,
  },
  plusBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
  plusIcon: { fontSize: 20, fontWeight: '600' },
  textInput: {
    flex: 1,
    borderWidth: 1,
    borderRadius: 20,
    paddingHorizontal: 14,
    paddingTop: 8,
    paddingBottom: 8,
    fontSize: 15,
    lineHeight: 20,
  },
  emojiBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
  emojiBtnText: { fontSize: 20 },
  voiceBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
  voiceIcon: { fontSize: 18 },
  sendBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
  sendText: { fontSize: 16, fontWeight: '600' },
});
