// @ts-nocheck
/**
 * Regression test for #3781 (mobile) — 图片键盘交互：长按选中 + 删除。
 * 交互路径：长按独立图片 → 点 删除 → onDeleteImage 收到该图片所在 markdown 行，选中清除、操作栏消失。
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

describe('#3781 MarkdownPreview — 长按选中 + 删除', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
  };

  it('点 删除 通过 onDeleteImage 回调删除图片所在行，并清除选中关闭操作栏', async () => {
    const onDeleteImage = jest.fn();
    const { getByTestId, queryByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'before\n![photo](https://example.com/p.png)\nafter',
        onDeleteImage,
      }),
    );

    // 长按选中 → 删除按钮出现
    await fireEvent(getByTestId('md-image-0'), 'longPress');
    expect(getByTestId('md-image-delete-btn')).toBeTruthy();

    // 点 删除 → 父组件收到被选图片的行索引（0-based）；本地选中清除，操作栏消失
    await fireEvent.press(getByTestId('md-image-delete-btn'));
    // content 'before\n![photo](https://example.com/p.png)\nafter' — image is line index 1
    expect(onDeleteImage).toHaveBeenCalledWith(1);
    expect(queryByTestId('md-image-action-bar')).toBeNull();
  });

  it('未提供 onDeleteImage 时隐藏 删除 按钮（向后兼容）', async () => {
    const { getByTestId, queryByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '![photo](https://example.com/p.png)',
      }),
    );

    await fireEvent(getByTestId('md-image-0'), 'longPress');
    expect(queryByTestId('md-image-delete-btn')).toBeNull();
    // 复制等其余按钮不受影响
    expect(getByTestId('md-image-copy-btn')).toBeTruthy();
  });
});
