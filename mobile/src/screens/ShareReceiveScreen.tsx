/**
 * ShareReceiveScreen — handles incoming share intents from the system Share Sheet.
 *
 * When the user shares content (text, URL, image) to VaultPilot from another app,
 * this screen receives the shared payload via expo-sharing's useIncomingShare()
 * and saves it as a new vault note.
 *
 * @feature #3073 — 移动端「分享到 VaultPilot」系统分享面板
 */
import React, { useEffect, useState, useCallback, useMemo } from 'react';
import {
  View, Text, TextInput, TouchableOpacity, Alert,
  ActivityIndicator, StyleSheet,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as FileSystem from 'expo-file-system';
import { useIncomingShare } from 'expo-sharing';
import type { ResolvedSharePayload } from 'expo-sharing';
import Ionicons from '@expo/vector-icons/Ionicons';
import { useAppStore, getColors } from '../store';
import { createNote } from '../db';
import { extractShareText, extractShareUrls, suggestShareTitle, resolveShareFileName } from '../utils/shareHelpers';

/** Extract a suggested note title */

export default function ShareReceiveScreen({ navigation }: any) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const { resolvedSharedPayloads, isResolving } = useIncomingShare();

  const [title, setTitle] = useState('分享笔记');
  const [saving, setSaving] = useState(false);
  const [savedNoteId, setSavedNoteId] = useState<string | null>(null);

  // Build note content from shared payloads
  const noteContent = useMemo(() => {
    if (!resolvedSharedPayloads || resolvedSharedPayloads.length === 0) return '';
    return resolvedSharedPayloads
      .map((p, i) => extractShareText(p, i))
      .filter(Boolean)
      .join('\n\n---\n\n');
  }, [resolvedSharedPayloads]);

  // Suggest a title from the first meaningful text
  useEffect(() => {
    if (!resolvedSharedPayloads || resolvedSharedPayloads.length === 0) return;
    setTitle(suggestShareTitle(resolvedSharedPayloads[0]));
  }, [resolvedSharedPayloads]);

  // Copy shared images/files into the vault directory
  const copyToVault = useCallback(async (payloads: ResolvedSharePayload[]) => {
    // Use legacy expo-file-system API (documentDirectory)
    const legacyFs = FileSystem as any;
    const vaultDir = `${legacyFs.documentDirectory}vault/`;
    const dirInfo = await legacyFs.getInfoAsync(vaultDir);
    if (!dirInfo.exists) {
      await legacyFs.makeDirectoryAsync(vaultDir, { intermediates: true });
    }

    const copies: string[] = [];
    for (const [index, p] of payloads.entries()) {
      if ((p.shareType === 'image' || p.shareType === 'file') && p.contentUri) {
        const fileName = resolveShareFileName(p, index);
        const dest = `${vaultDir}${fileName}`;
        try {
          await FileSystem.copyAsync({ from: p.contentUri, to: dest });
          copies.push(fileName);
        } catch (e) {
          console.warn('[ShareReceive] copy failed for', p.contentUri, e);
        }
      }
    }
    return copies;
  }, []);

  const handleSave = useCallback(async () => {
    if (!noteContent && resolvedSharedPayloads.length === 0) {
      Alert.alert('无内容', '未检测到可保存的分享内容。');
      return;
    }
    setSaving(true);
    try {
      // Copy shared files to vault first
      await copyToVault(resolvedSharedPayloads);

      // Build content with source URL attribution
      let finalContent = noteContent;
      const shareUrls = extractShareUrls(resolvedSharedPayloads);
      if (shareUrls.length > 0) {
        finalContent = `> 来源：${shareUrls.join('、')}\n\n` + finalContent;
      }

      const saved = await createNote(title, finalContent);
      setSavedNoteId(saved);
    } catch (e) {
      console.error('[ShareReceive] Save failed:', e);
      Alert.alert('保存失败', String(e));
    } finally {
      setSaving(false);
    }
  }, [noteContent, title, resolvedSharedPayloads, copyToVault]);

  // Go to the new note
  const handleOpenNote = useCallback(() => {
    if (savedNoteId) {
      navigation.navigate('Notes', {
        screen: 'NoteEdit',
        params: { noteId: savedNoteId },
      });
    }
  }, [savedNoteId, navigation]);

  const handleGoHome = useCallback(() => {
    navigation.navigate('Chat', { screen: 'ChatMain' });
  }, [navigation]);

  // Show loading while resolving share intent
  if (isResolving) {
    return (
      <SafeAreaView style={[styles.container, { backgroundColor: c.bg }]}>
        <ActivityIndicator size="large" color={accentColor} />
        <Text style={[styles.hint, { color: c.textSecondary }]}>接收分享内容中…</Text>
      </SafeAreaView>
    );
  }

  // No share data received
  if (!resolvedSharedPayloads || resolvedSharedPayloads.length === 0) {
    return (
      <SafeAreaView style={[styles.container, { backgroundColor: c.bg }]}>
        <View style={styles.emptyState}>
          <Ionicons name="share-outline" size={48} color={c.border} />
          <Text style={[styles.emptyTitle, { color: c.text }]}>无可保存的分享内容</Text>
          <Text style={[styles.emptyHint, { color: c.textSecondary }]}>
            从其他 App 分享内容到 VaultPilot 后会显示在这里
          </Text>
          <TouchableOpacity style={[styles.btn, { backgroundColor: accentColor }]} onPress={handleGoHome}>
            <Text style={styles.btnText}>返回首页</Text>
          </TouchableOpacity>
        </View>
      </SafeAreaView>
    );
  }

  // Show preview and save button
  return (
    <SafeAreaView style={[styles.container, { backgroundColor: c.bg }]} edges={['top']}>
      <View style={[styles.header, { borderBottomColor: c.border }]}>
        <TouchableOpacity onPress={handleGoHome}>
          <Ionicons name="close" size={24} color={c.text} />
        </TouchableOpacity>
        <Text style={[styles.headerTitle, { color: c.text }]}>快速速记</Text>
        <View style={{ width: 24 }} />
      </View>

      <View style={styles.body}>
        {/* Title input */}
        <Text style={[styles.label, { color: c.textSecondary }]}>标题</Text>
        <TextInput
          style={[styles.titleInput, {
            color: c.text,
            backgroundColor: isDark ? '#1F2937' : '#F3F4F6',
            borderColor: c.border,
          }]}
          value={title}
          onChangeText={setTitle}
          placeholder="输入标题…"
          placeholderTextColor={c.textSecondary}
        />

        {/* Content preview */}
        <Text style={[styles.label, { color: c.textSecondary }]}>内容预览</Text>
        <View style={[styles.preview, { backgroundColor: isDark ? '#1F2937' : '#F3F4F6', borderColor: c.border }]}>
          <Text
            style={[styles.previewText, { color: c.text }]}
            numberOfLines={15}
          >
            {noteContent || '(无文本内容)'}
          </Text>
          {resolvedSharedPayloads.some((p) => p.shareType === 'image') && (
            <Text style={[styles.imgHint, { color: c.textSecondary }]}>
              📷 {resolvedSharedPayloads.filter((p) => p.shareType === 'image').length} 张图片将保存到 vault
            </Text>
          )}
        </View>

        {savedNoteId ? (
          <View style={styles.savedActions}>
            <Ionicons name="checkmark-circle" size={24} color="#10B981" />
            <Text style={[styles.savedText, { color: '#10B981' }]}>已保存</Text>
            <TouchableOpacity
              style={[styles.btn, styles.openBtn, { backgroundColor: accentColor }]}
              onPress={handleOpenNote}
            >
              <Text style={styles.btnText}>打开笔记</Text>
            </TouchableOpacity>
          </View>
        ) : (
          <TouchableOpacity
            style={[styles.saveBtn, { backgroundColor: accentColor, opacity: saving ? 0.6 : 1 }]}
            onPress={handleSave}
            disabled={saving}
          >
            {saving ? (
              <ActivityIndicator color="#FFF" size="small" />
            ) : (
              <>
                <Ionicons name="save-outline" size={20} color="#FFF" />
                <Text style={styles.saveBtnText}>保存到 Vault</Text>
              </>
            )}
          </TouchableOpacity>
        )}
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderBottomWidth: 1,
  },
  headerTitle: { fontSize: 17, fontWeight: '600' },
  body: { flex: 1, padding: 16 },
  label: { fontSize: 13, fontWeight: '500', marginBottom: 6, marginTop: 12 },
  titleInput: {
    borderWidth: 1,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
    fontSize: 16,
  },
  preview: {
    borderWidth: 1,
    borderRadius: 8,
    padding: 12,
    minHeight: 100,
  },
  previewText: { fontSize: 14, lineHeight: 20 },
  imgHint: { fontSize: 12, marginTop: 8 },
  saveBtn: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    paddingVertical: 14,
    borderRadius: 12,
    marginTop: 24,
    gap: 8,
  },
  saveBtnText: { color: '#FFF', fontSize: 16, fontWeight: '600' },
  savedActions: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    marginTop: 24,
    gap: 10,
  },
  savedText: { fontSize: 15, fontWeight: '500' },
  openBtn: {
    paddingHorizontal: 20,
    paddingVertical: 10,
    borderRadius: 8,
  },
  btn: {
    paddingHorizontal: 24,
    paddingVertical: 12,
    borderRadius: 8,
    marginTop: 16,
  },
  btnText: { color: '#FFF', fontSize: 15, fontWeight: '600' },
  emptyState: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    padding: 32,
    gap: 12,
  },
  emptyTitle: { fontSize: 18, fontWeight: '600', marginTop: 8 },
  emptyHint: { fontSize: 14, textAlign: 'center', lineHeight: 20 },
  hint: { marginTop: 12, fontSize: 14 },
});