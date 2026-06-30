/**
 * AiActionPalette — Mobile bottom-sheet command palette for AI quick actions.
 *
 * Triggered from the note editor toolbar (and potentially other screens),
 * this component provides a floating bottom sheet where users can search
 * for and execute AI actions (summarize, translate, rewrite, etc.) on
 * selected text or note content.
 *
 * Mirrors the WinUI AiCommandPalette at:
 *   native/VaultPilot.WinUI/Controls/AiCommandPalette.xaml
 */

import React, { useState, useRef, useCallback, useMemo } from 'react';
import {
  View,
  Text,
  TextInput,
  FlatList,
  TouchableOpacity,
  Modal,
  StyleSheet,
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  ScrollView,
} from 'react-native';
import Ionicons from '@expo/vector-icons/Ionicons';
import * as Clipboard from 'expo-clipboard';
import { useAppStore, getColors } from '../../store';
import {
  listAiActions,
  executeAiAction,
  getAiActionInfo,
  AiActionInfo,
  AiActionId,
  AiActionResult,
} from '../../utils/aiActions';

interface Props {
  visible: boolean;
  onClose: () => void;
  /** The text to operate on (selection or note body). */
  sourceText: string;
  /** Callback when the user wants to insert the result. */
  onInsertResult?: (text: string) => void;
  /** Optional note ID for context. */
  noteId?: string;
}

export default function AiActionPalette({
  visible,
  onClose,
  sourceText,
  onInsertResult,
  noteId,
}: Props) {
  const { isDark, accentColor } = useAppStore();
  const c = getColors(isDark, accentColor);
  const searchRef = useRef<TextInput>(null);

  // All available actions (loaded once)
  const allActions = useMemo(() => listAiActions(), []);

  // State
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedAction, setSelectedAction] = useState<AiActionInfo | null>(null);
  const [executing, setExecuting] = useState(false);
  const [result, setResult] = useState<AiActionResult | null>(null);
  const [showResult, setShowResult] = useState(false);
  const [abortController, setAbortController] = useState<AbortController | null>(null);

  // Filtered actions based on search
  const filteredActions = useMemo(() => {
    if (!searchQuery.trim()) return allActions;
    const q = searchQuery.trim().toLowerCase();
    return allActions.filter(
      a =>
        a.label.toLowerCase().includes(q) ||
        a.id.toLowerCase().includes(q) ||
        a.description.toLowerCase().includes(q),
    );
  }, [allActions, searchQuery]);

  // Reset state when the palette opens
  const handleShow = useCallback(() => {
    setSearchQuery('');
    setSelectedAction(null);
    setResult(null);
    setShowResult(false);
    setExecuting(false);
    setTimeout(() => searchRef.current?.focus(), 300);
  }, []);

  // Execute the selected action
  const handleExecute = useCallback(async (action: AiActionInfo) => {
    if (executing) return;
    setSelectedAction(action);
    setExecuting(true);
    setResult(null);
    setShowResult(false);

    const ac = new AbortController();
    setAbortController(ac);

    try {
      const text = sourceText.trim() || '';
      const aiResult = await executeAiAction(
        {
          action: action.id as AiActionId,
          text,
          noteId,
        },
        ac.signal,
      );
      if (aiResult.error) {
        setResult(aiResult);
        setShowResult(true);
      } else if (aiResult.result) {
        setResult(aiResult);
        setShowResult(true);
      }
    } catch (e: any) {
      setResult({
        result: '',
        usage: { promptTokens: 0, completionTokens: 0, totalTokens: 0 },
        error: e.message || '操作执行失败',
        get isSuccess() { return false; },
      });
      setShowResult(true);
    } finally {
      setExecuting(false);
      setAbortController(null);
    }
  }, [executing, sourceText, noteId]);

  // Cancel execution
  const handleCancel = useCallback(() => {
    abortController?.abort();
    setExecuting(false);
    setSelectedAction(null);
  }, [abortController]);

  // Copy result to clipboard
  const handleCopy = useCallback(async () => {
    if (!result?.result) return;
    try {
      await Clipboard.setStringAsync(result.result);
    } catch {
      // silently fail
    }
  }, [result]);

  // Insert result into note
  const handleInsert = useCallback(() => {
    if (result?.result && onInsertResult) {
      onInsertResult(result.result);
    }
    onClose();
  }, [result, onInsertResult, onClose]);

  // Close result and go back to action list
  const handleBackToList = useCallback(() => {
    setShowResult(false);
    setResult(null);
    setSelectedAction(null);
  }, []);

  // Dismiss
  const handleDismiss = useCallback(() => {
    abortController?.abort();
    onClose();
  }, [abortController, onClose]);

  // ── Render: action item ────────────────────────────────────

  const renderActionItem = useCallback(
    ({ item }: { item: AiActionInfo }) => (
      <TouchableOpacity
        style={[s.actionItem, { borderBottomColor: c.border }]}
        onPress={() => handleExecute(item)}
        activeOpacity={0.6}
      >
        <View style={[s.actionIcon, { backgroundColor: accentColor + '20' }]}>
          <Ionicons name={item.icon as any} size={20} color={accentColor} />
        </View>
        <View style={s.actionInfo}>
          <Text style={[s.actionLabel, { color: c.text }]}>{item.label}</Text>
          <Text style={[s.actionDesc, { color: c.textSecondary }]} numberOfLines={1}>
            {item.description}
          </Text>
        </View>
        <Ionicons name="chevron-forward-outline" size={16} color={c.textSecondary} />
      </TouchableOpacity>
    ),
    [c, accentColor, handleExecute],
  );

  const keyExtractor = useCallback((item: AiActionInfo) => item.id, []);

  // ── Render ──────────────────────────────────────────────────

  return (
    <Modal
      visible={visible}
      transparent
      animationType="slide"
      onShow={handleShow}
      onRequestClose={handleDismiss}
    >
      <KeyboardAvoidingView
        style={s.overlay}
        behavior={Platform.OS === 'ios' ? 'padding' : undefined}
      >
        {/* Semi-transparent backdrop — tap to close */}
        <TouchableOpacity
          style={s.backdrop}
          activeOpacity={1}
          onPress={handleDismiss}
        />

        {/* Bottom sheet card */}
        <View style={[s.sheet, { backgroundColor: c.card }]}>
          {/* Handle bar */}
          <View style={[s.handleBar, { backgroundColor: c.border }]} />

          {!showResult && !executing && (
            <>
              {/* Search header */}
              <View style={[s.searchBar, { borderBottomColor: c.border }]}>
                <Ionicons name="search-outline" size={18} color={c.textSecondary} />
                <TextInput
                  ref={searchRef}
                  style={[s.searchInput, { color: c.text }]}
                  placeholder="搜索 AI 操作..."
                  placeholderTextColor={c.textSecondary}
                  value={searchQuery}
                  onChangeText={setSearchQuery}
                  returnKeyType="search"
                  autoCapitalize="none"
                  autoCorrect={false}
                />
                {searchQuery.length > 0 && (
                  <TouchableOpacity onPress={() => setSearchQuery('')}>
                    <Ionicons name="close-circle-outline" size={18} color={c.textSecondary} />
                  </TouchableOpacity>
                )}
              </View>

              {/* Action list */}
              {filteredActions.length > 0 ? (
                <FlatList
                  data={filteredActions}
                  renderItem={renderActionItem}
                  keyExtractor={keyExtractor}
                  style={s.actionList}
                  keyboardShouldPersistTaps="handled"
                  showsVerticalScrollIndicator={false}
                />
              ) : (
                <View style={s.emptyState}>
                  <Ionicons name="search-outline" size={32} color={c.textSecondary} />
                  <Text style={[s.emptyText, { color: c.textSecondary }]}>
                    没有找到与 "{searchQuery}" 匹配的操作
                  </Text>
                </View>
              )}

              {/* Footer hint */}
              <View style={[s.footer, { borderTopColor: c.border }]}>
                <Ionicons name="information-circle-outline" size={13} color={c.textSecondary} />
                <Text style={[s.footerText, { color: c.textSecondary }]}>
                  选择操作后将对选中文本执行
                </Text>
              </View>
            </>
          )}

          {/* Loading overlay */}
          {executing && (
            <View style={s.loadingContainer}>
              <ActivityIndicator size="large" color={accentColor} />
              <Text style={[s.loadingText, { color: c.text }]}>
                {selectedAction?.label || '正在处理...'}
              </Text>
              <TouchableOpacity style={[s.cancelBtn, { borderColor: c.border }]} onPress={handleCancel}>
                <Text style={[s.cancelBtnText, { color: c.textSecondary }]}>取消</Text>
              </TouchableOpacity>
            </View>
          )}

          {/* Result display */}
          {showResult && result && !executing && (
            <>
              {/* Result header */}
              <View style={[s.resultHeader, { borderBottomColor: c.border }]}>
                <TouchableOpacity onPress={handleBackToList}>
                  <Ionicons name="arrow-back-outline" size={20} color={accentColor} />
                </TouchableOpacity>
                <Text style={[s.resultTitle, { color: c.text }]}>
                  {selectedAction?.label || '操作结果'}
                </Text>
                <TouchableOpacity onPress={handleDismiss}>
                  <Ionicons name="close-outline" size={22} color={c.textSecondary} />
                </TouchableOpacity>
              </View>

              {/* Result content */}
              <ScrollView style={s.resultScroll} contentContainerStyle={s.resultContent}>
                {result.error ? (
                  <View style={s.errorContainer}>
                    <Ionicons name="alert-circle-outline" size={20} color="#EF4444" />
                    <Text style={[s.errorText, { color: '#EF4444' }]}>{result.error}</Text>
                  </View>
                ) : (
                  <Text style={[s.resultText, { color: c.text }]} selectable>
                    {result.result}
                  </Text>
                )}
              </ScrollView>

              {/* Result footer */}
              <View style={[s.resultFooter, { borderTopColor: c.border }]}>
                {!result.error && (
                  <TouchableOpacity
                    style={[s.resultActionBtn, { backgroundColor: accentColor }]}
                    onPress={handleInsert}
                  >
                    <Ionicons name="chatbox-outline" size={14} color="#FFF" />
                    <Text style={s.resultActionText}>插入到笔记</Text>
                  </TouchableOpacity>
                )}
                {result.result && (
                  <TouchableOpacity
                    style={[s.resultActionBtn, { backgroundColor: accentColor + '20' }]}
                    onPress={handleCopy}
                  >
                    <Ionicons name="copy-outline" size={14} color={accentColor} />
                    <Text style={[s.resultActionText, { color: accentColor }]}>复制</Text>
                  </TouchableOpacity>
                )}
              </View>
            </>
          )}
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

// ── Styles ──────────────────────────────────────────────────

const s = StyleSheet.create({
  overlay: {
    flex: 1,
    justifyContent: 'flex-end',
  },
  backdrop: {
    flex: 1,
    backgroundColor: 'rgba(0,0,0,0.4)',
  },
  sheet: {
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    maxHeight: '80%',
    minHeight: 300,
    overflow: 'hidden',
  },
  handleBar: {
    width: 36,
    height: 4,
    borderRadius: 2,
    alignSelf: 'center',
    marginTop: 8,
    marginBottom: 4,
  },

  // ── Search ──────────────────────────────────────────────
  searchBar: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 10,
    borderBottomWidth: 1,
    gap: 8,
  },
  searchInput: {
    flex: 1,
    fontSize: 16,
    paddingVertical: 6,
  },

  // ── Action list ─────────────────────────────────────────
  actionList: {
    flex: 1,
  },
  actionItem: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderBottomWidth: 0.5,
    gap: 12,
  },
  actionIcon: {
    width: 36,
    height: 36,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
  },
  actionInfo: {
    flex: 1,
  },
  actionLabel: {
    fontSize: 15,
    fontWeight: '600',
  },
  actionDesc: {
    fontSize: 12,
    marginTop: 2,
  },

  // ── Empty state ─────────────────────────────────────────
  emptyState: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    paddingVertical: 40,
    gap: 8,
  },
  emptyText: {
    fontSize: 14,
  },

  // ── Footer ──────────────────────────────────────────────
  footer: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 8,
    gap: 4,
    borderTopWidth: 1,
  },
  footerText: {
    fontSize: 11,
  },

  // ── Loading ─────────────────────────────────────────────
  loadingContainer: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    paddingVertical: 60,
    gap: 12,
  },
  loadingText: {
    fontSize: 15,
    fontWeight: '500',
  },
  cancelBtn: {
    marginTop: 8,
    paddingHorizontal: 20,
    paddingVertical: 8,
    borderRadius: 8,
    borderWidth: 1,
  },
  cancelBtnText: {
    fontSize: 14,
  },

  // ── Result ──────────────────────────────────────────────
  resultHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderBottomWidth: 1,
    gap: 12,
  },
  resultTitle: {
    flex: 1,
    fontSize: 16,
    fontWeight: '600',
  },
  resultScroll: {
    flex: 1,
  },
  resultContent: {
    padding: 16,
  },
  resultText: {
    fontSize: 14,
    lineHeight: 20,
  },
  errorContainer: {
    flexDirection: 'row',
    gap: 8,
    alignItems: 'flex-start',
  },
  errorText: {
    flex: 1,
    fontSize: 14,
  },
  resultFooter: {
    flexDirection: 'row',
    paddingHorizontal: 16,
    paddingVertical: 10,
    borderTopWidth: 1,
    gap: 8,
  },
  resultActionBtn: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
    paddingHorizontal: 14,
    paddingVertical: 7,
    borderRadius: 8,
  },
  resultActionText: {
    color: '#FFF',
    fontSize: 13,
    fontWeight: '600',
  },
});
