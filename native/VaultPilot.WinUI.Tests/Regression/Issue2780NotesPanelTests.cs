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
/// </summary>
public class Issue2780NotesPanelTests
{
    private static List<RelatedNoteItem> MakeItems(int count)
    {
        var list = new List<RelatedNoteItem>();
        for (int i = 0; i < count; i++)
        {
            list.Add(new RelatedNoteItem(new RelatedNote(new NoteMeta { Title = $"note {i}" }, 0, null)));
        }
        return list;
    }

    [Fact]
    public void Regression_2780_PanelStaysVisibleWhenRelatedNotesLoaded()
    {
        var items = MakeItems(3);
        Assert.True(NotesView.ShouldKeepRelatedNotesPanelVisible(false, items));
    }

    [Fact]
    public void Regression_2780_PanelStaysVisibleForNoRelatedNotesPlaceholder()
    {
        // The "no related notes" placeholder is a single-entry list, so the
        // panel should remain visible to show the message.
        var items = MakeItems(1);
        Assert.True(NotesView.ShouldKeepRelatedNotesPanelVisible(false, items));
    }

    [Fact]
    public void Regression_2780_PanelCollapsesOnCancellation()
    {
        var items = MakeItems(3);
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(true, items));
    }

    [Fact]
    public void Regression_2780_PanelCollapsesWhenEmpty()
    {
        var items = new List<RelatedNoteItem>();
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(false, items));
    }

    [Fact]
    public void Regression_2780_PanelCollapsesWhenNull()
    {
        Assert.False(NotesView.ShouldKeepRelatedNotesPanelVisible(false, null));
    }
}
