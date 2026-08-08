// @ts-nocheck
/**
 * Regression test for #3963 (mobile) — 长按删除图片不应误删相同 markdown 重复行。
 *
 * 场景：笔记中有两行完全相同的独立图片 markdown。用户长按选中第一张并删除，
 * 只应删除第一行（用户选中的那张），第二行必须保留。
 *
 * 修复：MarkdownPreview 通过 onDeleteImage 传递原始内容行索引（而非行文本），
 * 父组件按索引删除单行。
 *
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

describe('#3963 MarkdownPreview — 删除重复行图片只删选中行', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
  };

  it('两行相同图片 markdown：长按第一张删除，onDeleteImage 传索引 0（不传文本）', async () => {
    const onDeleteImage = jest.fn();
    // line 0: image A, line 1: text, line 2: image A (identical markdown)
    const content = '![photo](https://example.com/p.png)\nmiddle\n![photo](https://example.com/p.png)';

    const { getByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content,
        onDeleteImage,
      }),
    );

    // md-image-0 = first image (line index 0), md-image-1 = second image (line index 2)
    await fireEvent(getByTestId('md-image-0'), 'longPress');
    await fireEvent.press(getByTestId('md-image-delete-btn'));

    // Must pass a NUMBER (line index), NOT a string (old buggy behavior)
    expect(onDeleteImage).toHaveBeenCalledTimes(1);
    const arg = onDeleteImage.mock.calls[0][0];
    expect(typeof arg).toBe('number');
    expect(arg).toBe(0); // first image = line index 0 in original content
  });

  it('两行相同图片 markdown：长按第二张删除，onDeleteImage 传索引 2', async () => {
    const onDeleteImage = jest.fn();
    const content = '![photo](https://example.com/p.png)\nmiddle\n![photo](https://example.com/p.png)';

    const { getByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content,
        onDeleteImage,
      }),
    );

    // Select the SECOND image (globalIdx 1 → original line index 2)
    await fireEvent(getByTestId('md-image-1'), 'longPress');
    await fireEvent.press(getByTestId('md-image-delete-btn'));

    expect(onDeleteImage).toHaveBeenCalledTimes(1);
    expect(onDeleteImage).toHaveBeenCalledWith(2);
  });

  it('脚注定义不偏移图片行索引（脚注被隐藏但索引仍对齐原始内容）', async () => {
    const onDeleteImage = jest.fn();
    // line 0: footnote def (hidden), line 1: image, line 2: text, line 3: image
    const content = '[^1]: note text\n![photo](https://example.com/p.png)\nmiddle\n![photo](https://example.com/p.png)';

    const { getByTestId } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content,
        onDeleteImage,
      }),
    );

    // First VISIBLE image is globalIdx 0 but original line index 1
    await fireEvent(getByTestId('md-image-0'), 'longPress');
    await fireEvent.press(getByTestId('md-image-delete-btn'));

    expect(onDeleteImage).toHaveBeenCalledWith(1); // NOT 0 — footnote def doesn't shift it
  });
});
