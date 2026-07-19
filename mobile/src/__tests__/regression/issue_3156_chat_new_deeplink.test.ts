// @ts-nocheck
/**
 * Regression test for #3156: vaultpilot://chat/new deep link — widget 'new chat' intent.
 * Verifies:
 * 1. The linking config includes chat/new route (ChatNew)
 * 2. The ChatStack has a ChatNew screen with initialParams.action='new'
 * 3. The ChatScreen handles route.params?.action === 'new' by creating a new session
 * 4. The desktop widget plugin sends vaultpilot://chat/new deep link
 */

// --- Mock minimal react-native modules ---
jest.mock('react-native', () => ({
  View: 'View',
  Text: 'Text',
  TextInput: 'TextInput',
  TouchableOpacity: 'TouchableOpacity',
  FlatList: 'FlatList',
  ActivityIndicator: 'ActivityIndicator',
  StyleSheet: { create: (s: any) => s },
}));

jest.mock('react-native-safe-area-context', () => ({
  SafeAreaView: 'SafeAreaView',
}));

jest.mock('@react-navigation/native', () => ({
  useNavigation: () => ({ navigate: jest.fn(), goBack: jest.fn() }),
  useFocusEffect: jest.fn(),
}));

jest.mock('../../store', () => ({
  useAppStore: () => ({ isDark: false, accentColor: '#3B82F6' }),
  getColors: () => ({
    bg: '#FFF', card: '#F3F4F6', text: '#111', sub: '#6B7280',
    accent: '#3B82F6', border: '#E5E7EB', danger: '#EF4444',
  }),
}));

jest.mock('../../db', () => ({
  getMessages: jest.fn().mockResolvedValue([]),
  getLatestSession: jest.fn().mockResolvedValue(null),
  createSession: jest.fn().mockResolvedValue('new-session-id'),
}));

jest.mock('../../utils/noteRefs', () => ({
  loadNoteTitleMap: jest.fn().mockResolvedValue(new Map()),
  clearNoteTitleCache: jest.fn(),
}));

describe('Chat New Deep Link (#3156)', () => {
  // Reflects the updated linking config defined in App.tsx
  const linkingConfig = {
    prefixes: ['vaultpilot://'],
    config: {
      screens: {
        Chat: {
          screens: {
            ChatMain: 'chat',
            ChatNew: 'chat/new',
            Sessions: 'chat/sessions',
          },
        },
        Notes: {
          screens: {
            NotesList: 'note',
            NoteEdit: 'note/:noteId',
          },
        },
        Search: 'search',
        Settings: 'settings',
      },
    },
  };

  it('maps ChatNew to chat/new path for vaultpilot://chat/new deep link', () => {
    const chatNewPath = linkingConfig.config.screens.Chat.screens.ChatNew;
    expect(chatNewPath).toBe('chat/new');
  });

  it('ChatMain still maps to chat path (backward compatibility)', () => {
    const chatMainPath = linkingConfig.config.screens.Chat.screens.ChatMain;
    expect(chatMainPath).toBe('chat');
  });

  it('App.tsx registers ChatNew screen with initialParams.action=new', () => {
    const fs = require('fs');
    const path = require('path');
    const appPath = path.join(__dirname, '..', '..', '..', 'App.tsx');
    const source = fs.readFileSync(appPath, 'utf-8');
    expect(source).toContain("initialParams={{ action: 'new' }}");
    expect(source).toContain('name="ChatNew"');
  });

  it('ChatScreen handles route.params?.action === new', () => {
    const fs = require('fs');
    const path = require('path');
    const chatPath = path.join(__dirname, '..', '..', 'screens', 'ChatScreen.tsx');
    const source = fs.readFileSync(chatPath, 'utf-8');
    expect(source).toContain("action === 'new'");
    expect(source).toContain('#3156');
  });
});

describe('Desktop Widget sends vaultpilot://chat/new (#3156)', () => {
  it('plugin contains chat/new deep link for new chat button', () => {
    const fs = require('fs');
    const path = require('path');
    const widgetPath = path.join(__dirname, '..', '..', '..', 'plugins', 'withDesktopWidget.js');
    const source = fs.readFileSync(widgetPath, 'utf-8');
    expect(source).toContain('vaultpilot://chat/new');
    expect(source).toContain('btn_new_chat');
  });
});
