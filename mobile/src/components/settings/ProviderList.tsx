import React from 'react';
import { View, Text, TouchableOpacity, StyleSheet } from 'react-native';
import type { ProviderConfig } from '../../store';

interface ProviderListProps {
  providers: ProviderConfig[];
  activeIndex: number;
  accentColor: string;
  textColor: string;
  textColorSecondary: string;
  inputBgColor: string;
  borderColor: string;
  onSelect: (index: number) => void;
  onDelete: (index: number) => void;
  onAdd: () => void;
}

export default function ProviderList({
  providers,
  activeIndex,
  accentColor,
  textColor,
  textColorSecondary,
  inputBgColor,
  borderColor,
  onSelect,
  onDelete,
  onAdd,
}: ProviderListProps) {
  return (
    <>
      <View style={styles.sectionHeader}>
        <Text style={[styles.sectionTitle, { color: textColor }]}>API 提供商</Text>
        <TouchableOpacity onPress={onAdd} style={[styles.addBtn, { borderColor: accentColor }]}>
          <Text style={{ color: accentColor, fontWeight: '600' }}>+ 添加</Text>
        </TouchableOpacity>
      </View>

      {providers.map((p, i) => (
        <TouchableOpacity
          key={i}
          style={[
            styles.providerCard,
            {
              backgroundColor: i === activeIndex ? accentColor + '15' : inputBgColor,
              borderColor: i === activeIndex ? accentColor : borderColor,
            },
          ]}
          onPress={() => onSelect(i)}
        >
          <View style={{ flex: 1 }}>
            <View style={styles.providerCardHeader}>
              <Text style={[styles.providerCardName, { color: textColor }]}>
                {i === activeIndex ? '● ' : ''}{p.name}
              </Text>
              {providers.length > 1 && (
                <TouchableOpacity
                  onPress={() => onDelete(i)}
                  hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
                >
                  <Text style={{ color: '#EF4444', fontSize: 14 }}>删除</Text>
                </TouchableOpacity>
              )}
            </View>
            <Text style={[styles.providerCardDetail, { color: textColorSecondary }]} numberOfLines={1}>
              {p.model} · {p.apiFormat.toUpperCase()}
            </Text>
          </View>
        </TouchableOpacity>
      ))}
    </>
  );
}

const styles = StyleSheet.create({
  sectionHeader: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: 12,
  },
  sectionTitle: { fontSize: 20, fontWeight: '700' },
  addBtn: {
    paddingHorizontal: 14,
    paddingVertical: 6,
    borderRadius: 16,
    borderWidth: 1,
  },
  providerCard: {
    borderWidth: 1,
    borderRadius: 12,
    padding: 14,
    marginBottom: 8,
  },
  providerCardHeader: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
  },
  providerCardName: { fontSize: 16, fontWeight: '600' },
  providerCardDetail: { fontSize: 13, marginTop: 4 },
});