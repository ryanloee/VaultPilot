using Xunit;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using VaultPilot.WinUI.Views;
using System.Collections.Generic;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3626: OnNotesListRightTapped walks the visual
/// tree via VisualTreeHelper.GetParent to find a ListViewItem ancestor. For
/// virtualized ListView items, no ListViewItem container exists, so the walk
/// fails silently and the selected note is never updated.
///
/// Fix: Added a fallback hit-test strategy (ListViewHitTestHelper) that uses
/// FindElementsInHostCoordinates to locate the item by pointer position when
/// the visual-tree walk fails. The core logic — resolving a DataContext from
/// a list of hit elements — is extracted into a pure helper for testing.
/// </summary>
public class Issue3626VirtualizedRightTapTests
{
    [Fact]
    public void Regression_3626_ResolveDataContextReturnsMatchingItem()
    {
        // Simulate a hit-test result where the FrameworkElement's DataContext
        // matches one of the known items in the ListView.
        var knownItem1 = new { Id = "note-1", Title = "Note 1" };
        var knownItem2 = new { Id = "note-2", Title = "Note 2" };
        var knownItems = new List<object> { knownItem1, knownItem2 };

        // Create a mock hit element with knownItem2 as DataContext
        var textBlock = new TextBlock { DataContext = knownItem2 };
        var hits = new List<UIElement> { textBlock };

        var result = ListViewHitTestHelper.ResolveDataContextFromHits(hits, knownItems);
        Assert.Equal(knownItem2, result);
    }

    [Fact]
    public void Regression_3626_ResolveDataContextReturnsNullWhenNoMatch()
    {
        // When none of the hit elements have a DataContext matching a known
        // item, the method returns null (no selection change).
        var knownItem = new { Id = "note-1", Title = "Note 1" };
        var knownItems = new List<object> { knownItem };

        var unrelated = new { Id = "other", Title = "Other" };
        var textBlock = new TextBlock { DataContext = unrelated };
        var hits = new List<UIElement> { textBlock };

        var result = ListViewHitTestHelper.ResolveDataContextFromHits(hits, knownItems);
        Assert.Null(result);
    }

    [Fact]
    public void Regression_3626_ResolveDataContextReturnsNullForEmptyHits()
    {
        var knownItems = new List<object> { new { Id = "note-1" } };
        var hits = new List<UIElement>();

        var result = ListViewHitTestHelper.ResolveDataContextFromHits(hits, knownItems);
        Assert.Null(result);
    }

    [Fact]
    public void Regression_3626_ResolveDataContextSkipsElementsWithoutDataContext()
    {
        var knownItem = new { Id = "note-1", Title = "Note 1" };
        var knownItems = new List<object> { knownItem };

        // First element has no DataContext, second has the match
        var noContext = new Border();
        var withMatch = new TextBlock { DataContext = knownItem };
        var hits = new List<UIElement> { noContext, withMatch };

        var result = ListViewHitTestHelper.ResolveDataContextFromHits(hits, knownItems);
        Assert.Equal(knownItem, result);
    }
}
