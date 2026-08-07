// @ts-nocheck
/**
 * Regression test for #3832 (mobile): 相关笔记 / 局部图谱（local graph）。
 * 交互路径：进屏拉取 /api/graph → 客户端提取 1 跳邻居 → 渲染相关笔记列表 →
 * 点击邻居项跳转 NoteEdit。单文件只测这一条交互路径（RNTL v14 async 状态泄漏）。
 */
import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
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
  getNote: jest.fn(async () => ({ id: 'note-center', title: '中心笔记', content: '正文内容', folder: '' })),
  createNote: jest.fn(async () => 'note-center'),
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

// 后端 /api/graph 返回的 KnowledgeGraph JSON（结构与 src/knowledge_graph.rs 一致）
const graphJson = {
  nodes: [
    { id: 'note-center', title: '中心笔记', tags: [], in_degree: 2, out_degree: 1 },
    { id: 'note-a', title: '笔记A', tags: ['tag-a'], in_degree: 2, out_degree: 1 },
    { id: 'note-b', title: '笔记B', tags: [], in_degree: 1, out_degree: 0 },
    { id: 'note-c', title: '无关笔记C', tags: [], in_degree: 0, out_degree: 0 },
  ],
  edges: [
    { source: 'note-center', target: 'note-a', label: 'note-a', kind: 'wikilink' },
    { source: 'note-b', target: 'note-center', label: 'note-b', kind: 'wikilink' },
    { source: 'note-a', target: 'note-c', label: 'note-c', kind: 'wikilink' },
  ],
  note_count: 4,
  edge_count: 3,
  dangling_link_count: 0,
};

describe('#3832 相关笔记（局部图谱）交互路径', () => {
  let fetchMock;

  beforeEach(async () => {
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

  it('进屏拉取 /api/graph，渲染 1 跳邻居，点击跳转 NoteEdit', async () => {
    const navigate = jest.fn();
    // RNTL v14: render 是 async 的，返回 Promise<RenderResult>
    const utils = await render(React.createElement(NoteEditorScreen, {
      navigation: { navigate, goBack: jest.fn(), setParams: jest.fn() },
      route: { params: { noteId: 'note-center' } },
    }));
    const { getByTestId, getByText, queryByTestId } = utils;

    // 等待相关笔记加载完成（真实 fetchKnowledgeGraph 路径：AsyncStorage + fetch）
    await waitFor(() => expect(getByTestId('related-note-note-a')).toBeTruthy());
    await new Promise(r => setTimeout(r, 50));

    // 1. 用后端配置（cfg_backend_url + cfg_backend_token）请求 /api/graph
    expect(fetchMock).toHaveBeenCalledWith(
      'http://127.0.0.1:8080/api/graph',
      expect.objectContaining({
        headers: expect.objectContaining({ Authorization: 'Bearer test-token' }),
      }),
    );

    // 2. 渲染出两个 1 跳邻居（A：center→A；B：B→center），无关笔记 C 不出现
    expect(getByTestId('related-notes-section')).toBeTruthy();
    expect(getByTestId('related-note-note-a')).toBeTruthy();
    expect(getByTestId('related-note-note-b')).toBeTruthy();
    expect(queryByTestId('related-note-note-c')).toBeNull();
    expect(getByText('笔记A')).toBeTruthy();
    expect(getByText('笔记B')).toBeTruthy();

    // 3. 点击邻居项 → navigation.navigate('NoteEdit', { noteId })
    await act(async () => { fireEvent.press(getByTestId('related-note-note-a')); });
    expect(navigate).toHaveBeenCalledWith('NoteEdit', { noteId: 'note-a' });
  });
});
