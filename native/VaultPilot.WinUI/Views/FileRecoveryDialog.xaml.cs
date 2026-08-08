using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Diagnostics;
using System.Linq;
using System.Threading;
using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Views;

/// <summary>
/// File Recovery dialog (issue #3960) — browses, previews, and restores the
/// vault-EXTERNAL crash-recovery snapshots produced by the backend
/// (src/recovery.rs; the CLI equivalent is <c>vp recovery list/show/restore</c>).
/// Communicates with the Rust agent via JSON-RPC methods:
/// <c>recoveryList</c>, <c>recoveryShow</c>, <c>recoveryRestore</c>,
/// <c>recoveryDelete</c>. Distinct from the in-vault note version snapshots
/// (#2855, VersionHistoryControl).
/// </summary>
public sealed partial class FileRecoveryDialog : ContentDialog
{
    private readonly BackendClient _backendClient;
    private IReadOnlyList<RecoverySnapshotInfo> _allSnapshots = Array.Empty<RecoverySnapshotInfo>();
    private CancellationTokenSource? _loadDetailCts;
    private string _filterQuery = string.Empty;
    private bool _isLoading;

    /// <summary>
    /// Creates a new file recovery dialog.
    /// </summary>
    /// <param name="backendClient">Connected backend client for JSON-RPC calls.</param>
    /// <param name="xamlRoot">XamlRoot from the parent window.</param>
    public FileRecoveryDialog(BackendClient backendClient, XamlRoot xamlRoot)
    {
        _backendClient = backendClient;
        InitializeComponent();
        XamlRoot = xamlRoot;

        // Button/selection/filter handlers are wired in XAML (same pattern as
        // NotesView); only the Loaded hook is attached here.
        Loaded += OnLoaded;
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        try { await LoadSnapshotsAsync(); }
        catch (Exception ex) { Debug.WriteLine($"[FileRecoveryDialog] OnLoaded error: {ex.Message}"); }
    }

    private async void OnRefreshClicked(object sender, RoutedEventArgs e)
    {
        try { await LoadSnapshotsAsync(); }
        catch (Exception ex) { Debug.WriteLine($"[FileRecoveryDialog] OnRefreshClicked error: {ex.Message}"); }
    }

    /// <summary>
    /// Loads the snapshot list from the backend (<c>recoveryList</c>), sorted
    /// newest first. Disables interaction while in flight; errors are shown
    /// in the status InfoBar.
    /// </summary>
    private async Task LoadSnapshotsAsync()
    {
        SetLoading(true);
        _loadDetailCts?.Cancel();
        _loadDetailCts?.Dispose();
        _loadDetailCts = null;

        try
        {
            ClearStatus();
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(60));
            _allSnapshots = await _backendClient.RecoveryListAsync(token: cts.Token)
                ?? Array.Empty<RecoverySnapshotInfo>();
            ApplyFilter();
        }
        catch (OperationCanceledException)
        {
            ShowError("加载恢复快照超时", new Exception("后端响应超时（60 秒），请稍后重试。"));
        }
        catch (Exception error)
        {
            ShowError("加载恢复快照失败", error);
        }
        finally
        {
            SetLoading(false);
        }
    }

    private void OnFilterTextChanged(object sender, TextChangedEventArgs e)
    {
        _filterQuery = FilterBox.Text?.Trim() ?? string.Empty;
        ApplyFilter();
    }

    /// <summary>
    /// Filters <see cref="_allSnapshots"/> by path/title (case-insensitive),
    /// sorts newest first, and rebinds the ListView.
    /// </summary>
    private void ApplyFilter()
    {
        var filtered = string.IsNullOrEmpty(_filterQuery)
            ? _allSnapshots
            : _allSnapshots.Where(s =>
                (s.Title?.Contains(_filterQuery, StringComparison.OrdinalIgnoreCase) ?? false) ||
                (s.NotePath?.Contains(_filterQuery, StringComparison.OrdinalIgnoreCase) ?? false))
                .ToArray();

        var items = filtered
            .OrderByDescending(SortKey)
            .Select(s => new RecoverySnapshotItem(s))
            .ToList();

        SnapshotList.ItemsSource = items;
        UpdateEmptyState();
        UpdateActionButtons();
    }

    /// <summary>
    /// ISO-8601 string sort key with a parse fallback so unparseable
    /// timestamps still sort deterministically (oldest bucket).
    /// </summary>
    private static DateTimeOffset SortKey(RecoverySnapshotInfo snapshot)
    {
        return DateTimeOffset.TryParse(snapshot.CreatedAt, out var parsed)
            ? parsed
            : DateTimeOffset.MinValue;
    }

    private void UpdateEmptyState()
    {
        var count = SnapshotList.ItemsSource is IList<RecoverySnapshotItem> list ? list.Count : 0;
        if (_allSnapshots.Count == 0)
        {
            EmptyListText.Text = "没有可恢复的快照";
            EmptyListText.Visibility = Visibility.Visible;
        }
        else if (count == 0)
        {
            EmptyListText.Text = "没有匹配的快照";
            EmptyListText.Visibility = Visibility.Visible;
        }
        else
        {
            EmptyListText.Visibility = Visibility.Collapsed;
        }
    }

    private void OnSnapshotSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (SnapshotList.SelectedItem is not RecoverySnapshotItem item)
        {
            ClearPreview();
            return;
        }

        // Show the cheap list metadata immediately, then load content.
        PreviewMetaPanel.Visibility = Visibility.Visible;
        PreviewTitle.Text = item.Title;
        PreviewPath.Text = item.Snapshot.NotePath;
        PreviewTime.Text = item.TimeDisplay;
        PreviewSize.Text = item.SizeDisplay;
        PreviewBox.Text = string.Empty;
        PreviewBox.Visibility = Visibility.Visible;
        PreviewEmptyText.Text = "正在加载快照内容…";
        PreviewEmptyText.Visibility = Visibility.Visible;

        // Cancel any in-flight detail load for a previous selection (#2288).
        _loadDetailCts?.Cancel();
        _loadDetailCts?.Dispose();
        _loadDetailCts = new CancellationTokenSource();
        _ = LoadSnapshotDetailAsync(item, _loadDetailCts.Token);
        UpdateActionButtons();
    }

    /// <summary>
    /// Fetches the full snapshot content via <c>recoveryShow</c> and displays
    /// it in the read-only preview. A superseded selection cancels this load.
    /// </summary>
    private async Task LoadSnapshotDetailAsync(RecoverySnapshotItem item, CancellationToken cancellationToken)
    {
        try
        {
            var detail = await _backendClient.RecoveryShowAsync(item.Snapshot.Id, cancellationToken);
            if (cancellationToken.IsCancellationRequested)
                return;
            if (detail is null)
            {
                ShowError("加载快照内容失败", new Exception("后端返回了空结果。"));
                return;
            }

            // Belt-and-suspenders: only apply if the selection hasn't moved on
            // (the token normally covers this).
            if (SnapshotList.SelectedItem is RecoverySnapshotItem current
                && current.Snapshot.Id == item.Snapshot.Id)
            {
                PreviewBox.Text = detail.Content ?? string.Empty;
                PreviewBox.Visibility = Visibility.Visible;
                PreviewEmptyText.Visibility = Visibility.Collapsed;
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // A newer selection has superseded this one — don't update UI.
        }
        catch (Exception error)
        {
            if (!cancellationToken.IsCancellationRequested)
            {
                ShowError("加载快照内容失败", error);
                PreviewBox.Text = string.Empty;
                PreviewBox.Visibility = Visibility.Visible;
                PreviewEmptyText.Text = "快照内容加载失败";
                PreviewEmptyText.Visibility = Visibility.Visible;
            }
        }
    }

    private async void OnRestoreClicked(object sender, RoutedEventArgs e)
    {
        if (SnapshotList.SelectedItem is not RecoverySnapshotItem item)
            return;

        // Confirm first, showing the target (vault-relative) note path.
        var confirm = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = "恢复快照",
            Content = $"确认将快照恢复到笔记「{item.Title}」吗？\n\n目标路径：{item.Snapshot.NotePath}\n\n此操作会覆盖该路径下的现有内容。",
            PrimaryButtonText = "恢复",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close
        };

        if (await confirm.ShowAsync() != ContentDialogResult.Primary)
            return;

        try
        {
            SetLoading(true);
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(60));
            var result = await _backendClient.RecoveryRestoreAsync(item.Snapshot.Id, cts.Token);
            if (result is null || !result.Ok)
            {
                ShowError("恢复失败", new Exception(result is null ? "后端返回了空结果。" : "后端未能恢复快照。"));
                return;
            }
            ShowSuccess($"已恢复到 {result.NotePath}（{FormatBytes(result.BytesWritten)}）");
        }
        catch (OperationCanceledException)
        {
            ShowError("恢复超时", new Exception("后端响应超时，请稍后重试。"));
        }
        catch (Exception error)
        {
            ShowError("恢复失败", error);
        }
        finally
        {
            SetLoading(false);
        }
    }

    private async void OnDeleteClicked(object sender, RoutedEventArgs e)
    {
        if (SnapshotList.SelectedItem is not RecoverySnapshotItem item)
            return;

        var confirm = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = "删除快照",
            Content = $"确认删除快照「{item.Title}」（{item.TimeDisplay}）吗？此操作不可撤销。",
            PrimaryButtonText = "删除",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close
        };

        if (await confirm.ShowAsync() != ContentDialogResult.Primary)
            return;

        try
        {
            SetLoading(true);
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var result = await _backendClient.RecoveryDeleteAsync(item.Snapshot.Id, cts.Token);
            if (result is null || !result.Ok)
            {
                ShowError("删除失败", new Exception(result is null ? "后端返回了空结果。" : "后端未能删除快照。"));
                return;
            }

            // Remove locally and rebind — the SelectionChanged handler clears
            // the preview and disables the action buttons.
            _allSnapshots = _allSnapshots.Where(s => s.Id != item.Snapshot.Id).ToArray();
            ClearPreview();
            ApplyFilter();
            ShowSuccess("快照已删除");
        }
        catch (OperationCanceledException)
        {
            ShowError("删除超时", new Exception("后端响应超时，请稍后重试。"));
        }
        catch (Exception error)
        {
            ShowError("删除失败", error);
        }
        finally
        {
            SetLoading(false);
        }
    }

    private void ClearPreview()
    {
        _loadDetailCts?.Cancel();
        _loadDetailCts?.Dispose();
        _loadDetailCts = null;
        PreviewMetaPanel.Visibility = Visibility.Collapsed;
        PreviewBox.Text = string.Empty;
        PreviewBox.Visibility = Visibility.Collapsed;
        PreviewEmptyText.Text = "选择一个快照查看内容";
        PreviewEmptyText.Visibility = Visibility.Visible;
    }

    private void SetLoading(bool loading)
    {
        _isLoading = loading;
        LoadingRing.IsActive = loading;
        LoadingRing.Visibility = loading ? Visibility.Visible : Visibility.Collapsed;
        RefreshButton.IsEnabled = !loading;
        FilterBox.IsEnabled = !loading;
        UpdateActionButtons();
    }

    private void UpdateActionButtons()
    {
        var hasSelection = SnapshotList.SelectedItem is not null;
        RestoreButton.IsEnabled = !_isLoading && hasSelection;
        DeleteButton.IsEnabled = !_isLoading && hasSelection;
    }

    private void ClearStatus()
    {
        StatusInfoBar.IsOpen = false;
    }

    private void ShowError(string title, Exception error)
    {
        Debug.WriteLine($"[FileRecoveryDialog] {title}: {error.Message}");
        StatusInfoBar.Severity = InfoBarSeverity.Error;
        StatusInfoBar.Title = title;
        StatusInfoBar.Message = error.Message;
        StatusInfoBar.IsOpen = true;
    }

    private void ShowSuccess(string message)
    {
        StatusInfoBar.Severity = InfoBarSeverity.Success;
        StatusInfoBar.Title = "操作成功";
        StatusInfoBar.Message = message;
        StatusInfoBar.IsOpen = true;
    }

    /// <summary>
    /// Formats a byte count as a compact human-readable string (e.g. "1.2 KB").
    /// </summary>
    internal static string FormatBytes(long bytes)
    {
        if (bytes < 1024) return $"{bytes} B";
        if (bytes < 1024 * 1024) return $"{bytes / 1024.0:F1} KB";
        if (bytes < 1024L * 1024 * 1024) return $"{bytes / (1024.0 * 1024):F1} MB";
        return $"{bytes / (1024.0 * 1024 * 1024):F1} GB";
    }
}

/// <summary>
/// Display wrapper around <see cref="RecoverySnapshotInfo"/> for data binding
/// in the snapshot ListView (same pattern as <see cref="NoteListItem"/>).
/// </summary>
public sealed class RecoverySnapshotItem
{
    public RecoverySnapshotInfo Snapshot { get; }

    public string Title => Snapshot.Title;
    public string PathDisplay => Snapshot.NotePath;
    public string TimeDisplay => NotesView.FormatRelativeTime(Snapshot.CreatedAt);
    public string SizeDisplay => FileRecoveryDialog.FormatBytes(Snapshot.ContentSize);

    public RecoverySnapshotItem(RecoverySnapshotInfo snapshot)
    {
        Snapshot = snapshot;
    }
}
