// @ts-nocheck
/**
 * Regression test for #3781 (mobile) — 向后兼容：不传新 props 时点击图片仍打开 Lightbox。
 * 交互路径：渲染 MarkdownPreview（无任何 #3781 props）→ 点击独立图片 → Lightbox 打开、不出现选择操作栏。
 * 单文件只测这一条交互路径（RNTL v14 async 状态泄漏）。
 */
import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';

// MarkdownPreview lazy-requires expo-clipboard inside the copy handler
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));

// Mock Icon to avoid SVG dep
jest.mock('../../components/Icon', () => {
  const { Text } = require('react-native');
  return function MockIcon(_props) {
    return React.createElement(Text, null, '[icon]');
  };
});

// Mock Lightbox — expose `visible` so the test can assert it opened
jest.mock('../../components/Lightbox', () => {
  const { View } = require('react-native');
  return function MockLightbox(props) {
    return React.createElement(View, { testID: 'mock-lightbox', visible: props.visible });
  };
});

// Mock react-native-webview (ESM, Jest can't parse it; pulled in via MermaidDiagram)
jest.mock('react-native-webview', () => {
  const { View } = require('react-native');
  return {
    WebView: (props) => React.createElement(View, { testID: props.testID || 'mock-webview' }),
  };
});

import MarkdownPreview from '../../components/MarkdownPreview';

describe('#3781 MarkdownPreview — 向后兼容（点击打开 Lightbox）', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
  };

  it('未传新 props 时：点击图片打开 Lightbox，且不出现选择操作栏', async () => {
    const { getByTestId, queryByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '![photo](https://example.com/p.png)',
      }),
    );

    // 初始：Lightbox 未打开
    expect(queryByTestId('mock-lightbox')).toBeNull();

    // 点击图片（原有行为）→ Lightbox 打开；选择操作栏不应出现（选中仅由长按触发）
    await fireEvent.press(getByTestId('md-image-0'));
    expect(getByTestId('mock-lightbox').props.visible).toBe(true);
    expect(queryByTestId('md-image-action-bar')).toBeNull();
  });
});
