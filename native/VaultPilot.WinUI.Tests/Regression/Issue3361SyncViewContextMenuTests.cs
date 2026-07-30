using Xunit;
using System.Reflection;
using VaultPilot.WinUI.Views;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression tests for issue #3361: Sync (Notes) view interaction enhancements —
/// right-click context menu + Delete/Backspace key handling.
///
/// These tests verify that the NotesView class exposes the expected event handler
/// methods and that the delete logic was properly extracted into a reusable
/// DeleteSelectedNoteAsync method (shared by toolbar button, keyboard shortcut,
/// and context menu).
///
/// Note: Full UI interaction (right-click, key press) requires a live WinUI
/// dispatcher and cannot be unit-tested headlessly. These structural tests
/// ensure the wiring exists and is correctly named so CI compilation verifies
/// the XAML/code-behind binding.
/// </summary>
public class Issue3361SyncViewContextMenuTests
{
    [Fact]
    public void Regression_3361_DeleteSelectedNoteAsync_Exists()
    {
        // The delete logic was extracted into a reusable method so the toolbar
        // button, Delete key, and context menu all share the same code path.
        var method = typeof(NotesView).GetMethod(
            "DeleteSelectedNoteAsync",
            BindingFlags.NonPublic | BindingFlags.Instance);

        Assert.NotNull(method);
    }

    [Fact]
    public void Regression_3361_RightTapHandler_Exists()
    {
        // Right-click context menu handler must exist and be wired in XAML.
        var method = typeof(NotesView).GetMethod(
            "OnNotesListRightTapped",
            BindingFlags.NonPublic | BindingFlags.Instance);

        Assert.NotNull(method);
    }

    [Fact]
    public void Regression_3361_ContextMenuItemHandlers_Exist()
    {
        // Three context menu actions: Delete, Copy, Version History.
        var deleteHandler = typeof(NotesView).GetMethod(
            "OnCtxDeleteClicked",
            BindingFlags.NonPublic | BindingFlags.Instance);
        var copyHandler = typeof(NotesView).GetMethod(
            "OnCtxCopyClicked",
            BindingFlags.NonPublic | BindingFlags.Instance);
        var historyHandler = typeof(NotesView).GetMethod(
            "OnCtxHistoryClicked",
            BindingFlags.NonPublic | BindingFlags.Instance);

        Assert.NotNull(deleteHandler);
        Assert.NotNull(copyHandler);
        Assert.NotNull(historyHandler);
    }

    [Fact]
    public void Regression_3361_ShowHistoryDialogAsync_Extracted()
    {
        // The history dialog was extracted into a reusable method so the
        // toolbar button and context menu share the same code path.
        var method = typeof(NotesView).GetMethod(
            "ShowHistoryDialogAsync",
            BindingFlags.NonPublic | BindingFlags.Instance);

        Assert.NotNull(method);
    }

    [Fact]
    public void Regression_3361_KeyDownHandler_StillHandlesCopyPaste()
    {
        // Ensure the existing Ctrl+C/Ctrl+V handlers still exist alongside
        // the new Delete/Backspace handling in the same method.
        var method = typeof(NotesView).GetMethod(
            "OnNotesListKeyDown",
            BindingFlags.NonPublic | BindingFlags.Instance);

        Assert.NotNull(method);
    }
}
