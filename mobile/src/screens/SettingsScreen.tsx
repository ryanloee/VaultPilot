import React, { useState, useEffect } from 'react';
import { View, Text, TextInput, TouchableOpacity, ScrollView, StyleSheet, Alert } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useAppStore, getColors, isValidThemeMode, ApiFormat, ProviderConfig, PROVIDERS } from '../store';
import { checkApi, getSettings, saveSettings } from '../api/client';
import * as SecureStore from 'expo-secure-store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import appJson from '../../app.json';
import { checkForUpdate, type UpdateInfo } from '../utils/updateChecker';
import { exportSettings, importSettings } from '../utils/settingsSync';
import * as Clipboard from 'expo-clipboard';
import { getServerConfig, setServerConfig, syncNotesFromServer, getLastSyncTime } from '../services/sync';
import { ProviderList, ProviderEditor, ThemeSection, UpdateModal, AddProviderModal } from '../components/settings';

const THEME_KEY = 'cfg_theme_mode';
const ACCENT_KEY = 'cfg_accent_color';
const SKIP_UPDATE_KEY = 'cfg_skip_update_version';

export default function SettingsScreen() {
  const store = useAppStore();
  const c = getColors(store.isDark, store.accentColor);
  const [showKey, setShowKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [showAddModal, setShowAddModal] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [showUpdateModal, setShowUpdateModal] = useState(false);
  const [serverUrl, setServerUrl] = useState('');
  const [serverToken, setServerToken] = useState('');
  const [syncing, setSyncing] = useState(false);
  const [lastSync, setLastSync] = useState<string | null>(null);

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

  // Sync local state when active provider changes or key is restored from SecureStore
  useEffect(() => {
    if (active) {
      setApiBase(active.apiBase);
      if (active.apiKey) {
        setApiKey(active.apiKey);
      } else {
        SecureStore.getItemAsync('vaultpilot_provider_keys').then(raw => {
          if (!raw) return;
          try {
            const keys: string[] = JSON.parse(raw);
            const key = keys[activeIdx] ?? '';
            if (key) setApiKey(key);
          } catch (e) { console.warn('[Settings] Failed to restore provider key:', e); }
        });
      }
      setModel(active.model);
      setApiFormat(active.apiFormat);
      setProviderName(active.name);
    }
  }, [activeIdx, active?.apiKey]);

  // Load saved settings on mount
  useEffect(() => {
    (async () => {
      try {
        const [api, themeMode, accentColor] = await Promise.all([
          getSettings(),
          AsyncStorage.getItem(THEME_KEY),
          AsyncStorage.getItem(ACCENT_KEY),
        ]);
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
      try {
        const cfg = await getServerConfig();
        setServerUrl(cfg.url);
        setServerToken(cfg.token);
        const ls = await getLastSyncTime();
        setLastSync(ls);
      } catch (e) { console.warn('[Settings] Failed to load server config:', e); }

      // Auto-check for updates
      try {
        const skipVersion = await AsyncStorage.getItem(SKIP_UPDATE_KEY);
        const info = await checkForUpdate(appJson.expo.version);
        if (info && info.latestVersion !== skipVersion) {
          setUpdateInfo(info);
          setShowUpdateModal(true);
        }
      } catch (e) { console.warn('[Settings] Auto-update check failed:', e); }
    })();
  }, []);

  const handleCheckUpdate = async () => {
    setCheckingUpdate(true);
    try {
      const info = await checkForUpdate(appJson.expo.version);
      if (info) {
        setUpdateInfo(info);
        setShowUpdateModal(true);
      } else {
        Alert.alert('已是最新', `当前版本 v${appJson.expo.version} 已是最新`);
      }
    } catch {
      Alert.alert('检查失败', '无法连接到更新服务器');
    } finally {
      setCheckingUpdate(false);
    }
  };

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

  const addFromPreset = (preset: typeof PROVIDERS[number]) => {
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

  const selectPresetUrl = (preset: typeof PROVIDERS[number]) => {
    setApiBase(preset.base);
    setApiFormat(preset.format);
    if (preset.models.length && !model) setModel(preset.models[0]);
  };

  const handleThemeChange = (mode: 'light' | 'dark' | 'system') => {
    store.setThemeMode(mode);
    if (mode !== 'system') store.setIsDark(mode === 'dark');
    AsyncStorage.setItem(THEME_KEY, mode);
  };

  const handleAccentChange = (color: string) => {
    store.setAccentColor(color);
    AsyncStorage.setItem(ACCENT_KEY, color);
  };

  const handleSkipUpdate = async () => {
    if (updateInfo) {
      await AsyncStorage.setItem(SKIP_UPDATE_KEY, updateInfo.latestVersion);
    }
    setShowUpdateModal(false);
  };

  return (
    <SafeAreaView style={[styles.container, { backgroundColor: c.bg }]}>
      <ScrollView style={{ flex: 1 }} contentContainerStyle={{ padding: 16 }}>
        <ProviderList
          providers={store.providers}
          activeIndex={activeIdx}
          accentColor={store.accentColor}
          textColor={c.text}
          textColorSecondary={c.textSecondary}
          inputBgColor={c.inputBg}
          borderColor={c.border}
          onSelect={store.setActiveProvider}
          onDelete={deleteProvider}
          onAdd={() => setShowAddModal(true)}
        />

        {active && (
          <ProviderEditor
            provider={active}
            providerName={providerName}
            apiBase={apiBase}
            apiKey={apiKey}
            model={model}
            apiFormat={apiFormat}
            showKey={showKey}
            testing={testing}
            testResult={testResult}
            accentColor={store.accentColor}
            textColor={c.text}
            textColorSecondary={c.textSecondary}
            inputBgColor={c.inputBg}
            borderColor={c.border}
            onProviderNameChange={setProviderName}
            onApiBaseChange={setApiBase}
            onApiKeyChange={setApiKey}
            onModelChange={setModel}
            onApiFormatChange={setApiFormat}
            onShowKeyToggle={() => setShowKey(!showKey)}
            onTestConnection={testConnection}
            onSave={saveActiveProvider}
            onSelectPreset={selectPresetUrl}
          />
        )}

        <ThemeSection
          themeMode={store.themeMode}
          accentColor={store.accentColor}
          textColor={c.text}
          textColorSecondary={c.textSecondary}
          borderColor={c.border}
          onThemeChange={handleThemeChange}
          onAccentChange={handleAccentChange}
        />

        {/* ── Backend Server Section ── */}
        <Text style={[styles.sectionTitle, { color: c.text, marginTop: 24 }]}>后端服务器</Text>
        <Text style={[styles.label, { color: c.textSecondary }]}>连接电脑端同步知识库笔记</Text>
        <TextInput
          style={[styles.input, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
          value={serverUrl}
          onChangeText={setServerUrl}
          placeholder="http://192.168.1.100:3000"
          placeholderTextColor={c.textSecondary}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="url"
        />
        <TextInput
          style={[styles.input, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
          value={serverToken}
          onChangeText={setServerToken}
          placeholder="Token（可选）"
          placeholderTextColor={c.textSecondary}
          autoCapitalize="none"
          autoCorrect={false}
        />
        <View style={{ flexDirection: 'row', gap: 12, marginTop: 8 }}>
          <TouchableOpacity
            style={[styles.saveBtn, { backgroundColor: store.accentColor, flex: 1 }]}
            onPress={async () => {
              await setServerConfig(serverUrl, serverToken);
              Alert.alert('已保存', '后端服务器配置已保存');
            }}
          >
            <Text style={styles.saveBtnText}>保存</Text>
          </TouchableOpacity>
          <TouchableOpacity
            style={[styles.saveBtn, { backgroundColor: '#10B981', flex: 1 }]}
            onPress={async () => {
              setSyncing(true);
              try {
                await setServerConfig(serverUrl, serverToken);
                const result = await syncNotesFromServer();
                const last = await getLastSyncTime();
                setLastSync(last);
                Alert.alert('同步完成', `新增 ${result.imported} 更新 ${result.updated} 跳过 ${result.skipped} 耗时 ${(result.duration_ms / 1000).toFixed(1)}s`);
              } catch (e) {
                Alert.alert('同步失败', e instanceof Error ? e.message : String(e));
              } finally {
                setSyncing(false);
              }
            }}
            disabled={syncing}
          >
            <Text style={styles.saveBtnText}>{syncing ? '同步中...' : '立即同步'}</Text>
          </TouchableOpacity>
        </View>
        {lastSync && (
          <Text style={[styles.label, { color: c.textSecondary, marginTop: 4 }]}>
            上次同步: {new Date(lastSync).toLocaleString()}
          </Text>
        )}

        {/* ── Settings Export/Import (#1222) ── */}
        <View style={{ marginTop: 20, gap: 8 }}>
          <Text style={[styles.sectionTitle, { color: c.text, marginBottom: 8 }]}>设置同步</Text>
          <TouchableOpacity
            style={[styles.btn, { backgroundColor: store.accentColor + '15', borderColor: store.accentColor }]}
            onPress={async () => {
              try {
                const json = await exportSettings(false);
                await Clipboard.setStringAsync(json);
                Alert.alert('已复制', '设置已复制到剪贴板（不含 API Key）\n在其他设备粘贴导入即可');
              } catch (e) {
                Alert.alert('导出失败', String(e));
              }
            }}
          >
            <Text style={[styles.btnText, { color: store.accentColor }]}>📤 导出设置</Text>
          </TouchableOpacity>
          <TouchableOpacity
            style={[styles.btn, { borderColor: c.border }]}
            onPress={async () => {
              try {
                const json = await Clipboard.getStringAsync();
                if (!json.trim()) { Alert.alert('剪贴板为空'); return; }
                const result = await importSettings(json);
                Alert.alert('导入成功', `已导入 ${result.providersImported} 个 Provider 配置\n请重启应用生效`);
              } catch (e) {
                Alert.alert('导入失败', '剪贴板内容不是有效的设置 JSON');
              }
            }}
          >
            <Text style={[styles.btnText, { color: c.text }]}>📥 从剪贴板导入</Text>
          </TouchableOpacity>
        </View>

        <View style={styles.versionRow}>
          <Text style={[styles.version, { color: c.textSecondary }]}>VaultPilot Mobile v{appJson.expo.version}</Text>
          <TouchableOpacity onPress={handleCheckUpdate} disabled={checkingUpdate}>
            <Text style={{ color: store.accentColor, fontSize: 12 }}>
              {checkingUpdate ? '检查中...' : '检查更新'}
            </Text>
          </TouchableOpacity>
        </View>
      </ScrollView>

      <UpdateModal
        visible={showUpdateModal}
        updateInfo={updateInfo}
        accentColor={store.accentColor}
        textColor={c.text}
        textColorSecondary={c.textSecondary}
        borderColor={c.border}
        cardBgColor={c.card}
        onClose={() => setShowUpdateModal(false)}
        onSkip={handleSkipUpdate}
      />

      <AddProviderModal
        visible={showAddModal}
        accentColor={store.accentColor}
        textColor={c.text}
        textColorSecondary={c.textSecondary}
        borderColor={c.border}
        cardBgColor={c.card}
        onClose={() => setShowAddModal(false)}
        onSelectPreset={addFromPreset}
        onAddCustom={addCustom}
      />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  sectionTitle: { fontSize: 20, fontWeight: '700' },
  label: { fontSize: 13, fontWeight: '500', marginBottom: 6 },
  input: {
    borderWidth: 1, borderRadius: 10, paddingHorizontal: 14, paddingVertical: 10,
    fontSize: 15, marginBottom: 12,
  },
  btn: {
    flex: 1, paddingVertical: 12, borderRadius: 10, alignItems: 'center',
    borderWidth: 1, borderColor: 'transparent',
  },
  btnText: { fontSize: 16, fontWeight: '600' },
  version: { textAlign: 'center', fontSize: 12, marginBottom: 0 },
  versionRow: { flexDirection: 'row', justifyContent: 'center', alignItems: 'center', gap: 12, marginTop: 20, marginBottom: 40 },
  saveBtn: { paddingVertical: 12, borderRadius: 8, alignItems: 'center' },
  saveBtnText: { color: '#FFF', fontWeight: '600', fontSize: 14 },
});