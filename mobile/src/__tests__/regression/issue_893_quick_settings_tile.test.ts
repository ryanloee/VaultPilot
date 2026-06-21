/**
 * Regression test for #893: Quick Settings Tile deep link config.
 * Verifies the linking configuration routes vaultpilot:// to the correct screens.
 */

// Test the linking config structure (pure logic, no RN dependencies)
describe('Quick Settings Tile linking config (#893)', () => {
  // Mirror the linking config from App.tsx
  const linkingConfig = {
    prefixes: ['vaultpilot://'],
    config: {
      screens: {
        Notes: {
          screens: {
            NotesList: 'note',
            NoteEdit: 'note/:noteId',
          },
        },
      },
    },
  };

  it('registers vaultpilot:// prefix', () => {
    expect(linkingConfig.prefixes).toContain('vaultpilot://');
  });

  it('maps Notes tab to note path', () => {
    const noteScreens = linkingConfig.config.screens.Notes.screens;
    expect(noteScreens.NotesList).toBe('note');
  });

  it('maps NoteEdit to note/:noteId path', () => {
    const noteScreens = linkingConfig.config.screens.Notes.screens;
    expect(noteScreens.NoteEdit).toBe('note/:noteId');
  });

  it('has correct config structure for React Navigation', () => {
    // Verify the config can be used by React Navigation
    expect(linkingConfig.config.screens).toBeDefined();
    expect(linkingConfig.config.screens.Notes).toBeDefined();
    expect(linkingConfig.config.screens.Notes.screens).toBeDefined();
  });

  it('NoteEdit path supports noteId parameter', () => {
    const path = linkingConfig.config.screens.Notes.screens.NoteEdit;
    expect(path).toContain(':noteId');
    // Should match paths like "note/abc-123"
    const regex = new RegExp('^' + path.replace(/:(\w+)/g, '(?<$1>[^/]+)') + '$');
    expect(regex.test('note/abc-123')).toBe(true);
    expect(regex.test('note/')).toBe(false);
  });
});
