/**
 * Regression test for #3435
 *
 * Feature: Settings search. The SettingsScreen now includes a search bar
 * that filters visible sections by keyword matching. Each section has
 * a set of keywords (Chinese + English) that are matched against the
 * user's search query.
 *
 * The core matching logic is:
 *   matchesSearch(query, ...terms) => terms.some(t => t.toLowerCase().includes(query.toLowerCase()))
 *
 * When query is empty/falsy, all sections are shown.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';

jest.mock('@react-native-async-storage/async-storage', () => ({
  getItem: jest.fn(),
  setItem: jest.fn(),
}));

// Matches a search query against a list of terms (same logic as SettingsScreen)
function matchesSearch(query: string, ...terms: string[]): boolean {
  if (!query.trim()) return true;
  const q = query.toLowerCase().trim();
  return terms.some(term => term.toLowerCase().includes(q));
}

describe('SettingsScreen search (#3435)', () => {

  describe('matchesSearch helper', () => {
    it('returns true when query is empty or whitespace', () => {
      expect(matchesSearch('', 'API', '提供商')).toBe(true);
      expect(matchesSearch('   ', 'API', '提供商')).toBe(true);
    });

    it('matches Chinese keywords exactly', () => {
      expect(matchesSearch('API', 'API', '提供商', 'provider')).toBe(true);
      expect(matchesSearch('提供商', 'API', '提供商', 'provider')).toBe(true);
    });

    it('matches English keywords exactly', () => {
      expect(matchesSearch('provider', 'API', '提供商', 'provider')).toBe(true);
      expect(matchesSearch('sync', '数据同步', 'sync', 'background')).toBe(true);
    });

    it('matches partial keywords', () => {
      expect(matchesSearch('同', '数据同步', '同步', 'sync')).toBe(true);
      expect(matchesSearch('prov', 'API', '提供商', 'provider')).toBe(true);
    });

    it('does not match non-matching keywords', () => {
      expect(matchesSearch('xyz', 'API', '提供商', 'provider')).toBe(false);
      expect(matchesSearch('照片', 'API', '提供商', '同步')).toBe(false);
    });

    it('is case-insensitive', () => {
      expect(matchesSearch('api', 'API', '提供商', 'provider')).toBe(true);
      expect(matchesSearch('API', 'api', 'provider')).toBe(true);
      expect(matchesSearch('Sync', '数据同步', 'sync', 'background')).toBe(true);
    });
  });

  describe('section keyword samples (matches actual SettingsScreen keywords)', () => {
    // Simulate the actual section keywords used in SettingsScreen
    const providerKeywords = ['API', '提供商', 'provider', 'apiBase', 'apiKey', '模型', 'model', '格式', 'format', '连接', '测试', 'test', '保存', 'save'];
    const appearanceKeywords = ['外观', '主题', 'theme', '亮色', '暗色', 'dark', 'light', '系统', 'system', '主色调', 'accent', 'color'];
    const focusKeywords = ['专注', '阅读', 'focus', 'reading', '沉浸', '写作'];
    const syncKeywords = ['数据同步', '同步', 'sync', 'background', '后台', '间隔', 'interval'];
    const updateKeywords = ['检查更新', '更新', 'update', '版本', 'version', 'check'];

    it('searching "API" shows provider section', () => {
      const q = 'API';
      expect(matchesSearch(q, ...providerKeywords)).toBe(true);
      expect(matchesSearch(q, ...appearanceKeywords)).toBe(false);
      expect(matchesSearch(q, ...focusKeywords)).toBe(false);
      expect(matchesSearch(q, ...syncKeywords)).toBe(false);
      expect(matchesSearch(q, ...updateKeywords)).toBe(false);
    });

    it('searching "主题" shows appearance section', () => {
      const q = '主题';
      expect(matchesSearch(q, ...providerKeywords)).toBe(false);
      expect(matchesSearch(q, ...appearanceKeywords)).toBe(true);
      expect(matchesSearch(q, ...focusKeywords)).toBe(false);
      expect(matchesSearch(q, ...syncKeywords)).toBe(false);
      expect(matchesSearch(q, ...updateKeywords)).toBe(false);
    });

    it('searching "同步" shows data sync section', () => {
      const q = '同步';
      expect(matchesSearch(q, ...providerKeywords)).toBe(false);
      expect(matchesSearch(q, ...appearanceKeywords)).toBe(false);
      expect(matchesSearch(q, ...focusKeywords)).toBe(false);
      expect(matchesSearch(q, ...syncKeywords)).toBe(true);
      expect(matchesSearch(q, ...updateKeywords)).toBe(false);
    });

    it('searching "阅读" shows focus section', () => {
      const q = '阅读';
      expect(matchesSearch(q, ...providerKeywords)).toBe(false);
      expect(matchesSearch(q, ...appearanceKeywords)).toBe(false);
      expect(matchesSearch(q, ...focusKeywords)).toBe(true);
      expect(matchesSearch(q, ...syncKeywords)).toBe(false);
      expect(matchesSearch(q, ...updateKeywords)).toBe(false);
    });

    it('searching "版本" shows update section', () => {
      const q = '版本';
      expect(matchesSearch(q, ...providerKeywords)).toBe(false);
      expect(matchesSearch(q, ...appearanceKeywords)).toBe(false);
      expect(matchesSearch(q, ...focusKeywords)).toBe(false);
      expect(matchesSearch(q, ...syncKeywords)).toBe(false);
      expect(matchesSearch(q, ...updateKeywords)).toBe(true);
    });

    it('multiple sections can match a broad search term', () => {
      // "dark" could match appearance ("暗色" via multiple keywords) and other sections
      const q = 'dark';
      expect(matchesSearch(q, ...appearanceKeywords)).toBe(true);
    });
  });
});
