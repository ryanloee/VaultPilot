using Xunit;
using VaultPilot.WinUI.Views;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3625: the finally block in
/// LoadRelatedNotesAsync checked the shared RelatedNotesList.ItemsSource to
/// decide panel visibility. When two overlapping calls raced, a cancelled
/// earlier call's finally block could collapse the panel even though a later
/// call had successfully loaded items.
///
/// Fix: the caller now passes a local boolean (loadProducedItems) tracking
/// whether *this specific* invocation produced items, rather than reading the
/// shared mutable ItemsSource. The ShouldKeepRelatedNotesPanelVisible method
/// signature changed from (bool, object?) to (bool, bool).
/// </summary>
public class Issue3625StaleStatePanelTests
{
    [Fact]
    public void Regression_3625_CancelledCallDoesNotKeepPanelVisible()
    {
        // A cancelled call should never keep the panel visible, even if
        // loadProducedItems was set before cancellation.
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(true, true));
    }

    [Fact]
    public void Regression_3625_CompletedCallWithItemsKeepsPanelVisible()
    {
        // A completed call that produced items keeps the panel visible.
        Assert.True(NotesView.ShouldKeepRelatedNotesPanelVisible(false, true));
    }

    [Fact]
    public void Regression_3625_CancelledCallWithNoItemsCollapsesPanel()
    {
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(true, false));
    }

    [Fact]
    public void Regression_3625_CompletedCallWithNoItemsCollapsesPanel()
    {
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(false, false));
    }
}
