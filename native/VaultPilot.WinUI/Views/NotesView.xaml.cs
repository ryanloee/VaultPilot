using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System.Threading;

namespace VaultPilot.WinUI.Views;

/// <summary>
/// Notes browsing and management panel. Displays a searchable list of notes
/// with a detail pane for viewing the selected note's content and metadata.
/// </summary>
public sealed partial class NotesView : UserControl
{
    private readonly BackendClient _backendClient;
    private IReadOnlyList<NoteMeta> _allNotes = Array.Empty<NoteMeta>();
    private NoteMeta? _selectedNote;
    private string _searchQuery = string.Empty;
    private IReadOnlyList<NoteMeta>? _allNotesBeforeSearch;
    private CancellationTokenSource? _loadDetailCts;

    public NotesView(BackendClient backendClient)
    {
        _backendClient = backendClient;
        InitializeComponent();

        SearchBox.QuerySubmitted += OnSearchQuerySubmitted;
        SearchBox.TextChanged += OnSearchTextChanged;
        NotesList.SelectionChanged += OnNoteSelectionChanged;
        RefreshButton.Click += OnRefreshClicked;
        DeleteNoteButton.Click += OnDeleteNoteClicked;
        Loaded += OnLoaded;
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        try { await RefreshNotesAsync(); }
        catch (Exception ex) { System.Diagnostics.Debug.WriteLine($"[NotesView] OnLoaded error: {ex.Message}"); }
    }

    /// <summary>
    /// Public entry point to reload the notes list. Called when the user
    /// navigates to this view or manually triggers a refresh.
    /// </summary>
    public async Task RefreshNotesAsync()
    {
        try
        {
            ShowLoading(true);
            _loadDetailCts?.Cancel();
            _loadDetailCts?.Dispose();
            _loadDetailCts = null;
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            _allNotes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { }, cts.Token)
                ?? Array.Empty<NoteMeta>();
            _allNotesBeforeSearch = null;
            ApplyFilter();
            UpdateNotesCount();
        }
        catch (Exception error)
        {
            ShowError("加载笔记列表失败", error);
        }
        finally
        {
            ShowLoading(false);
        }
    }

    private async void OnRefreshClicked(object sender, RoutedEventArgs e)
    {
        try { await RefreshNotesAsync(); }
        catch (Exception ex) { System.Diagnostics.Debug.WriteLine($"[NotesView] OnRefreshClicked error: {ex.Message}"); }
    }

    private async void OnSearchQuerySubmitted(AutoSuggestBox sender, AutoSuggestBoxQuerySubmittedEventArgs args)
    {
        _searchQuery = args.QueryText?.Trim() ?? string.Empty;

        if (string.IsNullOrEmpty(_searchQuery))
        {
            if (_allNotesBeforeSearch is not null)
            {
                _allNotes = _allNotesBeforeSearch;
                _allNotesBeforeSearch = null;
            }
            ApplyFilter();
            return;
        }

        try
        {
            ShowLoading(true);
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var results = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>(
                "searchNotes", new { query = _searchQuery, limit = 50 }, cts.Token);
            if (results is not null)
            {
                _allNotesBeforeSearch ??= _allNotes;
                _allNotes = results;
                ApplyFilter();
            }
        }
        catch (Exception error)
        {
            // Fallback: if searchNotes is not available, filter locally
            System.Diagnostics.Debug.WriteLine($"searchNotes unavailable, using local filter: {error.Message}");
            ApplyFilter();
        }
        finally
        {
            ShowLoading(false);
        }
    }

    private void OnSearchTextChanged(AutoSuggestBox sender, AutoSuggestBoxTextChangedEventArgs args)
    {
        if (args.Reason == AutoSuggestionBoxTextChangeReason.UserInput)
        {
            _searchQuery = sender.Text?.Trim() ?? string.Empty;
            if (string.IsNullOrEmpty(_searchQuery))
            {
                if (_allNotesBeforeSearch is not null)
                {
                    _allNotes = _allNotesBeforeSearch;
                    _allNotesBeforeSearch = null;
                }
                ApplyFilter();
            }
        }
    }

    private void OnNoteSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (NotesList.SelectedItem is NoteListItem item)
        {
            _selectedNote = item.Meta;
            DeleteNoteButton.IsEnabled = true;
            _loadDetailCts?.Cancel();
            _loadDetailCts?.Dispose();
            _loadDetailCts = new CancellationTokenSource();
            _ = LoadNoteDetailAsync(item.Meta, _loadDetailCts.Token);
        }
        else
        {
            _selectedNote = null;
            DeleteNoteButton.IsEnabled = false;
        }
    }

    private async Task LoadNoteDetailAsync(NoteMeta meta, CancellationToken cancellationToken)
    {
        try
        {
            DetailTitle.Text = meta.Title;
            DetailTags.Text = meta.Tags.Count > 0
                ? $"🏷 {string.Join(", ", meta.Tags)}"
                : string.Empty;
            DetailUpdated.Text = FormatRelativeTime(meta.UpdatedAt);
            DetailPath.Text = meta.Path;
            DetailMetaPanel.Visibility = Visibility.Visible;
            DetailSeparator.Visibility = Visibility.Visible;

            // Try to load the full document body
            var doc = await _backendClient.SendAsync<NoteDocument>(
                "loadNote", new { id = meta.Id }, cancellationToken);

            if (doc is not null && !string.IsNullOrEmpty(doc.Body))
            {
                DetailBody.Text = doc.Body;
            }
            else
            {
                // Fallback: show summary if full body unavailable
                DetailBody.Text = meta.Summary ?? "（无法加载笔记正文）";
            }
        }
        catch (Exception error)
        {
            // loadNote may not be implemented; show what we have from metadata
            System.Diagnostics.Debug.WriteLine($"loadNote failed: {error.Message}");
            DetailBody.Text = !string.IsNullOrEmpty(meta.Summary)
                ? meta.Summary
                : "（笔记正文加载失败，请确认后端支持 loadNote 方法）";
        }
    }

    private async void OnDeleteNoteClicked(object sender, RoutedEventArgs e)
    {
        if (_selectedNote is null) return;

        try
        {
            var note = _selectedNote;
            var dialog = new ContentDialog
            {
                XamlRoot = XamlRoot,
                Title = "删除笔记",
                Content = $"确认删除「{note.Title}」吗？此操作不可撤销。",
                PrimaryButtonText = "删除",
                CloseButtonText = "取消",
                DefaultButton = ContentDialogButton.Close
            };

            if (await dialog.ShowAsync() != ContentDialogResult.Primary)
            {
                return;
            }

            ShowLoading(true);
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            await _backendClient.SendAsync<bool>("deleteNote", new { id = note.Id }, cts.Token);

            // Remove from local list and refresh
            _allNotes = _allNotes.Where(n => n.Id != note.Id).ToArray();
            _selectedNote = null;
            DeleteNoteButton.IsEnabled = false;
            ApplyFilter();
            UpdateNotesCount();

            ClearDetail();
        }
        catch (Exception error)
        {
            ShowError("删除笔记失败", error);
        }
        finally
        {
            ShowLoading(false);
        }
    }

    private void ApplyFilter()
    {
        var filtered = string.IsNullOrEmpty(_searchQuery)
            ? _allNotes
            : _allNotes.Where(n =>
                (n.Title?.Contains(_searchQuery, StringComparison.OrdinalIgnoreCase) ?? false) ||
                (n.Summary?.Contains(_searchQuery, StringComparison.OrdinalIgnoreCase) ?? false) ||
                n.Tags.Any(t => t.Contains(_searchQuery, StringComparison.OrdinalIgnoreCase)))
                .ToArray();

        var items = filtered
            .OrderByDescending(n => n.UpdatedAt)
            .Select(n => new NoteListItem(n))
            .ToList();

        NotesList.ItemsSource = items;
        UpdateNotesCount();
    }

    private void UpdateNotesCount()
    {
        var count = NotesList.ItemsSource is IList<NoteListItem> list ? list.Count : 0;
        NotesCountLabel.Text = _allNotes.Count == count
            ? $"笔记 ({_allNotes.Count})"
            : $"笔记 ({count}/{_allNotes.Count})";
    }

    private void ClearDetail()
    {
        DetailTitle.Text = "选择一篇笔记";
        DetailBody.Text = string.Empty;
        DetailTags.Text = string.Empty;
        DetailUpdated.Text = string.Empty;
        DetailPath.Text = string.Empty;
        DetailMetaPanel.Visibility = Visibility.Collapsed;
        DetailSeparator.Visibility = Visibility.Collapsed;
    }

    private void ShowLoading(bool show)
    {
        NotesLoading.IsActive = show;
        NotesLoading.Visibility = show ? Visibility.Visible : Visibility.Collapsed;
    }

    private void ShowError(string title, Exception error)
    {
        System.Diagnostics.Debug.WriteLine($"NotesView error [{title}]: {error.Message}");
        ErrorInfoBar.Title = title;
        ErrorInfoBar.Message = error.Message;
        ErrorInfoBar.IsOpen = true;
    }

    /// <summary>
    /// Format an ISO 8601 timestamp as a human-readable relative time string.
    /// Shared between <see cref="NotesView"/> and <see cref="NoteListItem"/>.
    /// </summary>
    internal static string FormatRelativeTime(string updatedAt)
    {
        if (string.IsNullOrEmpty(updatedAt)) return string.Empty;
        try
        {
            var dt = DateTimeOffset.Parse(updatedAt);
            var local = dt.ToLocalTime();
            var now = DateTimeOffset.Now;
            var diff = now - local;

            if (diff.TotalMinutes < 1) return "刚刚";
            if (diff.TotalHours < 1) return $"{(int)diff.TotalMinutes}分钟前";
            if (diff.TotalDays < 1) return $"{(int)diff.TotalHours}小时前";
            if (diff.TotalDays < 7) return $"{(int)diff.TotalDays}天前";
            return local.ToString("yyyy-MM-dd");
        }
        catch
        {
            return updatedAt;
        }
    }
}

/// <summary>
/// Display wrapper around <see cref="NoteMeta"/> that adds computed properties
/// for data binding in the XAML ListView.
/// </summary>
public sealed class NoteListItem
{
    public NoteMeta Meta { get; }
    public string Title => Meta.Title;
    public string Summary => Meta.Summary ?? string.Empty;
    public string TagsDisplay => Meta.Tags.Count > 0
        ? $"🏷 {string.Join(", ", Meta.Tags)}"
        : string.Empty;
    public string UpdatedDisplay => NotesView.FormatRelativeTime(Meta.UpdatedAt);

    public NoteListItem(NoteMeta meta)
    {
        Meta = meta;
    }
}
