// @ts-nocheck
/**
 * Regression test for #3781 (mobile) — 图片键盘交互：长按选中 + 复制。
 * 交互路径：长按独立图片 → 浮动操作栏出现 + 蓝色选中边框 → 点 复制 → expo-clipboard 收到图片 markdown。
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

// Mock Lightbox to avoid native module dependencies
jest.mock('../../components/Lightbox', () => {
  const { View } = require('react-native');
  return function MockLightbox(_props) {
    return React.createElement(View, { testID: 'mock-lightbox' });
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

describe('#3781 MarkdownPreview — 长按选中 + 复制', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
  };

  it('长按独立图片出现浮动操作栏与蓝色选中框，点 复制 将图片 markdown 写入剪贴板', async () => {
    const { getByTestId, queryByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '![screenshot](https://example.com/s.png)',
      }),
    );
    const Clipboard = require('expo-clipboard');

    // 长按前：无浮动操作栏
    expect(queryByTestId('md-image-action-bar')).toBeNull();

    // 长按选中图片
    await fireEvent(getByTestId('md-image-0'), 'longPress');

    // 浮动操作栏出现；选中图片带蓝色边框高亮
    expect(getByTestId('md-image-action-bar')).toBeTruthy();
    const img = getByTestId('md-image-0').children[0];
    const flat = require('react-native').StyleSheet.flatten(img.props.style);
    expect(flat.borderWidth).toBe(2);
    expect(flat.borderColor).toBe('#3b82f6');

    // 点 复制 → 剪贴板收到该图片的 markdown 语法
    await fireEvent.press(getByTestId('md-image-copy-btn'));
    expect(Clipboard.setStringAsync).toHaveBeenCalledWith('![screenshot](https://example.com/s.png)');
  });
});
