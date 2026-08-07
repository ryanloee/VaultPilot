// @ts-nocheck
/**
 * Regression test for #3927 (mobile): Lightbox 显示当前图片文件名（对标 Obsidian 1.13.4）。
 * 交互路径：visible Lightbox 渲染 → 文件名 caption 显示当前图 → 点 next 切图 → caption 更新。
 * 单文件只测这一条交互路径（RNTL v14 async 状态泄漏）。
 */
import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';

jest.mock('../../utils/imageMarkdown', () => {
  const actual = jest.requireActual('../../utils/imageMarkdown');
  return {
    ...actual,
    // 让测试不依赖 decodeURIComponent 的真实行为差异，直接用真实实现即可；
    // 这里仅重新导出，保持与生产一致。
  };
});

import Lightbox from '../../components/Lightbox';

// mock Expo vector icons (Icon 组件依赖)
jest.mock('@expo/vector-icons/Ionicons', () => {
  const { Text } = require('react-native');
  return (p) => React.createElement(Text, { testID: `icon-${p.name}` }, p.name);
});

describe('#3927 Lightbox 显示当前图片文件名', () => {
  const images = [
    { uri: 'https://example.com/attachments/photo-one.jpg', alt: '第一张' },
    { uri: 'https://example.com/attachments/photo-two.png', alt: '第二张' },
  ];

  it('渲染当前图文件名，切图后 caption 同步更新', async () => {
    let index = 0;
    const onIndexChange = (i: number) => { index = i; };
    const { getByTestId, rerender } = await render(
      React.createElement(Lightbox, {
        visible: true,
        images,
        index,
        onClose: jest.fn(),
        onIndexChange,
      }),
    );

    // 1. 初始显示第一张图的文件名
    expect(getByTestId('lightbox-file-name-text').props.children).toBe('photo-one.jpg');

    // 2. 点击 next → onIndexChange 更新 index → 重渲染 → caption 变第二张
    fireEvent.press(getByTestId('lightbox-next'));
    await rerender(
      React.createElement(Lightbox, {
        visible: true,
        images,
        index,
        onClose: jest.fn(),
        onIndexChange,
      }),
    );
    expect(getByTestId('lightbox-file-name-text').props.children).toBe('photo-two.png');
  });
});
