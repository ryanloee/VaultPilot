using System;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Controls;

/// <summary>
/// Quick Ask overlay — a global floating popup (Ctrl+Shift+Q) for asking
/// a one-shot AI question against the vault knowledge base without leaving
/// the current editing context.
/// </summary>
public sealed partial class QuickAskOverlay : UserControl
{
    private CancellationTokenSource? _activeRequestCts;
    private string? _lastAnswer;

    /// <summary>
    /// Backend client used to invoke the AI ask endpoint.
    /// Must be set by the owner (MainWindow) before Show() is called.
    /// </summary>
    public BackendClient? Backend { get; set; }

    /// <summary>
    /// Raised when the user wants to insert the AI answer into the note editor.
    /// </summary>
    public event EventHandler<string>? InsertToNoteRequested;

    public QuickAskOverlay()
    {
        InitializeComponent();

        Loaded += (_, _) =>
        {
            if (Visibility == Visibility.Visible)
            {
                FocusQuestionBox();
            }
        };
    }

    /// <summary>
    /// Show the Quick Ask overlay and focus the question input.
    /// </summary>
    public void Show()
    {
        Visibility = Visibility.Visible;
        QuickAskCard.Visibility = Visibility.Visible;
        LoadingOverlay.Visibility = Visibility.Collapsed;
        AnswerText.Visibility = Visibility.Collapsed;
        CitationList.Visibility = Visibility.Collapsed;
        InsertToNoteButton.Visibility = Visibility.Collapsed;
        CopyButton.Visibility = Visibility.Collapsed;
        AnswerStatusText.Visibility = Visibility.Collapsed;
        PlaceholderText.Visibility = Visibility.Visible;

        QuestionBox.Text = string.Empty;
        _lastAnswer = null;
        AskButton.IsEnabled = false;
        FooterHintText.Text = "Enter 提问 · Esc 关闭";

        FocusQuestionBox();
    }

    /// <summary>
    /// Hide the overlay and cancel any active request.
    /// </summary>
    public void Dismiss()
    {
        CancelActiveRequest();
        Visibility = Visibility.Collapsed;
    }

    /// <summary>
    /// Focus the question input box.
    /// </summary>
    private void FocusQuestionBox()
    {
        QuestionBox.Focus(FocusState.Programmatic);
    }

    /// <summary>
    /// Send the question to the AI backend and display the answer.
    /// #3674: use Interlocked.Exchange to prevent Dismiss from cancelling
    /// the new request, and capture ct locally to avoid ObjectDisposedException.
    /// </summary>
    private async void SubmitQuestion(string question)
    {
        if (Backend is null)
        {
            ShowError("后端未连接，请先启动 VaultPilot 后端。");
            return;
        }

        if (string.IsNullOrWhiteSpace(question))
            return;

        // #3674: atomically swap CTS so Dismiss() cancels the *previous*
        // request, not this one.
        var newCts = new CancellationTokenSource(TimeSpan.FromSeconds(120));
        var oldCts = Interlocked.Exchange(ref _activeRequestCts, newCts);
        oldCts?.Cancel();
        oldCts?.Dispose();

        // Capture token locally so Dismiss+Dispose doesn't throw
        // ObjectDisposedException when we touch _activeRequestCts.Token later.
        var ct = newCts.Token;

        // Show loading state
        QuickAskCard.Visibility = Visibility.Collapsed;
        LoadingOverlay.Visibility = Visibility.Visible;
        LoadingText.Text = "AI 思考中...";
        LoadingDetailText.Text = "正在检索知识库...";

        try
        {
            var result = await Backend.SendAsync<GroundedAnswer>(
                "askWithAi",
                new
                {
                    question = question,
                },
                ct);

            // #3674: check cancellation after await — if we were dismissed
            // mid-flight, don't overwrite the UI.
            if (ct.IsCancellationRequested)
                return;

            LoadingOverlay.Visibility = Visibility.Collapsed;
            QuickAskCard.Visibility = Visibility.Visible;

            if (result is not null && !string.IsNullOrWhiteSpace(result.Answer))
            {
                _lastAnswer = result.Answer;
                DisplayAnswer(result);
            }
            else
            {
                ShowError("AI 未能生成有效回答，请重试。");
            }
        }
        catch (OperationCanceledException) when (ct.IsCancellationRequested)
        {
            LoadingOverlay.Visibility = Visibility.Collapsed;
            QuickAskCard.Visibility = Visibility.Visible;
            ShowInfo("请求已取消。");
        }
        catch (TimeoutException)
        {
            LoadingOverlay.Visibility = Visibility.Collapsed;
            QuickAskCard.Visibility = Visibility.Visible;
            ShowError("请求超时，后端可能无响应。");
        }
        catch (Exception error) when (!ct.IsCancellationRequested)
        {
            LoadingOverlay.Visibility = Visibility.Collapsed;
            QuickAskCard.Visibility = Visibility.Visible;
            ShowError($"请求失败: {error.Message}");
        }
    }

    /// <summary>
    /// Display the grounded answer in the response area.
    /// </summary>
    private void DisplayAnswer(GroundedAnswer answer)
    {
        AnswerText.Text = answer.Answer;
        AnswerText.Visibility = Visibility.Visible;
        PlaceholderText.Visibility = Visibility.Collapsed;

        // Show citations if available
        if (answer.Citations is { Count: > 0 })
        {
            CitationList.ItemsSource = answer.Citations;
            CitationList.Visibility = Visibility.Visible;
        }
        else
        {
            CitationList.Visibility = Visibility.Collapsed;
        }

        // Show usage info
        if (answer.ContextStatus is not null)
        {
            AnswerStatusText.Text = $"Token: {answer.ContextStatus.LastRequestInputTokens ?? 0} in / {answer.ContextStatus.LastRequestOutputTokens ?? 0} out";
            AnswerStatusText.Visibility = Visibility.Visible;
        }
        else
        {
            AnswerStatusText.Visibility = Visibility.Collapsed;
        }

        InsertToNoteButton.Visibility = Visibility.Visible;
        CopyButton.Visibility = Visibility.Visible;
        FooterHintText.Text = "Esc 关闭 · 插入到笔记 或 复制";
    }

    /// <summary>
    /// Show an error message in the response area.
    /// </summary>
    private void ShowError(string message)
    {
        AnswerText.Text = message;
        AnswerText.Visibility = Visibility.Visible;
        PlaceholderText.Visibility = Visibility.Collapsed;
        CitationList.Visibility = Visibility.Collapsed;
        AnswerStatusText.Visibility = Visibility.Collapsed;
        InsertToNoteButton.Visibility = Visibility.Collapsed;
        CopyButton.Visibility = Visibility.Visible;
        FooterHintText.Text = "Esc 关闭";
    }

    /// <summary>
    /// Show an informational message in the response area.
    /// </summary>
    private void ShowInfo(string message)
    {
        AnswerText.Text = message;
        AnswerText.Visibility = Visibility.Visible;
        PlaceholderText.Visibility = Visibility.Collapsed;
        CitationList.Visibility = Visibility.Collapsed;
        AnswerStatusText.Visibility = Visibility.Collapsed;
        InsertToNoteButton.Visibility = Visibility.Collapsed;
        CopyButton.Visibility = Visibility.Collapsed;
        FooterHintText.Text = "Esc 关闭";
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

    // ── Event handlers ─────────────────────────────────────────────

    private void OnQuestionTextChanged(object sender, TextChangedEventArgs e)
    {
        AskButton.IsEnabled = !string.IsNullOrWhiteSpace(QuestionBox.Text);
    }

    private void OnQuestionBoxKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == Windows.System.VirtualKey.Enter)
        {
            e.Handled = true;
            var question = QuestionBox.Text?.Trim();
            if (!string.IsNullOrWhiteSpace(question))
            {
                SubmitQuestion(question);
            }
        }
        else if (e.Key == Windows.System.VirtualKey.Escape)
        {
            e.Handled = true;
            Dismiss();
        }
    }

    private void OnAskClicked(object sender, RoutedEventArgs e)
    {
        var question = QuestionBox.Text?.Trim();
        if (!string.IsNullOrWhiteSpace(question))
        {
            SubmitQuestion(question);
        }
    }

    private void OnBackdropPressed(object sender, PointerRoutedEventArgs e)
    {
        Dismiss();
    }

    private void OnInsertToNoteClicked(object sender, RoutedEventArgs e)
    {
        if (string.IsNullOrEmpty(_lastAnswer))
            return;

        InsertToNoteRequested?.Invoke(this, _lastAnswer);
        Dismiss();
    }

    private void OnCopyClicked(object sender, RoutedEventArgs e)
    {
        if (string.IsNullOrEmpty(_lastAnswer))
            return;

        try
        {
            var dataPackage = new Windows.ApplicationModel.DataTransfer.DataPackage();
            dataPackage.SetText(_lastAnswer);
            Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(dataPackage);
            FooterHintText.Text = "已复制到剪贴板";
        }
        catch
        {
            FooterHintText.Text = "复制失败";
        }
    }
}
