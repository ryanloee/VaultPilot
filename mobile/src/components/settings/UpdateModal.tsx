import React, { useState } from 'react';
import { View, Text, TouchableOpacity, ScrollView, Modal, StyleSheet, ActivityIndicator, Linking } from 'react-native';
import type { UpdateInfo } from '../../utils/updateChecker';
import { downloadAndInstall } from '../../utils/updateChecker';
import Icon from '../../components/Icon';

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
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);

  if (!updateInfo) return null;

  const handleDownload = async () => {
    if (!updateInfo.apkUrl) return;
    setDownloading(true);
    setProgress(0);
    setError(null);
    try {
      const ok = await downloadAndInstall(updateInfo.apkUrl, updateInfo.latestVersion, setProgress);
      if (!ok) {
        setError('下载或安装失败，请手动下载');
      }
    } catch (e) {
      console.warn('[UpdateModal] Download failed:', e);
      setError('下载失败，请手动下载');
    } finally {
      setDownloading(false);
    }
  };

  return (
    <Modal visible={visible} transparent animationType="slide">
      <View style={styles.modalOverlay}>
        <View style={[styles.modalContent, { backgroundColor: cardBgColor }]}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
            <Icon name="star" size={20} color={textColor} />
            <Text style={[styles.sectionTitle, { color: textColor }]}>发现新版本</Text>
          </View>
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

          {downloading ? (
            <View style={{ marginTop: 16, alignItems: 'center' }}>
              <ActivityIndicator color={accentColor} size="large" />
              <Text style={{ color: textColorSecondary, marginTop: 8 }}>
                正在下载... {progress}%
              </Text>
              <View style={[styles.progressBar, { borderColor }]}>
                <View style={[styles.progressFill, { width: `${progress}%`, backgroundColor: accentColor }]} />
              </View>
            </View>
          ) : error ? (
            <View style={{ marginTop: 16 }}>
              <Text style={{ color: '#ff4444', textAlign: 'center', marginBottom: 12 }}>{error}</Text>
              <View style={{ flexDirection: 'row', gap: 12 }}>
                <TouchableOpacity
                  style={[styles.modalClose, { borderColor, flex: 1 }]}
                  onPress={() => { setError(null); onClose(); }}
                >
                  <Text style={{ color: textColorSecondary, textAlign: 'center' }}>关闭</Text>
                </TouchableOpacity>
                <TouchableOpacity
                  style={[styles.modalClose, { borderColor: accentColor, backgroundColor: accentColor + '15', flex: 1 }]}
                  onPress={() => Linking.openURL(updateInfo.releaseUrl)}
                >
                  <Text style={{ color: accentColor, fontWeight: '600', textAlign: 'center' }}>手动下载</Text>
                </TouchableOpacity>
              </View>
            </View>
          ) : (
            <>
              <View style={{ flexDirection: 'row', gap: 12, marginTop: 16 }}>
                <TouchableOpacity
                  style={[styles.modalClose, { borderColor, flex: 1 }]}
                  onPress={onSkip}
                >
                  <Text style={{ color: textColorSecondary, textAlign: 'center' }}>跳过此版本</Text>
                </TouchableOpacity>
                <TouchableOpacity
                  style={[
                    styles.modalClose,
                    { borderColor: accentColor, backgroundColor: accentColor + '15', flex: 1 },
                  ]}
                  onPress={updateInfo.apkUrl ? handleDownload : () => Linking.openURL(updateInfo.releaseUrl)}
                >
                  <Text style={{ color: accentColor, fontWeight: '600', textAlign: 'center' }}>
                    {updateInfo.apkUrl ? '下载更新' : '查看发布页'}
                  </Text>
                </TouchableOpacity>
              </View>
              <TouchableOpacity style={{ paddingVertical: 10, alignItems: 'center' }} onPress={onClose}>
                <Text style={{ color: textColorSecondary, fontSize: 13 }}>稍后提醒</Text>
              </TouchableOpacity>
            </>
          )}
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
  progressBar: {
    width: '100%',
    height: 6,
    borderRadius: 3,
    borderWidth: 1,
    marginTop: 8,
    overflow: 'hidden',
  },
  progressFill: {
    height: '100%',
    borderRadius: 2,
  },
});
