// @ts-nocheck
/**
 * Regression test for #3781 (mobile) — 图片键盘交互：长按选中 + 缩放。
 * 交互路径：长按独立图片 → 点 + 以 25% 步进放大 → 点 - 缩小 → 点 0 恢复默认宽度。
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

describe('#3781 MarkdownPreview — 长按选中 + 缩放', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
  };

  it('点 + 放大、点 - 缩小（25% 步进），点 0 恢复默认宽度', async () => {
    const { getByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '![photo](https://example.com/p.png)',
      }),
    );
    const { StyleSheet } = require('react-native');

    const renderedWidth = () => {
      const img = getByTestId('md-image-0').children[0];
      return StyleSheet.flatten(img.props.style).width;
    };

    // 长按选中
    await fireEvent(getByTestId('md-image-0'), 'longPress');

    // 默认宽度（100%，无显式像素宽度）
    expect(renderedWidth()).toBe('100%');

    // +：300 * 1.25 = 375
    await fireEvent.press(getByTestId('md-image-zoom-in'));
    expect(renderedWidth()).toBe(375);

    // 再 +：300 * 1.5 = 450
    await fireEvent.press(getByTestId('md-image-zoom-in'));
    expect(renderedWidth()).toBe(450);

    // -：回到 375
    await fireEvent.press(getByTestId('md-image-zoom-out'));
    expect(renderedWidth()).toBe(375);

    // 0：恢复默认宽度
    await fireEvent.press(getByTestId('md-image-zoom-reset'));
    expect(renderedWidth()).toBe('100%');
  });
});
