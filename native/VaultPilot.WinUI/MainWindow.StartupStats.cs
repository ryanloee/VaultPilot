using Microsoft.UI.Xaml;
using System.Diagnostics;
using VaultPilot.WinUI.Models;
using VaultPilot.WinUI.Views;

namespace VaultPilot.WinUI;

/// <summary>
/// Startup statistics UI (issue #3910) — fetches fine-grained startup phase
/// timings from the backend agent via the <c>startupStats</c> JSON-RPC method
/// and shows them in a dialog. Invoked from the tray context menu
/// ('启动耗时统计…' in App.InitializeTrayIcon).
/// </summary>
public sealed partial class MainWindow : Window
{
    /// <summary>
    /// Fetches startup phase timings from the backend agent and shows the
    /// <see cref="StartupStatsDialog"/>. When the backend cannot be reached or
    /// returns no data, the dialog itself shows a friendly error state
    /// ('后端未连接，无法获取启动统计。').
    /// </summary>
    public async void ShowStartupStatsDialog()
    {
        StartupStatsResponse? stats = null;
        try
        {
            // #3910: make sure the sidecar is up before asking for stats —
            // SendAsync would otherwise throw "Rust 后端尚未连接。".
            if (!_backendClient.IsConnected)
            {
                var reconnected = await _backendClient.EnsureConnectedAsync();
                if (!reconnected)
                {
                    Trace.TraceWarning("ShowStartupStatsDialog: backend not connected, showing empty state.");
                }
            }

            if (_backendClient.IsConnected)
            {
                stats = await _backendClient.GetStartupStatsAsync();
            }
        }
        catch (OperationCanceledException)
        {
            return;
        }
        catch (Exception ex)
        {
            Trace.TraceError($"ShowStartupStatsDialog: {ex}");
            // Fall through — the dialog shows the friendly empty state.
        }

        try
        {
            var dialog = new StartupStatsDialog(stats, RootGrid.XamlRoot);
            await dialog.ShowAsync();
        }
        catch (Exception ex)
        {
            Trace.TraceError($"ShowStartupStatsDialog: failed to show dialog: {ex}");
        }
    }
}
