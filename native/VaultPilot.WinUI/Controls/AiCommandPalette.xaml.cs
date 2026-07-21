using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Controls;

/// <summary>
/// Global AI command palette — a floating Cmd+K / Raycast-style overlay
/// for executing AI quick actions on selected text or note content.
/// </summary>
public sealed partial class AiCommandPalette : UserControl
{
    private AiActionInfo[]? _allActions;
    private AiActionInfo[] _filteredActions = Array.Empty<AiActionInfo>();
    private CancellationTokenSource? _activeRequestCts;
    private AiActionResult? _lastResult;

    /// <summary>
    /// Backend client used to invoke AI actions.
    /// Must be set by the owner (MainWindow) before Show() is called.
    /// </summary>
    public BackendClient? Backend { get; set; }

    /// <summary>
    /// The source text to operate on — typically the user's selection or
    /// current note body. Set by the owner before Show().
    /// </summary>
    public string SourceText { get; set; } = string.Empty;

    /// <summary>
    /// Optional note ID for context (used by FindRelatedNotes).
    /// </summary>
    public string? ContextNoteId { get; set; }

    public AiCommandPalette()
    {
        InitializeComponent();
        // Re-initialize focus when the palette becomes visible
        Loaded += (_, _) =>
        {
            if (Visibility == Visibility.Visible)
            {
                FocusSearchBox();
            }
        };
    }

    /// <summary>
    /// Show the palette and load the action list from the backend.
    /// </summary>
    public async void Show()
    {
        Visibility = Visibility.Visible;
        PaletteCard.Visibility = Visibility.Visible;
        BackdropGrid.Visibility = Visibility.Visible;
        ResultCard.Visibility = Visibility.Collapsed;
        LoadingOverlay.Visibility = Visibility.Collapsed;

        SearchBox.Text = string.Empty;
        _lastResult = null;
        FooterHint.Text = "↑↓ 选择 · Enter 执行 · Esc 关闭";

        FocusSearchBox();

        if (Backend is null) return;

        try
        {
            LoadingOverlay.Visibility = Visibility.Visible;
            LoadingText.Text = "正在加载 AI 操作...";

            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(15));
            var actions = await Backend.SendAsync<AiActionInfo[]>("listAiActions", new { }, cts.Token);
            _allActions = actions ?? Array.Empty<AiActionInfo>();
            _filteredActions = _allActions;
            UpdateActionList();

            LoadingOverlay.Visibility = Visibility.Collapsed;
            ActionCountText.Text = $"{_filteredActions.Length} 个操作";
        }
        catch (Exception error)
        {
            LoadingOverlay.Visibility = Visibility.Collapsed;
            ShowError($"加载 AI 操作失败: {error.Message}");
        }
    }

    /// <summary>
    /// Hide the palette and cancel any active request.
    /// </summary>
    public void Dismiss()
    {
        CancelActiveRequest();
        Visibility = Visibility.Collapsed;
    }

    /// <summary>
    /// Get the currently selected text from the source context.
    /// If SourceText is set, use it; otherwise return empty string.
    /// </summary>
    private string GetEffectiveText()
    {
        // If we have explicit source text, use it
        if (!string.IsNullOrWhiteSpace(SourceText))
            return SourceText;

        // Clipboard fallback intentionally omitted: this method is synchronous and
        // WinUI clipboard access is async-only (GetTextAsync). Implementing it would
        // require making GetEffectiveText (and its callers) async. See issue #2574.
        return string.Empty;
    }

    /// <summary>
    /// Execute the selected AI action.
    /// </summary>
    private async void ExecuteAction(AiActionInfo actionInfo)
    {
        if (Backend is null)
        {
            ShowError("后端未连接，请先启动后端。");
            return;
        }

        var text = GetEffectiveText();
        if (string.IsNullOrWhiteSpace(text) && actionInfo.Id != "findRelatedNotes")
        {
            ShowError("请先选择文本或打开笔记后再执行此操作。\n\n提示：在输入框中输入文本，或选中笔记内容后调用此面板。");
            return;
        }

        // Show loading
        PaletteCard.Visibility = Visibility.Collapsed;
        LoadingOverlay.Visibility = Visibility.Visible;

        // Determine action type from the action info
        var actionType = ParseActionType(actionInfo.Id);

        // Cancel any previous request and atomically install a new CTS.
        // Using Interlocked.Exchange prevents the race where Dismiss()
        // runs between CancelActiveRequest and the new assignment,
        // seeing null and letting the new request continue after dismiss.
        var oldCts = Interlocked.Exchange(ref _activeRequestCts,
            new CancellationTokenSource(TimeSpan.FromSeconds(120)));
        oldCts?.Cancel();
        oldCts?.Dispose();
        var ct = _activeRequestCts.Token; // Capture token before any await (re-entrancy guard)

        try
        {
            var request = new AiActionRequest(actionType, text);

            // For Translate, show a simple language picker (default to Chinese)
            if (actionType == AiActionType.Translate && string.IsNullOrWhiteSpace(request.TargetLanguage))
            {
                request.TargetLanguage = await ShowLanguagePickerAsync();
                if (request.TargetLanguage is null)
                {
                    // User cancelled
                    LoadingOverlay.Visibility = Visibility.Collapsed;
                    PaletteCard.Visibility = Visibility.Visible;
                    return;
                }
            }

            // For Rewrite, set default tone
            if (actionType == AiActionType.Rewrite && string.IsNullOrWhiteSpace(request.Tone))
            {
                request.Tone = "professional";
            }

            if (!string.IsNullOrWhiteSpace(ContextNoteId))
            {
                request.NoteId = ContextNoteId;
            }

            // Serialize the AiActionRequest directly instead of a manually-constructed
            // anonymous object. The record type already carries proper [JsonPropertyName]
            // annotations for all fields including Instruction (which is null for
            // non-EditNote actions and set when UI support is added). (#2862, #2863)
            var result = await Backend.SendAsync<AiActionResult>(
                "executeAiAction", request,
                ct);

            _lastResult = result;

            LoadingOverlay.Visibility = Visibility.Collapsed;

            if (result is not null && string.IsNullOrEmpty(result.Error))
            {
                // Show result card
                ShowResultCard(actionInfo, result);
            }
            else
            {
                ShowError(result?.Error ?? "操作执行失败，请重试。");
            }
        }
        catch (OperationCanceledException)
        {
            LoadingOverlay.Visibility = Visibility.Collapsed;
            PaletteCard.Visibility = Visibility.Visible;
            FooterHint.Text = "操作已取消。";
        }
        catch (Exception error)
        {
            LoadingOverlay.Visibility = Visibility.Collapsed;
            ShowError($"操作执行失败: {error.Message}");
        }
    }

    /// <summary>
    /// Show the result card with action output.
    /// </summary>
    private void ShowResultCard(AiActionInfo action, AiActionResult result)
    {
        ResultTitle.Text = action.Label;
        ResultTextBlock.Text = result.Result;
        // Reset foreground in case a previous ShowError() left it red
        ResultTextBlock.Foreground = GetThemeBrush("TextFillColorPrimaryBrush");
        InsertToChatButton.Visibility = Visibility.Visible;
        CopyResultButton.Visibility = Visibility.Visible;

        if (result.Usage is not null)
        {
            ResultUsageText.Text = $"Token: {result.Usage.TotalTokens}";
        }
        else
        {
            ResultUsageText.Text = string.Empty;
        }

        ResultCard.Visibility = Visibility.Visible;
    }

    /// <summary>
    /// Show an error message in the result area.
    /// </summary>
    private void ShowError(string message)
    {
        ResultTitle.Text = "错误";
        ResultTextBlock.Text = message;
        ResultTextBlock.Foreground = GetThemeBrush("SystemFillColorCriticalBrush");
        InsertToChatButton.Visibility = Visibility.Collapsed;
        CopyResultButton.Visibility = Visibility.Visible;
        ResultUsageText.Text = string.Empty;
        PaletteCard.Visibility = Visibility.Collapsed;
        ResultCard.Visibility = Visibility.Visible;
    }

    /// <summary>
    /// Show a simple content dialog to pick a target language for translation.
    /// Returns the selected language or null if cancelled.
    /// </summary>
    private async Task<string?> ShowLanguagePickerAsync()
    {
        // Use a simple approach: show options in a popup
        var dialog = new ContentDialog
        {
            Title = "选择目标语言",
            Content = new ComboBox
            {
                ItemsSource = new[] { "中文", "English", "日本語", "한국어", "Français", "Deutsch", "Español", "Русский" },
                SelectedIndex = 0,
                PlaceholderText = "选择语言...",
                MinWidth = 200,
            },
            PrimaryButtonText = "确认",
            CloseButtonText = "取消",
            XamlRoot = this.XamlRoot,
        };

        var result = await dialog.ShowAsync();
        if (result == ContentDialogResult.Primary
            && dialog.Content is ComboBox combo
            && combo.SelectedItem is string lang)
        {
            return lang;
        }

        return null;
    }

    /// <summary>
    /// Cancel any active AI request.
    /// </summary>
    private void CancelActiveRequest()
    {
        var old = Interlocked.Exchange(ref _activeRequestCts, null);
        old?.Cancel();
        old?.Dispose();
    }

    /// <summary>
    /// Safe theme-brush lookup: returns the brush for the given key, or a
    /// transparent fallback if the key is missing. Avoids the
    /// KeyNotFoundException/InvalidCastException that direct indexer casts
    /// (e.g. <c>(Brush)Application.Current.Resources[key]</c>) would throw.
    /// </summary>
    private static Brush GetThemeBrush(string key)
    {
        if (Application.Current?.Resources.TryGetValue(key, out var value) == true && value is Brush brush)
        {
            return brush;
        }
        return new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
    }

    /// <summary>
    /// Parse action type from its string id.
    /// </summary>
    private static AiActionType ParseActionType(string id) => id switch
    {
        "summarize" => AiActionType.Summarize,
        "rewrite" => AiActionType.Rewrite,
        "translate" => AiActionType.Translate,
        "explain" => AiActionType.Explain,
        "continueWriting" => AiActionType.ContinueWriting,
        "extractTodos" => AiActionType.ExtractTodos,
        "findRelatedNotes" => AiActionType.FindRelatedNotes,
        "cleanUp" or "clean_up" => AiActionType.CleanUp,
        "generateOutline" or "generate_outline" => AiActionType.GenerateOutline,
        "editNote" or "edit_note" => AiActionType.EditNote,
        "summarizeUrl" or "summarize_url" => AiActionType.SummarizeUrl,
        "brainstorm" => AiActionType.Brainstorm,
        "reviewNote" or "review_note" or "review" => AiActionType.ReviewNote,
        "synthesizeWiki" or "synthesize_wiki" or "synthesize" or "wiki" => AiActionType.SynthesizeWiki,
        "workspaceQuery" or "workspace_query" or "workspace" => AiActionType.WorkspaceQuery,
        "transcribeAudio" or "transcribe_audio" or "transcribe" => AiActionType.TranscribeAudio,
        _ => AiActionType.Summarize
    };

    /// <summary>
    /// Focus the search text box.
    /// </summary>
    private void FocusSearchBox()
    {
        SearchBox.Focus(FocusState.Programmatic);
        SearchBox.SelectAll();
    }

    /// <summary>
    /// Filter the action list based on the current search query.
    /// </summary>
    private void FilterActions()
    {
        var query = SearchBox.Text?.Trim() ?? string.Empty;
        if (string.IsNullOrEmpty(query))
        {
            _filteredActions = _allActions ?? Array.Empty<AiActionInfo>();
        }
        else
        {
            _filteredActions = (_allActions ?? Array.Empty<AiActionInfo>())
                .Where(a =>
                    (a.Label?.Contains(query, StringComparison.OrdinalIgnoreCase) ?? false) ||
                    (a.Id?.Contains(query, StringComparison.OrdinalIgnoreCase) ?? false))
                .ToArray();
        }

        UpdateActionList();
        ActionCountText.Text = $"{_filteredActions.Length} 个操作";
        FooterHint.Text = _filteredActions.Length == 0
            ? $"没有找到与 \"{query}\" 匹配的操作"
            : "↑↓ 选择 · Enter 执行 · Esc 关闭";
    }

    /// <summary>
    /// Refresh the ListView with the current filtered actions.
    /// </summary>
    private void UpdateActionList()
    {
        ActionListView.ItemsSource = null;
        ActionListView.ItemsSource = _filteredActions;

        if (_filteredActions.Length > 0)
        {
            ActionListView.SelectedIndex = 0;
        }
    }

    // ─── Event handlers ─────────────────────────────────────────────

    private void OnSearchTextChanged(object sender, TextChangedEventArgs e)
    {
        FilterActions();
    }

    private void OnSearchBoxKeyDown(object sender, KeyRoutedEventArgs e)
    {
        switch (e.Key)
        {
            case Windows.System.VirtualKey.Enter:
                e.Handled = true;
                if (ActionListView.SelectedItem is AiActionInfo selected)
                {
                    ExecuteAction(selected);
                }
                else if (_filteredActions.Length > 0)
                {
                    ExecuteAction(_filteredActions[0]);
                }
                break;

            case Windows.System.VirtualKey.Escape:
                e.Handled = true;
                Dismiss();
                break;

            case Windows.System.VirtualKey.Down:
                e.Handled = true;
                if (ActionListView.SelectedIndex < _filteredActions.Length - 1)
                {
                    ActionListView.SelectedIndex++;
                }
                break;

            case Windows.System.VirtualKey.Up:
                e.Handled = true;
                if (ActionListView.SelectedIndex > 0)
                {
                    ActionListView.SelectedIndex--;
                }
                break;
        }
    }

    private void OnActionSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        // Update footer hint based on selection
        if (ActionListView.SelectedItem is AiActionInfo action)
        {
            FooterHint.Text = $"↑↓ 选择 · Enter 执行 · Esc 关闭  |  {action.Label}";
        }
    }

    private void OnActionDoubleTapped(object sender, DoubleTappedRoutedEventArgs e)
    {
        if (ActionListView.SelectedItem is AiActionInfo action)
        {
            ExecuteAction(action);
        }
    }

    private void OnBackdropPressed(object sender, PointerRoutedEventArgs e)
    {
        // Dismiss when backdrop (overlay behind the palette card) is tapped
        BackdropGrid.Visibility = Visibility.Collapsed;
        Dismiss();
    }

    private void OnInsertToChatClicked(object sender, RoutedEventArgs e)
    {
        if (_lastResult is null || string.IsNullOrEmpty(_lastResult.Result))
            return;

        // Raise an event so the owner (MainWindow) can insert the text into the composer
        InsertToChatRequested?.Invoke(this, _lastResult.Result);
        Dismiss();
    }

    private void OnCopyResultClicked(object sender, RoutedEventArgs e)
    {
        if (_lastResult is null || string.IsNullOrEmpty(_lastResult.Result))
            return;

        try
        {
            var dataPackage = new Windows.ApplicationModel.DataTransfer.DataPackage();
            dataPackage.SetText(_lastResult.Result);
            Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(dataPackage);
            FooterHint.Text = "已复制到剪贴板";
        }
        catch
        {
            FooterHint.Text = "复制失败";
        }
    }

    private void OnCloseResultClicked(object sender, RoutedEventArgs e)
    {
        // Go back to the action list
        ResultCard.Visibility = Visibility.Collapsed;
        PaletteCard.Visibility = Visibility.Visible;
        FocusSearchBox();
    }

    /// <summary>
    /// Raised when the user wants to insert the action result into the chat composer.
    /// </summary>
    public event EventHandler<string>? InsertToChatRequested;
}
