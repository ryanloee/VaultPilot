/**
 * Regression test for #3589: OnboardingScreen stale testResult discards modified provider config
 *
 * Bug: After a successful connection test, `testResult` stays '✅ 连接成功' forever.
 * If the user navigates back to Step 2 to modify config and returns to Step 3, the stale
 * success marker shows "完成" immediately — handleTestAndSave is never re-run, and the
 * modified config is silently discarded.
 *
 * Fix: Clear `testResult` in the Step 3 "← 修改配置" handler and Step 2 "测试连接 →" handler.
 */
import React from 'react';
import { create, act } from 'react-test-renderer';

// ── Mocks (override react-native mock with one that captures React at module level) ──

jest.mock('react-native', () => {
  const React = require('react');
  const C = (name: string) => (props: any) =>
    React.createElement(name, props, props.children);
  const TouchableOpacity = (props: any) =>
    React.createElement('TouchableOpacity', props, props.children);
  const TextInput = (props: any) =>
    React.createElement('TextInput', props, props.children);
  return {
    View: C('View'),
    Text: C('Text'),
    ScrollView: C('ScrollView'),
    TouchableOpacity,
    TextInput,
    StyleSheet: {
      create: (s: any) => s,
      flatten: (s: any) => (Array.isArray(s) ? Object.assign({}, ...s) : s),
    },
    ActivityIndicator: C('ActivityIndicator'),
    Platform: { OS: 'android' },
  };
});

jest.mock('react-native-safe-area-context', () => {
  const React = require('react');
  return {
    SafeAreaView: (props: any) => React.createElement('SafeAreaView', props, props.children),
  };
});

jest.mock('../../store', () => ({
  useAppStore: () => ({
    isDark: false, accentColor: '#6C5CE7', providers: [] as any[], addProvider: jest.fn(),
  }),
  getColors: () => ({
    bg: '#fff', text: '#000', textSecondary: '#666', border: '#ddd', inputBg: '#f5f5f5',
  }),
  PROVIDERS: [{
    name: 'OpenAI', base: 'https://api.openai.com/v1',
    format: 'openai', models: ['gpt-4o-mini'],
  }],
}));

let mockApiResult: { ok: boolean; error?: string } = { ok: true };
jest.mock('../../api/client', () => ({
  checkApi: jest.fn().mockImplementation(async () => mockApiResult),
  saveSettings: jest.fn().mockResolvedValue(undefined),
}));

jest.mock('../../components/Icon', () => {
  const React = require('react');
  return {
    __esModule: true,
    default: (props: any) => React.createElement('Icon', props),
  };
});

// ── Helpers ─────────────────────────────────────────────────────────

import OnboardingScreen from '../../screens/OnboardingScreen';

/** Find a pressable element (has onPress prop) whose child Text matches the label. */
function findButton(root: any, label: string): any {
  let found: any = null;
  function walk(inst: any) {
    if (found) return;
    if (inst?.props?.onPress) {
      let text = '';
      function getText(n: any) {
        if (typeof n === 'string') text += n;
        else if (n?.children) (Array.isArray(n.children) ? n.children : [n.children]).forEach((c: any) => getText(c));
      }
      getText(inst);
      if (text.includes(label)) { found = inst; return; }
    }
    for (const child of (inst?.children ?? [])) {
      if (typeof child === 'object') walk(child);
    }
  }
  walk(root);
  return found;
}

/** Find element by testID. */
function findByTestID(root: any, testID: string): any {
  let found: any = null;
  function walk(inst: any) {
    if (found) return;
    if (inst?.props?.testID === testID) { found = inst; return; }
    for (const child of (inst?.children ?? [])) {
      if (typeof child === 'object') walk(child);
    }
  }
  walk(root);
  return found;
}

/** Find first TextInput and call onChangeText. */
function setFirstInput(root: any, value: string) {
  let found: any = null;
  function walk(inst: any) {
    if (found) return;
    if (inst?.props?.onChangeText) { found = inst; return; }
    for (const child of (inst?.children ?? [])) {
      if (typeof child === 'object') walk(child);
    }
  }
  walk(root);
  if (found?.props?.onChangeText) found.props.onChangeText(value);
}

/** Check if any text node in tree matches regex. */
function treeContainsText(root: any, regex: RegExp): boolean {
  let found = false;
  function walk(inst: any) {
    if (found) return;
    if (typeof inst === 'string') {
      if (regex.test(inst)) { found = true; return; }
    }
    const kids = inst?.children;
    if (kids) {
      (Array.isArray(kids) ? kids : [kids]).forEach((c: any) => {
        if (typeof c === 'string') { if (regex.test(c)) { found = true; } }
        else if (typeof c === 'object') walk(c);
      });
    }
  }
  walk(root);
  return found;
}

// ── Tests ───────────────────────────────────────────────────────────

describe('#3589 — OnboardingScreen stale testResult', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockApiResult = { ok: true };
  });

  it('clears testResult when navigating back from Step 3 to Step 2', async () => {
    const onComplete = jest.fn();
    let r: any;
    await act(async () => {
      r = create(<OnboardingScreen onComplete={onComplete} />);
    });

    // Step 0 → Step 1
    await act(async () => {
      findButton(r.root, '开始设置')?.props.onPress();
    });

    // Step 1 → Step 2 (select OpenAI)
    await act(async () => {
      findButton(r.root, 'OpenAI')?.props.onPress();
    });

    // Step 2: enter API key
    await act(async () => {
      setFirstInput(r.root, 'sk-test-key');
    });

    // Step 2 → Step 3
    await act(async () => {
      findButton(r.root, '测试连接 →')?.props.onPress();
    });

    // Step 3: should show "测试连接" (not "完成") — no test yet
    expect(treeContainsText(r.root, /测试连接/)).toBe(true);
    expect(treeContainsText(r.root, /^完成$/)).toBe(false);

    // Press test button → successful test
    await act(async () => {
      await findByTestID(r.root, 'onboarding-test-btn')?.props.onPress();
    });

    // After success: should show "完成"
    expect(treeContainsText(r.root, /完成/)).toBe(true);

    // ── KEY REGRESSION CHECK ──
    // Go back to Step 2 via "← 修改配置"
    await act(async () => {
      findByTestID(r.root, 'onboarding-modify-btn')?.props.onPress();
    });

    // Step 2 → Step 3 again (testResult should be cleared)
    await act(async () => {
      findButton(r.root, '测试连接 →')?.props.onPress();
    });

    // Step 3: testResult MUST be cleared — shows "测试连接", NOT "完成"
    expect(treeContainsText(r.root, /测试连接/)).toBe(true);
    expect(treeContainsText(r.root, /^完成$/)).toBe(false);

    r.unmount();
  });
});
