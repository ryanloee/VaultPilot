// @ts-nocheck
/**
 * Issue #2619: Press send → chatWithReconnect called. Isolated file.
 */
import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';

jest.mock('react-native-safe-area-context', () => {
  const { View } = require('react-native');
  return { SafeAreaView: (p) => React.createElement(View, p, p.children), useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }) };
});
jest.mock('../../store', () => ({
  useAppStore: () => ({ isDark: false, accentColor: '#007AFF' }),
  getColors: () => ({ bg: '#FFF', bgSecondary: '#F5F5F5', text: '#000', textSecondary: '#666', border: '#E0E0E0', inputBg: '#F9F9F9', userBubble: '#007AFF', aiBubble: '#F0F0F0', userText: '#FFF', aiText: '#000' }),
}));
jest.mock('../../api/client', () => ({
  chatWithReconnect: jest.fn(async (_h, onChunk) => { onChunk({ content: 'Hello', done: false }); onChunk({ done: true }); }),
}));
jest.mock('../../services/rag', () => ({
  buildNoteContext: jest.fn(async () => null), buildSystemPrompt: jest.fn(() => ''), executeToolCalls: jest.fn(async (t) => ({ cleaned: t, actions: [], savedNoteIds: [] })), RESPONSE_STYLE_LABELS: {},
}));
jest.mock('../../db', () => ({
  getMessages: jest.fn(async () => []),
  addMessage: jest.fn(async (_sid, role) => `msg-${role}-${Date.now()}`),
  updateMessage: jest.fn(async () => {}), deleteMessage: jest.fn(async () => {}),
  createSession: jest.fn(async () => 'test-session-id'),
  getLatestSession: jest.fn(async () => ({ id: 'test-session-id', title: '测试对话' })),
  getNoteTitleMap: jest.fn(async () => new Map()),
}));
jest.mock('../../utils/noteRefs', () => ({ loadNoteTitleMap: jest.fn(async () => new Map()), clearNoteTitleCache: jest.fn() }));
jest.mock('../../components/chat', () => {
  const { View, Text, TextInput, TouchableOpacity } = require('react-native');
  const MessageBubble = ({ item }) => React.createElement(View, null, React.createElement(Text, null, item.content));
  // Lightweight InputBar stub exposing the testIDs the #2619 suite relies on.
  const InputBar = (p) => {
    const disabled = !((p.input && p.input.trim()) || (p.attachments && p.attachments.length > 0));
    return React.createElement(View, null,
      React.createElement(TextInput, {
        testID: 'chat-input',
        value: p.input,
        onChangeText: p.onInputChange,
        onSubmitEditing: p.onSend,
        editable: !p.streaming,
      }),
      p.streaming
        ? React.createElement(TouchableOpacity, { testID: 'stop-btn', onPress: p.onStop }, React.createElement(Text, null, 'stop'))
        : React.createElement(TouchableOpacity, { testID: 'send-btn', onPress: p.onSend, disabled }, React.createElement(Text, null, 'send')),
    );
  };
  return { MessageBubble, InputBar };
});
jest.mock('../../components/MarkdownPreview', () => ({ default: ({ content }) => { const { Text } = require('react-native'); return React.createElement(Text, null, content); } }));
jest.mock('expo-haptics', () => ({ impactAsync: jest.fn().mockResolvedValue(undefined), ImpactFeedbackStyle: { Medium: 'medium' } }));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
jest.mock('@expo/vector-icons/Ionicons', () => {
  const { Text } = require('react-native');
  return (p) => React.createElement(Text, { testID: `icon-${p.name}` }, p.name);
});

import { chatWithReconnect } from '../../api/client';
import { getLatestSession } from '../../db';
import ChatScreen from '../../screens/ChatScreen';

async function renderChat() {
  let utils;
  const prev = getLatestSession.mock.calls.length;
  await act(async () => {
    utils = render(React.createElement(ChatScreen, { navigation: { navigate: jest.fn() }, route: { params: {} } }));
  });
  await waitFor(() => expect(getLatestSession.mock.calls.length).toBeGreaterThan(prev));
  await new Promise(r => setTimeout(r, 50));
  return utils;
}

it('press send → chatWithReconnect called', async () => {
  const { getByTestId } = await renderChat();
  await act(async () => { fireEvent.changeText(getByTestId('chat-input'), '你好'); });
  await act(async () => { fireEvent.press(getByTestId('send-btn')); });
  await act(async () => {});
  await waitFor(() => expect(chatWithReconnect).toHaveBeenCalled());
});
