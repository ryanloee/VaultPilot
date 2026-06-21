using VaultPilot.WinUI.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System.Diagnostics;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace VaultPilot.WinUI;

/// <summary>
/// Chat request flow, session management, rendering, and context management
/// extracted from MainWindow.xaml.cs (#1206).
/// </summary>
public sealed partial class MainWindow : Window
{
    private GroundedAnswer? _lastAiAnswer;

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

    private void OnAgentStatusReceived(AgentStatusEvent status)
    {
        DispatcherQueue.TryEnqueue(() =>
        {
            UpdateStatusBar("info", LocalizeStage(status.Stage), LocalizeStatusDetail(status.Detail));
        });
    }

    private void OnConnectionStateChanged(bool connected)
    {
        DispatcherQueue.TryEnqueue(() =>
        {
            if (connected)
            {
                UpdateStatusBar("success", "后端已连接", "连接已恢复");
            }
            else
            {
                UpdateStatusBar("warning", "后端断开", "正在尝试重新连接...");
            }
        });
    }

    private void OnClosed(object sender, WindowEventArgs args)
    {
        if (_isShuttingDown)
        {
            // ShutdownAsync already performed full cleanup; nothing to do.
            return;
        }

        // We are NOT truly exiting — this is a hide-to-tray close.
        // Do NOT cancel the active request — let it complete in the background
        // so the user doesn't lose their in-flight AI response (issue #636).
        // Do NOT dispose the backend or unsubscribe events so the window
        // can be re-shown from the tray.
        try
        {
            RemoveThinkingIndicator();
        }
        catch (Exception error)
        {
            ShowError("关闭窗口失败", error);
        }
    }

    /// <summary>
    /// Unsubscribes all event handlers registered in the constructor to prevent
    /// memory leaks from dangling references after the window is closed.
    /// </summary>
    private void UnsubscribeEvents()
    {
        _backendClient.AgentStatusReceived -= OnAgentStatusReceived;
        _backendClient.ConnectionStateChanged -= OnConnectionStateChanged;
        RootGrid.Loaded -= OnLoaded;
        SendButton.Click -= OnSendClicked;
        RecordButton.Click -= OnRecordClicked;
        SettingsButton.Click -= OnSettingsClicked;
        RebuildButton.Click -= OnRebuildClicked;
        ImportButton.Click -= OnImportClicked;
        ComposerBox.KeyDown -= OnComposerKeyDown;
        ComposerBox.TextChanged -= OnComposerTextChanged;
        SessionList.SelectionChanged -= OnSessionSelectionChanged;
        DeleteSessionButton.Click -= OnDeleteSessionClicked;
        NewSessionButton.Click -= OnNewSessionClicked;
        ToggleSidebarButton.Click -= OnToggleSidebarClicked;
        ChatScrollViewer.ViewChanged -= OnChatScrollViewerViewChanged;
        JumpLatestButton.Click -= OnJumpLatestClicked;
        RootGrid.SizeChanged -= OnRootGridSizeChanged;
    }

    /// <summary>
    /// Performs all cleanup (backend client disposal, resource release) before
    /// the application exits.  Called from the tray "Exit" handler so that
    /// MainWindow.OnClosed logic is executed even when the window is only
    /// hidden to tray.  See: https://github.com/user/repo/issues/62
    /// </summary>
    public async Task ShutdownAsync()
    {
        _isShuttingDown = true;

        // Cancel any active AI request before releasing resources
        // to prevent catch/finally blocks from accessing disposed objects.
        // Use Interlocked.Exchange for thread safety with ExecuteAiRequestAsync (#588).
        var activeCts = Interlocked.Exchange(ref _activeRequestCts, null);
        activeCts?.Cancel();

        // Wait for the active AI request to finish its catch/finally cleanup
        // before disposing shared resources (#446)
        var activeTask = Volatile.Read(ref _activeRequestTask);
        if (activeTask != null)
        {
            try
            {
                await activeTask.WaitAsync(TimeSpan.FromSeconds(35));
            }
            catch (TimeoutException)
            {
                // Proceed with disposal even if the request doesn't finish in time
            }
        }

        activeCts?.Dispose();

        RemoveThinkingIndicator();
        StopAutoWakeTimer();
        UnsubscribeEvents();
        TryReleaseWindowFileDropHook();
        await SaveChatStateAsync();
        await _backendClient.DisposeAsync();
        PruneClipboardImages();
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

    /// <summary>
    /// Checks if the model name contains the given token as a distinct segment
    /// (preceded by start, '-', '_', '.', '/', or ' ', or followed by the same).
    /// Prevents false positives like "co1l" matching "o1".
    /// </summary>
    internal static bool ContainsModelToken(string model, string token)
    {
        var index = model.IndexOf(token, StringComparison.Ordinal);
        while (index >= 0)
        {
            var beforeOk = index == 0 || IsModelSeparator(model[index - 1]);
            var afterPos = index + token.Length;
            var afterOk = afterPos >= model.Length || IsModelSeparator(model[afterPos]);
            if (beforeOk && afterOk) return true;
            index = model.IndexOf(token, index + 1, StringComparison.Ordinal);
        }
        return false;
    }

    /// <summary>
    /// OpenAI o-series models: o1, o3, o4 (with optional suffix like -mini, -preview).
    /// Matches "o1", "o1-mini", "o3-mini", "o4-mini" etc. but not "co1l" or "po3".
    /// </summary>
    private static readonly string[] _openAiOSeriesPrefixes = { "o1", "o3", "o4" };

    internal static bool IsOpenAiOSeriesModel(string model)
    {
        // Check for o1/o3/o4 at word boundary followed by end, separator, or hyphen
        foreach (var prefix in _openAiOSeriesPrefixes)
        {
            var index = model.IndexOf(prefix, StringComparison.Ordinal);
            while (index >= 0)
            {
                var beforeOk = index == 0 || IsModelSeparator(model[index - 1]);
                var afterPos = index + prefix.Length;
                var afterOk = afterPos >= model.Length || IsModelSeparator(model[afterPos]);
                if (beforeOk && afterOk) return true;
                index = model.IndexOf(prefix, index + 1, StringComparison.Ordinal);
            }
        }
        return false;
    }

    internal static bool IsModelSeparator(char c) =>
        c is '-' or '_' or '.' or '/' or ' ' or '(' or ')' or ':' or ',';

    internal static string FormatTokenCount(ulong tokens)
    {
        if (tokens >= 1_000_000)
        {
            return $"{tokens / 1_000_000.0:0.#}M";
        }
        if (tokens >= 1_000)
        {
            return $"{tokens / 1_000.0:0.#}K";
        }
        return tokens.ToString();
    }

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

    private async Task<T?> SendWithTimeoutAsync<T>(
        Func<CancellationToken, Task<T?>> action,
        string step,
        int timeoutMs = 8000)
    {
        using var cts = new CancellationTokenSource(timeoutMs);
        try
        {
            return await action(cts.Token).WaitAsync(cts.Token);
        }
        catch (TimeoutException)
        {
            throw new InvalidOperationException($"启动超时：{step}");
        }
        catch (OperationCanceledException)
        {
            throw new InvalidOperationException($"启动超时：{step}");
        }
    }

    private void UpdateStatusBar(string level, string title, string message)
    {
        StatusBarTitle.Text = title;
        StatusBarMessage.Text = message;
        StatusBarIcon.Foreground = level switch
        {
            "error" => BrushRed,
            "warning" => BrushOrange,
            "success" => BrushGreen,
            _ => GetThemeBrush("TextFillColorSecondaryBrush")
        };
        StatusBarIcon.Glyph = level switch
        {
            "error" => "\uE783",
            "warning" => "\uE7BA",
            "success" => "\uE73E",
            _ => "\uE946"
        };
    }

    private void RestoreIdleStatus(string title = "就绪", string message = "已收到回复")
    {
        if (_updateDownloadPercent >= 0)
        {
            UpdateStatusBar("info", "正在下载更新", $"正在下载 {_updateDownloadVersion}... {_updateDownloadPercent}%");
        }
        else
        {
            UpdateStatusBar("success", title, message);
        }
    }

    private void ShowLoadingOverlay(string message = "正在处理...")
    {
        ComposerProgressRing.IsActive = true;
        ComposerProgressRing.Visibility = Visibility.Visible;
    }

    private void HideLoadingOverlay()
    {
        ComposerProgressRing.IsActive = false;
        ComposerProgressRing.Visibility = Visibility.Collapsed;
    }

    private volatile bool _isShuttingDown;

    private static string StartupLogPath()
    {
        var root = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "com.local.vaultpilot");
        Directory.CreateDirectory(root);
        return Path.Combine(root, "startup.log");
    }

    private static async Task LogStartup(string message)
    {
        try
        {
            var line = $"{DateTimeOffset.Now:O} {message}";
            await File.AppendAllTextAsync(StartupLogPath(), line + Environment.NewLine, System.Text.Encoding.UTF8);
        }
        catch
        {
            // Ignore logging failures.
        }
    }

    private async Task UpdateStartupStepAsync(string step)
    {
        try
        {
            _startupStep = step;
            UpdateStatusBar("info", "正在启动", $"{step}...");
            await LogStartup($"Step: {step}");
        }
        catch (Exception error)
        {
            System.Diagnostics.Debug.WriteLine($"[UpdateStartupStepAsync] Error: {error}");
        }
    }

    private async Task ShowStartupFailureAsync(Exception error, string stderrTail)
    {
        var detail = LocalizeError(error.Message);
        if (!string.IsNullOrWhiteSpace(stderrTail))
        {
            detail = $"{detail}\n\n后端日志:\n{stderrTail}";
        }

        var dialog = new ContentDialog
        {
            XamlRoot = RootGrid.XamlRoot,
            Title = "启动失败",
            Content = $"无法连接本地后端：{detail}",
            CloseButtonText = "关闭"
        };
        await dialog.ShowAsync();
    }

    private void ShowError(string title, Exception error, bool addMessage = true)
    {
        UpdateStatusBar("error", title, LocalizeError(error.Message));
        if (addMessage)
        {
            AppendMessage("错误", LocalizeError(error.Message));
        }
    }

    private void ScrollToLatest()
    {
        DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
        {
            ChatScrollViewer.UpdateLayout();
            ChatScrollViewer.ChangeView(null, ChatScrollViewer.ScrollableHeight, null, disableAnimation: false);
            JumpLatestButton.Visibility = Visibility.Collapsed;
        });
    }

    private void RefreshJumpLatestButton()
    {
        var canScroll = ChatScrollViewer.ScrollableHeight > 0;
        var awayFromLatest = ChatScrollViewer.VerticalOffset < ChatScrollViewer.ScrollableHeight - 32;
        JumpLatestButton.Visibility = canScroll && awayFromLatest
            ? Visibility.Visible
            : Visibility.Collapsed;
    }

    private void RefreshVaultSummary()
    {
        NotesText.Text = $"笔记：{_noteCount}";
    }

    /// <summary>Converts an ISO-8601 timestamp to a human-readable relative time string.</summary>
    internal static string ToRelativeTime(string timestamp)
    {
        if (!DateTimeOffset.TryParse(timestamp, System.Globalization.CultureInfo.InvariantCulture,
                System.Globalization.DateTimeStyles.None, out var dto))
        {
            return timestamp;
        }

        var span = DateTimeOffset.Now - dto;
        if (span.TotalSeconds < 60) return "刚刚";
        if (span.TotalMinutes < 60) return $"{(int)span.TotalMinutes} 分钟前";
        if (span.TotalHours < 24) return $"{(int)span.TotalHours} 小时前";
        if (span.TotalDays < 2) return "昨天";
        if (span.TotalDays < 7) return $"{(int)span.TotalDays} 天前";
        if (span.TotalDays < 30) return $"{(int)(span.TotalDays / 7)} 周前";
        if (span.TotalDays < 365) return $"{(int)(span.TotalDays / 30)} 个月前";
        return $"{(int)(span.TotalDays / 365)} 年前";
    }

    internal static string BuildSessionTitle(string text)
    {
        var normalized = string.Join(" ", text.Split(Array.Empty<char>(), StringSplitOptions.RemoveEmptyEntries));
        return normalized.Length <= 28 ? normalized : $"{normalized[..28]}...";
    }

    internal static string LocalizeStage(string stage)
    {
        return stage switch
        {
            "analyzing" => "正在分析",
            "compressing" => "正在压缩上下文",
            "responding" => "正在组织回复",
            "retrieving" => "正在检索",
            "ranking" => "正在排序",
            "executing" => "正在执行工具",
            "saving" => "正在保存",
            _ => stage
        };
    }

    internal static string LocalizeStatusDetail(string detail)
    {
        return detail switch
        {
            "Analyzing request" => "正在分析请求",
            "Preparing request..." => "正在准备请求...",
            "Preparing answer" => "正在准备回复",
            "Preparing final answer" => "正在准备最终回复",
            "Loading recent notes" => "正在加载最近笔记",
            "No direct match; listing recent notes" => "没有直接命中，正在加载最近笔记",
            "Compressing earlier conversation context" => "正在压缩较早的对话内容",
            "Saving generated note" => "正在保存生成的笔记",
            _ when detail.StartsWith("Searching notes: ", StringComparison.Ordinal) =>
                $"正在搜索笔记：{detail["Searching notes: ".Length..]}",
            _ when detail.StartsWith("Ranking ", StringComparison.Ordinal) =>
                detail.Replace("Ranking ", "正在排序 ", StringComparison.Ordinal)
                    .Replace(" candidate notes", " 条候选笔记", StringComparison.Ordinal),
            _ when detail.StartsWith("Listing directory: ", StringComparison.Ordinal) =>
                $"正在列出目录：{detail["Listing directory: ".Length..]}",
            _ when detail.StartsWith("Reading file: ", StringComparison.Ordinal) =>
                $"正在读取文件：{detail["Reading file: ".Length..]}",
            _ when detail.StartsWith("Running command: ", StringComparison.Ordinal) =>
                $"正在执行命令：{detail["Running command: ".Length..]}",
            _ => detail
        };
    }

    internal static string LocalizeError(string message)
    {
        return message
            .Replace("API key is empty", "API Key 为空，请先在设置中配置模型服务。", StringComparison.Ordinal)
            .Replace("The Rust backend process is not connected.", "Rust 后端尚未连接。", StringComparison.Ordinal)
            .Replace("The Rust backend process closed stdout.", "Rust 后端已关闭输出通道。", StringComparison.Ordinal)
            .Replace("Backend request failed.", "后端请求失败。", StringComparison.Ordinal)
            // Network errors
            .Replace("Connection refused", "连接被拒绝，后端服务可能未启动。", StringComparison.Ordinal)
            .Replace("Connection timed out", "连接超时，请检查网络或后端服务状态。", StringComparison.Ordinal)
            .Replace("A task was canceled.", "操作已取消。", StringComparison.Ordinal)
            .Replace("The operation was canceled.", "操作已取消。", StringComparison.Ordinal)
            // HTTP errors
            .Replace("401 Unauthorized", "认证失败（401），请检查 API Key 是否正确。", StringComparison.Ordinal)
            .Replace("403 Forbidden", "访问被拒绝（403），API Key 可能没有足够权限。", StringComparison.Ordinal)
            .Replace("429 Too Many Requests", "请求过于频繁（429），请稍后重试。", StringComparison.Ordinal)
            .Replace("500 Internal Server Error", "服务器内部错误（500），请稍后重试。", StringComparison.Ordinal)
            .Replace("502 Bad Gateway", "网关错误（502），服务可能正在重启。", StringComparison.Ordinal)
            .Replace("503 Service Unavailable", "服务不可用（503），请稍后重试。", StringComparison.Ordinal)
            // Model errors
            .Replace("model not found", "指定的模型不存在，请在设置中检查模型名称。", StringComparison.Ordinal)
            .Replace("Model not found", "指定的模型不存在，请在设置中检查模型名称。", StringComparison.Ordinal)
            .Replace("Invalid API key", "API Key 无效，请在设置中重新配置。", StringComparison.Ordinal)
            .Replace("insufficient_quota", "API 配额不足，请检查账户余额或提升套餐。", StringComparison.Ordinal)
            // File/IO errors
            .Replace("Access to the path", "文件访问被拒绝，可能正在被其他程序使用。", StringComparison.Ordinal)
            .Replace("The file is being used by another process", "文件正在被其他程序使用，请关闭后重试。", StringComparison.Ordinal)
            .Replace("No such file or directory", "文件或目录不存在。", StringComparison.Ordinal)
            .Replace("Directory not found", "目录不存在，请检查知识库路径设置。", StringComparison.Ordinal)
            // Generic fallback wrapping
            .Replace("An error occurred while sending the request.", "发送请求时发生错误，请检查网络连接。", StringComparison.Ordinal)
            .Replace("The SSL connection could not be established", "SSL 连接建立失败，请检查网络安全性设置。", StringComparison.Ordinal);
    }

    private sealed record SessionListItem(
        string Id,
        string Title,
        string Detail,
        string RelativeTime,
        int TurnCount,
        string SummaryPreview);
}
