using Microsoft.UI.Xaml;
using System.Diagnostics;
using VaultPilot.WinUI.Views;

namespace VaultPilot.WinUI;

/// <summary>
/// File Recovery UI (issue #3960) — opens the <see cref="FileRecoveryDialog"/>
/// to browse, preview, and restore vault-EXTERNAL crash-recovery snapshots
/// (src/recovery.rs). Invoked from the tray context menu
/// ('文件恢复…' in App.InitializeTrayIcon), mirroring the startup stats
/// dialog pattern (#3910).
/// </summary>
public sealed partial class MainWindow : Window
{
    /// <summary>
    /// Ensures the backend agent is connected, then opens the file recovery
    /// dialog. SendAsync failures surface as an error InfoBar inside the
    /// dialog itself.
    /// </summary>
    public async void ShowFileRecoveryDialog()
    {
        try
        {
            // #3960: make sure the sidecar is up before asking for snapshots —
            // SendAsync would otherwise throw "Rust 后端尚未连接。".
            if (!_backendClient.IsConnected)
            {
                var reconnected = await _backendClient.EnsureConnectedAsync();
                if (!reconnected)
                {
                    Trace.TraceWarning("ShowFileRecoveryDialog: backend not connected, dialog will show error state.");
                }
            }

            var dialog = new FileRecoveryDialog(_backendClient, RootGrid.XamlRoot);
            await dialog.ShowAsync();
        }
        catch (Exception ex)
        {
            Trace.TraceError($"ShowFileRecoveryDialog: failed to show dialog: {ex}");
        }
    }
}
