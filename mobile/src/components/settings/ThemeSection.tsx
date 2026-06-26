import React from 'react';
import { View, Text, TouchableOpacity, StyleSheet } from 'react-native';
import { ACCENT_COLORS } from '../../store';
import Icon from '../../components/Icon';

interface ThemeSectionProps {
  themeMode: string;
  accentColor: string;
  textColor: string;
  textColorSecondary: string;
  borderColor: string;
  onThemeChange: (mode: 'light' | 'dark' | 'system') => void;
  onAccentChange: (color: string) => void;
}

export default function ThemeSection({
  themeMode,
  accentColor,
  textColor,
  textColorSecondary,
  borderColor,
  onThemeChange,
  onAccentChange,
}: ThemeSectionProps) {
  return (
    <>
      <Text style={[styles.sectionTitle, { color: textColor, marginTop: 24 }]}>外观</Text>

      <Text style={[styles.label, { color: textColorSecondary }]}>主题</Text>
      <View style={styles.themeRow}>
        {(['light', 'dark', 'system'] as const).map(mode => (
          <TouchableOpacity
            key={mode}
            style={[
              styles.themeBtn,
              {
                borderColor: themeMode === mode ? accentColor : borderColor,
                backgroundColor: themeMode === mode ? accentColor + '20' : 'transparent',
              },
            ]}
            onPress={() => onThemeChange(mode)}
          >
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
              <Icon
                name={mode === 'light' ? 'sun' : mode === 'dark' ? 'moon' : 'refresh'}
                size={14}
                color={themeMode === mode ? accentColor : textColor}
              />
              <Text style={{ color: themeMode === mode ? accentColor : textColor }}>
                {mode === 'light' ? '亮色' : mode === 'dark' ? '暗色' : '跟随系统'}
              </Text>
            </View>
          </TouchableOpacity>
        ))}
      </View>

      <Text style={[styles.label, { color: textColorSecondary }]}>主色调</Text>
      <View style={styles.colorRow}>
        {ACCENT_COLORS.map(ac => (
          <TouchableOpacity
            key={ac.value}
            style={[
              styles.colorDot,
              {
                backgroundColor: ac.value,
                borderWidth: accentColor === ac.value ? 3 : 0,
                borderColor: '#FFF',
              },
            ]}
            onPress={() => onAccentChange(ac.value)}
            accessibilityRole="radio"
            accessibilityLabel={`主色调: ${ac.name}`}
            accessibilityState={{ selected: accentColor === ac.value }}
          />
        ))}
      </View>
    </>
  );
}

const styles = StyleSheet.create({
  sectionTitle: { fontSize: 20, fontWeight: '700' },
  label: { fontSize: 13, fontWeight: '500', marginBottom: 6 },
  themeRow: { flexDirection: 'row', gap: 8, marginBottom: 16 },
  themeBtn: {
    flex: 1,
    paddingVertical: 10,
    borderRadius: 10,
    borderWidth: 1,
    alignItems: 'center',
  },
  colorRow: { flexDirection: 'row', gap: 12, marginBottom: 24 },
  colorDot: { width: 36, height: 36, borderRadius: 18 },
});