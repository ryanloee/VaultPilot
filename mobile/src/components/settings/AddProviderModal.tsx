import React from 'react';
import { View, Text, TouchableOpacity, ScrollView, Modal, StyleSheet } from 'react-native';
import { PROVIDERS } from '../../store';
import Icon from '../../components/Icon';

interface AddProviderModalProps {
  visible: boolean;
  accentColor: string;
  textColor: string;
  textColorSecondary: string;
  borderColor: string;
  cardBgColor: string;
  onClose: () => void;
  onSelectPreset: (preset: typeof PROVIDERS[number]) => void;
  onAddCustom: () => void;
}

export default function AddProviderModal({
  visible,
  accentColor,
  textColor,
  textColorSecondary,
  borderColor,
  cardBgColor,
  onClose,
  onSelectPreset,
  onAddCustom,
}: AddProviderModalProps) {
  return (
    <Modal visible={visible} transparent animationType="slide">
      <View style={styles.modalOverlay}>
        <View style={[styles.modalContent, { backgroundColor: cardBgColor }]}>
          <Text style={[styles.sectionTitle, { color: textColor }]}>添加提供商</Text>
          <ScrollView>
            {PROVIDERS.map(p => (
              <TouchableOpacity
                key={p.name}
                style={[styles.modalItem, { borderColor }]}
                onPress={() => onSelectPreset(p)}
              >
                <Text style={[styles.modalItemName, { color: textColor }]}>{p.name}</Text>
                <Text style={[styles.modalItemDetail, { color: textColorSecondary }]}>
                  {p.models.slice(0, 2).join(', ')}{p.models.length > 2 ? '...' : ''}
                </Text>
              </TouchableOpacity>
            ))}
            <TouchableOpacity style={[styles.modalItem, { borderColor }]} onPress={onAddCustom}>
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
                <Icon name="edit" size={16} color={accentColor} />
                <Text style={[styles.modalItemName, { color: accentColor }]}>自定义提供商</Text>
              </View>
            </TouchableOpacity>
          </ScrollView>
          <TouchableOpacity style={[styles.modalClose, { borderColor }]} onPress={onClose}>
            <Text style={{ color: textColorSecondary }}>取消</Text>
          </TouchableOpacity>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  sectionTitle: { fontSize: 20, fontWeight: '700' },
  modalOverlay: {
    flex: 1,
    justifyContent: 'flex-end',
    backgroundColor: 'rgba(0,0,0,0.5)',
  },
  modalContent: {
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    padding: 20,
    maxHeight: '70%',
  },
  modalItem: { borderBottomWidth: 1, paddingVertical: 14 },
  modalItemName: { fontSize: 16, fontWeight: '600' },
  modalItemDetail: { fontSize: 13, marginTop: 2 },
  modalClose: {
    paddingVertical: 14,
    alignItems: 'center',
    marginTop: 8,
    borderTopWidth: 1,
  },
});