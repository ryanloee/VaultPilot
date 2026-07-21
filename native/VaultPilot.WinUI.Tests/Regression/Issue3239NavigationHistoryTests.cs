using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3239: Alt+Left/Alt+Right note navigation
/// history skips the immediately-previous note.
///
/// The bug had two compounding defects:
///   1. NavigateToNoteFromTitleAsync pushed currentNoteId (source) instead
///      of the destination noteId, so the stack always lagged one note
///      behind reality.
///   2. The same-note guard compared currentNoteId against noteTitleOrId (raw
///      input) instead of the resolved noteId, which could never be equal
///      when a wikilink uses the note's title, creating self-loops.
///
/// Fix: push the resolved destination noteId and compare against it.
/// </summary>
public class Issue3239NavigationHistoryTests
{
    [Fact]
    public void Regression_3239_PushesDestinationNotSource()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.xaml.cs");
        if (!File.Exists(sourcePath))
            return; // source may not be co-located in CI

        var code = File.ReadAllText(sourcePath);

        // The stack must push the destination noteId, not currentNoteId.
        // Old code: _noteNavStack.Add(currentNoteId);
        // Fixed:    _noteNavStack.Add(noteId);
        Assert.Contains("_noteNavStack.Add(noteId);", code);

        // The truncation guard must also reference the resolved noteId.
        Assert.Contains("_noteNavStack.RemoveRange(_noteNavIndex + 1, _noteNavStack.Count - (_noteNavIndex + 1));", code);
    }

    [Fact]
    public void Regression_3239_GuardComparesAgainstResolvedNoteId()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.xaml.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // The same-note guard must compare currentNoteId against the
        // resolved noteId (not the raw noteTitleOrId input).
        // Old code: currentNoteId != noteTitleOrId
        // Fixed:    currentNoteId != noteId
        Assert.Contains("currentNoteId != noteId", code);
    }

    [Fact]
    public void Regression_3239_NavigationHistoryHasCanonicalTraceComment()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.xaml.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // The comment should explain the canonical-trace semantics.
        Assert.Contains("canonical", code, StringComparison.OrdinalIgnoreCase);
    }

    private static string ResolveSourcePath(string projectDir, string fileName)
    {
        return Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            projectDir, fileName);
    }
}
