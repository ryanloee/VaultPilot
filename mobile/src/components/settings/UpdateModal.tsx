import React from 'react';
import { View, Text, TouchableOpacity, ScrollView, Modal, Linking, StyleSheet } from 'react-native';
import type { UpdateInfo } from '../../utils/updateChecker';

interface UpdateModalProps {
  visible: boolean;
  updateInfo: UpdateInfo | null;
  accentColor: string;
  textColor: string;
  textColorSecondary: string;
  borderColor: string;
  cardBgColor: string;
  onClose: () => void;
  onSkip: () => void;
}

export default function UpdateModal({
  visible,
  updateInfo,
  accentColor,
  textColor,
  textColorSecondary,
  borderColor,
  cardBgColor,
  onClose,
  onSkip,
}: UpdateModalProps) {
  if (!updateInfo) return null;

  return (
    <Modal visible={visible} transparent animationType="slide">
      <View style={styles.modalOverlay}>
        <View style={[styles.modalContent, { backgroundColor: cardBgColor }]}>
          <Text style={[styles.sectionTitle, { color: textColor }]}>🎉 发现新版本</Text>
          <Text style={{ color: textColor, fontSize: 16, marginTop: 8 }}>
            v{updateInfo.currentVersion} → v{updateInfo.latestVersion}
          </Text>
          {updateInfo.body ? (
            <ScrollView style={{ maxHeight: 200, marginTop: 12 }}>
              <Text style={{ color: textColorSecondary, fontSize: 14, lineHeight: 20 }}>
                {updateInfo.body.slice(0, 1000)}
              </Text>
            </ScrollView>
          ) : null}
          <View style={{ flexDirection: 'row', gap: 12, marginTop: 16 }}>
            <TouchableOpacity
              style={[styles.modalClose, { borderColor, flex: 1 }]}
              onPress={onSkip}
            >
              <Text style={{ color: textColorSecondary }}>跳过此版本</Text>
            </TouchableOpacity>
            <TouchableOpacity
              style={[
                styles.modalClose,
                { borderColor: accentColor, backgroundColor: accentColor + '15', flex: 1 },
              ]}
              onPress={() => {
                const url = updateInfo.apkUrl ?? updateInfo.releaseUrl;
                Linking.openURL(url);
                onClose();
              }}
            >
              <Text style={{ color: accentColor, fontWeight: '600' }}>
                {updateInfo.apkUrl ? '下载 APK' : '查看发布页'}
              </Text>
            </TouchableOpacity>
          </View>
          <TouchableOpacity style={{ paddingVertical: 10, alignItems: 'center' }} onPress={onClose}>
            <Text style={{ color: textColorSecondary, fontSize: 13 }}>稍后提醒</Text>
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
  modalClose: {
    paddingVertical: 14,
    alignItems: 'center',
    marginTop: 8,
    borderTopWidth: 1,
  },
});