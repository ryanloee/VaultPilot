using VaultPilot.WinUI.Controls;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System.Collections.ObjectModel;
using System.Diagnostics;
using Windows.ApplicationModel.DataTransfer;

namespace VaultPilot.WinUI;

/// <summary>
/// Chat message rendering, thinking indicator, citation cards, and empty state —
/// split from MainWindow.Chat.cs (#1344).
///
/// #3581: Uses ItemsRepeater + StackLayout (virtualizing) instead of a plain
/// StackPanel. Only viewport-visible turns exist in the visual tree; off-screen
/// turns have zero UI overhead. Rendered visuals are cached per turn so
/// scrolling back doesn't re-create them.
/// </summary>
public sealed partial class MainWindow : Window
{
    // ── ItemsRepeater data source (#3581) ──
    // The virtualized message list. Each item maps to one ChatTurn.
    // _messageItems drives the ItemsRepeater; _itemRenderCache holds the
    // actual FrameworkElement visuals so scrolling back hits the cache.
    private readonly ObservableCollection<MessageItem> _messageItems = [];
    private readonly Dictionary<string, FrameworkElement> _itemRenderCache = new(StringComparer.Ordinal);
    private const string ThinkingItemKey = "__thinking__";
    private const int MaxRenderCacheSize = 300;

    /// <summary>
    /// ItemsRepeater handler: populate the ContentControl with the cached
    /// visual tree for the data item. The visual tree is built once per turn
    /// and cached; scrolling back reuses it.
    /// </summary>
    private void OnMessageElementPrepared(ItemsRepeater sender, ItemsRepeaterElementPreparedEventArgs args)
    {
        if (args.Element is not ContentControl container)
            return;

        // WinAppSDK 1.6 doesn't expose args.Item, so we fetch the data item by index
        // from the observable collection driving the ItemsRepeater.
        var item = args.Index >= 0 && args.Index < _messageItems.Count
            ? _messageItems[args.Index]
            : null;
        if (item is null)
            return;

        if (_itemRenderCache.TryGetValue(item.TurnId, out var cached))
        {
            container.Content = cached;
            return;
        }

        var visual = item.TurnId == ThinkingItemKey
            ? BuildThinkingVisual()
            : BuildTurnVisual(item);
        _itemRenderCache[item.TurnId] = visual;
        container.Content = visual;
    }

    /// <summary>
    /// ItemsRepeater handler: null the ContentControl's content so the
    /// recycled turn's visual tree is detached. The cache keeps a reference
    /// so scrolling back reuses it.
    /// </summary>
    private void OnMessageElementClearing(ItemsRepeater sender, ItemsRepeaterElementClearingEventArgs args)
    {
        if (args.Element is ContentControl container)
            container.Content = null;
    }

    /// <summary>
    /// Clears the render cache for the current session (called after
    /// compression or session switch). Also enforces MaxRenderCacheSize.
    /// </summary>
    private void ClearRenderCache()
    {
        _itemRenderCache.Clear();
    }

    // ── Chat rendering ──

    private void RenderCurrentSession()
    {
        _messageItems.Clear();
        ClearRenderCache();

        var session = CurrentSession();
        if (session is null || session.Turns.Count == 0)
        {
            _lastRenderedSessionId = session?.Id;
            _lastRenderedTurnCount = 0;
            ShowEmptyState();
            RefreshContextStatus();
            return;
        }

        foreach (var turn in session.Turns)
        {
            _messageItems.Add(TurnToMessageItem(turn));
        }
        // #3508: track what we rendered so AppendNewTurns can incremental-update.
        _lastRenderedSessionId = session.Id;
        _lastRenderedTurnCount = session.Turns.Count;
        RefreshContextStatus();
    }

    /// <summary>
    /// Incrementally append only newly-added turns to the message list,
    /// avoiding the O(n) full rebuild of RenderCurrentSession on every
    /// message send (#3508). Falls back to full render if the session
    /// changed, turns were truncated (compression), or the panel was
    /// showing the empty-state placeholder.
    /// </summary>
    private void AppendNewTurns()
    {
        var session = CurrentSession();
        if (session is null || session.Turns.Count == 0)
            return;

        // Session switched, list out of sync — full rebuild.
        if (_lastRenderedSessionId != session.Id ||
            _lastRenderedTurnCount > session.Turns.Count ||
            _lastRenderedTurnCount == 0)
        {
            RenderCurrentSession();
            return;
        }

        // Append only turns that were added since last render.
        for (int i = _lastRenderedTurnCount; i < session.Turns.Count; i++)
        {
            _messageItems.Add(TurnToMessageItem(session.Turns[i]));
        }
        _lastRenderedSessionId = session.Id;
        _lastRenderedTurnCount = session.Turns.Count;
        RefreshContextStatus();
    }

    /// <summary>
    /// Converts a ChatTurn into a MessageItem (the data item for ItemsRepeater).
    /// </summary>
    private static MessageItem TurnToMessageItem(ChatTurn turn)
    {
        var isScheduledWake = turn.Source == "scheduled_wake";
        var author = turn.Role == "user"
            ? (isScheduledWake ? "⏰ 定时唤醒" : "你")
            : (isScheduledWake && turn.Text.StartsWith("⏰") ? "⏰ 定时唤醒" : "助手");
        return new MessageItem
        {
            TurnId = turn.Id,
            Role = turn.Role,
            Text = turn.Text,
            Author = author,
            CreatedAt = turn.CreatedAt,
            Citations = turn.Citations,
            Attachments = turn.Attachments,
            ThinkingTrace = turn.ThinkingTrace,
            SavedNote = turn.SavedNote,
            Source = turn.Source,
        };
    }

    /// <summary>
    /// Adds a system/AI-message to the ItemsRepeater (replaces the old
    /// AppendMessage which added directly to MessagesPanel.Children).
    /// Callers include ShowError, AgentMode, and status notifications.
    /// </summary>
    private void AddSystemMessage(string author, string text)
    {
        _messageItems.Add(new MessageItem
        {
            TurnId = $"__msg__{Guid.NewGuid():N}",
            Role = "system",
            Text = text,
            Author = author,
        });
    }

    /// <summary>
    /// Builds the complete visual tree for one turn (message bubble,
    /// attachments, citations, thinking trace). Called once per turn;
    /// result is cached in _itemRenderCache.
    /// </summary>
    private FrameworkElement BuildTurnVisual(MessageItem item)
    {
        var container = new StackPanel { Spacing = 2 };
        AppendMessageTo(container, item.Author, item.Text, item.CreatedAt);
        if (item.Attachments is { Count: > 0 })
        {
            AppendAttachmentPreviewsTo(container, item.Attachments, item.Role);
        }
        if (item.Role == "assistant")
        {
            if (item.ThinkingTrace is { Steps.Count: > 0 } trace)
            {
                AppendThinkingTraceTo(container, trace);
            }
            if (item.Citations is { Count: > 0 } citations)
            {
                AppendCitationCardsTo(container, citations);
            }
            if (item.SavedNote is not null)
            {
                AppendMessageTo(container, "系统", $"已保存笔记：{item.SavedNote.Title}", item.CreatedAt);
            }
        }
        return container;
    }

    /// <summary>
    /// Builds the thinking indicator visual (shown during AI request).
    /// </summary>
    private FrameworkElement BuildThinkingVisual()
    {
        var spinner = new ProgressRing
        {
            IsActive = true,
            Width = 16,
            Height = 16,
            VerticalAlignment = VerticalAlignment.Center,
        };
        var thinkingLabel = new TextBlock
        {
            Text = "思考中…",
            FontSize = 12,
            Foreground = GetThemeBrush("VaultTextSecondary"),
            VerticalAlignment = VerticalAlignment.Center,
        };
        var dotsPanel = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 8,
            Padding = new Thickness(2, 2, 2, 2),
        };
        dotsPanel.Children.Add(spinner);
        dotsPanel.Children.Add(thinkingLabel);

        var bubble = new Border
        {
            MaxWidth = 720,
            Padding = new Thickness(14, 12, 14, 12),
            CornerRadius = new CornerRadius(12),
            Background = GetThemeBrush("VaultCardElevatedBg"),
            BorderBrush = GetThemeBrush("VaultBorder"),
            BorderThickness = new Thickness(1),
            HorizontalAlignment = HorizontalAlignment.Left,
            Child = dotsPanel,
        };

        var label = new TextBlock
        {
            Text = "助手",
            FontSize = 11,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            Foreground = GetThemeBrush("VaultTextSecondary"),
            HorizontalAlignment = HorizontalAlignment.Left,
        };

        var stack = new StackPanel
        {
            Spacing = 6,
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        AutomationProperties.SetName(stack, "AI 正在思考");
        stack.Children.Add(label);
        stack.Children.Add(bubble);
        return stack;
    }

    // ── Empty state ──

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

        // Add empty state as the only item in the repeater.
        var emptyItem = new MessageItem { TurnId = "__empty__", Role = "system", Text = "" };
        _itemRenderCache["__empty__"] = container;
        _messageItems.Add(emptyItem);
    }

    // ── Message bubble builder (now takes a target Panel) ──

    private void AppendMessageTo(Panel target, string author, string text, string? createdAt = null)
    {
        var isUser = author == "你";
        var isAssistant = author == "助手";

        // ── Avatar circle ──
        // User avatar: custom image when configured, else letter fallback.
        // AI avatar stays the letter "V" (no custom AI avatar yet).
        var avatar = new Border
        {
            Width = 32,
            Height = 32,
            CornerRadius = new CornerRadius(16),
            Background = isUser
                ? GetThemeBrush("VaultAvatarUserBg")
                : GetThemeBrush("VaultAvatarAiBg"),
            MinHeight = 0, MinWidth = 0,
            VerticalAlignment = VerticalAlignment.Top,
            Child = isUser && AvatarPreferences.AvatarFilePath is not null
                ? (UIElement)BuildAvatarImage(32)
                : new TextBlock
                {
                    Text = isUser ? "我" : "V",
                    FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                    FontSize = 13,
                    Foreground = isUser
                        ? GetThemeBrush("VaultAvatarUserFg")
                        : GetThemeBrush("VaultAvatarAiFg"),
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                },
        };

        // ── Timestamp ──
        DateTime displayTime;
        if (!string.IsNullOrEmpty(createdAt) && DateTime.TryParse(createdAt, out var parsed))
            displayTime = parsed;
        else
            displayTime = DateTime.Now;
        var timeStr = displayTime.ToString("HH:mm");

        // ── Bubble content ──
        var bubbleText = isUser || isAssistant ? text : $"{author}: {text}";
        var bubbleContent = CreateMessageContent(bubbleText, isAssistant, isUser);

        // Card-style bubble.
        var bubble = new Border
        {
            Padding = new Thickness(14, 12, 14, 12),
            CornerRadius = new CornerRadius(12),
            Background = isUser
                ? GetThemeBrush("VaultBubbleUserBg")
                : GetThemeBrush("VaultCardElevatedBg"),
            BorderBrush = isUser ? null : GetThemeBrush("VaultBorder"),
            BorderThickness = isUser ? new Thickness(0) : new Thickness(1),
            Child = bubbleContent,
            HorizontalAlignment = isUser ? HorizontalAlignment.Right : HorizontalAlignment.Stretch,
        };
        if (isUser)
            bubble.MaxWidth = 560;

        // ── Author label row ──
        var authorText = new TextBlock
        {
            Text = author,
            FontSize = 11,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            Foreground = GetThemeBrush("VaultTextSecondary"),
            VerticalAlignment = VerticalAlignment.Center,
        };
        var timeText = new TextBlock
        {
            Text = timeStr,
            FontSize = 11,
            Foreground = GetThemeBrush("VaultTextMuted"),
            VerticalAlignment = VerticalAlignment.Center,
        };
        var metaRow = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 6,
        };
        metaRow.Children.Add(authorText);
        metaRow.Children.Add(timeText);

        // ── Assemble: avatar + (meta + bubble) ──
        var messageRow = new StackPanel { Spacing = 6 };
        messageRow.Children.Add(metaRow);
        messageRow.Children.Add(bubble);

        Grid outerRow;
        if (isUser)
        {
            outerRow = new Grid { ColumnSpacing = 10 };
            outerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            outerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            messageRow.HorizontalAlignment = HorizontalAlignment.Right;
            Grid.SetColumn(messageRow, 0);
            Grid.SetColumn(avatar, 1);
            outerRow.Children.Add(messageRow);
            outerRow.Children.Add(avatar);
        }
        else
        {
            outerRow = new Grid { ColumnSpacing = 10 };
            outerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            outerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            messageRow.HorizontalAlignment = HorizontalAlignment.Stretch;
            Grid.SetColumn(avatar, 0);
            Grid.SetColumn(messageRow, 1);
            outerRow.Children.Add(avatar);
            outerRow.Children.Add(messageRow);
        }

        AutomationProperties.SetName(bubble, isUser ? "用户消息" : "AI 消息");

        // Skip meta row for system messages
        if (!isUser && !isAssistant)
            messageRow.Children.Remove(metaRow);

        // Invalidate note title cache when a tool action has saved a note (#2035)
        if (author == "系统" && text.Contains("已保存笔记"))
            InvalidateNoteTitleCache();

        target.Children.Add(outerRow);
    }

    // ── Sub-element builders (now take a target Panel) ──

    /// <summary>
    /// Builds a circular avatar image element from the persisted custom
    /// avatar (AvatarPreferences). Falls back to null when unreadable.
    /// </summary>
    private static Image BuildAvatarImage(int size)
    {
        var bitmap = AvatarPreferences.LoadBitmap();
        var image = new Image
        {
            Width = size,
            Height = size,
            Stretch = Microsoft.UI.Xaml.Media.Stretch.UniformToFill,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        if (bitmap is not null)
        {
            image.Source = bitmap;
        }
        return image;
    }


    private void AppendAttachmentPreviewsTo(Panel target, IReadOnlyList<ChatAttachment> attachments, string role)
    {
        if (attachments.Count == 0) return;

        var wrap = new WrapPanel
        {
            Orientation = Orientation.Horizontal,
            ItemWidth = 142,
            ItemHeight = 178,
            HorizontalAlignment = role == "user" ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            Margin = new Thickness(0, 2, 0, 0),
        };

        foreach (var attachment in attachments)
            wrap.Children.Add(CreateChatAttachmentPreview(attachment, removable: false));

        target.Children.Add(wrap);
    }

    private void AppendThinkingTraceTo(Panel target, ThinkingTrace trace)
    {
        var stepsPanel = new StackPanel { Spacing = 4 };
        foreach (var step in trace.Steps)
        {
            var stepBlock = new TextBlock
            {
                Text = $"• {step.Title}: {step.Detail}",
                FontSize = 12,
                Opacity = 0.7,
                TextWrapping = TextWrapping.Wrap,
            };
            stepsPanel.Children.Add(stepBlock);
        }

        var expander = new Expander
        {
            Header = $"思考过程 ({trace.Steps.Count} 步){(string.IsNullOrWhiteSpace(trace.Summary) ? "" : $" — {trace.Summary}")}",
            IsExpanded = false,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            Content = stepsPanel,
        };
        AutomationProperties.SetName(expander, $"思考过程: {trace.Steps.Count} 步");

        target.Children.Add(expander);
    }

    private void AppendCitationCardsTo(Panel target, IReadOnlyList<AnswerCitation> citations)
    {
        var citationsPanel = new StackPanel
        {
            Spacing = 6,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            Margin = new Thickness(0, 4, 0, 0),
        };

        var header = new TextBlock
        {
            Text = $"引用 ({citations.Count})",
            FontSize = 12,
            Foreground = GetThemeBrush("VaultTextSecondary"),
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
        };
        citationsPanel.Children.Add(header);

        foreach (var citation in citations)
        {
            var card = new Border
            {
                Background = GetThemeBrush("VaultCardElevatedBg"),
                BorderBrush = GetThemeBrush("VaultBorder"),
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(8),
                Padding = new Thickness(10, 8, 10, 8),
                Child = new StackPanel
                {
                    Spacing = 2,
                    Children =
                    {
                        new TextBlock
                        {
                            Text = citation.Title,
                            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                            FontSize = 12,
                            Foreground = GetThemeBrush("VaultTextPrimary"),
                        },
                        new TextBlock
                        {
                            Text = citation.Snippet,
                            FontSize = 11,
                            Opacity = 0.8,
                            Foreground = GetThemeBrush("VaultTextSecondary"),
                            TextWrapping = TextWrapping.Wrap,
                            MaxLines = 3,
                            TextTrimming = TextTrimming.CharacterEllipsis,
                        },
                    },
                },
            };
            citationsPanel.Children.Add(card);
        }

        target.Children.Add(citationsPanel);
    }

    // ── Thinking indicator ──

    private void ShowThinkingIndicator()
    {
        RemoveThinkingIndicator();
        _messageItems.Add(new MessageItem { TurnId = ThinkingItemKey, Role = "thinking", Author = "助手" });
        ScrollToLatest();
    }

    private void RemoveThinkingIndicator()
    {
        for (int i = _messageItems.Count - 1; i >= 0; i--)
        {
            if (_messageItems[i].TurnId == ThinkingItemKey)
            {
                _messageItems.RemoveAt(i);
                _itemRenderCache.Remove(ThinkingItemKey);
                break;
            }
        }
    }

    // ── Clipboard ──

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

    // ── Scrolling ──

    private void OnChatScrollViewerViewChanged(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        RefreshJumpLatestButton();
    }

    private void OnJumpLatestClicked(object sender, RoutedEventArgs e)
    {
        ScrollToLatest();
    }
}
