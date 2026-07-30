using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace VaultPilot.WinUI.Views;

/// <summary>
/// Notes browsing and management panel. Displays a searchable list of notes
/// with a detail pane for viewing the selected note's content and metadata.
/// </summary>
public sealed partial class NotesView : UserControl
{
    private readonly BackendClient _backendClient;
    private readonly MainWindow _mainWindow;
    private IReadOnlyList<NoteMeta> _allNotes = Array.Empty<NoteMeta>();
    private NoteMeta? _selectedNote;
    private string _searchQuery = string.Empty;
    private IReadOnlyList<NoteMeta>? _allNotesBeforeSearch;
    private CancellationTokenSource? _loadDetailCts;
    private CancellationTokenSource? _searchCts;
    private CancellationTokenSource? _relatedCts;
    private string? _currentBodyText;

    /// <summary>
    /// In-memory clipboard for note copy/paste (#3094).
    /// Holds the NoteMeta of the most recently copied note so that Ctrl+V
    /// can duplicate it. Static so it survives navigation between views.
    /// </summary>
    private static NoteMeta? s_clipboardNote;

    public NotesView(BackendClient backendClient, MainWindow mainWindow)
    {
        _backendClient = backendClient;
        _mainWindow = mainWindow;
        InitializeComponent();

        SearchBox.QuerySubmitted += OnSearchQuerySubmitted;
        SearchBox.TextChanged += OnSearchTextChanged;
        NotesList.SelectionChanged += OnNoteSelectionChanged;
        RefreshButton.Click += OnRefreshClicked;
        DeleteNoteButton.Click += OnDeleteNoteClicked;
        CopyNoteButton.Click += OnCopyNoteClicked;
        PasteNoteButton.Click += OnPasteNoteClicked;
        RelatedNotesList.SelectionChanged += OnRelatedNoteSelectionChanged;
        HistoryButton.Click += OnHistoryClicked;
        Loaded += OnLoaded;

        // Register Ctrl+C / Ctrl+V keyboard shortcuts on the NotesList (#3094)
        NotesList.KeyDown += OnNotesListKeyDown;

        UpdatePasteButtonState();
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
            _searchCts?.Cancel();
            _searchCts?.Dispose();
            _searchCts = null;
            _relatedCts?.Cancel();
            _relatedCts?.Dispose();
            _relatedCts = null;
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
            _searchCts?.Cancel();
            _searchCts?.Dispose();
            _searchCts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var submittedQuery = _searchQuery;
            var results = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>(
                "searchNotes", new { query = _searchQuery, limit = 50 }, _searchCts.Token);
            if (results is not null && _searchQuery == submittedQuery)
            {
                _allNotesBeforeSearch ??= _allNotes;
                _allNotes = results;
                ApplyFilter();
            }
        }
        catch (OperationCanceledException)
        {
            // Previous search was cancelled — ignore
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
            CopyNoteButton.IsEnabled = true;
            HistoryButton.IsEnabled = true;
            _loadDetailCts?.Cancel();
            _loadDetailCts?.Dispose();
            _loadDetailCts = new CancellationTokenSource();
            _ = LoadNoteDetailAsync(item.Meta, _loadDetailCts.Token);
        }
        else
        {
            _loadDetailCts?.Cancel();
            _loadDetailCts?.Dispose();
            _loadDetailCts = null;
            _selectedNote = null;
            DeleteNoteButton.IsEnabled = false;
            CopyNoteButton.IsEnabled = false;
            HistoryButton.IsEnabled = false;
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
                SetDetailBody(doc.Body);
            }
            else
            {
                // Fallback: show summary if full body unavailable
                SetDetailBody(meta.Summary ?? "（无法加载笔记正文）");
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // A newer selection has superseded this one — don't update UI
        }
        catch (Exception error)
        {
            // loadNote may not be implemented; show what we have from metadata
            System.Diagnostics.Debug.WriteLine($"loadNote failed: {error.Message}");
            SetDetailBody(!string.IsNullOrEmpty(meta.Summary)
                ? meta.Summary
                : "（笔记正文加载失败，请确认后端支持 loadNote 方法）");
        }

        // Kick off related notes lookup in the background (debounced via _relatedCts)
        // Only proceed if this request hasn't been superseded (#2288)
        if (!cancellationToken.IsCancellationRequested)
        {
            _relatedCts?.Cancel();
            _relatedCts?.Dispose();
            _relatedCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            _ = LoadRelatedNotesAsync(meta.Id, _relatedCts.Token);
        }
    }

    private async void OnDeleteNoteClicked(object sender, RoutedEventArgs e)
    {
        await DeleteSelectedNoteAsync();
    }

    /// <summary>
    /// Deletes the currently selected note after showing a confirmation dialog.
    /// Shared by the toolbar Delete button, the Delete/Backspace key shortcut,
    /// and the context menu Delete item (#3361).
    /// </summary>
    private async Task DeleteSelectedNoteAsync()
    {
        if (_selectedNote is null) return;

        // Cancel any in-flight detail load to prevent stale UI overwrite
        _loadDetailCts?.Cancel();
        _loadDetailCts?.Dispose();
        _loadDetailCts = null;

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
            var success = await _backendClient.SendAsync<bool>("deleteNote", new { id = note.Id }, cts.Token);
            if (!success)
            {
                ShowError("删除失败", new Exception("后端未能删除笔记，请重试"));
                return;
            }

            // Remove from local list and refresh
            _allNotes = _allNotes.Where(n => n.Id != note.Id).ToArray();
            if (_allNotesBeforeSearch is not null)
            {
                _allNotesBeforeSearch = _allNotesBeforeSearch.Where(n => n.Id != note.Id).ToArray();
            }
            _selectedNote = null;
            DeleteNoteButton.IsEnabled = false;
            CopyNoteButton.IsEnabled = false;
            HistoryButton.IsEnabled = false;
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

    // ─── Context Menu / Right-Tap (#3361) ─────────────────────────────────
    // Right-click on a note item opens a context menu with Delete, Copy, and
    // Version History actions. Tapping a note item also selects it so the
    // menu actions operate on the right note.

    private void OnNotesListRightTapped(object sender, RightTappedRoutedEventArgs e)
    {
        // Select the note under the cursor so context menu actions target it.
        // For virtualized ListView items the ListViewItem container may not
        // exist in the visual tree (it's been recycled or not yet
        // materialized), so the visual-tree walk fails silently (#3626).
        // We fall back to hit-testing by pointer position.
        if (sender is ListView listView)
        {
            // Strategy 1: Walk the visual tree to find a ListViewItem ancestor
            // of the tapped element (fast path for visible, materialized items).
            var element = e.OriginalSource as DependencyObject;
            while (element is not null && element != listView)
            {
                if (element is ListViewItem item)
                {
                    listView.SelectedItem = item.DataContext;
                    return;
                }
                element = VisualTreeHelper.GetParent(element);
            }

            // Strategy 2: Visual-tree walk failed — the item is likely
            // virtualized (scrolled out of view or at the viewport edge).
            // Hit-test using the pointer position to find the element at that
            // point, then resolve its DataContext to select the right item.
            // This handles the virtualization gap (#3626).
            var hitElement = ListViewHitTestHelper.FindItemFromPoint(listView, e);
            if (hitElement is not null)
            {
                listView.SelectedItem = hitElement;
            }
        }
    }

    private async void OnCtxDeleteClicked(object sender, RoutedEventArgs e)
    {
        await DeleteSelectedNoteAsync();
    }

    private void OnCtxCopyClicked(object sender, RoutedEventArgs e)
    {
        CopySelectedNote();
    }

    private async void OnCtxHistoryClicked(object sender, RoutedEventArgs e)
    {
        if (_selectedNote is null) return;
        await ShowHistoryDialogAsync(_selectedNote);
    }

    // ─── Version History ─────────────────────────────────────────────────
    // #3305: Open version history dialog for the selected note.
    private async void OnHistoryClicked(object sender, RoutedEventArgs e)
    {
        if (_selectedNote is null) return;
        await ShowHistoryDialogAsync(_selectedNote);
    }

    /// <summary>
    /// Opens the version history dialog for the given note.
    /// Shared by the History toolbar button and the context menu item (#3361).
    /// </summary>
    private async Task ShowHistoryDialogAsync(NoteMeta note)
    {
        try
        {
            var control = new Controls.VersionHistoryControl(_backendClient, note.Id);
            control.NoteRestored += (_, _) => _ = RefreshNotesAsync();

            var dialog = new ContentDialog
            {
                XamlRoot = XamlRoot,
                Title = $"版本历史 — {note.Title}",
                Content = control,
                PrimaryButtonText = "关闭",
                IsPrimaryButtonEnabled = true,
                DefaultButton = ContentDialogButton.Primary,
                MinWidth = 800,
                MinHeight = 600,
            };

            _ = await dialog.ShowAsync();
        }
        catch (Exception ex)
        {
            ShowError("打开版本历史失败", ex);
        }
    }

    // ─── Copy / Paste (Duplicate) ────────────────────────────────────────
    // #3094: Ctrl+C copies the selected note, Ctrl+V (or the Paste button)
    // creates a duplicate with a fresh ID and "(副本)" suffix.

    private void OnNotesListKeyDown(object sender, KeyRoutedEventArgs e)
    {
        // Delete / Backspace key: delete the currently selected note (#3361)
        if (e.Key == Windows.System.VirtualKey.Delete || e.Key == Windows.System.VirtualKey.Back)
        {
            if (_selectedNote is not null)
            {
                _ = DeleteSelectedNoteAsync();
                e.Handled = true;
            }
            return;
        }

        var ctrlDown = Microsoft.UI.Input.InputKeyboardSource
            .GetKeyStateForCurrentThread(Windows.System.VirtualKey.Control)
            .HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);

        if (!ctrlDown) return;

        if (e.Key == Windows.System.VirtualKey.C)
        {
            CopySelectedNote();
            e.Handled = true;
        }
        else if (e.Key == Windows.System.VirtualKey.V)
        {
            _ = PasteNoteAsync();
            e.Handled = true;
        }
    }

    private void OnCopyNoteClicked(object sender, RoutedEventArgs e)
    {
        CopySelectedNote();
    }

    private async void OnPasteNoteClicked(object sender, RoutedEventArgs e)
    {
        await PasteNoteAsync();
    }

    /// <summary>
    /// Copy the currently selected note to the in-memory clipboard.
    /// </summary>
    private void CopySelectedNote()
    {
        if (_selectedNote is null) return;

        s_clipboardNote = _selectedNote;
        UpdatePasteButtonState();
    }

    /// <summary>
    /// Duplicate the note from the in-memory clipboard: load its full
    /// document, create a copy with a fresh ID and "(副本)" title, then
    /// save it via the backend's saveNote method.
    /// </summary>
    private async Task PasteNoteAsync()
    {
        if (s_clipboardNote is null) return;

        var source = s_clipboardNote;

        try
        {
            ShowLoading(true);

            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));

            // Load the full document (meta + body) for the source note
            var doc = await _backendClient.SendAsync<NoteDocument>(
                "loadNote", new { id = source.Id }, cts.Token);

            // Build a new NoteDocument with a fresh ID and "(副本)" suffix
            var newMeta = CreateDuplicateMeta(source);

            var body = doc?.Body ?? source.Summary ?? string.Empty;
            var newDoc = new NoteDocument(newMeta, body);

            var saved = await _backendClient.SendAsync<NoteDocument>(
                "saveNote", new { note = newDoc }, cts.Token);

            if (saved is null)
            {
                ShowError("复制失败", new Exception("后端未能保存副本笔记"));
                return;
            }

            // Refresh the list to show the new duplicate
            await RefreshNotesAsync();

            // Select the newly created note
            SelectNoteById(saved.Meta.Id);
        }
        catch (Exception error)
        {
            ShowError("复制笔记失败", error);
        }
        finally
        {
            ShowLoading(false);
        }
    }

    /// <summary>
    /// Enable or disable the Paste button based on clipboard state.
    /// </summary>
    private void UpdatePasteButtonState()
    {
        PasteNoteButton.IsEnabled = s_clipboardNote is not null;
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

    /// <summary>
    /// Programmatically selects a note by its ID, scrolling it into view.
    /// Used by MainWindow.NavigateToNoteFromTitleAsync (#2035).
    /// </summary>
    public void SelectNoteById(string noteId)
    {
        if (string.IsNullOrEmpty(noteId))
            return;

        var item = NotesList.Items
            .OfType<NoteListItem>()
            .FirstOrDefault(n => n.Meta.Id == noteId);
        if (item is not null)
        {
            NotesList.SelectedItem = item;
            NotesList.ScrollIntoView(item);
        }
    }

    /// <summary>
    /// Returns the currently selected note's ID, or null if nothing is selected.
    /// Used by MainWindow for navigation history (#3230).
    /// </summary>
    public string? SelectedNoteId() => _selectedNote?.Id;

    private void ClearDetail()
    {
        DetailTitle.Text = "选择一篇笔记";
        SetDetailBody(null);
        DetailTags.Text = string.Empty;
        DetailUpdated.Text = string.Empty;
        DetailPath.Text = string.Empty;
        DetailMetaPanel.Visibility = Visibility.Collapsed;
        DetailSeparator.Visibility = Visibility.Collapsed;
        ClearRelatedNotes();
    }

    /// <summary>
    /// Load related notes for the given note ID and display them in the side panel.
    /// </summary>
    private async Task LoadRelatedNotesAsync(string noteId, CancellationToken cancellationToken)
    {
        try
        {
            RelatedNotesPanel.Visibility = Visibility.Visible;
            RelatedNotesLoading.IsActive = true;
            RelatedNotesLoading.Visibility = Visibility.Visible;

            var related = await _backendClient.FindRelatedNotesAsync(noteId, limit: 5, token: cancellationToken);
            if (cancellationToken.IsCancellationRequested)
                return;

            if (related is not null && related.Count > 0)
            {
                var items = related.Select(r => new RelatedNoteItem(r)).ToList();
                RelatedNotesList.ItemsSource = items;
            }
            else
            {
                // Show "no related notes" placeholder
                RelatedNotesList.ItemsSource = new List<RelatedNoteItem>
                {
                    new RelatedNoteItem(new RelatedNote(new NoteMeta { Title = "（暂无相关笔记）" }, 0, null))
                };
            }
        }
        catch (OperationCanceledException)
        {
            // Cancelled by a newer selection — don't update UI
        }
        catch (Exception error)
        {
            System.Diagnostics.Debug.WriteLine($"LoadRelatedNotesAsync error: {error.Message}");
        }
        finally
        {
            RelatedNotesLoading.IsActive = false;
            RelatedNotesLoading.Visibility = Visibility.Collapsed;
            // Keep the panel visible only when the load completed and produced
            // at least one entry (a real related note, or the "no related notes"
            // placeholder). Cancellation or empty results collapse it.
            // Previously the panel was collapsed unconditionally here, hiding
            // successfully loaded results and breaking the feature (#2780).
            if (!ShouldKeepRelatedNotesPanelVisible(
                    cancellationToken.IsCancellationRequested,
                    RelatedNotesList.ItemsSource))
            {
                RelatedNotesPanel.Visibility = Visibility.Collapsed;
            }
        }
    }

    /// <summary>
    /// Determines whether the related-notes panel should remain visible after a
    /// load attempt. It stays visible only when the load was not cancelled and
    /// at least one entry was produced (a real related note, or the
    /// "no related notes" placeholder). Fixes #2780 where the panel was
    /// collapsed unconditionally in the load's finally block, hiding
    /// successfully loaded results and making the feature unusable.
    /// </summary>
    public static bool ShouldKeepRelatedNotesPanelVisible(
        bool isCancellationRequested,
        object? itemsSource)
    {
        if (isCancellationRequested) return false;
        return itemsSource is IList<RelatedNoteItem> { Count: > 0 };
    }

    /// <summary>
    /// Creates a duplicate NoteMeta from a source note, with a fresh GUID ID,
    /// "(副本)" title suffix, reset path, and current timestamps (#3094).
    /// Extracted as a pure function for unit testing.
    /// </summary>
    public static NoteMeta CreateDuplicateMeta(NoteMeta source)
    {
        return new NoteMeta
        {
            Id = Guid.NewGuid().ToString("N"),
            Title = $"{source.Title} (副本)",
            Tags = source.Tags,
            Keywords = source.Keywords,
            Platform = source.Platform,
            Board = source.Board,
            Kernel = source.Kernel,
            Status = source.Status,
            CreatedAt = DateTimeOffset.UtcNow.ToString("o"),
            UpdatedAt = DateTimeOffset.UtcNow.ToString("o"),
            Source = source.Source,
            Path = string.Empty,
            Summary = source.Summary,
        };
    }

    private void ClearRelatedNotes()
    {
        RelatedNotesList.ItemsSource = null;
        RelatedNotesPanel.Visibility = Visibility.Collapsed;
    }

    /// <summary>
    /// When the user clicks a related note, select it in the main list.
    /// </summary>
    private void OnRelatedNoteSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (e.AddedItems.Count > 0 && e.AddedItems[0] is RelatedNoteItem item)
        {
            SelectNoteById(item.Meta.Id);
            // Clear the selection so the same item can be re-selected
            RelatedNotesList.SelectedItem = null;
        }
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
    /// Returns the full body text of the currently selected note, or null if no note is selected.
    /// Used by MainWindow.CommandPalette to provide note context for AI actions.
    /// </summary>
    public string? GetSelectedNoteBody()
    {
        var body = _currentBodyText?.Trim();
        return !string.IsNullOrWhiteSpace(body) ? body : null;
    }

    /// <summary>
    /// Sets the detail body content. If the text is non-empty and the MainWindow
    /// Markdown renderer is available, renders the body as Markdown; otherwise
    /// falls back to a plain TextBlock.
    /// </summary>
    private void SetDetailBody(string? body)
    {
        _currentBodyText = body;
        DetailBodyContainer.Children.Clear();

        if (string.IsNullOrEmpty(body))
            return;

        try
        {
            var rendered = _mainWindow.CreateMarkdownContent(body);
            DetailBodyContainer.Children.Add(rendered);
        }
        catch
        {
            // Fallback: plain text if markdown rendering fails
            var tb = new TextBlock
            {
                Text = body,
                TextWrapping = TextWrapping.Wrap,
                IsTextSelectionEnabled = true
            };
            DetailBodyContainer.Children.Add(tb);
        }
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

/// <summary>
/// Display wrapper around <see cref="RelatedNote"/> for data binding in the
/// related notes ListView.
/// </summary>
public sealed class RelatedNoteItem
{
    public RelatedNote Note { get; }
    public NoteMeta Meta => Note.Meta;
    public string Title => Note.Meta.Title;
    public long Score => Note.Score;
    public string TagsDisplay => Note.Meta.Tags.Count > 0
        ? $"🏷 {string.Join(", ", Note.Meta.Tags)}"
        : string.Empty;

    public RelatedNoteItem(RelatedNote note)
    {
        Note = note;
    }
}
