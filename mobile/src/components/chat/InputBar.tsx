import React, { useState, useRef, useEffect } from 'react';
import { View, Text, TextInput, TouchableOpacity, StyleSheet, Animated, FlatList } from 'react-native';
import * as Haptics from 'expo-haptics';
import Icon from '../Icon';

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
  voiceVolume: number; // 0-1 normalized
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

/* ── Animated audio waveform bar ── */
const WAVEFORM_BAR_COUNT = 24;

function AudioWaveform({ volume, color }: { volume: number; color: string }) {
  const bars = useRef(
    Array.from({ length: WAVEFORM_BAR_COUNT }, () => new Animated.Value(0.15)),
  ).current;

  useEffect(() => {
    const animations = bars.map((bar, i) => {
      // Each bar reacts slightly differently to create natural wave
      const phase = (i / WAVEFORM_BAR_COUNT) * Math.PI * 2;
      const jitter = Math.sin(phase) * 0.15 + Math.cos(phase * 0.7) * 0.1;
      const target = Math.max(0.1, Math.min(1, volume + jitter));
      return Animated.timing(bar, {
        toValue: target,
        duration: 80,
        useNativeDriver: false,
      });
    });
    const group = Animated.parallel(animations);
    group.start();
    return () => group.stop();
  }, [volume]);

  return (
    <View style={waveformStyles.container}>
      {bars.map((bar, i) => (
        <Animated.View
          key={i}
          style={[
            waveformStyles.bar,
            {
              backgroundColor: color,
              height: bar.interpolate({
                inputRange: [0, 1],
                outputRange: [3, 28],
              }),
              opacity: bar.interpolate({
                inputRange: [0.1, 1],
                outputRange: [0.3, 1],
              }),
            },
          ]}
        />
      ))}
    </View>
  );
}

const waveformStyles = StyleSheet.create({
  container: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    height: 40,
    gap: 2,
    paddingHorizontal: 8,
  },
  bar: {
    width: 3,
    borderRadius: 1.5,
    minHeight: 3,
  },
});

/* ── Emoji data ── */
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
  voiceVolume,
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
  const pulseAnim = useRef(new Animated.Value(1)).current;

  // Pulsing mic icon animation while recording
  useEffect(() => {
    if (voiceListening) {
      const pulse = Animated.loop(
        Animated.sequence([
          Animated.timing(pulseAnim, { toValue: 1.3, duration: 600, useNativeDriver: true }),
          Animated.timing(pulseAnim, { toValue: 1, duration: 600, useNativeDriver: true }),
        ]),
      );
      pulse.start();
      return () => pulse.stop();
    } else {
      pulseAnim.setValue(1);
    }
  }, [voiceListening]);

  const togglePlus = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e));
    if (plusExpanded) {
      Animated.timing(expandAnim, { toValue: 0, duration: 150, useNativeDriver: false }).start(({ finished }) => {
        if (finished) setPlusExpanded(false);
      });
    } else {
      setShowEmojiPicker(false);
      Animated.timing(expandAnim, { toValue: 1, duration: 150, useNativeDriver: false }).start();
      setPlusExpanded(true);
    }
  };

  const toggleEmojiPicker = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e));
    if (showEmojiPicker) {
      setShowEmojiPicker(false);
    } else {
      setPlusExpanded(false);
      expandAnim.setValue(0);
      setShowEmojiPicker(true);
    }
  };

  const handleEmojiSelect = (emoji: string) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e));
    onEmojiSelect?.(emoji);
  };

  const closeAll = () => {
    setPlusExpanded(false);
    setShowEmojiPicker(false);
    expandAnim.setValue(0);
  };

  const quickActions = [
    { iconName: 'camera' as const, label: '拍照', onPress: () => { closeAll(); onTakePhoto(); } },
    { iconName: 'image' as const, label: '相册', onPress: () => { closeAll(); onPickImage(); } },
    { iconName: 'document' as const, label: '文件', onPress: () => { closeAll(); onPickDocument(); } },
    { iconName: 'smile' as const, label: '表情', onPress: () => { toggleEmojiPicker(); } },
  ];

  return (
    <View style={[styles.wrapper, { backgroundColor: bgColor, borderTopColor: borderColor }]}>
      {/* Attachment preview chips */}
      {attachments.length > 0 && (
        <View style={[styles.attachRow, { borderBottomColor: borderColor }]}>
          {attachments.map((att, i) => (
            <View key={i} style={[styles.attachChip, { borderColor, backgroundColor: inputBgColor }]}>
              <Text style={[styles.attachName, { color: textColorSecondary }]} numberOfLines={1}>
                <Icon name={att.type === 'image' ? 'image' : 'document'} size={12} color={textColorSecondary} /> {att.name}
              </Text>
              <TouchableOpacity onPress={() => onRemoveAttachment(i)} hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }} accessibilityRole="button" accessibilityLabel="删除附件">
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
                accessibilityRole="button"
                accessibilityLabel={action.label}
              >
                <Icon name={action.iconName} size={22} color={textColorSecondary} />
                <Text style={[styles.expandedLabel, { color: textColorSecondary }]}>{action.label}</Text>
              </TouchableOpacity>
            ))}
          </View>
        </Animated.View>
      )}

      {/* Emoji picker panel */}
      {showEmojiPicker && (
        <View style={[styles.emojiPanel, { backgroundColor: inputBgColor, borderTopColor: borderColor }]}>
          <View style={[styles.emojiTabs, { borderBottomColor: borderColor }]}>
            {EMOJI_CATEGORIES.map((cat, i) => (
              <TouchableOpacity
                key={i}
                style={[styles.emojiTab, activeEmojiCategory === i && { borderBottomColor: accentColor, borderBottomWidth: 2 }]}
                onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e)); setActiveEmojiCategory(i); }}
                accessibilityRole="tab"
                accessibilityLabel={cat.name}
                accessibilityState={{ selected: activeEmojiCategory === i }}
              >
                <Text style={[styles.emojiTabText, { color: activeEmojiCategory === i ? accentColor : textColorSecondary }]}>
                  {cat.name}
                </Text>
              </TouchableOpacity>
            ))}
            <View style={{ flex: 1 }} />
            <TouchableOpacity
              style={styles.emojiTab}
              onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(e => console.warn('[Haptics] error:', e)); setShowEmojiPicker(false); }}
              accessibilityRole="button"
              accessibilityLabel="关闭表情"
            >
              <Text style={[styles.emojiTabText, { color: textColorSecondary }]}>✕</Text>
            </TouchableOpacity>
          </View>
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
                accessibilityRole="button"
                accessibilityLabel={item}
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
          accessibilityRole="button"
          accessibilityLabel={plusExpanded ? '收起菜单' : '展开更多操作'}
        >
          <Icon name={plusExpanded ? 'close' : 'plus'} size={20} color={plusExpanded ? accentColor : textColorSecondary} />
        </TouchableOpacity>

        {/* Text input / Waveform (when recording) */}
        {voiceListening ? (
          <View style={[styles.waveformContainer, { backgroundColor: inputBgColor, borderColor }]}>
            <Animated.View style={{ transform: [{ scale: pulseAnim }] }}>
              <Icon name="mic" size={16} color="#FF3B30" />
            </Animated.View>
            <AudioWaveform volume={voiceVolume} color={accentColor} />
            <Text style={[styles.recordingHint, { color: textColorSecondary }]}>点击麦克风停止</Text>
          </View>
        ) : (
          <TextInput
            testID="chat-input"
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
            returnKeyType="send"
            onSubmitEditing={onSend}
            blurOnSubmit={false}
            testID="chat-input"
            onFocus={closeAll}
          />
        )}

        {/* Emoji button */}
        {!voiceListening && (
          <TouchableOpacity
            style={[styles.emojiBtn, showEmojiPicker && { backgroundColor: accentColor + '20' }]}
            onPress={toggleEmojiPicker}
            accessibilityRole="button"
            accessibilityLabel="表情"
          >
            <Icon name="smile" size={20} color={showEmojiPicker ? accentColor : textColorSecondary} />
          </TouchableOpacity>
        )}

        {/* Voice button — always visible when available */}
        {voiceAvailable && (
          <TouchableOpacity
            style={[styles.voiceBtn, voiceListening && { backgroundColor: '#FF3B3020' }]}
            onPress={onVoiceToggle}
            accessibilityRole="button"
            accessibilityLabel={voiceListening ? '停止录音' : '语音输入'}
          >
            <Icon name={voiceListening ? 'stop' : 'mic'} size={18} color={voiceListening ? '#FF3B30' : textColorSecondary} />
          </TouchableOpacity>
        )}

        {/* Send / Stop button */}
        {streaming ? (
          <TouchableOpacity testID="stop-btn" style={[styles.sendBtn, { backgroundColor: '#FF3B30' }]} onPress={onStop} accessibilityRole="button" accessibilityLabel="停止生成">
            <Icon name="stop" size={16} color="#fff" />
          </TouchableOpacity>
        ) : (
          <TouchableOpacity
            testID="send-btn"
            style={[styles.sendBtn, { backgroundColor: (input.trim() || attachments.length > 0) ? accentColor : borderColor }]}
            onPress={onSend}
            disabled={!input.trim() && attachments.length === 0}
            accessibilityRole="button"
            accessibilityLabel="发送消息"
          >
            <Icon name="send" size={16} color={(input.trim() || attachments.length > 0) ? '#fff' : textColorSecondary} />
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
  expandedLabel: { fontSize: 10, fontWeight: '500' },
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
  waveformContainer: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    borderWidth: 1,
    borderRadius: 20,
    paddingHorizontal: 10,
    height: 40,
    gap: 6,
  },
  recordingHint: {
    fontSize: 11,
    flexShrink: 0,
  },
  emojiBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
  voiceBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
  sendBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
});
