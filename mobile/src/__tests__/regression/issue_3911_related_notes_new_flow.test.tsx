// @ts-nocheck
/**
 * Regression test for #3911 (mobile): 新建笔记流程后相关笔记永久显示「未连接后端」。
 * 交互路径：noteId="new" 进屏 → effect1 createNote() → navigation.setParams 换成
 * 真实 id → related-notes effect 用真实 id 重跑拉取 /api/graph → 渲染 1 跳邻居。
 * 断言：全程不出现「未连接后端」，最终显示相关笔记数据。
 * 单文件只测这一条交互路径（RNTL v14 async 状态泄漏）。
 */
import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
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

import NoteEditorScreen from '../../screens/NoteEditorScreen';
import { createNote } from '../../db';

// 后端 /api/graph 返回的 KnowledgeGraph JSON（中心节点 id 与 createNote 返回值一致）
const graphJson = {
  nodes: [
    { id: 'note-new-id', title: '新建的笔记', tags: [], in_degree: 2, out_degree: 1 },
    { id: 'note-a', title: '笔记A', tags: ['tag-a'], in_degree: 2, out_degree: 1 },
    { id: 'note-b', title: '笔记B', tags: [], in_degree: 1, out_degree: 0 },
    { id: 'note-c', title: '无关笔记C', tags: [], in_degree: 0, out_degree: 0 },
  ],
  edges: [
    { source: 'note-new-id', target: 'note-a', label: 'note-a', kind: 'wikilink' },
    { source: 'note-b', target: 'note-new-id', label: 'note-b', kind: 'wikilink' },
    { source: 'note-a', target: 'note-c', label: 'note-c', kind: 'wikilink' },
  ],
  note_count: 4,
  edge_count: 3,
  dangling_link_count: 0,
};

// 模拟 React Navigation：setParams 更新 route.params 并触发重渲染，
// 使 NoteEditorScreen 的 noteId 从 "new" 变为真实 id（与真实导航行为一致）。
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

describe('#3911 新建笔记流程后相关笔记不再卡「未连接后端」', () => {
  let fetchMock;

  beforeEach(async () => {
    navigation = undefined;
    (createNote as jest.Mock).mockClear();
    await AsyncStorage.clear();
    await AsyncStorage.setItem('cfg_backend_url', 'http://127.0.0.1:8080');
    await AsyncStorage.setItem('cfg_backend_token', 'test-token');
    fetchMock = jest.spyOn(global, 'fetch').mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => graphJson,
    });
  });

  afterEach(() => {
    fetchMock?.mockRestore();
  });

  it('noteId="new" → createNote → setParams 换真实 id → 显示相关笔记且无「未连接后端」', async () => {
    const utils = await render(React.createElement(EditorWithNewFlow));
    const { getByTestId, queryByText, queryByTestId } = utils;

    // 1. effect1 在 "new" 分支调用 createNote 创建真实笔记
    expect(createNote).toHaveBeenCalledTimes(1);

    // 2. effect1 用 createNote 返回的 id 调 setParams，把 noteId 换成真实 id
    await waitFor(() => expect(navigation.setParams).toHaveBeenCalledWith({ noteId: 'note-new-id' }));

    // 3. related-notes effect 用真实 id 重跑 → 拉取 /api/graph（后端配置来自 AsyncStorage）
    await waitFor(() => expect(getByTestId('related-note-note-a')).toBeTruthy());
    await new Promise(r => setTimeout(r, 50));
    expect(fetchMock).toHaveBeenCalledWith(
      'http://127.0.0.1:8080/api/graph',
      expect.objectContaining({
        headers: expect.objectContaining({ Authorization: 'Bearer test-token' }),
      }),
    );

    // 4. 渲染出 1 跳邻居（A：center→A；B：B→center），无关笔记 C 不出现
    expect(getByTestId('related-notes-section')).toBeTruthy();
    expect(getByTestId('related-note-note-a')).toBeTruthy();
    expect(getByTestId('related-note-note-b')).toBeTruthy();
    expect(queryByTestId('related-note-note-c')).toBeNull();

    // 5. #3911 核心断言：有数据时不得再显示「未连接后端」（修复前 relatedError 残留导致卡死）
    expect(queryByText('未连接后端')).toBeNull();
  });
});
