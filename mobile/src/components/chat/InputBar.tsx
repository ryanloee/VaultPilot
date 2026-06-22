import React from 'react';
import { View, Text, TextInput, TouchableOpacity, StyleSheet } from 'react-native';
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
}

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
}: InputBarProps) {
  return (
    <>
      {/* Attachment preview chips */}
      {attachments.length > 0 && (
        <View style={[styles.attachRow, { backgroundColor: bgColor }]}>
          {attachments.map((att, i) => (
            <View key={i} style={[styles.attachChip, { borderColor }]}>
              <Text style={[styles.attachName, { color: textColor }]} numberOfLines={1}>
                {att.type === 'image' ? '🖼' : '📄'} {att.name}
              </Text>
              <TouchableOpacity onPress={() => onRemoveAttachment(i)}>
                <Text style={{ color: '#EF4444', fontSize: 14 }}>✕</Text>
              </TouchableOpacity>
            </View>
          ))}
        </View>
      )}

      {/* Input bar */}
      <View style={[styles.inputBar, { borderTopColor: borderColor, backgroundColor: bgColor }]}>
        <View style={styles.quickActions}>
          <TouchableOpacity
            style={[styles.quickBtn, { borderColor }]}
            onPress={() => {
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              onTakePhoto();
            }}
            accessibilityRole="button"
            accessibilityLabel="拍照"
          >
            <Text style={{ fontSize: 16 }}>📷</Text>
          </TouchableOpacity>
          <TouchableOpacity
            style={[styles.quickBtn, { borderColor }]}
            onPress={() => {
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              onPickImage();
            }}
            accessibilityRole="button"
            accessibilityLabel="从相册选择"
          >
            <Text style={{ fontSize: 16 }}>🖼</Text>
          </TouchableOpacity>
          <TouchableOpacity
            style={[styles.quickBtn, { borderColor }]}
            onPress={() => {
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              onPickDocument();
            }}
            accessibilityRole="button"
            accessibilityLabel="选择文件"
          >
            <Text style={{ fontSize: 16 }}>📄</Text>
          </TouchableOpacity>
        </View>
        <TextInput
          style={[
            styles.textInput,
            {
              backgroundColor: inputBgColor,
              color: textColor,
              borderColor,
              height: Math.max(40, Math.min(inputHeight, 120)),
            },
          ]}
          value={input}
          onChangeText={onInputChange}
          onContentSizeChange={(e) =>
            onInputHeightChange(e.nativeEvent.contentSize.height)
          }
          placeholder="输入消息..."
          placeholderTextColor={textColorSecondary}
          maxLength={4000}
          editable={!streaming}
          returnKeyType="send"
          blurOnSubmit
          onSubmitEditing={onSend}
          accessibilityLabel="消息输入框"
        />
        {/* Voice input button */}
        {voiceAvailable && !streaming && (
          <TouchableOpacity
            onPress={() => {
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              onVoiceToggle();
            }}
            style={[
              styles.sendBtn,
              { backgroundColor: voiceListening ? '#EF4444' : borderColor },
            ]}
            accessibilityRole="button"
            accessibilityLabel={voiceListening ? '停止录音' : '语音输入'}
          >
            <Text style={styles.sendText}>{voiceListening ? '⏹' : '🎤'}</Text>
          </TouchableOpacity>
        )}
        {streaming ? (
          <TouchableOpacity
            onPress={onStop}
            style={[styles.sendBtn, { backgroundColor: '#EF4444' }]}
            accessibilityRole="button"
            accessibilityLabel="停止生成"
          >
            <Text style={styles.sendText}>■</Text>
          </TouchableOpacity>
        ) : (
          <TouchableOpacity
            onPress={onSend}
            style={[
              styles.sendBtn,
              {
                backgroundColor:
                  input.trim() || attachments.length > 0
                    ? accentColor
                    : borderColor,
              },
            ]}
            disabled={!input.trim() && attachments.length === 0}
            accessibilityRole="button"
            accessibilityLabel="发送消息"
          >
            <Text style={styles.sendText}>➤</Text>
          </TouchableOpacity>
        )}
      </View>
    </>
  );
}

const styles = StyleSheet.create({
  attachRow: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 6,
    paddingHorizontal: 12,
    paddingVertical: 6,
  },
  attachChip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: 10,
    paddingVertical: 4,
    borderRadius: 12,
    borderWidth: 1,
  },
  attachName: { fontSize: 12, maxWidth: 150 },
  inputBar: {
    flexDirection: 'row',
    alignItems: 'flex-end',
    padding: 8,
    borderTopWidth: 1,
  },
  quickActions: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
    marginRight: 6,
  },
  quickBtn: {
    width: 34,
    height: 34,
    borderRadius: 17,
    justifyContent: 'center',
    alignItems: 'center',
    borderWidth: 1,
  },
  textInput: {
    flex: 1,
    borderWidth: 1,
    borderRadius: 20,
    paddingHorizontal: 16,
    paddingVertical: 10,
    fontSize: 15,
  },
  sendBtn: {
    width: 40,
    height: 40,
    borderRadius: 20,
    justifyContent: 'center',
    alignItems: 'center',
    marginLeft: 8,
  },
  sendText: { color: '#FFF', fontSize: 18 },
});