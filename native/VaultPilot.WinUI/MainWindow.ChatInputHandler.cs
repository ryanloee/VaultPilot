using System.Diagnostics;
using VaultPilot.WinUI.Controls;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Windows.System;

namespace VaultPilot.WinUI;

/// <summary>
/// Chat input handling — composer keyboard shortcuts, send/record actions, and
/// the shared ExecuteAiRequestAsync flow — split from MainWindow.Chat.cs (#1344).
/// </summary>
public sealed partial class MainWindow : Window
{
    // ── Chat event handlers ──

    private async void OnSendClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            await SendCurrentMessageAsync();
        }
        catch (Exception error)
        {
            ShowError("发送消息失败", error);
        }
    }

    private async void OnRecordClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            await RecordCurrentMessageAsync();
        }
        catch (Exception error)
        {
            ShowError("录音消息失败", error);
        }
    }

    private void OnComposerTextChanged(object sender, TextChangedEventArgs e)
    {
        var textBox = (TextBox)sender;
        if (textBox.ActualWidth <= 0) return;

        // #X: Simple line-count estimation instead of expensive TextBlock.Measure().
        // The old code created a TextBlock and called Measure() on every keystroke,
        // which caused severe lag with long text (#X).
        var text = textBox.Text ?? string.Empty;
        if (string.IsNullOrEmpty(text))
        {
            textBox.Height = 40;
            return;
        }

        // Approximate: count explicit newlines + wrap estimate
        var newlineCount = text.Count(c => c == '\n');
        // Estimate visible line width: ~80 chars per line for the configured font
        var wrapLines = text.Length / 80;
        var estimatedLines = Math.Max(1, newlineCount + wrapLines);
        var lineHeight = textBox.FontSize + 6; // ~26px for 14pt font
        var estimatedHeight = Math.Max(40, Math.Min(140, estimatedLines * lineHeight));
        textBox.Height = estimatedHeight;
    }

    private async void OnComposerKeyDown(object sender, KeyRoutedEventArgs e)
    {
        try
        {
            if (e.Key == VirtualKey.V)
            {
                var controlState = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control);
                if (controlState.HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down))
                {
                    // #805: Only suppress default paste when clipboard has no text content.
                    bool suppressForImagePaste = true;
                    try
                    {
                        var content = Clipboard.GetContent();
                        suppressForImagePaste = content?.Contains(StandardDataFormats.Text) != true;
                    }
                    catch { /* clipboard access can fail; default to image paste attempt */ }

                    if (suppressForImagePaste)
                    {
                        e.Handled = true;
                        if (await TryHandleClipboardImagePasteAsync())
                        {
                            return;
                        }
                        e.Handled = false;
                    }
                }
            }

            if (e.Key != VirtualKey.Enter)
            {
                return;
            }

            var shiftState = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift);
            if (shiftState.HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down))
            {
                // #859: AcceptsReturn is false so we manually insert a newline
                var cursorPos = ComposerBox.SelectionStart;
                ComposerBox.Text = ComposerBox.Text.Insert(cursorPos, Environment.NewLine);
                ComposerBox.SelectionStart = cursorPos + Environment.NewLine.Length;
                e.Handled = true;
                return;
            }

            e.Handled = true;
            await SendCurrentMessageAsync();
        }
        catch (Exception error)
        {
            ShowError("键盘事件处理失败", error);
        }
    }

    // ── Message sending ──

    private async Task SendCurrentMessageAsync()
    {
        if (!SendButton.IsEnabled)
        {
            return;
        }

        var text = ComposerBox.Text.Trim();
        if (string.IsNullOrEmpty(text) && _attachments.Count == 0)
        {
            return;
        }

        var pendingAttachments = _attachments.ToArray();
        var prompt = string.IsNullOrWhiteSpace(text)
            ? "请结合我发送的图片理解并回复。"
            : text;
        var userDisplay = string.IsNullOrWhiteSpace(text)
            ? "（发送了一张图片）"
            : text;

        await ExecuteAiRequestAsync(
            prompt, userDisplay, pendingAttachments, text,
            "助手处理中", "正在准备请求...", "请求失败");

        if (_lastAiAnswer?.SavedNote is not null)
        {
            AppendMessage("系统", $"已保存笔记：{_lastAiAnswer.SavedNote.Title}");
            ScrollToLatest();
        }

        RestoreIdleStatus();
    }

    private async Task RecordCurrentMessageAsync()
    {
        if (!RecordButton.IsEnabled)
        {
            return;
        }

        var text = ComposerBox.Text.Trim();
        var pendingAttachments = _attachments.ToArray();

        if (string.IsNullOrEmpty(text) && pendingAttachments.Length == 0)
        {
            var session = CurrentSession();
            var lastAssistantTurn = session?.Turns.LastOrDefault(t => t.Role == "assistant");
            if (lastAssistantTurn is null)
            {
                return;
            }

            text = $"请将刚才讨论的内容整理记录到知识库";
        }

        var prompt = $"请将以下内容记录到知识库：{text}";
        var userDisplay = string.IsNullOrWhiteSpace(ComposerBox.Text)
            ? "（记录了当前对话内容）"
            : ComposerBox.Text.Trim();

        await ExecuteAiRequestAsync(
            prompt, userDisplay, pendingAttachments, text,
            "正在记录知识", "正在整理并保存...", "记录失败");

        // If the AI request itself failed (network error, timeout, etc.),
        // ExecuteAiRequestAsync already caught and displayed the error via
        // ShowError, and _lastAiAnswer remains null. Return early to avoid
        // throwing a misleading "知识库写入未完成" error that the caller would
        // display as a second, confusing error dialog. (Issue #2585)
        if (_lastAiAnswer is null)
        {
            return;
        }

        if (_lastAiAnswer.SavedNote is null)
        {
            throw new InvalidOperationException("知识库写入未完成，模型未返回已保存笔记。");
        }

        var savedNote = _lastAiAnswer.SavedNote;
        AppendMessage("系统", $"已保存笔记：{savedNote.Title}");
        ScrollToLatest();
        try
        {
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { }, cts.Token);
            _noteCount = notes?.Count ?? 0;
            RefreshVaultSummary();
            InvalidateNoteTitleCache();
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[VaultPilot] listNotes after save failed: {ex.Message}");
            // Note was saved successfully (user already saw confirmation),
            // vault summary/note count may be stale until next refresh.
            // Don't propagate — the save succeeded, this is just a cache update.
        }

        RestoreIdleStatus("知识已记录", $"已保存为笔记：{savedNote.Title}");
    }

    /// <summary>
    /// Shared implementation for Send and Record flows: clears the composer,
    /// sends the prompt to the AI backend, and updates the session.
    /// </summary>
    private async Task ExecuteAiRequestAsync(
        string prompt,
        string userDisplay,
        ChatAttachment[] pendingAttachments,
        string originalText,
        string statusTitle,
        string statusDetail,
        string errorTitle)
    {
        // #676: Guard against concurrent requests from rapid button clicks
        if (Interlocked.CompareExchange(ref _requestInProgress, 1, 0) != 0)
        {
            return;
        }

        try
        {
        var newCts = new CancellationTokenSource();
        var oldCts = Interlocked.Exchange(ref _activeRequestCts, newCts);
        oldCts?.Dispose();
        var cancellationToken = newCts.Token;

        var completionSignal = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        Volatile.Write(ref _activeRequestTask, completionSignal.Task);

        _lastAiAnswer = null;

        // Capture session ID at request start to prevent response landing
        // in a different session if user switches during the AI call (#626)
        var requestSessionId = _currentSessionId;

        try
        {
            SendButton.IsEnabled = false;
            RecordButton.IsEnabled = false;
            NewSessionButton.IsEnabled = false;
            DeleteSessionButton.IsEnabled = false;
            CancelButton.Visibility = Visibility.Visible;
            ComposerBox.Text = string.Empty;
            _attachments.Clear();
            RefreshAttachments();
            ShowLoadingOverlay(statusTitle);
            UpdateStatusBar("info", statusTitle, statusDetail);

            await CompressCurrentSessionIfNeededAsync(requestSessionId, prompt, pendingAttachments, cancellationToken);
            var history = GetConversationHistory(requestSessionId);
            await AddTurnAsync("user", userDisplay, attachments: pendingAttachments, sessionId: requestSessionId);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();

            ShowThinkingIndicator();
            ScrollToLatest();

            // Issue #710: use the user-configured request timeout (plus buffer for
            // IPC overhead) instead of the hardcoded 90s default.
            // #X: increased buffer from 30s to 90s to prevent premature timeout
            // when the agent has a 120s internal timeout.
            var aiTimeout = TimeSpan.FromMilliseconds(
                (_settings?.Provider.RequestTimeoutMs ?? 60_000) + 90_000);
            var answer = await _backendClient.SendAsync<GroundedAnswer>(
                    "askWithAi",
                    new
                    {
                        question = prompt,
                        history,
                        imagePaths = pendingAttachments.Select(item => item.Path).ToArray()
                    },
                    cancellationToken,
                    aiTimeout);
            RemoveThinkingIndicator();
            _lastAiAnswer = answer;

            await AddTurnAsync("assistant", answer?.Answer ?? string.Empty, answer, sessionId: requestSessionId);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();
        }
        catch (Exception error)
        {
            RemoveThinkingIndicator();
            ComposerBox.Text = originalText;
            _attachments.AddRange(pendingAttachments);
            RefreshAttachments();
            var message = LocalizeError(error.Message);
            await AddTurnAsync("assistant", message, sessionId: requestSessionId);
            RenderCurrentSession();
            ScrollToLatest();
            if (!_isShuttingDown)
            {
                await SaveChatStateAsync();
            }
            ShowError(errorTitle, error, addMessage: false);
        }
        finally
        {
            Interlocked.Exchange(ref _activeRequestCts, null)?.Dispose();
            completionSignal.TrySetResult(true);
            Volatile.Write(ref _activeRequestTask, null);
            SendButton.IsEnabled = true;
            RecordButton.IsEnabled = true;
            NewSessionButton.IsEnabled = true;
            // DeleteSessionButton.IsEnabled restored by RefreshSessions()
            CancelButton.Visibility = Visibility.Collapsed;
            HideLoadingOverlay();
            RefreshSessions();
        }
        }
        finally
        {
            Interlocked.Exchange(ref _requestInProgress, 0);
        }
    }

    /// <summary>
    /// Cancels the currently active AI request, if any.
    /// Safe to call when no request is in progress.
    /// </summary>
    public void CancelActiveRequest()
    {
        // #2732: Only cancel, don't dispose or null out _activeRequestCts.
        // The inner finally block in ExecuteAiRequestAsync is the sole owner
        // of disposal. If we exchange-and-dispose here, we race with the
        // finally block: it sees null and skips disposal, leaking the CTS.
        var cts = Volatile.Read(ref _activeRequestCts);
        if (cts != null)
        {
            try { cts.Cancel(); }
            catch (ObjectDisposedException) { }
        }
    }
}
