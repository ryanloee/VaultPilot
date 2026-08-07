// @ts-nocheck
/**
 * Regression test for #3781 (mobile) — 图片键盘交互：点击空白区域清除选中。
 * 交互路径：长按选中 → 长按手势自身的 touchEnd 不清除选中 → 点击空白区域（新手势 touchEnd）清除选中、操作栏消失。
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

describe('#3781 MarkdownPreview — 点击空白区域清除选中', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
  };

  it('长按手势自身的 touchEnd 不清除选中；点击空白区域清除选中', async () => {
    const { getByTestId, queryByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '![photo](https://example.com/p.png)',
      }),
    );
    const touchArea = () => getByTestId('md-preview-touch-area');

    // 完整长按手势：touchStart → longPress → touchEnd —— touchEnd 不应立刻清除刚建立的选中
    await fireEvent(touchArea(), 'touchStart');
    await fireEvent(getByTestId('md-image-0'), 'longPress');
    await fireEvent(touchArea(), 'touchEnd');
    expect(getByTestId('md-image-action-bar')).toBeTruthy();

    // 新手势：点击空白区域（touchStart → touchEnd）→ 清除选中，操作栏消失
    await fireEvent(touchArea(), 'touchStart');
    await fireEvent(touchArea(), 'touchEnd');
    expect(queryByTestId('md-image-action-bar')).toBeNull();
  });
});
