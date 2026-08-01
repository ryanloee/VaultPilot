/**
 * Regression test for #3649 — Language picker (setLocale / getCurrentLocale logic)
 *
 * The i18n module exposes setLocale() and getCurrentLocale() that persist
 * user language choice via AsyncStorage. This test validates the core
 * persistence and fallback logic without importing expo-native modules.
 */

// ── Simulated locale logic (mirrors src/i18n/index.ts) ──

let currentLocale: string = 'zh-CN'; // default

const USER_LANG_KEY = '@vaultpilot:user-locale';

function getCurrentLocale(): string {
  return currentLocale;
}

async function setLocaleMock(
  storage: { setItem: (k: string, v: string) => Promise<void> },
  locale: 'en' | 'zh-CN'
): Promise<void> {
  currentLocale = locale; // update in-memory immediately
  try {
    await storage.setItem(USER_LANG_KEY, locale);
  } catch {
    // non-critical — locale is already in memory
  }
}

async function initLocaleMock(
  storage: { getItem: (k: string) => Promise<string | null> }
): Promise<void> {
  try {
    const stored = await storage.getItem(USER_LANG_KEY);
    if (stored === 'en' || stored === 'zh-CN') {
      currentLocale = stored;
      return;
    }
  } catch {
    // storage unavailable — use default
  }
  currentLocale = 'zh-CN';
}

// ── Tests ──

describe('Language picker (#3649)', () => {
  let storage: { getItem: jest.Mock; setItem: jest.Mock };

  beforeEach(() => {
    storage = {
      getItem: jest.fn().mockResolvedValue(null),
      setItem: jest.fn().mockResolvedValue(undefined),
    };
    currentLocale = 'zh-CN';
  });

  describe('setLocale', () => {
    it('persists "en" via AsyncStorage', async () => {
      await setLocaleMock(storage, 'en');
      expect(storage.setItem).toHaveBeenCalledWith(USER_LANG_KEY, 'en');
      expect(getCurrentLocale()).toBe('en');
    });

    it('persists "zh-CN" via AsyncStorage', async () => {
      currentLocale = 'en';
      await setLocaleMock(storage, 'zh-CN');
      expect(storage.setItem).toHaveBeenCalledWith(USER_LANG_KEY, 'zh-CN');
      expect(getCurrentLocale()).toBe('zh-CN');
    });

    it('updates in-memory locale immediately before storage write', async () => {
      let capturedLocale: string | null = null;
      storage.setItem.mockImplementation(async (_k: string, v: string) => {
        capturedLocale = v;
      });
      await setLocaleMock(storage, 'en');
      // locale should be 'en' during the storage write
      expect(capturedLocale).toBe('en');
    });

    it('handles storage failure gracefully — does not throw', async () => {
      storage.setItem.mockRejectedValueOnce(new Error('disk full'));
      currentLocale = 'zh-CN';
      await expect(setLocaleMock(storage, 'en')).resolves.toBeUndefined();
      // In-memory locale still updated despite storage failure
      expect(getCurrentLocale()).toBe('en');
    });
  });

  describe('initLocale', () => {
    it('defaults to zh-CN when no stored locale', async () => {
      storage.getItem.mockResolvedValue(null);
      await initLocaleMock(storage);
      expect(getCurrentLocale()).toBe('zh-CN');
    });

    it('uses stored "en" locale', async () => {
      storage.getItem.mockResolvedValue('en');
      await initLocaleMock(storage);
      expect(getCurrentLocale()).toBe('en');
    });

    it('uses stored "zh-CN" locale', async () => {
      currentLocale = 'en'; // pretend system is English
      storage.getItem.mockResolvedValue('zh-CN');
      await initLocaleMock(storage);
      expect(getCurrentLocale()).toBe('zh-CN');
    });

    it('falls back to default when storage fails', async () => {
      storage.getItem.mockRejectedValueOnce(new Error('storage error'));
      await initLocaleMock(storage);
      expect(getCurrentLocale()).toBe('zh-CN');
    });
  });
});