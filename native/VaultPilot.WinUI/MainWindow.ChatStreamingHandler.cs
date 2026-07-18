using VaultPilot.WinUI.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Linq;

namespace VaultPilot.WinUI;

/// <summary>
/// Context status display, token estimation, and context window resolution —
/// split from MainWindow.Chat.cs (#1344).
/// </summary>
public sealed partial class MainWindow : Window
{
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
        // Defensive null-propagation (issue #3090): _settings?.Provider only
        // guards _settings, not Provider itself — System.Text.Json can leave
        // Provider null if the backend explicitly sent "provider": null.
        var configuredLimit = _settings?.Provider?.ContextWindowTokens;
        if (configuredLimit.HasValue && configuredLimit.Value > 0)
        {
            return configuredLimit.Value;
        }

        var model = (_settings?.Provider?.Model ?? string.Empty).Trim().ToLowerInvariant();
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
}
