using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Text;

namespace VaultPilot.WinUI;

/// <summary>
/// Static utility methods — extracted from MainWindow.xaml.cs (#1206).
/// Theme helpers, version, token/model detection, localization, logging.
/// </summary>
public sealed partial class MainWindow : Window
{
    // ── Theme helpers ──

    /// <summary>Looks up a theme-aware brush from application resources.</summary>
    private static readonly SolidColorBrush _transparentBrush = new(Microsoft.UI.Colors.Transparent);

    private static Brush GetThemeBrush(string key)
    {
        if (Application.Current?.Resources is not null
            && Application.Current.Resources.TryGetValue(key, out var value) && value is Brush brush)
        {
            return brush;
        }
        Debug.WriteLine($"[GetThemeBrush] Missing resource key: '{key}', falling back to Transparent.");
        return _transparentBrush;
    }

    /// <summary>Looks up a theme-aware Style from application resources, returning null if missing.</summary>
    private static Style? GetThemeStyle(string key)
    {
        if (Application.Current?.Resources is not null
            && Application.Current.Resources.TryGetValue(key, out var value) && value is Style style)
            return style;
        Debug.WriteLine($"[GetThemeStyle] Missing resource key: '{key}'.");
        return null;
    }

    // ── Version ──

    private static string ResolveDisplayVersion()
    {
        var informationalVersion = typeof(MainWindow).Assembly
            .GetCustomAttribute<AssemblyInformationalVersionAttribute>()?
            .InformationalVersion;
        var cleanVersion = (informationalVersion ?? string.Empty).Split('+', 2)[0].Trim();
        if (string.IsNullOrWhiteSpace(cleanVersion))
        {
            cleanVersion = typeof(MainWindow).Assembly.GetName().Version?.ToString() ?? "0.0.0";
        }

        return cleanVersion.StartsWith("v", StringComparison.OrdinalIgnoreCase)
            ? cleanVersion
            : $"v{cleanVersion}";
    }

    // ── Error & status helpers ──

    private void ShowError(string title, Exception error, bool addMessage = true)
    {
        UpdateStatusBar("error", title, LocalizeError(error.Message));
        if (addMessage)
        {
            AddSystemMessage("错误", LocalizeError(error.Message));
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
            Debug.WriteLine($"[UpdateStartupStepAsync] Error: {error}");
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

    // ── Logging ──

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
            // #3604: prefix every milestone with elapsed ms since process start
            // so the startup timeline is measurable (e.g. "[+1234ms] Ping ok").
            var line = $"[+{App.StartupWatch.Elapsed.TotalMilliseconds,6:F0}ms] {DateTimeOffset.Now:O} {message}";
            await File.AppendAllTextAsync(StartupLogPath(), line + Environment.NewLine, Encoding.UTF8);
        }
        catch
        {
            // Ignore logging failures.
        }
    }

    // ── Model & token helpers ──

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

    // ── Localization ──

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
}
