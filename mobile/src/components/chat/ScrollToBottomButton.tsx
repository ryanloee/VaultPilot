import React from 'react';
import { TouchableOpacity, Text, StyleSheet } from 'react-native';

interface ScrollToBottomButtonProps {
  visible: boolean;
  accentColor: string;
  bgColor: string;
  borderColor: string;
  onPress: () => void;
}

export default function ScrollToBottomButton({
  visible,
  accentColor,
  bgColor,
  borderColor,
  onPress,
}: ScrollToBottomButtonProps) {
  if (!visible) return null;

  return (
    <TouchableOpacity
      onPress={onPress}
      style={[styles.button, { backgroundColor: bgColor, borderColor }]}
      accessibilityRole="button"
      accessibilityLabel="滚动到底部"
    >
      <Text style={{ color: accentColor, fontSize: 16 }}>↓</Text>
    </TouchableOpacity>
  );
}

const styles = StyleSheet.create({
  button: {
    position: 'absolute',
    bottom: 70,
    alignSelf: 'center',
    width: 36,
    height: 36,
    borderRadius: 18,
    justifyContent: 'center',
    alignItems: 'center',
    borderWidth: 1,
    elevation: 2,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.15,
    shadowRadius: 2,
  },
});