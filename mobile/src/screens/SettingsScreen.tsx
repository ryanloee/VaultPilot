import React, { useState, useEffect } from 'react';
import {
  View, Text, TextInput, TouchableOpacity, ScrollView, StyleSheet, Alert, ActivityIndicator, Modal,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useAppStore, getColors, ACCENT_COLORS, PROVIDERS, isValidThemeMode, ApiFormat, ProviderConfig } from '../store';
import { checkApi, getSettings, saveSettings } from '../api/client';
import AsyncStorage from '@react-native-async-storage/async-storage';
import appJson from '../../app.json';

const THEME_KEY = 'cfg_theme_mode';
const ACCENT_KEY = 'cfg_accent_color';

export default function SettingsScreen() {
  const store = useAppStore();
  const c = getColors(store.isDark, store.accentColor);
  const [showKey, setShowKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [showAddModal, setShowAddModal] = useState(false);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);

  // Active provider values for editing
  const activeIdx = store.providers.length > 0
    ? Math.min(store.activeProviderIndex, store.providers.length - 1)
    : -1;
  const active = activeIdx >= 0 ? store.providers[activeIdx] : null;

  const [apiBase, setApiBase] = useState(active?.apiBase ?? store.apiBase);
  const [apiKey, setApiKey] = useState(active?.apiKey ?? store.apiKey);
  const [model, setModel] = useState(active?.model ?? store.model);
  const [apiFormat, setApiFormat] = useState<ApiFormat>(active?.apiFormat ?? store.apiFormat);
  const [providerName, setProviderName] = useState(active?.name ?? '');

  // Sync local state when active provider changes
  useEffect(() => {
    if (active) {
      setApiBase(active.apiBase);
      setApiKey(active.apiKey);
      setModel(active.model);
      setApiFormat(active.apiFormat);
      setProviderName(active.name);
    }
  }, [activeIdx]);

  // Load saved settings on mount
  useEffect(() => {
    (async () => {
      try {
        const [api, themeMode, accentColor] = await Promise.all([
          getSettings(),
          AsyncStorage.getItem(THEME_KEY),
          AsyncStorage.getItem(ACCENT_KEY),
        ]);
        // If no providers yet, migrate from legacy flat fields
        if (store.providers.length === 0 && api.apiBase) {
          const migrated: ProviderConfig = {
            name: '默认',
            apiBase: api.apiBase,
            apiKey: api.apiKey || '',
            model: api.model || 'deepseek-v4-flash-free',
            apiFormat: api.apiFormat || 'openai',
          };
          store.addProvider(migrated);
        } else if (store.providers.length === 0) {
          // Add default OpenCode Zen
          store.addProvider({
            name: 'OpenCode Zen',
            apiBase: 'https://opencode.ai/zen/v1',
            apiKey: '',
            model: 'deepseek-v4-flash-free',
            apiFormat: 'openai',
          });
        }
        if (themeMode && isValidThemeMode(themeMode)) store.setThemeMode(themeMode);
        if (accentColor) store.setAccentColor(accentColor);
      } catch (e) {
        console.warn('[Settings] Failed to load:', e);
      }
    })();
  }, []);

  const saveActiveProvider = async () => {
    if (activeIdx < 0) return;
    store.updateProvider(activeIdx, { name: providerName, apiBase, apiKey, model, apiFormat });
    try {
      await saveSettings({ apiBase, apiKey, model, apiFormat });
      Alert.alert('已保存', '设置已保存');
    } catch (e: unknown) {
      Alert.alert('保存失败', e instanceof Error ? e.message : '请重试');
    }
  };

  const testConnection = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const res = await checkApi({ apiBase, apiKey, apiFormat });
      setTestResult(res.ok ? '✅ 连接成功' : `❌ ${res.error}`);
    } catch (e: unknown) {
      setTestResult(`❌ ${e instanceof Error ? e.message : '连接失败'}`);
    } finally {
      setTesting(false);
    }
  };

  const addFromPreset = (preset: typeof PROVIDERS[0]) => {
    store.addProvider({
      name: preset.name,
      apiBase: preset.base,
      apiKey: '',
      model: preset.models[0] || '',
      apiFormat: preset.format,
    });
    setShowAddModal(false);
  };

  const addCustom = () => {
    store.addProvider({
      name: '自定义',
      apiBase: 'https://',
      apiKey: '',
      model: '',
      apiFormat: 'openai',
    });
    setShowAddModal(false);
  };

  const deleteProvider = (index: number) => {
    if (store.providers.length <= 1) {
      Alert.alert('无法删除', '至少保留一个提供商');
      return;
    }
    Alert.alert('删除', `确定删除「${store.providers[index].name}」？`, [
      { text: '取消', style: 'cancel' },
      { text: '删除', style: 'destructive', onPress: () => store.removeProvider(index) },
    ]);
  };

  const selectPresetUrl = (preset: typeof PROVIDERS[0]) => {
    setApiBase(preset.base);
    setApiFormat(preset.format);
    if (preset.models.length && !model) setModel(preset.models[0]);
  };

  return (
    <SafeAreaView style={[s.container, { backgroundColor: c.bg }]}>
    <ScrollView style={{ flex: 1 }} contentContainerStyle={{ padding: 16 }}>

      {/* ── Provider List ── */}
      <View style={s.sectionHeader}>
        <Text style={[s.sectionTitle, { color: c.text }]}>API 提供商</Text>
        <TouchableOpacity onPress={() => setShowAddModal(true)} style={[s.addBtn, { borderColor: store.accentColor }]}>
          <Text style={{ color: store.accentColor, fontWeight: '600' }}>+ 添加</Text>
        </TouchableOpacity>
      </View>

      {store.providers.map((p, i) => (
        <TouchableOpacity
          key={i}
          style={[s.providerCard, {
            backgroundColor: i === activeIdx ? store.accentColor + '15' : c.inputBg,
            borderColor: i === activeIdx ? store.accentColor : c.border,
          }]}
          onPress={() => store.setActiveProvider(i)}
        >
          <View style={{ flex: 1 }}>
            <View style={s.providerCardHeader}>
              <Text style={[s.providerCardName, { color: c.text }]}>
                {i === activeIdx ? '● ' : ''}{p.name}
              </Text>
              {store.providers.length > 1 && (
                <TouchableOpacity onPress={() => deleteProvider(i)} hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}>
                  <Text style={{ color: '#EF4444', fontSize: 14 }}>删除</Text>
                </TouchableOpacity>
              )}
            </View>
            <Text style={[s.providerCardDetail, { color: c.textSecondary }]} numberOfLines={1}>
              {p.model} · {p.apiFormat.toUpperCase()}
            </Text>
          </View>
        </TouchableOpacity>
      ))}

      {/* ── Edit Active Provider ── */}
      {active && (
        <View style={[s.editSection, { borderColor: c.border }]}>
          <Text style={[s.label, { color: c.textSecondary }]}>名称</Text>
          <TextInput
            style={[s.input, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
            value={providerName}
            onChangeText={setProviderName}
            autoCapitalize="none"
          />

          <Text style={[s.label, { color: c.textSecondary }]}>快速选择</Text>
          <ScrollView horizontal showsHorizontalScrollIndicator={false} style={{ marginBottom: 12 }}>
            {PROVIDERS.map(p => (
              <TouchableOpacity
                key={p.name}
                style={[s.presetBtn, {
                  borderColor: (apiBase === p.base) ? store.accentColor : c.border,
                  backgroundColor: (apiBase === p.base) ? store.accentColor + '20' : 'transparent',
                }]}
                onPress={() => selectPresetUrl(p)}
              >
                <Text style={{ color: (apiBase === p.base) ? store.accentColor : c.text, fontWeight: '500', fontSize: 13 }}>
                  {p.name}
                </Text>
              </TouchableOpacity>
            ))}
          </ScrollView>

          <Text style={[s.label, { color: c.textSecondary }]}>API Base URL</Text>
          <TextInput
            style={[s.input, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
            value={apiBase}
            onChangeText={setApiBase}
            autoCapitalize="none"
            autoCorrect={false}
          />

          <Text style={[s.label, { color: c.textSecondary }]}>API Key</Text>
          <View style={s.keyRow}>
            <TextInput
              style={[s.input, { flex: 1, backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
              value={apiKey}
              onChangeText={setApiKey}
              secureTextEntry={!showKey}
              autoCapitalize="none"
              autoCorrect={false}
            />
            <TouchableOpacity onPress={() => setShowKey(!showKey)} style={s.eyeBtn}>
              <Text style={{ color: c.textSecondary, fontSize: 18 }}>{showKey ? '🙈' : '👁'}</Text>
            </TouchableOpacity>
          </View>

          <Text style={[s.label, { color: c.textSecondary }]}>格式</Text>
          <View style={s.formatRow}>
            {(['openai', 'anthropic'] as const).map(fmt => (
              <TouchableOpacity
                key={fmt}
                style={[s.formatBtn, {
                  borderColor: apiFormat === fmt ? store.accentColor : c.border,
                  backgroundColor: apiFormat === fmt ? store.accentColor + '20' : 'transparent',
                }]}
                onPress={() => setApiFormat(fmt)}
              >
                <Text style={{ color: apiFormat === fmt ? store.accentColor : c.text }}>
                  {fmt === 'openai' ? 'OpenAI' : 'Anthropic'}
                </Text>
              </TouchableOpacity>
            ))}
          </View>

          <Text style={[s.label, { color: c.textSecondary }]}>模型</Text>
          <TextInput
            style={[s.input, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
            value={model}
            onChangeText={setModel}
            autoCapitalize="none"
            autoCorrect={false}
          />

          {/* Test & Save */}
          <View style={s.btnRow}>
            <TouchableOpacity style={[s.btn, { backgroundColor: c.inputBg, borderColor: c.border }]} onPress={testConnection} disabled={testing}>
              {testing ? <ActivityIndicator color={store.accentColor} /> : <Text style={[s.btnText, { color: c.text }]}>测试连接</Text>}
            </TouchableOpacity>
            <TouchableOpacity style={[s.btn, { backgroundColor: store.accentColor }]} onPress={saveActiveProvider}>
              <Text style={[s.btnText, { color: '#FFF' }]}>保存</Text>
            </TouchableOpacity>
          </View>
          {testResult && <Text style={[s.testResult, { color: testResult.includes('✅') ? '#10B981' : '#EF4444' }]}>{testResult}</Text>}
        </View>
      )}

      {/* ── Theme Section ── */}
      <Text style={[s.sectionTitle, { color: c.text, marginTop: 24 }]}>外观</Text>

      <Text style={[s.label, { color: c.textSecondary }]}>主题</Text>
      <View style={s.themeRow}>
        {(['light', 'dark', 'system'] as const).map(mode => (
          <TouchableOpacity
            key={mode}
            style={[s.themeBtn, {
              borderColor: store.themeMode === mode ? store.accentColor : c.border,
              backgroundColor: store.themeMode === mode ? store.accentColor + '20' : 'transparent',
            }]}
            onPress={() => {
              store.setThemeMode(mode);
              if (mode !== 'system') store.setIsDark(mode === 'dark');
              AsyncStorage.setItem(THEME_KEY, mode);
            }}
          >
            <Text style={{ color: store.themeMode === mode ? store.accentColor : c.text }}>
              {mode === 'light' ? '☀️ 亮色' : mode === 'dark' ? '🌙 暗色' : '🔄 跟随系统'}
            </Text>
          </TouchableOpacity>
        ))}
      </View>

      <Text style={[s.label, { color: c.textSecondary }]}>主色调</Text>
      <View style={s.colorRow}>
        {ACCENT_COLORS.map(ac => (
          <TouchableOpacity
            key={ac.value}
            style={[s.colorDot, {
              backgroundColor: ac.value,
              borderWidth: store.accentColor === ac.value ? 3 : 0,
              borderColor: '#FFF',
            }]}
            onPress={() => {
              store.setAccentColor(ac.value);
              AsyncStorage.setItem(ACCENT_KEY, ac.value);
            }}
          />
        ))}
      </View>

      <Text style={[s.version, { color: c.textSecondary }]}>VaultPilot Mobile v{appJson.expo.version}</Text>
    </ScrollView>

    {/* ── Add Provider Modal ── */}
    <Modal visible={showAddModal} transparent animationType="slide">
      <View style={s.modalOverlay}>
        <View style={[s.modalContent, { backgroundColor: c.card }]}>
          <Text style={[s.sectionTitle, { color: c.text }]}>添加提供商</Text>
          <ScrollView>
            {PROVIDERS.map(p => (
              <TouchableOpacity
                key={p.name}
                style={[s.modalItem, { borderColor: c.border }]}
                onPress={() => addFromPreset(p)}
              >
                <Text style={[s.modalItemName, { color: c.text }]}>{p.name}</Text>
                <Text style={[s.modalItemDetail, { color: c.textSecondary }]}>
                  {p.models.slice(0, 2).join(', ')}{p.models.length > 2 ? '...' : ''}
                </Text>
              </TouchableOpacity>
            ))}
            <TouchableOpacity
              style={[s.modalItem, { borderColor: c.border }]}
              onPress={addCustom}
            >
              <Text style={[s.modalItemName, { color: store.accentColor }]}>✏️ 自定义提供商</Text>
            </TouchableOpacity>
          </ScrollView>
          <TouchableOpacity
            style={[s.modalClose, { borderColor: c.border }]}
            onPress={() => setShowAddModal(false)}
          >
            <Text style={{ color: c.textSecondary }}>取消</Text>
          </TouchableOpacity>
        </View>
      </View>
    </Modal>
    </SafeAreaView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1 },
  sectionHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 12 },
  sectionTitle: { fontSize: 20, fontWeight: '700' },
  addBtn: { paddingHorizontal: 14, paddingVertical: 6, borderRadius: 16, borderWidth: 1 },
  label: { fontSize: 13, fontWeight: '500', marginBottom: 6 },
  input: {
    borderWidth: 1, borderRadius: 10, paddingHorizontal: 14, paddingVertical: 10,
    fontSize: 15, marginBottom: 12,
  },
  keyRow: { flexDirection: 'row', alignItems: 'center' },
  eyeBtn: { paddingHorizontal: 12, paddingVertical: 10 },
  btnRow: { flexDirection: 'row', gap: 12, marginTop: 4 },
  btn: {
    flex: 1, paddingVertical: 12, borderRadius: 10, alignItems: 'center',
    borderWidth: 1, borderColor: 'transparent',
  },
  btnText: { fontSize: 16, fontWeight: '600' },
  testResult: { textAlign: 'center', marginTop: 10, fontSize: 14 },
  providerCard: {
    borderWidth: 1, borderRadius: 12, padding: 14, marginBottom: 8,
  },
  providerCardHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center' },
  providerCardName: { fontSize: 16, fontWeight: '600' },
  providerCardDetail: { fontSize: 13, marginTop: 4 },
  editSection: { marginTop: 16, paddingTop: 16, borderTopWidth: 1 },
  presetBtn: {
    paddingHorizontal: 14, paddingVertical: 7, borderRadius: 18,
    borderWidth: 1, marginRight: 8,
  },
  formatRow: { flexDirection: 'row', gap: 8, marginBottom: 12 },
  formatBtn: { flex: 1, paddingVertical: 10, borderRadius: 10, borderWidth: 1, alignItems: 'center' },
  themeRow: { flexDirection: 'row', gap: 8, marginBottom: 16 },
  themeBtn: { flex: 1, paddingVertical: 10, borderRadius: 10, borderWidth: 1, alignItems: 'center' },
  colorRow: { flexDirection: 'row', gap: 12, marginBottom: 24 },
  colorDot: { width: 36, height: 36, borderRadius: 18 },
  version: { textAlign: 'center', fontSize: 12, marginTop: 20, marginBottom: 40 },
  modalOverlay: { flex: 1, justifyContent: 'flex-end', backgroundColor: 'rgba(0,0,0,0.5)' },
  modalContent: { borderTopLeftRadius: 20, borderTopRightRadius: 20, padding: 20, maxHeight: '70%' },
  modalItem: { borderBottomWidth: 1, paddingVertical: 14 },
  modalItemName: { fontSize: 16, fontWeight: '600' },
  modalItemDetail: { fontSize: 13, marginTop: 2 },
  modalClose: { paddingVertical: 14, alignItems: 'center', marginTop: 8, borderTopWidth: 1 },
});
