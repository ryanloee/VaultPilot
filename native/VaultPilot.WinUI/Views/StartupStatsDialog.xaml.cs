using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Views;

/// <summary>
/// Dialog showing fine-grained startup phase timings reported by the backend
/// agent via the <c>startupStats</c> JSON-RPC method (issue #3910).
/// Each phase is rendered as "<c>name  N.N ms</c>" plus a total row.
/// When no statistics are available (backend not connected, method
/// unsupported, or empty result), a friendly placeholder message is shown
/// instead of an empty list.
/// </summary>
public sealed partial class StartupStatsDialog : ContentDialog
{
    /// <summary>
    /// Creates a new startup statistics dialog.
    /// </summary>
    /// <param name="stats">Startup stats from the backend agent, or null when
    /// unavailable — the dialog then shows a friendly error state.</param>
    /// <param name="xamlRoot">XamlRoot from the parent window.</param>
    public StartupStatsDialog(StartupStatsResponse? stats, XamlRoot xamlRoot)
    {
        InitializeComponent();
        XamlRoot = xamlRoot;
        Populate(stats);
    }

    private void Populate(StartupStatsResponse? stats)
    {
        var phases = stats?.Phases;
        if (phases is null || phases.Count == 0)
        {
            PhasePanel.Children.Add(new TextBlock
            {
                Text = "后端未连接，无法获取启动统计。",
                FontSize = 14,
                TextWrapping = TextWrapping.Wrap,
                Foreground = new SolidColorBrush(Microsoft.UI.Colors.Gray)
            });
            return;
        }

        foreach (var phase in phases)
        {
            PhasePanel.Children.Add(new TextBlock
            {
                Text = $"{phase.Name}  {phase.ElapsedMs:F1} ms",
                FontFamily = new FontFamily("Cascadia Code"),
                FontSize = 13,
                TextWrapping = TextWrapping.Wrap
            });
        }

        PhasePanel.Children.Add(new Microsoft.UI.Xaml.Shapes.Rectangle
        {
            Height = 1,
            Margin = new Thickness(0, 10, 0, 4),
            Fill = new SolidColorBrush(Microsoft.UI.Colors.Gray)
        });

        PhasePanel.Children.Add(new TextBlock
        {
            Text = $"总计  {stats!.TotalMs:F1} ms",
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            FontSize = 14
        });
    }
}
