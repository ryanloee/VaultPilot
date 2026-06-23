import React from 'react';
import { View, Text, TextInput, TouchableOpacity, ScrollView, ActivityIndicator, StyleSheet } from 'react-native';
import type { ApiFormat, ProviderConfig } from '../../store';
import { PROVIDERS } from '../../store';
import Icon from '../../components/Icon';

interface ProviderEditorProps {
  provider: ProviderConfig;
  providerName: string;
  apiBase: string;
  apiKey: string;
  model: string;
  apiFormat: ApiFormat;
  showKey: boolean;
  testing: boolean;
  testResult: string | null;
  accentColor: string;
  textColor: string;
  textColorSecondary: string;
  inputBgColor: string;
  borderColor: string;
  onProviderNameChange: (v: string) => void;
  onApiBaseChange: (v: string) => void;
  onApiKeyChange: (v: string) => void;
  onModelChange: (v: string) => void;
  onApiFormatChange: (v: ApiFormat) => void;
  onShowKeyToggle: () => void;
  onTestConnection: () => void;
  onSave: () => void;
  onSelectPreset: (preset: typeof PROVIDERS[number]) => void;
}

export default function ProviderEditor({
  providerName,
  apiBase,
  apiKey,
  model,
  apiFormat,
  showKey,
  testing,
  testResult,
  accentColor,
  textColor,
  textColorSecondary,
  inputBgColor,
  borderColor,
  onProviderNameChange,
  onApiBaseChange,
  onApiKeyChange,
  onModelChange,
  onApiFormatChange,
  onShowKeyToggle,
  onTestConnection,
  onSave,
  onSelectPreset,
  provider,
}: ProviderEditorProps) {
  return (
    <View style={[styles.editSection, { borderColor }]}>
      <Text style={[styles.label, { color: textColorSecondary }]}>名称</Text>
      <TextInput
        style={[styles.input, { backgroundColor: inputBgColor, color: textColor, borderColor }]}
        value={providerName}
        onChangeText={onProviderNameChange}
        autoCapitalize="none"
      />

      <Text style={[styles.label, { color: textColorSecondary }]}>快速选择</Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} style={{ marginBottom: 12 }}>
        {PROVIDERS.map((p) => (
          <TouchableOpacity
            key={p.name}
            style={[
              styles.presetBtn,
              {
                borderColor: apiBase === p.base ? accentColor : borderColor,
                backgroundColor: apiBase === p.base ? accentColor + '20' : 'transparent',
              },
            ]}
            onPress={() => onSelectPreset(p)}
          >
            <Text
              style={{
                color: apiBase === p.base ? accentColor : textColor,
                fontWeight: '500',
                fontSize: 13,
              }}
            >
              {p.name}
            </Text>
          </TouchableOpacity>
        ))}
      </ScrollView>

      <Text style={[styles.label, { color: textColorSecondary }]}>API Base URL</Text>
      <TextInput
        style={[styles.input, { backgroundColor: inputBgColor, color: textColor, borderColor }]}
        value={apiBase}
        onChangeText={onApiBaseChange}
        autoCapitalize="none"
        autoCorrect={false}
      />

      <Text style={[styles.label, { color: textColorSecondary }]}>API Key</Text>
      <View style={styles.keyRow}>
        <TextInput
          style={[styles.input, { flex: 1, backgroundColor: inputBgColor, color: textColor, borderColor }]}
          value={apiKey}
          onChangeText={onApiKeyChange}
          secureTextEntry={!showKey}
          autoCapitalize="none"
          autoCorrect={false}
        />
        <TouchableOpacity onPress={onShowKeyToggle} style={styles.eyeBtn}>
          <Icon name={showKey ? 'eye-off' : 'eye'} size={18} color={textColorSecondary} />
        </TouchableOpacity>
      </View>

      <Text style={[styles.label, { color: textColorSecondary }]}>格式</Text>
      <View style={styles.formatRow}>
        {(['openai', 'anthropic'] as const).map(fmt => (
          <TouchableOpacity
            key={fmt}
            style={[
              styles.formatBtn,
              {
                borderColor: apiFormat === fmt ? accentColor : borderColor,
                backgroundColor: apiFormat === fmt ? accentColor + '20' : 'transparent',
              },
            ]}
            onPress={() => onApiFormatChange(fmt)}
          >
            <Text style={{ color: apiFormat === fmt ? accentColor : textColor }}>
              {fmt === 'openai' ? 'OpenAI' : 'Anthropic'}
            </Text>
          </TouchableOpacity>
        ))}
      </View>

      <Text style={[styles.label, { color: textColorSecondary }]}>模型</Text>
      <TextInput
        style={[styles.input, { backgroundColor: inputBgColor, color: textColor, borderColor }]}
        value={model}
        onChangeText={onModelChange}
        autoCapitalize="none"
        autoCorrect={false}
      />

      <View style={styles.btnRow}>
        <TouchableOpacity
          style={[styles.btn, { backgroundColor: inputBgColor, borderColor }]}
          onPress={onTestConnection}
          disabled={testing}
        >
          {testing ? (
            <ActivityIndicator color={accentColor} />
          ) : (
            <Text style={[styles.btnText, { color: textColor }]}>测试连接</Text>
          )}
        </TouchableOpacity>
        <TouchableOpacity style={[styles.btn, { backgroundColor: accentColor }]} onPress={onSave}>
          <Text style={[styles.btnText, { color: '#FFF' }]}>保存</Text>
        </TouchableOpacity>
      </View>
      {testResult && (
        <Text
          style={[
            styles.testResult,
            { color: testResult.includes('✅') ? '#10B981' : '#EF4444' },
          ]}
        >
          {testResult}
        </Text>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  label: { fontSize: 13, fontWeight: '500', marginBottom: 6 },
  input: {
    borderWidth: 1,
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 10,
    fontSize: 15,
    marginBottom: 12,
  },
  keyRow: { flexDirection: 'row', alignItems: 'center' },
  eyeBtn: { paddingHorizontal: 12, paddingVertical: 10 },
  btnRow: { flexDirection: 'row', gap: 12, marginTop: 4 },
  btn: {
    flex: 1,
    paddingVertical: 12,
    borderRadius: 10,
    alignItems: 'center',
    borderWidth: 1,
    borderColor: 'transparent',
  },
  btnText: { fontSize: 16, fontWeight: '600' },
  testResult: { textAlign: 'center', marginTop: 10, fontSize: 14 },
  editSection: { marginTop: 16, paddingTop: 16, borderTopWidth: 1 },
  presetBtn: {
    paddingHorizontal: 14,
    paddingVertical: 7,
    borderRadius: 18,
    borderWidth: 1,
    marginRight: 8,
  },
  formatRow: { flexDirection: 'row', gap: 8, marginBottom: 12 },
  formatBtn: {
    flex: 1,
    paddingVertical: 10,
    borderRadius: 10,
    borderWidth: 1,
    alignItems: 'center',
  },
});