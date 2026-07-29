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

    // #3581: cache token estimates per session so we don't re-iterate all
    // turns on every RefreshContextStatus call. Invalidated on turn changes.
    private readonly Dictionary<string, ulong> _tokenEstimateCache = new();

    /// <summary>Invalidates the cached token estimate for the given session.</summary>
    private void InvalidateTokenEstimate(string? sessionId)
    {
        if (sessionId is not null)
            _tokenEstimateCache.Remove(sessionId);
    }

    private void RefreshContextStatus()
    {
        var session = CurrentSession();
        if (session is null)
        {
            _contextUsagePercent = 0;
            UpdateContextUsageBarVisual();
            return;
        }
        var contextWindow = ResolveContextWindowTokens();
        var usedTokens = EstimateSessionTokensCached(session);
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

    private ulong EstimateSessionTokensCached(ChatSession session)
    {
        if (session.Id is not null && _tokenEstimateCache.TryGetValue(session.Id, out var cached))
            return cached;

        var total = EstimateSessionTokensUncached(session);
        if (session.Id is not null)
            _tokenEstimateCache[session.Id] = total;
        return total;
    }

    private ulong EstimateSessionTokensUncached(ChatSession session)
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

    private ProviderConfig ResolveActiveProvider()
    {
        // Mirror the active-provider resolution used by MainWindow.xaml.cs
        // (#266-268): when the multi-provider list is populated, the real
        // provider used for requests/model selection is Providers[ActiveProviderIndex],
        // NOT the legacy single Provider field. Reading Provider here is a logic
        // bug (#3191) that yields wrong context-window budgets and model names
        // when the active provider differs from the legacy field.
        if (_settings?.Providers is { Count: > 0 } providers)
        {
            var index = Math.Clamp(_settings.ActiveProviderIndex, 0, providers.Count - 1);
            return providers[index];
        }

        // Fallback to legacy single Provider field (backward compat; defensive
        // null-coalescing per issue #3090).
        return _settings?.Provider ?? new ProviderConfig();
    }

    private ulong ResolveContextWindowTokens()
    {
        var activeProvider = ResolveActiveProvider();
        var configuredLimit = activeProvider.ContextWindowTokens;
        if (configuredLimit.HasValue && configuredLimit.Value > 0)
        {
            return configuredLimit.Value;
        }

        var model = (activeProvider.Model ?? string.Empty).Trim().ToLowerInvariant();
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
