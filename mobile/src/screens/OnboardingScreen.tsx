import React, { useState } from 'react';
import {
  View, Text, TextInput, TouchableOpacity, ScrollView, StyleSheet, ActivityIndicator,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useAppStore, getColors, PROVIDERS, type ApiFormat, type ProviderConfig } from '../store';
import { checkApi, saveSettings } from '../api/client';
import * as SecureStore from 'expo-secure-store';
import AsyncStorage from '@react-native-async-storage/async-storage';
import Icon from '../components/Icon';
import { getCurrentLocale, setLocale } from '../i18n';

const ONBOARDING_KEY = 'cfg_onboarding_done';

interface Props {
  onComplete: () => void;
}

export default function OnboardingScreen({ onComplete }: Props) {
  const store = useAppStore();
  const c = getColors(store.isDark, store.accentColor);

  const [step, setStep] = useState(0);
  const [selectedPreset, setSelectedPreset] = useState<typeof PROVIDERS[0] | null>(null);
  const [apiBase, setApiBase] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [model, setModel] = useState('');
  const [apiFormat, setApiFormat] = useState<ApiFormat>('openai');
  const [showKey, setShowKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [customMode, setCustomMode] = useState(false);
  const [lang, setLang] = useState<'en' | 'zh-CN'>(
    getCurrentLocale() === 'en' ? 'en' : 'zh-CN'
  );

  const handleSelectLang = async (value: 'en' | 'zh-CN') => {
    setLang(value);
    try {
      await setLocale(value);
    } catch (e) {
      console.warn('[Onboarding] setLocale failed:', e);
    }
  };

  const selectProvider = (preset: typeof PROVIDERS[0]) => {
    setSelectedPreset(preset);
    setApiBase(preset.base);
    setApiFormat(preset.format);
    setModel(preset.models[0] || '');
    setCustomMode(false);
    setStep(2);
  };

  const startCustom = () => {
    setSelectedPreset(null);
    setApiBase('https://');
    setApiKey('');
    setModel('');
    setApiFormat('openai');
    setCustomMode(true);
    setStep(2);
  };

  const handleTestAndSave = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const res = await checkApi({ apiBase, apiKey, apiFormat });
      if (res.ok) {
        setTestResult('✅ 连接成功');
        // Save provider
        const provider: ProviderConfig = {
          name: selectedPreset?.name ?? '自定义',
          apiBase,
          apiKey,
          model,
          apiFormat,
        };
        await saveSettings({ apiBase, apiKey, model, apiFormat });
        store.addProvider(provider);
        await AsyncStorage.setItem(ONBOARDING_KEY, 'true');
      } else {
        setTestResult(`❌ ${res.error ?? '连接失败'}`);
      }
    } catch (e: unknown) {
      setTestResult(`❌ ${e instanceof Error ? e.message : '连接失败'}`);
    } finally {
      setTesting(false);
    }
  };

  const skipOnboarding = async () => {
    // Add default provider without key so user can configure later
    if (store.providers.length === 0) {
      store.addProvider({
        name: 'OpenCode Zen',
        apiBase: 'https://opencode.ai/zen/v1',
        apiKey: '',
        model: 'deepseek-v4-flash-free',
        apiFormat: 'openai',
      });
    }
    try {
      await AsyncStorage.setItem(ONBOARDING_KEY, 'true');
      onComplete();
    } catch {
      onComplete();
    }
  };

  // ── Step 0: Welcome ──
  if (step === 0) {
    return (
      <SafeAreaView style={[styles.container, { backgroundColor: c.bg }]}>
        <View style={styles.center}>
          <Icon name="rocket" size={64} color={store.accentColor} />
          <Text style={[styles.title, { color: c.text }]}>欢迎使用 VaultPilot</Text>
          <Text style={[styles.subtitle, { color: c.textSecondary }]}>
            AI 驱动的个人知识管理助手{'\n'}三端原生 · 本地优先 · 用户自备 Key
          </Text>
          <View style={styles.langRow}>
            {[
              { value: 'zh-CN' as const, label: '中文' },
              { value: 'en' as const, label: 'English' },
            ].map(({ value, label }) => (
              <TouchableOpacity
                key={value}
                onPress={() => handleSelectLang(value)}
                style={[
                  styles.langBtn,
                  {
                    backgroundColor: lang === value ? store.accentColor : 'transparent',
                    borderColor: lang === value ? store.accentColor : c.border,
                  },
                ]}
                accessibilityLabel={label}
                accessibilityRole="radio"
                accessibilityState={{ selected: lang === value }}
              >
                <Text style={{ color: lang === value ? '#FFF' : c.textSecondary, fontWeight: '600' }}>
                  {label}
                </Text>
              </TouchableOpacity>
            ))}
          </View>
          <TouchableOpacity
            style={[styles.primaryBtn, { backgroundColor: store.accentColor }]}
            onPress={() => setStep(1)}
          >
            <Text style={styles.primaryBtnText}>开始设置</Text>
          </TouchableOpacity>
          <TouchableOpacity style={styles.skipBtn} onPress={skipOnboarding}>
            <Text style={[styles.skipText, { color: c.textSecondary }]}>跳过，稍后设置</Text>
          </TouchableOpacity>
        </View>
      </SafeAreaView>
    );
  }

  // ── Step 1: Select Provider ──
  if (step === 1) {
    return (
      <SafeAreaView style={[styles.container, { backgroundColor: c.bg }]}>
        <ScrollView contentContainerStyle={styles.scrollContent}>
          <Text style={[styles.stepTitle, { color: c.text }]}>选择 AI 提供商</Text>
          <Text style={[styles.stepDesc, { color: c.textSecondary }]}>
            选择一个你已有 API Key 的提供商
          </Text>
          {PROVIDERS.map(p => (
            <TouchableOpacity
              key={p.name}
              style={[styles.providerCard, { borderColor: c.border, backgroundColor: c.inputBg }]}
              onPress={() => selectProvider(p)}
            >
              <Text style={[styles.providerName, { color: c.text }]}>{p.name}</Text>
              <Text style={[styles.providerDetail, { color: c.textSecondary }]}>
                {p.models.slice(0, 2).join(', ')}{p.models.length > 2 ? ' ...' : ''}
              </Text>
            </TouchableOpacity>
          ))}
          <TouchableOpacity
            style={[styles.providerCard, { borderColor: store.accentColor, backgroundColor: c.inputBg }]}
            onPress={startCustom}
          >
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
              <Icon name="edit" size={17} color={store.accentColor} />
              <Text style={[styles.providerName, { color: store.accentColor }]}>自定义提供商</Text>
            </View>
            <Text style={[styles.providerDetail, { color: c.textSecondary }]}>手动填写 API 地址</Text>
          </TouchableOpacity>
          <TouchableOpacity style={styles.backBtn} onPress={() => setStep(0)}>
            <Text style={{ color: c.textSecondary }}>← 返回</Text>
          </TouchableOpacity>
        </ScrollView>
      </SafeAreaView>
    );
  }

  // ── Step 2: Enter API Key ──
  if (step === 2) {
    return (
      <SafeAreaView style={[styles.container, { backgroundColor: c.bg }]}>
        <ScrollView contentContainerStyle={styles.scrollContent}>
          <Text style={[styles.stepTitle, { color: c.text }]}>配置 API</Text>
          <Text style={[styles.stepDesc, { color: c.textSecondary }]}>
            {selectedPreset ? `为 ${selectedPreset.name} 输入 API Key` : '填写 API 信息'}
          </Text>

          {customMode && (
            <>
              <Text style={[styles.label, { color: c.textSecondary }]}>API Base URL</Text>
              <TextInput
                style={[styles.input, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
                value={apiBase}
                onChangeText={setApiBase}
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="url"
              />
            </>
          )}

          <Text style={[styles.label, { color: c.textSecondary }]}>API Key</Text>
          <View style={styles.keyRow}>
            <TextInput
              style={[styles.input, { flex: 1, backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
              value={apiKey}
              onChangeText={setApiKey}
              secureTextEntry={!showKey}
              autoCapitalize="none"
              autoCorrect={false}
              placeholder="sk-..."
              placeholderTextColor={c.textSecondary}
            />
            <TouchableOpacity onPress={() => setShowKey(!showKey)} style={styles.eyeBtn}>
              <Icon name={showKey ? 'eye-off' : 'eye'} size={18} color={c.textSecondary} />
            </TouchableOpacity>
          </View>

          {customMode && (
            <>
              <Text style={[styles.label, { color: c.textSecondary }]}>模型</Text>
              <TextInput
                style={[styles.input, { backgroundColor: c.inputBg, color: c.text, borderColor: c.border }]}
                value={model}
                onChangeText={setModel}
                autoCapitalize="none"
                placeholder="gpt-4o-mini"
                placeholderTextColor={c.textSecondary}
              />
            </>
          )}

          <View style={styles.btnRow}>
            <TouchableOpacity style={[styles.secondaryBtn, { borderColor: c.border }]} onPress={() => setStep(1)}>
              <Text style={{ color: c.textSecondary }}>← 返回</Text>
            </TouchableOpacity>
            <TouchableOpacity
              testID="onboarding-go-test-btn"
              style={[styles.primaryBtn, { backgroundColor: store.accentColor, flex: 1, opacity: apiKey ? 1 : 0.5 }]}
              onPress={() => {
                setTestResult(null);
                if (apiKey) setStep(3);
              }}
              disabled={!apiKey}
            >
              <Text style={styles.primaryBtnText}>测试连接 →</Text>
            </TouchableOpacity>
          </View>
          <TouchableOpacity style={[styles.skipBtn, { marginTop: 16 }]} onPress={skipOnboarding}>
            <Text style={[styles.skipText, { color: c.textSecondary }]}>跳过</Text>
          </TouchableOpacity>
        </ScrollView>
      </SafeAreaView>
    );
  }

  // ── Step 3: Test Connection ──
  return (
    <SafeAreaView style={[styles.container, { backgroundColor: c.bg }]}>
      <View style={styles.center}>
        <Text style={[styles.stepTitle, { color: c.text }]}>测试连接</Text>
        <Text style={[styles.stepDesc, { color: c.textSecondary }]}>
          正在验证 {selectedPreset?.name ?? 'API'} 连接...
        </Text>

        <View style={[styles.testCard, { borderColor: c.border, backgroundColor: c.inputBg }]}>
          <Text style={[styles.testLabel, { color: c.textSecondary }]}>API 地址</Text>
          <Text style={[styles.testValue, { color: c.text }]} numberOfLines={1}>{apiBase}</Text>
          <Text style={[styles.testLabel, { color: c.textSecondary, marginTop: 8 }]}>模型</Text>
          <Text style={[styles.testValue, { color: c.text }]}>{model}</Text>
        </View>

        {testResult && (
          <Text style={[styles.testResult, { color: testResult.startsWith('✅') ? '#10B981' : '#EF4444' }]}>
            {testResult}
          </Text>
        )}

        <TouchableOpacity
          testID="onboarding-test-btn"
          style={[styles.primaryBtn, { backgroundColor: testing ? c.textSecondary : store.accentColor }]}
          onPress={testResult?.startsWith('✅') ? onComplete : handleTestAndSave}
          disabled={testing}
        >
          {testing ? (
            <ActivityIndicator color="#FFF" />
          ) : (
            <Text style={styles.primaryBtnText}>{testResult?.startsWith('✅') ? '完成' : '测试连接'}</Text>
          )}
        </TouchableOpacity>

        <TouchableOpacity testID="onboarding-modify-btn" style={[styles.skipBtn, { marginTop: 16 }]} onPress={() => { setTestResult(null); setStep(2); }}>
          <Text style={[styles.skipText, { color: c.textSecondary }]}>← 修改配置</Text>
        </TouchableOpacity>
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  center: { flex: 1, justifyContent: 'center', alignItems: 'center', padding: 24 },
  scrollContent: { padding: 24, paddingTop: 48 },
  logo: { fontSize: 64, marginBottom: 16 },
  title: { fontSize: 28, fontWeight: '700', marginBottom: 8 },
  subtitle: { fontSize: 15, textAlign: 'center', lineHeight: 22, marginBottom: 32 },
  // Language picker (welcome step) — mirrors SettingsScreen's langBtn (#3649)
  langRow: { flexDirection: 'row', gap: 12, marginBottom: 24 },
  langBtn: { flex: 1, paddingVertical: 12, borderRadius: 10, borderWidth: 1, alignItems: 'center' },
  stepTitle: { fontSize: 24, fontWeight: '700', marginBottom: 8 },
  stepDesc: { fontSize: 15, marginBottom: 24, lineHeight: 22 },
  primaryBtn: {
    paddingHorizontal: 32, paddingVertical: 14, borderRadius: 12,
    alignItems: 'center', minWidth: 200,
  },
  primaryBtnText: { color: '#FFF', fontSize: 17, fontWeight: '600' },
  skipBtn: { marginTop: 16, padding: 8 },
  skipText: { fontSize: 14 },
  providerCard: {
    borderWidth: 1, borderRadius: 12, padding: 16, marginBottom: 10,
  },
  providerName: { fontSize: 17, fontWeight: '600' },
  providerDetail: { fontSize: 13, marginTop: 4 },
  label: { fontSize: 13, fontWeight: '500', marginBottom: 6 },
  input: {
    borderWidth: 1, borderRadius: 10, paddingHorizontal: 14, paddingVertical: 10,
    fontSize: 15, marginBottom: 12,
  },
  keyRow: { flexDirection: 'row', alignItems: 'center' },
  eyeBtn: { paddingHorizontal: 12, paddingVertical: 10 },
  btnRow: { flexDirection: 'row', gap: 12, marginTop: 8 },
  secondaryBtn: {
    paddingHorizontal: 20, paddingVertical: 14, borderRadius: 12,
    borderWidth: 1, alignItems: 'center',
  },
  backBtn: { marginTop: 16, padding: 8 },
  testCard: { borderWidth: 1, borderRadius: 12, padding: 16, marginBottom: 20, alignSelf: 'stretch' },
  testLabel: { fontSize: 12, fontWeight: '500' },
  testValue: { fontSize: 15, marginTop: 2 },
  testResult: { fontSize: 16, fontWeight: '600', marginBottom: 16 },
});
