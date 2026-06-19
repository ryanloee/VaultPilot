import React, { useState, useEffect } from 'react';
import {
  View, Text, TextInput, TouchableOpacity, ScrollView, StyleSheet, Alert, ActivityIndicator,
} from 'react-native';
import { useAppStore, getColors, ACCENT_COLORS, PROVIDERS } from '../store';
import { checkApi, getSettings, saveSettings } from '../api/client';
import AsyncStorage from '@react-native-async-storage/async-storage';

// Theme keys — stored separately from API settings (cfg_* keys)
const THEME_KEY = 'cfg_theme_mode';
const ACCENT_KEY = 'cfg_accent_color';

export default function SettingsScreen() {
  const store = useAppStore();
  const c = getColors(store.isDark, store.accentColor);
  const [apiBase, setApiBase] = useState(store.apiBase);
  const [apiKey, setApiKey] = useState(store.apiKey);
  const [model, setModel] = useState(store.model);
  const [showKey, setShowKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);

  // Load saved settings from the same source the API client uses
  useEffect(() => {
    (async () => {
      try {
        const [api, themeMode, accentColor] = await Promise.all([
          getSettings(),
          AsyncStorage.getItem(THEME_KEY),
          AsyncStorage.getItem(ACCENT_KEY),
        ]);
        if (api.apiBase) { setApiBase(api.apiBase); store.setApiSettings({ apiBase: api.apiBase }); }
        if (api.apiKey) { setApiKey(api.apiKey); store.setApiSettings({ apiKey: api.apiKey }); }
        if (api.model) { setModel(api.model); store.setApiSettings({ model: api.model }); }
        if (themeMode) store.setThemeMode(themeMode as any);
        if (accentColor) store.setAccentColor(accentColor);
      } catch (e) {
        console.warn('[Settings] Failed to load settings, using defaults:', e);
      }
    })();
  }, []);

  const saveAll = async () => {
    try {
      await Promise.all([
        saveSettings({ apiBase, apiKey, model }),
        AsyncStorage.setItem(THEME_KEY, store.themeMode),
        AsyncStorage.setItem(ACCENT_KEY, store.accentColor),
      ]);
      // Update store only after persistence succeeds — avoid state inconsistency on failure
      store.setApiSettings({ apiBase, apiKey, model });
      Alert.alert('已保存', '设置已保存');
    } catch (e: any) {
      Alert.alert('保存失败', e.message || '请重试');
    }
  };

  const testConnection = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const res = await checkApi({ apiBase, apiKey });
      setTestResult(res.ok ? '✅ 连接成功' : `❌ ${res.error}`);
    } catch (e: any) {
      setTestResult(`❌ ${e.message || '连接失败'}`);
    } finally {
      setTesting(false);
    }
  };

  const selectProvider = (p: typeof PROVIDERS[0]) => {
    if (p.name === '自定义') return;
    setApiBase(p.base);
    if (p.models.length) setModel(p.models[0]);
  };

  return (
    <ScrollView style={[s.container, { backgroundColor: c.bg }]} contentContainerStyle={{ padding: 16 }}>
      {/* API Section */}
      <Text style={[s.sectionTitle, { color: c.text }]}>API 配置</Text>

      <Text style={[s.label, { color: c.textSecondary }]}>提供商</Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} style={{ marginBottom: 12 }}>
        {PROVIDERS.map(p => (
          <TouchableOpacity
            key={p.name}
            style={[s.providerBtn, {
              borderColor: apiBase === p.base ? store.accentColor : c.border,
              backgroundColor: apiBase === p.base ? store.accentColor + '20' : 'transparent',
            }]}
            onPress={() => selectProvider(p)}
          >
            <Text style={{ color: apiBase === p.base ? store.accentColor : c.text, fontWeight: '500' }}>
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
        <TouchableOpacity style={[s.btn, { backgroundColor: store.accentColor }]} onPress={saveAll}>
          <Text style={[s.btnText, { color: '#FFF' }]}>保存</Text>
        </TouchableOpacity>
      </View>
      {testResult && <Text style={[s.testResult, { color: testResult.includes('✅') ? '#10B981' : '#EF4444' }]}>{testResult}</Text>}

      {/* Theme Section */}
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

      <Text style={[s.version, { color: c.textSecondary }]}>VaultPilot Mobile v0.1.0</Text>
    </ScrollView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1 },
  sectionTitle: { fontSize: 20, fontWeight: '700', marginBottom: 16 },
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
  providerBtn: {
    paddingHorizontal: 16, paddingVertical: 8, borderRadius: 20,
    borderWidth: 1, marginRight: 8,
  },
  themeRow: { flexDirection: 'row', gap: 8, marginBottom: 16 },
  themeBtn: { flex: 1, paddingVertical: 10, borderRadius: 10, borderWidth: 1, alignItems: 'center' },
  colorRow: { flexDirection: 'row', gap: 12, marginBottom: 24 },
  colorDot: { width: 36, height: 36, borderRadius: 18 },
  version: { textAlign: 'center', fontSize: 12, marginTop: 20, marginBottom: 40 },
});
