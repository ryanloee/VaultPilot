using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Text;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace VaultPilot.WinUI.Controls;

/// <summary>
/// Version history panel for browsing, diffing, and restoring note snapshots.
/// Communicates with the Rust backend via JSON-RPC methods:
///   listSnapshots, getSnapshot, diffSnapshot, restoreSnapshot.
/// </summary>
public sealed partial class VersionHistoryControl : UserControl
{
    private readonly BackendClient _backendClient;
    private readonly string _noteId;
    private IReadOnlyList<NoteSnapshot> _snapshots = Array.Empty<NoteSnapshot>();
    private CancellationTokenSource? _diffCts;

    /// <summary>
    /// Raised when a snapshot has been successfully restored.
    /// </summary>
    public event EventHandler<EventArgs>? NoteRestored;

    public VersionHistoryControl(BackendClient backendClient, string noteId)
    {
        _backendClient = backendClient;
        _noteId = noteId;
        InitializeComponent();

        SnapshotList.SelectionChanged += OnSnapshotSelectionChanged;
        RestoreButton.Click += OnRestoreClicked;
        Loaded += OnLoaded;
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        await LoadSnapshotsAsync();
    }

    /// <summary>
    /// Load the list of snapshots for this note from the backend.
    /// </summary>
    private async Task LoadSnapshotsAsync()
    {
        try
        {
            var snapshots = await _backendClient.SendAsync<IReadOnlyList<NoteSnapshot>>(
                "listSnapshots", new { noteId = _noteId }, CancellationToken.None);

            _snapshots = snapshots ?? Array.Empty<NoteSnapshot>();

            if (_snapshots.Count > 0)
            {
                SnapshotList.ItemsSource = _snapshots;
                SnapshotList.SelectedIndex = 0;
            }
            else
            {
                // No snapshots — disable restore and show message
                RestoreButton.IsEnabled = false;
                DiffEmptyText.Text = "该笔记暂无版本快照";
            }
        }
        catch (Exception ex)
        {
            System.Diagnostics.Debug.WriteLine($"[VersionHistory] LoadSnapshotsAsync error: {ex.Message}");
            DiffEmptyText.Text = "加载快照列表失败";
        }
    }

    private async void OnSnapshotSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        DiffEmptyText.Visibility = Visibility.Collapsed;
        _diffCts?.Cancel();
        _diffCts?.Dispose();
        _diffCts = new CancellationTokenSource();

        if (SnapshotList.SelectedItem is NoteSnapshot snapshot)
        {
            RestoreButton.IsEnabled = true;
            await LoadDiffAsync(snapshot.Id, _diffCts.Token);
        }
        else
        {
            RestoreButton.IsEnabled = false;
        }
    }

    /// <summary>
    /// Load the diff between the selected snapshot and the current note version.
    /// </summary>
    private async Task LoadDiffAsync(string snapshotId, CancellationToken cancellationToken)
    {
        try
        {
            var diff = await _backendClient.SendAsync<DiffResult>(
                "diffSnapshot", new { noteId = _noteId, snapshotId = snapshotId }, cancellationToken);

            if (diff is null || cancellationToken.IsCancellationRequested)
                return;

            // Update stats
            DiffAdditions.Text = $"+{diff.Additions} 行添加";
            DiffDeletions.Text = $"-{diff.Deletions} 行删除";

            // Build diff UI
            var items = new List<DiffLineItem>();
            foreach (var hunk in diff.Hunks)
            {
                foreach (var line in hunk.Lines)
                {
                    items.Add(new DiffLineItem(line));
                }
            }
            DiffContent.ItemsSource = items;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex)
        {
            System.Diagnostics.Debug.WriteLine($"[VersionHistory] LoadDiffAsync error: {ex.Message}");
            DiffEmptyText.Text = "加载差异对比失败";
            DiffEmptyText.Visibility = Visibility.Visible;
        }
    }

    private async void OnRestoreClicked(object sender, RoutedEventArgs e)
    {
        if (SnapshotList.SelectedItem is not NoteSnapshot snapshot)
            return;

        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = "恢复版本",
            Content = $"确认恢复到 {snapshot.DisplayText} 的快照吗？当前版本将被保存为新快照。",
            PrimaryButtonText = "恢复",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close
        };

        if (await dialog.ShowAsync() != ContentDialogResult.Primary)
            return;

        try
        {
            RestoreButton.IsEnabled = false;
            var result = await _backendClient.SendAsync<NoteDocument>(
                "restoreSnapshot", new { noteId = _noteId, snapshotId = snapshot.Id }, CancellationToken.None);

            if (result is not null)
            {
                NoteRestored?.Invoke(this, EventArgs.Empty);
                // Refresh snapshot list since a pre-restore snapshot was created
                await LoadSnapshotsAsync();
            }
        }
        catch (Exception ex)
        {
            System.Diagnostics.Debug.WriteLine($"[VersionHistory] Restore error: {ex.Message}");
        }
        finally
        {
            RestoreButton.IsEnabled = true;
        }
    }
}

/// <summary>
/// View model for a single diff line in the ItemsControl.
/// </summary>
public sealed class DiffLineItem
{
    public DiffLine Line { get; }

    public DiffLineItem(DiffLine line)
    {
        Line = line;
    }

    public string Content => Line.DisplayText;

    public Brush BackgroundBrush => Line.Kind switch
    {
        DiffLineKind.Insert => new SolidColorBrush(Microsoft.UI.Colors.LightGreen) { Opacity = 0.3 },
        DiffLineKind.Delete => new SolidColorBrush(Microsoft.UI.Colors.LightCoral) { Opacity = 0.3 },
        _ => new SolidColorBrush(Microsoft.UI.Colors.Transparent),
    };

    public FontWeight FontWeight => Line.Kind == DiffLineKind.Context
        ? Microsoft.UI.Text.FontWeights.Normal
        : Microsoft.UI.Text.FontWeights.SemiBold;
}
