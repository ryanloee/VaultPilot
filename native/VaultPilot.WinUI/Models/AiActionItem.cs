using Microsoft.UI.Xaml;

namespace VaultPilot.WinUI.Models;

/// <summary>
/// Display model for an AI quick-action item shown in the command palette.
/// </summary>
public sealed class AiActionItem
{
    /// <summary>The action type.</summary>
    public AiActionType ActionType { get; set; }

    /// <summary>Human-readable label.</summary>
    public string Label { get; set; } = string.Empty;

    /// <summary>Longer description shown under the label.</summary>
    public string Description { get; set; } = string.Empty;

    /// <summary>Segoe Fluent icon glyph.</summary>
    public string IconGlyph { get; set; } = string.Empty;

    /// <summary>Optional keyboard shortcut hint (e.g. "Ctrl+Shift+S").</summary>
    public string ShortcutHint { get; set; } = string.Empty;

    /// <summary>
    /// Collapsed when <see cref="ShortcutHint"/> is empty, otherwise Visible.
    /// </summary>
    public Visibility ShortcutVisibility =>
        string.IsNullOrEmpty(ShortcutHint) ? Visibility.Collapsed : Visibility.Visible;
}
