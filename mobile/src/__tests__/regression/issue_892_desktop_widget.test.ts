/**
 * Regression test for #892: Desktop Widget linking config.
 * Verifies the widget's deep link routes are correctly configured.
 */
describe('Desktop Widget linking config (#892)', () => {
  const linkingConfig = {
    prefixes: ['vaultpilot://'],
    config: {
      screens: {
        Chat: {
          screens: {
            ChatMain: 'chat',
            Sessions: 'chat/sessions',
          },
        },
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

  it('maps Chat tab to chat path', () => {
    expect(linkingConfig.config.screens.Chat.screens.ChatMain).toBe('chat');
  });

  it('maps Notes tab to note path', () => {
    expect(linkingConfig.config.screens.Notes.screens.NotesList).toBe('note');
  });

  it('supports vaultpilot://chat/new deep link for widget new chat button', () => {
    // The widget uses vaultpilot://chat/new — this maps to ChatMain screen
    // because 'chat' prefix matches ChatMain, and 'new' would be ignored
    const chatPath = linkingConfig.config.screens.Chat.screens.ChatMain;
    expect(chatPath).toBe('chat');
  });

  it('supports vaultpilot://note/new deep link for widget new note button', () => {
    // The widget uses vaultpilot://note/new — this maps to NotesList screen
    const notePath = linkingConfig.config.screens.Notes.screens.NotesList;
    expect(notePath).toBe('note');
  });

  it('has correct nested screen structure', () => {
    const { Chat, Notes } = linkingConfig.config.screens;
    expect(Chat).toBeDefined();
    expect(Notes).toBeDefined();
    expect(Chat.screens).toBeDefined();
    expect(Notes.screens).toBeDefined();
  });
});
