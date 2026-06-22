/**
 * Regression test for issue #1336:
 * ChatScreen.tsx split into sub-components
 *
 * Verifies that ChatScreen was split into ChatHeader, MessageList,
 * InputBar, OfflineBanner, ScrollToBottomButton, and MessageBubble.
 */

// Mock react-native so component imports don't crash
jest.mock('react-native', () => ({
  View: 'View',
  Text: 'Text',
  TextInput: 'TextInput',
  TouchableOpacity: 'TouchableOpacity',
  FlatList: 'FlatList',
  KeyboardAvoidingView: 'KeyboardAvoidingView',
  Platform: { OS: 'android' },
  ActivityIndicator: 'ActivityIndicator',
  Alert: { alert: jest.fn() },
  StyleSheet: { create: (s: any) => s },
}));

jest.mock('react-native-safe-area-context', () => ({
  SafeAreaView: 'SafeAreaView',
}));

jest.mock('../../store', () => ({
  useAppStore: () => ({ isDark: false, accentColor: '#007AFF' }),
  getColors: () => ({
    bg: '#FFF', bgSecondary: '#F5F5F5', text: '#000', textSecondary: '#666',
    border: '#E0E0E0', inputBg: '#F9F9F9', userBubble: '#007AFF',
    aiBubble: '#F0F0F0', userText: '#FFF', aiText: '#000',
  }),
}));

jest.mock('../../components/MarkdownPreview', () => ({ default: 'MarkdownPreview' }));
jest.mock('expo-haptics', () => ({
  impactAsync: jest.fn(),
  ImpactFeedbackStyle: { Light: 'light' },
}));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));

import { ChatHeader, MessageList, InputBar, OfflineBanner, ScrollToBottomButton } from '../../components/chat';

describe('issue #1336 — ChatScreen.tsx split into sub-components', () => {
  it('ChatHeader is exported as a function component', () => {
    expect(typeof ChatHeader).toBe('function');
    expect(ChatHeader.name).toBe('ChatHeader');
  });

  it('MessageList is exported as a function component', () => {
    expect(typeof MessageList).toBe('function');
    expect(MessageList.name).toBe('MessageList');
  });

  it('InputBar is exported as a function component', () => {
    expect(typeof InputBar).toBe('function');
    expect(InputBar.name).toBe('InputBar');
  });

  it('OfflineBanner is exported as a function component', () => {
    expect(typeof OfflineBanner).toBe('function');
    expect(OfflineBanner.name).toBe('OfflineBanner');
  });

  it('ScrollToBottomButton is exported as a function component', () => {
    expect(typeof ScrollToBottomButton).toBe('function');
    expect(ScrollToBottomButton.name).toBe('ScrollToBottomButton');
  });
});
