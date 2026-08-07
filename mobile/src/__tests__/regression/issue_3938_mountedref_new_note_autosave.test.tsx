// @ts-nocheck
/**
 * Regression test for #3938 (mobile): NoteEditorScreen mountedRef 在 noteId
 * "new"→真实 id 转换后永不复位——widget/快捷设置新建笔记后 autosave 静默失效。
 *
 * 交互路径：noteId="new" 进屏 → effect createNote() → navigation.setParams
 * 换成真实 id → [noteId] effect 先跑 cleanup（mountedRef=false）再重跑 body
 * （修复后 body 开头把 mountedRef 复位为 true）→ 输入标题触发 autoSave →
 * 1s 定时器到点调用 save() → updateNote 必须被调用。
 *
 * 修复前：cleanup 置 false 后无人复位，save() 首行 `if (!mountedRef.current)
 * return;` 让 updateNote 永不执行 → 断言 updateNote 被调用即可捕获回归。
 * 单文件只测这一条交互路径（RNTL v14 async 状态泄漏）。
 */
import React from 'react';
import { render, waitFor, fireEvent } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';

jest.mock('react-native-safe-area-context', () => {
  const { View } = require('react-native');
  return { SafeAreaView: (p) => React.createElement(View, p, p.children), useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }) };
});
jest.mock('../../store', () => ({
  useAppStore: () => ({ isDark: false, accentColor: '#3B82F6', focusMode: false }),
  getColors: () => ({ bg: '#FFF', bgSecondary: '#F3F4F6', text: '#111', textSecondary: '#6B7280', border: '#E5E7EB', card: '#FFF', inputBg: '#F9FAFB', accent: '#3B82F6' }),
  filterFocusModeToolbar: (items) => items,
}));
jest.mock('../../db', () => ({
  getNote: jest.fn(async () => ({ id: 'note-new-id', title: '新建的笔记', content: '正文内容', folder: '' })),
  createNote: jest.fn(async () => 'note-new-id'),
  updateNote: jest.fn(async () => {}),
  deleteNote: jest.fn(async () => {}),
  moveToFolder: jest.fn(async () => {}),
  getFolders: jest.fn(async () => []),
  getNoteTags: jest.fn(async () => []),
  addTag: jest.fn(async () => {}),
  removeTag: jest.fn(async () => {}),
  saveAsTemplate: jest.fn(async () => null),
}));
jest.mock('../../components/MarkdownPreview', () => ({ default: ({ content }) => { const { Text } = require('react-native'); return React.createElement(Text, null, content); } }));
jest.mock('../../components/ai/AiActionPalette', () => () => null);
jest.mock('expo-haptics', () => ({ impactAsync: jest.fn().mockResolvedValue(undefined), ImpactFeedbackStyle: { Light: 'light', Medium: 'medium' } }));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
jest.mock('@expo/vector-icons/Ionicons', () => {
  const { Text } = require('react-native');
  return (p) => React.createElement(Text, { testID: `icon-${p.name}` }, p.name);
});
// #3938: 拉取相关笔记的 fetch 也会在 setParams 后触发——mock 掉避免真实网络
jest.spyOn(global, 'fetch').mockResolvedValue({
  ok: true,
  status: 200,
  json: async () => ({ nodes: [], edges: [], note_count: 0, edge_count: 0, dangling_link_count: 0 }),
});

import NoteEditorScreen from '../../screens/NoteEditorScreen';
import { createNote, updateNote } from '../../db';

// 模拟 React Navigation：setParams 更新 route.params 并触发重渲染，
// 使 NoteEditorScreen 的 noteId 从 "new" 变为真实 id（与 #2915 真实行为一致）。
let navigation: any;
function EditorWithNewFlow() {
  const [params, setParams] = React.useState({ noteId: 'new' });
  if (!navigation) {
    const realSetParams = (p: any) => setParams((prev: any) => ({ ...prev, ...p }));
    navigation = {
      navigate: jest.fn(),
      goBack: jest.fn(),
      setParams: jest.fn(realSetParams),
    };
  }
  return React.createElement(NoteEditorScreen, { route: { params }, navigation });
}

describe('#3938 新建笔记后 autosave 不因 mountedRef 卡死', () => {
  beforeEach(async () => {
    navigation = undefined;
    (createNote as jest.Mock).mockClear();
    (updateNote as jest.Mock).mockClear();
    await AsyncStorage.clear();
  });

  it('noteId="new" → setParams 换真实 id → 输入标题 → updateNote 被调用（保存不静默失效）', async () => {
    const utils = await render(React.createElement(EditorWithNewFlow));
    const { getByPlaceholderText } = utils;

    // 1. effect 在 "new" 分支调用 createNote 创建真实笔记
    expect(createNote).toHaveBeenCalledTimes(1);

    // 2. setParams 把 noteId 换成真实 id（触发 [noteId] effect cleanup + 重跑）
    await waitFor(() => expect(navigation.setParams).toHaveBeenCalledWith({ noteId: 'note-new-id' }));

    // 3. 等 noteId 变为真实 id 后的 effect 重跑完成（mountedRef 复位 + getNote 加载）
    await waitFor(() => expect(createNote).toHaveBeenCalledTimes(1));

    // 4. 输入标题 → onChangeText 调 autoSave，调度 1s 定时器
    fireEvent.changeText(getByPlaceholderText('笔记标题'), '新建笔记的标题');

    // 5. #3938 核心断言：1s autosave 定时器到点后 updateNote 必须被调用
    //    （修复前 mountedRef=false 让 save() 首行直接 return，updateNote 永不执行）
    await waitFor(
      () => expect(updateNote).toHaveBeenCalled(),
      { timeout: 5000 },
    );
    expect(updateNote).toHaveBeenCalledWith(
      'note-new-id',
      '新建笔记的标题',
      expect.any(String),
    );
  });
});
