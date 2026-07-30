using Xunit;
using VaultPilot.WinUI.Views;
using VaultPilot.WinUI.Models;
using System.Collections.Generic;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #2780: the related-notes panel in NotesView was
/// collapsed unconditionally in LoadRelatedNotesAsync's finally block, so even
/// successfully loaded related notes were immediately hidden and the feature
/// appeared completely broken.
///
/// Fix: ShouldKeepRelatedNotesPanelVisible keeps the panel visible only when the
/// load completed (not cancelled) and produced at least one entry (a real
/// related note or the "no related notes" placeholder).
///
/// Updated for #3625: the method now accepts a bool (loadProducedItems) instead
/// of the raw ItemsSource object, to avoid stale-state decisions when multiple
/// overlapping LoadRelatedNotesAsync calls race.
/// </summary>
public class Issue2780NotesPanelTests
{
    [Fact]
    public void Regression_2780_PanelStaysVisibleWhenRelatedNotesLoaded()
    {
        Assert.True(NotesView.ShouldKeepRelatedNotesPanelVisible(false, true));
    }

    [Fact]
    public void Regression_2780_PanelStaysVisibleForNoRelatedNotesPlaceholder()
    {
        // The "no related notes" placeholder is a single-entry list, so the
        // panel should remain visible to show the message.
        Assert.True(NotesView.ShouldKeepRelatedNotesPanelVisible(false, true));
    }

    [Fact]
    public void Regression_2780_PanelCollapsesOnCancellation()
    {
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(true, true));
    }

    [Fact]
    public void Regression_2780_PanelCollapsesWhenEmpty()
    {
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(false, false));
    }

    [Fact]
    public void Regression_2780_PanelCollapsesWhenNull()
    {
        // loadProducedItems = false means no items were loaded
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(false, false));
    }
}
