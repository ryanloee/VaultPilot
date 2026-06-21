using VaultPilot.WinUI.Controls;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using System.Diagnostics;
using System.Linq;
using System.Threading;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Windows.System;

namespace VaultPilot.WinUI;

/// <summary>
/// Chat session management, message sending/receiving, context compression,
/// token estimation, and chat UI rendering — extracted from MainWindow.xaml.cs (#1206).
/// </summary>
public sealed partial class MainWindow : Window
{
    // ── Chat constants ──
    private const double ContextCompressionThreshold = 0.95;
    private const int RecentTurnsAfterCompression = 8;
    private const ulong ImageAttachmentTokenEstimate = 1200;
    private const string MarkdownOpenTag = "<vp-markdown>";
    private const string MarkdownCloseTag = "</vp-markdown>";

    // ── Chat fields ──
    private ChatState _chatState = new(string.Empty, Array.Empty<ChatSession>());
    private readonly SemaphoreSlim _chatStateLock = new(1, 1);
    private string _currentSessionId = string.Empty;
    private readonly List<ChatAttachment> _attachments = [];
    private double _contextUsagePercent;
    private FrameworkElement? _thinkingIndicator;
    private DispatcherTimer? _thinkingDotsTimer;
    private int _thinkingDotStep;
    private CancellationTokenSource? _activeRequestCts;
    private volatile Task? _activeRequestTask;
    private int _requestInProgress; // #676: guard against concurrent ExecuteAiRequestAsync calls
    private GroundedAnswer? _lastAiAnswer;
    private TextBlock? _composerMeasureBlock;

    // ── Chat session loading ──

    private async Task<ChatState> TryLoadChatStateAsync()
    {
        try
        {
            return await SendWithTimeoutAsync(
                (token) => _backendClient.SendAsync<ChatState>("loadChatState", new { }, token),
                "loadChatState")
                ?? new ChatState(string.Empty, Array.Empty<ChatSession>());
        }
        catch (Exception error)
        {
            AppendMessage("错误", $"聊天记录读取失败，已使用空会话：{LocalizeError(error.Message)}");
            return new ChatState(string.Empty, Array.Empty<ChatSession>());
        }
    }

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

    private void OnSessionSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (SessionList.SelectedItem is not SessionListItem item)
        {
            return;
        }

        if (item.Id == _currentSessionId)
        {
            return;
        }

        _currentSessionId = item.Id;
        RenderCurrentSession();
    }

    private void OnChatScrollViewerViewChanged(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        RefreshJumpLatestButton();
    }

    private void OnJumpLatestClicked(object sender, RoutedEventArgs e)
    {
        ScrollToLatest();
    }

    private async void OnDeleteSessionClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            var session = CurrentSession();
            if (session is null)
            {
                return;
            }

            var dialog = new ContentDialog
            {
                XamlRoot = RootGrid.XamlRoot,
                Title = "删除会话",
                Content = $"确认删除「{session.Title}」吗？此操作不可撤销。",
                PrimaryButtonText = "删除",
                CloseButtonText = "取消",
                DefaultButton = ContentDialogButton.Close
            };

            if (await dialog.ShowAsync() != ContentDialogResult.Primary)
            {
                return;
            }

            await _chatStateLock.WaitAsync();
            try
            {
                var remaining = _chatState.Sessions
                    .Where(item => item.Id != session.Id)
                    .ToArray();

                _chatState = new ChatState(
                    remaining.FirstOrDefault()?.Id ?? string.Empty,
                    remaining);
                _currentSessionId = _chatState.CurrentSessionId;
            }
            finally
            {
                _chatStateLock.Release();
            }
            EnsureCurrentSession();
            await SaveChatStateAsync();
            RefreshSessions();
            RenderCurrentSession();

            UpdateStatusBar("success", "会话已删除", $"已删除「{session.Title}」。");
        }
        catch (Exception error)
        {
            ShowError("删除会话失败", error);
        }
    }

    private async void OnNewSessionClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            var now = DateTimeOffset.UtcNow.ToString("O");
            var session = new ChatSession(
                Guid.NewGuid().ToString("N"),
                "新对话",
                Array.Empty<ChatTurn>(),
                null,
                now,
                now);

            await _chatStateLock.WaitAsync();
            try
            {
                _chatState = new ChatState(
                    session.Id,
                    [session, .. _chatState.Sessions]);
                _currentSessionId = session.Id;
            }
            finally
            {
                _chatStateLock.Release();
            }
            EnsureCurrentSession();
            await SaveChatStateAsync();
            RefreshSessions();
            RenderCurrentSession();
            ScrollToLatest();

            UpdateStatusBar("success", "新对话", "已创建新对话。");
        }
        catch (Exception error)
        {
            ShowError("新建会话失败", error);
        }
    }

    private void OnComposerTextChanged(object sender, TextChangedEventArgs e)
    {
        var textBox = (TextBox)sender;
        if (textBox.ActualWidth <= 0) return;

        _composerMeasureBlock ??= new TextBlock
        {
            FontFamily = textBox.FontFamily,
            FontSize = textBox.FontSize,
            FontWeight = textBox.FontWeight,
            TextWrapping = TextWrapping.Wrap,
        };

        _composerMeasureBlock.Text = textBox.Text ?? string.Empty;
        var availableWidth = textBox.ActualWidth - 20; // padding + scrollbar
        _composerMeasureBlock.Measure(new Windows.Foundation.Size(availableWidth, double.PositiveInfinity));

        var desiredHeight = _composerMeasureBlock.DesiredSize.Height + 20; // inner padding
        var clampedHeight = Math.Max(88, Math.Min(200, desiredHeight));
        textBox.Height = clampedHeight;
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
                    // When text is available, prefer default paste to avoid text loss
                    // when StorageItems contain no images (regression from #627 fix).
                    bool suppressForImagePaste = true;
                    try
                    {
                        var content = Clipboard.GetContent();
                        suppressForImagePaste = content?.Contains(StandardDataFormats.Text) != true;
                    }
                    catch { /* clipboard access can fail; default to image paste attempt */ }

                    if (suppressForImagePaste)
                    {
                        // Set Handled pre-emptively to block the default paste handler
                        // during the await. Reset if image paste doesn't apply. (#627)
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
                // at the cursor position for Shift+Enter.
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

        if (_lastAiAnswer?.SavedNote is null)
        {
            throw new InvalidOperationException("知识库写入未完成，模型未返回已保存笔记。");
        }

        var savedNote = _lastAiAnswer.SavedNote;
        AppendMessage("系统", $"已保存笔记：{savedNote.Title}");
        ScrollToLatest();
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
        var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { }, cts.Token);
        _noteCount = notes?.Count ?? 0;
        RefreshVaultSummary();

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
            var aiTimeout = TimeSpan.FromMilliseconds(
                (_settings?.Provider.RequestTimeoutMs ?? 60_000) + 30_000);
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
        try { Volatile.Read(ref _activeRequestCts)?.Cancel(); }
        catch (ObjectDisposedException) { }
    }

    // ── Chat rendering ──

    private void EnsureCurrentSession()
    {
        if (_chatState.Sessions.Count > 0)
        {
            _currentSessionId = string.IsNullOrWhiteSpace(_chatState.CurrentSessionId)
                ? _chatState.Sessions[0].Id
                : _chatState.CurrentSessionId;
            return;
        }

        var now = DateTimeOffset.UtcNow.ToString("O");
        _currentSessionId = Guid.NewGuid().ToString("N");
        _chatState = new ChatState(
            _currentSessionId,
            new[]
            {
                new ChatSession(
                    _currentSessionId,
                    "新对话",
                    Array.Empty<ChatTurn>(),
                    null,
                    now,
                    now)
            });
    }

    private void RenderCurrentSession()
    {
        MessagesPanel.Children.Clear();
        var session = CurrentSession();
        if (session is null || session.Turns.Count == 0)
        {
            ShowEmptyState();
            RefreshContextStatus();
            return;
        }

        foreach (var turn in session.Turns)
        {
            var isScheduledWake = turn.Source == "scheduled_wake";
            var author = turn.Role == "user"
                ? (isScheduledWake ? "⏰ 定时唤醒" : "你")
                : (isScheduledWake && turn.Text.StartsWith("⏰") ? "⏰ 定时唤醒" : "助手");
            AppendMessage(author, turn.Text);
            if (turn.Attachments is { Count: > 0 })
            {
                AppendAttachmentPreviews(turn.Attachments, turn.Role);
            }

            if (turn.Role == "assistant")
            {
                if (turn.ThinkingTrace is { Steps.Count: > 0 } trace)
                {
                    AppendThinkingTrace(trace);
                }

                if (turn.Citations is { Count: > 0 } citations)
                {
                    AppendCitationCards(citations);
                }

                if (turn.SavedNote is not null)
                {
                    AppendMessage("系统", $"已保存笔记：{turn.SavedNote.Title}");
                }
            }
        }
        RefreshContextStatus();
    }

    private void ShowEmptyState()
    {
        var isFirstRun = string.IsNullOrEmpty(_settings?.VaultDir);

        var icon = new FontIcon
        {
            Glyph = isFirstRun ? "&#xE736;" : "&#xE8BD;",
            FontSize = 48,
            Opacity = 0.4,
            HorizontalAlignment = HorizontalAlignment.Center,
        };

        var title = new TextBlock
        {
            Text = isFirstRun ? "欢迎使用 VaultPilot" : "开始新的对话",
            Style = GetThemeStyle("SubtitleTextBlockStyle"),
            HorizontalAlignment = HorizontalAlignment.Center,
            TextAlignment = TextAlignment.Center,
        };

        var subtitle = new TextBlock
        {
            Text = isFirstRun
                ? "请先在设置中配置 API Key 和知识库目录，然后就可以开始对话了。"
                : "在下方输入框中输入问题，或试试这些示例：",
            Opacity = 0.7,
            HorizontalAlignment = HorizontalAlignment.Center,
            TextAlignment = TextAlignment.Center,
            TextWrapping = TextWrapping.Wrap,
            MaxWidth = 400,
        };

        var container = new StackPanel
        {
            Spacing = 12,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(0, 80, 0, 0),
        };
        container.Children.Add(icon);
        container.Children.Add(title);
        container.Children.Add(subtitle);

        if (!isFirstRun)
        {
            var suggestions = new[]
            {
                "帮我总结一下最近的笔记",
                "搜索关于项目管理的内容",
                "记录一条新想法",
            };
            foreach (var suggestion in suggestions)
            {
                var btn = new Button
                {
                    Content = suggestion,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    Style = GetThemeStyle("DefaultButtonStyle"),
                };
                AutomationProperties.SetName(btn, $"建议: {suggestion}");
                btn.Click += (_, _) =>
                {
                    ComposerBox.Text = suggestion;
                    ComposerBox.Focus(FocusState.Programmatic);
                };
                container.Children.Add(btn);
            }
        }
        else
        {
            var settingsBtn = new Button
            {
                Content = "打开设置",
                HorizontalAlignment = HorizontalAlignment.Center,
            };
            AutomationProperties.SetName(settingsBtn, "打开设置");
            settingsBtn.Click += (_, _) => OnSettingsClicked(settingsBtn, new RoutedEventArgs());
            container.Children.Add(settingsBtn);
        }

        MessagesPanel.Children.Add(container);
    }

    private void AppendMessage(string author, string text)
    {
        var isUser = author == "你";
        var isAssistant = author == "助手";
        var bubbleText = isUser || isAssistant ? text : $"{author}: {text}";
        var bubbleContent = CreateMessageContent(bubbleText, isAssistant, isUser);

        var bubble = new Border
        {
            MaxWidth = 680,
            Padding = new Thickness(12, 9, 12, 9),
            CornerRadius = new CornerRadius(8),
            Background = isUser
                ? GetThemeBrush("AccentFillColorDefaultBrush")
                : GetThemeBrush("CardBackgroundFillColorSecondaryBrush"),
            BorderBrush = isUser
                ? null
                : GetThemeBrush("CardStrokeColorDefaultBrush"),
            BorderThickness = isUser ? new Thickness(0) : new Thickness(1),
            HorizontalAlignment = isUser ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            Child = bubbleContent
        };

        var label = new TextBlock
        {
            Text = author,
            Opacity = 0.72,
            HorizontalAlignment = bubble.HorizontalAlignment
        };
        AutomationProperties.SetName(bubble, isUser ? "用户消息" : "AI 消息");

        var stack = new StackPanel
        {
            Spacing = 4,
            HorizontalAlignment = isUser ? HorizontalAlignment.Right : HorizontalAlignment.Left
        };
        stack.Children.Add(label);
        stack.Children.Add(bubble);

        if (!isUser && !isAssistant)
        {
            stack.Children.Remove(label);
        }

        MessagesPanel.Children.Add(stack);
    }

    private void ShowThinkingIndicator()
    {
        RemoveThinkingIndicator();

        _thinkingDotStep = 0;

        var dotBrush = GetThemeBrush("TextFillColorPrimaryBrush");
        var dots = new TextBlock[3];
        for (var i = 0; i < 3; i++)
        {
            dots[i] = new TextBlock
            {
                Text = "●",
                Opacity = 0.25,
                FontSize = 12,
                Foreground = dotBrush,
                VerticalAlignment = VerticalAlignment.Center,
            };
        }

        var dotsPanel = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 6,
            Padding = new Thickness(2, 2, 2, 2),
        };
        foreach (var dot in dots)
        {
            dotsPanel.Children.Add(dot);
        }

        var bubble = new Border
        {
            MaxWidth = 680,
            Padding = new Thickness(14, 10, 14, 10),
            CornerRadius = new CornerRadius(8),
            Background = GetThemeBrush("CardBackgroundFillColorSecondaryBrush"),
            BorderBrush = GetThemeBrush("CardStrokeColorDefaultBrush"),
            BorderThickness = new Thickness(1),
            HorizontalAlignment = HorizontalAlignment.Left,
            Child = dotsPanel,
        };

        var label = new TextBlock
        {
            Text = "助手",
            Opacity = 0.72,
            HorizontalAlignment = HorizontalAlignment.Left,
        };

        var stack = new StackPanel
        {
            Spacing = 4,
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        // Note: LiveSetting is not available in WinUI 3; using automation name only
        AutomationProperties.SetName(stack, "AI 正在思考");
        stack.Children.Add(label);
        stack.Children.Add(bubble);

        _thinkingIndicator = stack;
        MessagesPanel.Children.Add(stack);

        _thinkingDotsTimer = new DispatcherTimer
        {
            Interval = TimeSpan.FromMilliseconds(350),
        };
        _thinkingDotsTimer.Tick += (_, _) =>
        {
            _thinkingDotStep = (_thinkingDotStep + 1) % 4;
            for (var i = 0; i < 3; i++)
            {
                dots[i].Opacity = i < _thinkingDotStep ? 1.0 : 0.25;
            }
        };
        _thinkingDotsTimer.Start();
    }

    private void RemoveThinkingIndicator()
    {
        _thinkingDotsTimer?.Stop();
        _thinkingDotsTimer = null;
        _thinkingDotStep = 0;
        if (_thinkingIndicator is not null)
        {
            MessagesPanel.Children.Remove(_thinkingIndicator);
            _thinkingIndicator = null;
        }
    }

    private void CopyTextToClipboard(string text)
    {
        try
        {
            var package = new DataPackage();
            package.SetText(text);
            Clipboard.SetContent(package);
            Clipboard.Flush();
            UpdateStatusBar("success", "已复制", "消息内容已复制到剪贴板。");
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"Clipboard copy failed: {ex.Message}");
            UpdateStatusBar("warning", "复制失败", "无法写入剪贴板，可能被其他程序占用。");
        }
    }

    private void AppendThinkingTrace(ThinkingTrace trace)
    {
        var stepsPanel = new StackPanel { Spacing = 4 };
        foreach (var step in trace.Steps)
        {
            var stepBlock = new TextBlock
            {
                Text = $"• {step.Title}: {step.Detail}",
                FontSize = 12,
                Opacity = 0.7,
                TextWrapping = TextWrapping.Wrap
            };
            stepsPanel.Children.Add(stepBlock);
        }

        var expander = new Expander
        {
            Header = $"💭 思考过程 ({trace.Steps.Count} 步){(string.IsNullOrWhiteSpace(trace.Summary) ? "" : $" — {trace.Summary}")}",
            IsExpanded = false,
            HorizontalAlignment = HorizontalAlignment.Left,
            MaxWidth = 680,
            Content = stepsPanel
        };
        AutomationProperties.SetName(expander, $"思考过程: {trace.Steps.Count} 步");

        MessagesPanel.Children.Add(expander);
    }

    private void AppendCitationCards(IReadOnlyList<AnswerCitation> citations)
    {
        var citationsPanel = new StackPanel
        {
            Spacing = 4,
            HorizontalAlignment = HorizontalAlignment.Left,
            MaxWidth = 680,
            Margin = new Thickness(0, 4, 0, 0)
        };

        var header = new TextBlock
        {
            Text = $"📚 引用 ({citations.Count})",
            FontSize = 12,
            Opacity = 0.7,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold
        };
        citationsPanel.Children.Add(header);

        foreach (var citation in citations)
        {
            var card = new Border
            {
                Background = GetThemeBrush("CardBackgroundFillColorDefaultBrush"),
                BorderBrush = GetThemeBrush("CardStrokeColorDefaultBrush"),
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(4),
                Padding = new Thickness(8, 4, 8, 4),
                Child = new StackPanel
                {
                    Spacing = 2,
                    Children =
                    {
                        new TextBlock
                        {
                            Text = citation.Title,
                            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                            FontSize = 12
                        },
                        new TextBlock
                        {
                            Text = citation.Snippet,
                            FontSize = 11,
                            Opacity = 0.8,
                            TextWrapping = TextWrapping.Wrap,
                            MaxLines = 3,
                            TextTrimming = TextTrimming.CharacterEllipsis
                        }
                    }
                }
            };
            citationsPanel.Children.Add(card);
        }

        MessagesPanel.Children.Add(citationsPanel);
    }

    // ── Chat session queries ──

    private ChatSession? CurrentSession()
    {
        return _chatState.Sessions.FirstOrDefault(session => session.Id == _currentSessionId)
            ?? _chatState.Sessions.FirstOrDefault();
    }

    private ChatSession? FindSessionById(string sessionId)
    {
        return _chatState.Sessions.FirstOrDefault(session => session.Id == sessionId);
    }

    private ConversationTurn[] GetConversationHistory(string sessionId)
    {
        var session = FindSessionById(sessionId) ?? CurrentSession();
        if (session is null)
        {
            return Array.Empty<ConversationTurn>();
        }

        var history = new List<ConversationTurn>();
        if (!string.IsNullOrWhiteSpace(session.Summary?.Text))
        {
            history.Add(new ConversationTurn("system", $"此前对话摘要：{session.Summary.Text}"));
        }

        history.AddRange(session.Turns
            .Where(turn => !string.IsNullOrWhiteSpace(turn.Text))
            .Select(turn => new ConversationTurn(turn.Role, turn.Text)));
        return history.ToArray();
    }

    // ── Context compression ──

    private async Task CompressCurrentSessionIfNeededAsync(string sessionId,
        string pendingText,
        IReadOnlyList<ChatAttachment> pendingAttachments,
        CancellationToken cancellationToken = default)
    {
        var session = FindSessionById(sessionId) ?? CurrentSession();
        if (session is null)
        {
            return;
        }

        var contextWindow = ResolveContextWindowTokens();
        var projectedTokens = EstimateSessionTokens(session) + EstimateTurnTokens(pendingText, pendingAttachments);
        if (contextWindow == 0 || projectedTokens < (ulong)(contextWindow * ContextCompressionThreshold))
        {
            return;
        }

        var compressibleCount = Math.Max(0, session.Turns.Count - RecentTurnsAfterCompression);
        if (compressibleCount < 2)
        {
            UpdateStatusBar("warning", "上下文接近上限", "可压缩的历史消息太少，将继续发送当前请求。");
            return;
        }

        UpdateStatusBar("info", "正在压缩上下文", "历史对话已接近上限，正在自动生成摘要...");

        var compressibleTurns = session.Turns
            .Take(compressibleCount)
            .Where(turn => !string.IsNullOrWhiteSpace(turn.Text))
            .Select(turn => new ConversationTurn(turn.Role, turn.Text))
            .ToArray();
        if (compressibleTurns.Length < 2)
        {
            return;
        }

        using var cts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        cts.CancelAfter(TimeSpan.FromSeconds(30));
        var summary = await _backendClient.SendAsync<ConversationSummary>(
            "compressChatHistory",
            new
            {
                summary = session.Summary,
                history = compressibleTurns
            },
            cts.Token);
        if (summary is null)
        {
            return;
        }

        var now = DateTimeOffset.UtcNow.ToString("O");
        var updated = session with
        {
            Summary = summary,
            Turns = session.Turns.Skip(compressibleCount).ToArray(),
            UpdatedAt = now
        };
        await _chatStateLock.WaitAsync();
        try
        {
            var sessions = _chatState.Sessions
                .Select(item => item.Id == updated.Id ? updated : item)
                .ToArray();
            _chatState = new ChatState(updated.Id, sessions);
            _currentSessionId = updated.Id;
        }
        finally
        {
            _chatStateLock.Release();
        }
        await SaveChatStateAsync();
        RefreshSessions();
        RenderCurrentSession();
    }

    // ── Context status & token estimation ──

    private void RefreshContextStatus()
    {
        var session = CurrentSession();
        var contextWindow = ResolveContextWindowTokens();
        var usedTokens = session is null ? 0 : EstimateSessionTokens(session);
        var remainingTokens = usedTokens >= contextWindow ? 0 : contextWindow - usedTokens;
        var remainingPercent = contextWindow == 0
            ? 100.0
            : Math.Clamp((double)remainingTokens / contextWindow * 100.0, 0.0, 100.0);
        var usedPercent = Math.Clamp(100.0 - remainingPercent, 0.0, 100.0);
        var usageBrush = remainingPercent switch
        {
            > 50 => BrushLimeGreen,
            > 20 => BrushOrange,
            _ => BrushRed
        };
        _contextUsagePercent = usedPercent;

        ContextUsageFill.Background = usageBrush;
        UpdateContextUsageBarVisual();

        var tooltip = $"上下文已用：{usedPercent:0.#}%；剩余：{remainingPercent:0.#}%（约 {FormatTokenCount(usedTokens)} / {FormatTokenCount(contextWindow)}）";
        ToolTipService.SetToolTip(ContextUsageBarHost, tooltip);
        ToolTipService.SetToolTip(ContextUsageTrack, tooltip);
        ToolTipService.SetToolTip(ContextUsageFill, tooltip);
    }

    private void OnContextUsageBarHostSizeChanged(object sender, SizeChangedEventArgs e)
    {
        UpdateContextUsageBarVisual();
    }

    private void UpdateContextUsageBarVisual()
    {
        var width = ContextUsageBarHost.ActualWidth;
        if (width <= 0)
        {
            return;
        }

        ContextUsageFill.Width = width * (_contextUsagePercent / 100.0);
    }

    private ulong EstimateSessionTokens(ChatSession session)
    {
        var total = EstimateTokensForText(session.Summary?.Text);
        foreach (var turn in session.Turns)
        {
            total += EstimateTurnTokens(turn.Text, turn.Attachments ?? Array.Empty<ChatAttachment>());
        }
        return total;
    }

    private static ulong EstimateTurnTokens(string? text, IReadOnlyList<ChatAttachment> attachments)
    {
        return EstimateTokensForText(text) + (ulong)attachments.Count * ImageAttachmentTokenEstimate;
    }

    private static ulong EstimateTokensForText(string? text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return 0;
        }

        ulong ascii = 0;
        ulong nonAscii = 0;
        foreach (var item in text)
        {
            if (char.IsWhiteSpace(item))
            {
                continue;
            }

            if (item <= 0x7f)
            {
                ascii++;
            }
            else
            {
                nonAscii++;
            }
        }

        return nonAscii + ((ascii + 3) / 4);
    }

    private ulong ResolveContextWindowTokens()
    {
        var configuredLimit = _settings?.Provider.ContextWindowTokens;
        if (configuredLimit.HasValue && configuredLimit.Value > 0)
        {
            return configuredLimit.Value;
        }

        var model = (_settings?.Provider.Model ?? string.Empty).Trim().ToLowerInvariant();
        if (ContainsModelToken(model, "glm-5.1"))
        {
            return 200_000;
        }
        if (ContainsModelToken(model, "claude"))
        {
            return ContainsModelToken(model, "1m") ? 1_000_000UL : 200_000;
        }
        if (ContainsModelToken(model, "gpt-4.1") || ContainsModelToken(model, "gpt-5"))
        {
            return 1_047_576;
        }
        if (ContainsModelToken(model, "gpt-4o"))
        {
            return 128_000;
        }
        if (IsOpenAiOSeriesModel(model))
        {
            return 200_000;
        }
        if (ContainsModelToken(model, "gemini"))
        {
            return 1_000_000;
        }

        return 128_000;
    }

    // ── Turn management ──

    private async Task AddTurnAsync(
        string role,
        string text,
        GroundedAnswer? answer = null,
        IReadOnlyList<ChatAttachment>? attachments = null,
        string? sessionId = null,
        string? source = null)
    {
        await _chatStateLock.WaitAsync();
        try
        {
            ChatSession? session;
            if (sessionId is not null)
            {
                session = _chatState.Sessions.FirstOrDefault(s => s.Id == sessionId);
            }
            else
            {
                session = CurrentSession();
            }
            if (session is null)
            {
                EnsureCurrentSession();
                session = sessionId is not null
                    ? _chatState.Sessions.FirstOrDefault(s => s.Id == sessionId)
                    : CurrentSession();
            }

            if (session is null)
            {
                return;
            }

            var now = DateTimeOffset.UtcNow.ToString("O");
            var turn = new ChatTurn(
                Guid.NewGuid().ToString("N"),
                role,
                text,
                answer?.Citations,
                answer?.SavedNote,
                answer?.ThinkingTrace,
                attachments ?? Array.Empty<ChatAttachment>(),
                now,
                source);

            var turns = new List<ChatTurn>(session.Turns.Count + 1);
            turns.AddRange(session.Turns);
            turns.Add(turn);
            var title = session.Title == "新对话" && role == "user"
                ? BuildSessionTitle(text)
                : session.Title;
            var updated = session with { Title = title, Turns = turns, UpdatedAt = now };
            var sessions = _chatState.Sessions
                .Select(item => item.Id == updated.Id ? updated : item)
                .OrderByDescending(item => item.UpdatedAt)
                .ToArray();

            _chatState = new ChatState(updated.Id, sessions);
            _currentSessionId = updated.Id;
        }
        finally
        {
            _chatStateLock.Release();
        }
    }

    private async Task SaveChatStateAsync()
    {
        ChatState snapshot;
        await _chatStateLock.WaitAsync();
        try
        {
            snapshot = _chatState;
        }
        finally
        {
            _chatStateLock.Release();
        }

        try
        {
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            await _backendClient.SendAsync<ChatState>(
                "saveChatState",
                new { state = snapshot },
                cts.Token);
        }
        catch (Exception error)
        {
            UpdateStatusBar("warning", "聊天记录未保存", LocalizeError(error.Message));
        }
    }

    private void RefreshSessions()
    {
        SessionList.ItemsSource = _chatState.Sessions
            .Select(session => new SessionListItem(
                session.Id,
                session.Title,
                $"{session.Turns.Count} 条消息",
                ToRelativeTime(session.UpdatedAt),
                session.Turns.Count,
                session.Summary?.Text is { Length: > 0 } summary
                    ? (summary.Length <= 50 ? summary : $"{summary[..50]}...")
                    : string.Empty))
            .ToList();
        SessionList.SelectedItem = SessionList.Items
            .OfType<SessionListItem>()
            .FirstOrDefault(item => item.Id == _currentSessionId);
        DeleteSessionButton.IsEnabled = _chatState.Sessions.Count > 0;
    }

    // ── Session list item ──

    private sealed record SessionListItem(
        string Id,
        string Title,
        string Detail,
        string RelativeTime,
        int TurnCount,
        string SummaryPreview);
}
