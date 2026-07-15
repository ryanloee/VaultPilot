// Regression test for issue #2894: Focus / Reading mode.
//
// Feature: a "专注 / 阅读模式" toggle hides AI command palette entry, assistant
// floating button and context suggestion panels so the user can write without
// AI UI distractions. The toggle must:
//   1. default to OFF (distraction-free is opt-in)
//   2. be controllable via setFocusMode
//   3. persist across app restarts (included in zustand persist partialize)
//   4. actually remove AI toolbar actions from the editor toolbar when ON
//
// We verify persistence by capturing the REAL AsyncStorage write that zustand's
// persist middleware produces (key 'vaultpilot-store'), not by reconstructing
// the partialize object by hand — so the test fails if partialize ever drops
// focusMode.

jest.mock('expo-secure-store', () => ({
  setItemAsync: jest.fn(),
  getItemAsync: jest.fn(),
}));
jest.mock('@react-native-async-storage/async-storage', () => ({
  __esModule: true,
  default: {
    setItem: jest.fn(),
    getItem: jest.fn(),
  },
}));

const AsyncStorage = require('@react-native-async-storage/async-storage').default;
const { useAppStore, filterFocusModeToolbar } = require('../../store');

const TOOLBAR = [
  { label: 'B', insert: '**', desc: '加粗' },
  { label: 'I', insert: '*', desc: '斜体' },
  { label: '`', insert: '`', desc: '代码' },
  { label: '#', insert: '# ', desc: '标题' },
  { label: '-', insert: '- ', desc: '列表' },
  { label: 'link', insert: '[]()', desc: '链接', icon: 'link-outline' },
  { label: 'AI', insert: '', desc: 'AI 写作', icon: 'color-wand-outline', action: 'aiWrite' },
  { label: 'Cmd', insert: '', desc: 'AI 命令面板', icon: 'terminal-outline', action: 'aiCmd' },
];

describe('Issue #2894 — Focus / Reading mode', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    AsyncStorage.getItem.mockResolvedValue(null);
    AsyncStorage.setItem.mockResolvedValue(undefined);
    // Reset to a clean default state
    useAppStore.setState({ focusMode: false });
  });

  it('focusMode defaults to false', () => {
    expect(useAppStore.getState().focusMode).toBe(false);
  });

  it('setFocusMode toggles the value both ways', () => {
    useAppStore.getState().setFocusMode(true);
    expect(useAppStore.getState().focusMode).toBe(true);
    useAppStore.getState().setFocusMode(false);
    expect(useAppStore.getState().focusMode).toBe(false);
  });

  it('focusMode is persisted to AsyncStorage via partialize', async () => {
    const captured: Record<string, string> = {};
    (AsyncStorage.setItem as jest.Mock).mockImplementation((key: string, value: string) => {
      captured[key] = value;
      return Promise.resolve(undefined);
    });

    useAppStore.getState().setFocusMode(true);

    // Allow zustand persist's async write to flush
    await new Promise((r) => setTimeout(r, 50));

    const persistedRaw = captured['vaultpilot-store'];
    expect(persistedRaw).toBeDefined();
    const persisted = JSON.parse(persistedRaw);
    expect(persisted.state).toHaveProperty('focusMode');
    expect(persisted.state.focusMode).toBe(true);
  });

  it('filterFocusModeToolbar keeps formatting items but drops AI actions when focus mode is ON', () => {
    const on = filterFocusModeToolbar(TOOLBAR, true);
    expect(on).toHaveLength(6);
    expect(on.some((t: any) => t.action === 'aiWrite')).toBe(false);
    expect(on.some((t: any) => t.action === 'aiCmd')).toBe(false);
    expect(on.map((t: any) => t.label)).toEqual(['B', 'I', '`', '#', '-', 'link']);
  });

  it('filterFocusModeToolbar returns all items unchanged when focus mode is OFF', () => {
    const off = filterFocusModeToolbar(TOOLBAR, false);
    expect(off).toHaveLength(8);
    expect(off.some((t: any) => t.action === 'aiWrite')).toBe(true);
    expect(off.some((t: any) => t.action === 'aiCmd')).toBe(true);
  });
});
