using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System.Text.Json;

namespace VaultPilot.WinUI;

/// <summary>
/// Agent Mode UI — tool call display, write approval, status bar integration.
/// Split from MainWindow.Chat.cs for Agent Mode feature (#1348).
/// </summary>
public sealed partial class MainWindow : Window
{
    private bool _agentModeActive;
    private int _agentCurrentStep;
    private int _agentMaxSteps = 20;
    private CancellationTokenSource? _agentCts;

    private void OnAgentModeClicked(object sender, RoutedEventArgs e)
    {
        if (_agentModeActive)
        {
            return;
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
        _agentCts?.Cancel();
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

            var result = await SendWithTimeoutAsync(
                token => _backendClient.SendAsync<JsonElement>("runAgent", request, token),
                "runAgent");

            if (ct.IsCancellationRequested) return;

            // Display final answer
            if (result.TryGetProperty("answer", out var answerEl))
            {
                var answer = answerEl.GetString() ?? "Agent 未返回结果";
                AppendMessage("Agent", answer);
            }

            if (result.TryGetProperty("stepsUsed", out var stepsEl))
            {
                _agentCurrentStep = stepsEl.GetInt32();
                AgentStepCount.Text = $"步骤: {_agentCurrentStep}/{_agentMaxSteps}";
            }

            if (result.TryGetProperty("tokensUsed", out var tokensEl))
            {
                AgentTokenCount.Text = $"Token: {tokensEl.GetUInt64()}";
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

    private void HandleAgentEvent(JsonElement evt)
    {
        if (!evt.TryGetProperty("type", out var typeEl))
            return;

        var type = typeEl.GetString();
        DispatcherQueue.TryEnqueue(() =>
        {
            switch (type)
            {
                case "thinking":
                    if (evt.TryGetProperty("step", out var stepEl))
                    {
                        _agentCurrentStep = stepEl.GetInt32();
                        AgentStepCount.Text = $"步骤: {_agentCurrentStep}/{_agentMaxSteps}";
                        AgentStatusText.Text = "Agent 思考中...";
                    }
                    break;

                case "tool_call":
                    if (evt.TryGetProperty("tool", out var toolEl))
                    {
                        var tool = toolEl.GetString() ?? "unknown";
                        var args = evt.TryGetProperty("args", out var argsEl) ? argsEl.GetString() : "";
                        AddToolCallEntry(tool, args ?? "", isRunning: true);
                        AgentStatusText.Text = $"执行工具: {tool}";
                    }
                    break;

                case "tool_result":
                    if (evt.TryGetProperty("tool", out var resultToolEl))
                    {
                        var tool = resultToolEl.GetString() ?? "unknown";
                        var preview = evt.TryGetProperty("result_preview", out var previewEl) ? previewEl.GetString() : "";
                        var isError = evt.TryGetProperty("is_error", out var errEl) && errEl.GetBoolean();
                        UpdateLastToolCallResult(tool, preview ?? "", isError);
                    }
                    break;

                case "write_approval_needed":
                    if (evt.TryGetProperty("tool", out var writeToolEl))
                    {
                        var tool = writeToolEl.GetString() ?? "unknown";
                        var args = evt.TryGetProperty("args", out var writeArgsEl) ? writeArgsEl.GetString() : "";
                        ShowWriteApprovalDialog(tool, args ?? "");
                    }
                    break;

                case "final_answer":
                    AgentStatusText.Text = "Agent 生成最终回答...";
                    break;

                case "step_limit_reached":
                    StopAgentMode("步骤限制已达");
                    break;

                case "token_budget_exceeded":
                    StopAgentMode("Token 预算已超");
                    break;

                case "timeout":
                    StopAgentMode("执行超时");
                    break;

                case "error":
                    var msg = evt.TryGetProperty("message", out var msgEl) ? msgEl.GetString() : "未知错误";
                    AppendMessage("Agent 错误", msg ?? "未知错误");
                    StopAgentMode("执行出错");
                    break;
            }
        });
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
        var dialog = new ContentDialog
        {
            Title = "写入操作需要批准",
            Content = $"Agent 需要执行写入操作:\n\n工具: {tool}\n参数: {TruncateString(args, 200)}\n\n是否允许?",
            PrimaryButtonText = "批准",
            SecondaryButtonText = "拒绝",
            DefaultButton = ContentDialogButton.Secondary,
            XamlRoot = Content.XamlRoot
        };

        var result = await dialog.ShowAsync();
        // TODO: Send approval/denial back to backend
        // For now, just log the decision
        if (result == ContentDialogResult.Primary)
        {
            AppendMessage("Agent", $"已批准写入操作: {tool}");
        }
        else
        {
            AppendMessage("Agent", $"已拒绝写入操作: {tool}");
        }
    }

    private static string TruncateString(string s, int maxLen)
    {
        if (s.Length <= maxLen) return s;
        return s[..maxLen] + "…";
    }
}
