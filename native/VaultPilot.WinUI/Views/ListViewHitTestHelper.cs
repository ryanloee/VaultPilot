using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using System.Collections.Generic;

namespace VaultPilot.WinUI.Views;

/// <summary>
/// Helper for hit-testing a ListView by pointer position. Used as a fallback
/// when the visual-tree walk for finding a ListViewItem ancestor fails due to
/// virtualization (#3626).
///
/// Virtualized ListView items may not have a ListViewItem container in the
/// visual tree (the container has been recycled or not yet materialized), so
/// the standard VisualTreeHelper.GetParent walk fails to locate the item.
/// This helper uses FindElementsInHostCoordinates to locate the element at the
/// pointer position and resolve its DataContext.
/// </summary>
public static class ListViewHitTestHelper
{
    /// <summary>
    /// Finds the data item (NoteItem) at the right-tap pointer position
    /// within the given ListView. Returns null if no item is found.
    /// </summary>
    public static object? FindItemFromPoint(ListView listView, RightTappedRoutedEventArgs e)
    {
        try
        {
            // GetPosition(null) returns the pointer position in host
            // (window-root) coordinates, which is what FindElementsInHostCoordinates
            // expects.
            var hostPoint = e.GetPosition(null);

            // FindElementsInHostCoordinates returns all elements at the given
            // point (in host/absolute coordinates), listed by inverse z-order.
            // We iterate to find one whose DataContext matches an item in the
            // ListView's Items collection.
            var hits = VisualTreeHelper.FindElementsInHostCoordinates(hostPoint, listView);
            foreach (var hit in hits)
            {
                if (hit is FrameworkElement fe && fe.DataContext is not null)
                {
                    // Verify this DataContext is actually one of our items
                    if (listView.Items.Contains(fe.DataContext))
                    {
                        return fe.DataContext;
                    }
                }
            }
        }
        catch
        {
            // Hit-testing can throw for virtualized containers; ignore
        }
        return null;
    }

    /// <summary>
    /// Pure helper: given a list of hit elements and a set of known items,
    /// returns the first element's DataContext that appears in the known items.
    /// Extracted for unit testing without a real visual tree.
    /// </summary>
    public static object? ResolveDataContextFromHits(
        IEnumerable<UIElement> hits,
        IReadOnlyList<object> knownItems)
    {
        foreach (var hit in hits)
        {
            if (hit is FrameworkElement fe && fe.DataContext is not null)
            {
                foreach (var item in knownItems)
                {
                    if (ReferenceEquals(fe.DataContext, item) || fe.DataContext.Equals(item))
                    {
                        return fe.DataContext;
                    }
                }
            }
        }
        return null;
    }
}
