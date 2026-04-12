using Microsoft.UI.Xaml.Controls;
using Velopack;
using Velopack.Sources;

namespace VaultPilot.WinUI;

public sealed partial class MainWindow
{
    private const string UpdateRepoUrl = "https://github.com/ryanloee/VaultPilot";
    private UpdateManager? _updateManager;
    private bool _updateCheckStarted;

    private async Task CheckForAppUpdatesAsync()
    {
        if (_updateCheckStarted)
        {
            return;
        }

        _updateCheckStarted = true;

        try
        {
            _updateManager ??= CreateUpdateManager();
            if (_updateManager is null || !_updateManager.IsInstalled)
            {
                LogStartup("Update check skipped: application is not installed via Velopack.");
                UpdateStatusBar("warning", "\u672a\u542f\u7528\u81ea\u52a8\u66f4\u65b0", "\u5f53\u524d\u5b89\u88c5\u5305\u4e0d\u652f\u6301 Velopack \u5728\u7ebf\u5347\u7ea7\u3002");
                return;
            }

            if (_updateManager.UpdatePendingRestart is { } pendingUpdate)
            {
                await PromptToInstallUpdateAsync(pendingUpdate);
                return;
            }

            var update = await _updateManager.CheckForUpdatesAsync();
            if (update is null)
            {
                LogStartup("Update check complete: no updates available.");
                UpdateStatusBar("success", "\u5df2\u68c0\u67e5\u66f4\u65b0", "\u5f53\u524d\u5df2\u662f\u6700\u65b0\u7248\u672c\u3002");
                return;
            }

            var version = update.TargetFullRelease.Version.ToFullString();
            LogStartup($"Update available: {version}");
            UpdateStatusBar("info", "\u53d1\u73b0\u65b0\u7248\u672c", $"\u6b63\u5728\u4e0b\u8f7d {version}...");
            await _updateManager.DownloadUpdatesAsync(update);

            UpdateStatusBar("warning", "\u66f4\u65b0\u5df2\u4e0b\u8f7d", $"\u7248\u672c {version} \u5df2\u51c6\u5907\u5c31\u7eea\u3002");
            await PromptToInstallUpdateAsync(update.TargetFullRelease);
        }
        catch (Exception error)
        {
            LogStartup($"Update check failed: {error}");
            UpdateStatusBar("warning", "\u66f4\u65b0\u68c0\u67e5\u5931\u8d25", LocalizeError(error.Message));
        }
    }

    private async Task PromptToInstallUpdateAsync(VelopackAsset update)
    {
        var version = update.Version.ToFullString();
        var dialog = new ContentDialog
        {
            XamlRoot = RootGrid.XamlRoot,
            Title = "\u66f4\u65b0\u53ef\u7528",
            Content = $"\u7248\u672c {version} \u5df2\u4e0b\u8f7d\u5b8c\u6210\u3002\u662f\u5426\u73b0\u5728\u91cd\u542f\u5e76\u5b8c\u6210\u66f4\u65b0\uff1f",
            PrimaryButtonText = "\u7acb\u5373\u66f4\u65b0",
            CloseButtonText = "\u7a0d\u540e"
        };

        if (await dialog.ShowAsync() != ContentDialogResult.Primary)
        {
            UpdateStatusBar("warning", "\u66f4\u65b0\u5f85\u5b89\u88c5", $"\u7248\u672c {version} \u5c06\u5728\u4e0b\u6b21\u542f\u52a8\u65f6\u81ea\u52a8\u5b89\u88c5\u3002");
            return;
        }

        try
        {
            UpdateStatusBar("info", "\u6b63\u5728\u51c6\u5907\u66f4\u65b0", "\u5173\u95ed\u5e94\u7528\u540e\u4f1a\u81ea\u52a8\u5b89\u88c5\u65b0\u7248\u672c\u3002");
            _updateManager?.WaitExitThenApplyUpdates(update, silent: false, restart: true);
            Close();
        }
        catch (Exception error)
        {
            LogStartup($"Failed to apply update {version}: {error}");
            ShowError("\u542f\u52a8\u66f4\u65b0\u5931\u8d25", error, addMessage: false);
        }
    }

    private static UpdateManager? CreateUpdateManager()
    {
        try
        {
            return new UpdateManager(new GithubSource(UpdateRepoUrl, string.Empty, false));
        }
        catch (Exception error)
        {
            LogStartup($"Failed to create update manager: {error}");
            return null;
        }
    }
}
