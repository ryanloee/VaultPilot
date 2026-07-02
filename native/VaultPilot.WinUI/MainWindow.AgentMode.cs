using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System.Text.Json;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI;

/// <summary>
/// Agent Mode UI — tool call display, write approval, status bar integration.
/// Split from MainWindow.Chat.cs for Agent Mode feature (#1348).
/// </summary>
public sealed partial class MainWindow : Window
{
    private bool _agentModeActive;
    private static bool _agentGuideShown;
    private int _agentCurrentStep;
    private int _agentMaxSteps = 20;
    private CancellationTokenSource? _agentCts;

    private async void OnAgentModeClicked(object sender, RoutedEventArgs e)
    {
        if (_agentModeActive)
        {
            return;
        }

        if (!_agentGuideShown)
        {
            _agentGuideShown = true;
            var guide = new ContentDialog
            {
                Title = "🤖 Agent 模式",
                Content = "Agent 模式让 AI 自主执行多步操作：\n\n" +
                    "• 读取和搜索 vault 中的笔记\n" +
                    "• 创建、编辑笔记（需要你批准）\n" +
                    "• 自动使用工具完成复杂任务\n\n" +
                    "安全特性：\n" +
                    "• 写入操作会弹出确认对话框\n" +
                    "• 步数和 Token 有上限保护\n" +
                    "• 随时可以点击「停止」终止",
                PrimaryButtonText = "了解，开始使用",
                CloseButtonText = "取消",
                DefaultButton = ContentDialogButton.Primary,
                XamlRoot = Content.XamlRoot
            };
            var guideResult = await guide.ShowAsync();
            if (guideResult != ContentDialogResult.Primary)
            {
                return;
            }
        }

        var prompt = ComposerBox.Text?.Trim();
        if (string.IsNullOrEmpty(prompt))
        {
            AppendMessage("系统", "请输入提示词再启动 Agent 模式。");
            return;
        }

        StartAgentMode(prompt, _agentMaxSteps, autoApprove: false);
    }

    private void OnStopAgentClicked(object sender, RoutedEventArgs e)
    {
        StopAgentMode("用户手动停止");
    }

    private void StartAgentMode(string prompt, int maxSteps, bool autoApprove)
    {
        _agentModeActive = true;
        _agentCurrentStep = 0;
        _agentMaxSteps = maxSteps;
        _agentCts?.Dispose();
        _agentCts = new CancellationTokenSource();

        // Update UI state
        AgentModeButton.Visibility = Visibility.Collapsed;
        StopAgentButton.Visibility = Visibility.Visible;
        AgentToolCallPanel.Visibility = Visibility.Visible;
        AgentToolCallList.Children.Clear();
        AgentProgressRing.IsActive = true;
        AgentStatusText.Text = "Agent 启动中...";
        AgentStepCount.Text = $"步骤: 0/{maxSteps}";
        AgentTokenCount.Text = "Token: 0";

        UpdateStatusBar("info", "Agent 模式", "正在执行自主工具调用循环...");

        // Send agent request to backend
        _ = ExecuteAgentRequestAsync(prompt, maxSteps, autoApprove, _agentCts.Token);
    }

    private void StopAgentMode(string reason)
    {
        var old = Interlocked.Exchange(ref _agentCts, null);
        old?.Cancel();
        old?.Dispose();
        _agentModeActive = false;

        AgentModeButton.Visibility = Visibility.Visible;
        StopAgentButton.Visibility = Visibility.Collapsed;
        AgentProgressRing.IsActive = false;
        AgentStatusText.Text = $"Agent 已停止: {reason}";

        UpdateStatusBar("info", "Agent 模式", $"已停止: {reason}");
    }

    private async Task ExecuteAgentRequestAsync(string prompt, int maxSteps, bool autoApprove, CancellationToken ct)
    {
        try
        {
            var request = new
            {
                prompt,
                maxSteps,
                autoApprove
            };

            // runAgent returns immediately with { status: "started" }
            // Agent events arrive via AgentStatusReceived → HandleAgentEvent
            var result = await SendWithTimeoutAsync(
                token => _backendClient.SendAsync<JsonElement>("runAgent", request, token),
                "runAgent");

            if (ct.IsCancellationRequested) return;

            // Check if the request was accepted
            if (result.TryGetProperty("status", out var statusEl) && statusEl.GetString() == "started")
            {
                // Agent is running in background — events will arrive via HandleAgentEvent
                return;
            }

            // Fallback: if the response contains a final answer (e.g., single-step agent)
            if (result.TryGetProperty("answer", out var answerEl))
            {
                var answer = answerEl.GetString() ?? "Agent 未返回结果";
                AppendMessage("Agent", answer);
            }

            DispatcherQueue.TryEnqueue(() =>
            {
                _agentModeActive = false;
                AgentModeButton.Visibility = Visibility.Visible;
                StopAgentButton.Visibility = Visibility.Collapsed;
                AgentProgressRing.IsActive = false;
                AgentStatusText.Text = "Agent 完成";
                UpdateStatusBar("success", "Agent 模式", "任务完成");
            });
        }
        catch (OperationCanceledException)
        {
            // Already handled by StopAgentMode
        }
        catch (Exception ex)
        {
            DispatcherQueue.TryEnqueue(() =>
            {
                AppendMessage("错误", $"Agent 执行失败: {LocalizeError(ex.Message)}");
                StopAgentMode("执行出错");
            });
        }
    }

    private void HandleAgentEvent(AgentStatusEvent status)
    {
        var stage = status.Stage;
        switch (stage)
        {
            case "thinking":
                if (status.Step is { } step)
                {
                    _agentCurrentStep = step;
                    AgentStepCount.Text = $"步骤: {_agentCurrentStep}/{_agentMaxSteps}";
                    AgentStatusText.Text = "Agent 思考中...";
                }
                break;

            case "toolCall":
                var tool = status.Tool ?? "unknown";
                var args = status.Args ?? "";
                AddToolCallEntry(tool, args, isRunning: true);
                AgentStatusText.Text = $"执行工具: {tool}";
                break;

            case "toolResult":
                var resultTool = status.Tool ?? "unknown";
                var preview = status.ResultPreview ?? "";
                var isError = status.IsError ?? false;
                UpdateLastToolCallResult(resultTool, preview, isError);
                break;

            case "writeApprovalNeeded":
                var writeTool = status.Tool ?? "unknown";
                var writeArgs = status.Args ?? "";
                ShowWriteApprovalDialog(writeTool, writeArgs);
                break;

            case "finalAnswer":
                AgentStatusText.Text = "Agent 生成最终回答...";
                break;

            case "agentCompleted":
                AppendMessage("Agent", status.Detail);
                if (status.StepsUsed is { } stepsUsed)
                {
                    _agentCurrentStep = stepsUsed;
                    AgentStepCount.Text = $"步骤: {_agentCurrentStep}/{_agentMaxSteps}";
                }
                if (status.TokensUsed is { } tokensUsed)
                {
                    AgentTokenCount.Text = $"Token: {tokensUsed}";
                }
                DispatcherQueue.TryEnqueue(() =>
                {
                    _agentModeActive = false;
                    AgentModeButton.Visibility = Visibility.Visible;
                    StopAgentButton.Visibility = Visibility.Collapsed;
                    AgentProgressRing.IsActive = false;
                    AgentStatusText.Text = "Agent 完成";
                    UpdateStatusBar("success", "Agent 模式", "任务完成");
                });
                break;

            case "stepLimitReached":
                StopAgentMode("步骤限制已达");
                break;

            case "tokenBudgetExceeded":
                StopAgentMode("Token 预算已超");
                break;

            case "timeout":
                StopAgentMode("执行超时");
                break;

            case "error":
                AppendMessage("Agent 错误", status.Detail ?? "未知错误");
                StopAgentMode("执行出错");
                break;
        }
    }

    private void AddToolCallEntry(string tool, string args, bool isRunning)
    {
        var panel = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 8,
            Padding = new Thickness(8, 4, 8, 4),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent)
        };

        var icon = new FontIcon
        {
            Glyph = isRunning ? "\uE768" : "\uE73E", // Running: play, Done: checkmark
            FontSize = 12,
            Foreground = isRunning
                ? (Brush)Application.Current.Resources["SystemFillColorAttentionBrush"]
                : (Brush)Application.Current.Resources["SystemFillColorSuccessBrush"]
        };

        var toolText = new TextBlock
        {
            Text = tool,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            FontSize = 12
        };

        var argsText = new TextBlock
        {
            Text = TruncateString(args, 80),
            Opacity = 0.7,
            FontSize = 11,
            TextTrimming = Microsoft.UI.Xaml.TextTrimming.CharacterEllipsis,
            MaxWidth = 300
        };

        panel.Children.Add(icon);
        panel.Children.Add(toolText);
        panel.Children.Add(argsText);

        AgentToolCallList.Children.Add(panel);
    }

    private void UpdateLastToolCallResult(string tool, string preview, bool isError)
    {
        // Find the last tool call entry and update its icon
        for (int i = AgentToolCallList.Children.Count - 1; i >= 0; i--)
        {
            if (AgentToolCallList.Children[i] is StackPanel panel)
            {
                foreach (var child in panel.Children)
                {
                    if (child is FontIcon icon)
                    {
                        icon.Glyph = isError ? "\uE783" : "\uE73E"; // Error: X, Success: checkmark
                        icon.Foreground = isError
                            ? (Brush)Application.Current.Resources["SystemFillColorCriticalBrush"]
                            : (Brush)Application.Current.Resources["SystemFillColorSuccessBrush"];
                        break;
                    }
                }
                break;
            }
        }
    }

    private async void ShowWriteApprovalDialog(string tool, string args)
    {
        var (description, preview) = ParseWriteArgs(tool, args);

        var contentStack = new StackPanel { Spacing = 8 };
        contentStack.Children.Add(new TextBlock
        {
            Text = description,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            TextWrapping = TextWrapping.Wrap
        });

        if (!string.IsNullOrEmpty(preview))
        {
            contentStack.Children.Add(new ScrollViewer
            {
                MaxHeight = 300,
                Content = new TextBlock
                {
                    Text = preview,
                    FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Consolas"),
                    FontSize = 12,
                    TextWrapping = TextWrapping.Wrap,
                    IsTextSelectionEnabled = true
                }
            });
        }

        var dialog = new ContentDialog
        {
            Title = "写入操作需要批准",
            Content = contentStack,
            PrimaryButtonText = "批准",
            SecondaryButtonText = "拒绝",
            DefaultButton = ContentDialogButton.Primary,
            XamlRoot = Content.XamlRoot
        };

        var result = await dialog.ShowAsync();
        var approved = result == ContentDialogResult.Primary;

        AppendMessage("Agent", approved
            ? $"已批准写入操作: {tool}"
            : $"已拒绝写入操作: {tool}");

        // Send approval decision back to backend
        try
        {
            await _backendClient.SendAsync<object>("respondToWriteApproval",
                new { approved }, _agentCts?.Token ?? CancellationToken.None);
        }
        catch (Exception ex)
        {
            AppendMessage("错误", $"发送审批决策失败: {LocalizeError(ex.Message)}");
        }
    }

    /// <summary>
    /// Parse write tool args JSON into human-readable description + content preview.
    /// Returns fallback (truncated raw args) if JSON parsing fails.
    /// </summary>
    private static (string Description, string Preview) ParseWriteArgs(string tool, string args)
    {
        try
        {
            using var doc = System.Text.Json.JsonDocument.Parse(args);
            var root = doc.RootElement;

            var path = root.TryGetProperty("path", out var p) ? p.GetString() : null;
            var content = root.TryGetProperty("content", out var c) ? c.GetString() : null;

            string description = tool switch
            {
                "write_note" or "save_note" => path != null
                    ? $"将修改文件: {path}"
                    : "将修改笔记",
                "delete_note" => path != null
                    ? $"将删除文件: {path}"
                    : "将删除笔记",
                "rename_note" => path != null
                    ? $"将重命名文件: {path}"
                    : "将重命名笔记",
                _ => $"工具: {tool}"
            };

            string preview = "";
            if (!string.IsNullOrEmpty(content))
            {
                var lines = content.Split('\n');
                preview = lines.Length > 50
                    ? string.Join('\n', lines.Take(50)) + $"\n… (共 {lines.Length} 行)"
                    : content;
            }

            return (description, preview);
        }
        catch
        {
            // JSON parse failure — fallback to raw truncated args
            return ($"工具: {tool}", TruncateString(args, 500));
        }
    }

    private static string TruncateString(string s, int maxLen)
    {
        if (s.Length <= maxLen) return s;
        return s[..maxLen] + "…";
    }
}
