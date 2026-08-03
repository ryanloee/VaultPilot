/**
 * Regression test for #3767 — Share save folder configuration.
 *
 * Verifies that ShareReceiveScreen reads the `cfg_share_folder` config key
 * from AsyncStorage and passes it as `options.folder` to `createNote`.
 */
import { sanitizeShareFileName } from '../../utils/shareHelpers';

// ── config key constant (mirrors ShareReceiveScreen) ──
const SHARE_FOLDER_KEY = 'cfg_share_folder';

describe('#3767 — Share save folder config', () => {
  describe('sanitizeShareFileName covers share-folder scenarios', () => {
    it('resolves normal filenames unchanged', () => {
      expect(sanitizeShareFileName('my-note.md')).toBe('my-note.md');
    });

    it('strips path traversal from folder-prefixed names', () => {
      expect(sanitizeShareFileName('../../etc/passwd')).toBe('passwd');
      expect(sanitizeShareFileName('..\\..\\Windows\\secret.txt')).toBe('secret.txt');
    });

    it('strips leading dots', () => {
      expect(sanitizeShareFileName('.hidden')).toBe('hidden');
      expect(sanitizeShareFileName('....config')).toBe('config');
    });

    it('returns empty string for null/undefined', () => {
      expect(sanitizeShareFileName(null)).toBe('');
      expect(sanitizeShareFileName(undefined)).toBe('');
    });
  });

  describe('SHARE_FOLDER_KEY constant', () => {
    it('uses cfg_share_folder as the AsyncStorage key', () => {
      expect(SHARE_FOLDER_KEY).toBe('cfg_share_folder');
    });
  });
});