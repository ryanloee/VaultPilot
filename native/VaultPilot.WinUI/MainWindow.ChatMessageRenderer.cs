using VaultPilot.WinUI.Controls;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System.Diagnostics;
using Windows.ApplicationModel.DataTransfer;

namespace VaultPilot.WinUI;

/// <summary>
/// Chat message rendering, thinking indicator, citation cards, and empty state —
/// split from MainWindow.Chat.cs (#1344).
/// </summary>
public sealed partial class MainWindow : Window
{
    // ── Chat rendering ──

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

        // Author label with timestamp
        var now = DateTime.Now;
        var timeStr = now.ToString("HH:mm");
        var authorLine = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 8,
            HorizontalAlignment = bubble.HorizontalAlignment
        };
        var authorText = new TextBlock
        {
            Text = author,
            Opacity = 0.72
        };
        var timeText = new TextBlock
        {
            Text = timeStr,
            Opacity = 0.45,
            FontSize = 11
        };
        authorLine.Children.Add(authorText);
        authorLine.Children.Add(timeText);
        AutomationProperties.SetName(bubble, isUser ? "用户消息" : "AI 消息");

        var stack = new StackPanel
        {
            Spacing = 4,
            HorizontalAlignment = isUser ? HorizontalAlignment.Right : HorizontalAlignment.Left
        };
        stack.Children.Add(authorLine);
        stack.Children.Add(bubble);

        if (!isUser && !isAssistant)
        {
            stack.Children.Remove(authorLine);
        }

        // Invalidate note title cache when a tool action has saved a note (#2035)
        if (author == "系统" && text.Contains("已保存笔记"))
        {
            InvalidateNoteTitleCache();
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

    private void OnChatScrollViewerViewChanged(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        RefreshJumpLatestButton();
    }

    private void OnJumpLatestClicked(object sender, RoutedEventArgs e)
    {
        ScrollToLatest();
    }
}
