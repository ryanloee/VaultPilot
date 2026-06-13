using VaultPilot.WinUI;

namespace VaultPilot.WinUI.Tests;

public class MainWindowUtilityTests
{
    // ── ContainsModelToken ──

    [Theory]
    [InlineData("gpt-4o", "4o", true)]
    [InlineData("gpt-4o-mini", "4o", true)]
    [InlineData("co1l", "o1", false)]  // not at word boundary
    [InlineData("o1", "o1", true)]
    [InlineData("openai/o1-mini", "o1", true)]
    [InlineData("gpt-4", "4", true)]
    [InlineData("gpt-4-turbo", "4", true)]
    [InlineData("claude-3-opus", "opus", true)]
    public void ContainsModelToken_BoundaryMatching(string model, string token, bool expected)
    {
        Assert.Equal(expected, MainWindow.ContainsModelToken(model, token));
    }

    // ── IsOpenAiOSeriesModel ──

    [Theory]
    [InlineData("o1", true)]
    [InlineData("o1-mini", true)]
    [InlineData("o1-preview", true)]
    [InlineData("o3", true)]
    [InlineData("o3-mini", true)]
    [InlineData("o4-mini", true)]
    [InlineData("openai/o1-mini", true)]
    [InlineData("gpt-4", false)]
    [InlineData("gpt-4o", false)]
    [InlineData("co1l", false)]
    [InlineData("po3", false)]
    [InlineData("claude-3-opus", false)]
    public void IsOpenAiOSeriesModel_DetectsOSeries(string model, bool expected)
    {
        Assert.Equal(expected, MainWindow.IsOpenAiOSeriesModel(model));
    }

    // ── IsModelSeparator ──

    [Theory]
    [InlineData('-', true)]
    [InlineData('_', true)]
    [InlineData('.', true)]
    [InlineData('/', true)]
    [InlineData(' ', true)]
    [InlineData('(', true)]
    [InlineData(')', true)]
    [InlineData(':', true)]
    [InlineData(',', true)]
    [InlineData('a', false)]
    [InlineData('4', false)]
    public void IsModelSeparator_CorrectClassification(char c, bool expected)
    {
        Assert.Equal(expected, MainWindow.IsModelSeparator(c));
    }

    // ── FormatTokenCount ──

    [Theory]
    [InlineData(0UL, "0")]
    [InlineData(42UL, "42")]
    [InlineData(999UL, "999")]
    [InlineData(1000UL, "1K")]
    [InlineData(1500UL, "1.5K")]
    [InlineData(128000UL, "128K")]
    [InlineData(1000000UL, "1M")]
    [InlineData(1500000UL, "1.5M")]
    [InlineData(20000000UL, "20M")]
    public void FormatTokenCount_VariousRanges(ulong tokens, string expected)
    {
        Assert.Equal(expected, MainWindow.FormatTokenCount(tokens));
    }

    // ── BuildSessionTitle ──

    [Fact]
    public void BuildSessionTitle_ShortText_Preserved()
    {
        Assert.Equal("hello world", MainWindow.BuildSessionTitle("hello world"));
    }

    [Fact]
    public void BuildSessionTitle_ExactBoundary_Preserved()
    {
        var text = new string('a', 28);
        Assert.Equal(text, MainWindow.BuildSessionTitle(text));
    }

    [Fact]
    public void BuildSessionTitle_OverBoundary_Truncated()
    {
        var text = new string('a', 29);
        var result = MainWindow.BuildSessionTitle(text);
        Assert.Equal(28 + 3, result.Length); // 28 chars + "..."
        Assert.EndsWith("...", result);
    }

    [Fact]
    public void BuildSessionTitle_NormalizesWhitespace()
    {
        Assert.Equal("hello world", MainWindow.BuildSessionTitle("  hello   world  "));
    }

    [Fact]
    public void BuildSessionTitle_EmptyString()
    {
        Assert.Equal("", MainWindow.BuildSessionTitle(""));
    }

    // ── LocalizeStage ──

    [Theory]
    [InlineData("analyzing", "正在分析")]
    [InlineData("compressing", "正在压缩上下文")]
    [InlineData("responding", "正在组织回复")]
    [InlineData("retrieving", "正在检索")]
    [InlineData("ranking", "正在排序")]
    [InlineData("executing", "正在执行工具")]
    [InlineData("saving", "正在保存")]
    [InlineData("unknown_stage", "unknown_stage")]  // fallback
    public void LocalizeStage_KnownAndUnknown(string input, string expected)
    {
        Assert.Equal(expected, MainWindow.LocalizeStage(input));
    }

    // ── LocalizeStatusDetail ──

    [Theory]
    [InlineData("Analyzing request", "正在分析请求")]
    [InlineData("Preparing request...", "正在准备请求...")]
    [InlineData("Preparing answer", "正在准备回复")]
    [InlineData("Loading recent notes", "正在加载最近笔记")]
    [InlineData("unknown detail", "unknown detail")]  // fallback
    public void LocalizeStatusDetail_KnownAndUnknown(string input, string expected)
    {
        Assert.Equal(expected, MainWindow.LocalizeStatusDetail(input));
    }

    [Fact]
    public void LocalizeStatusDetail_SearchPrefix_Converted()
    {
        var result = MainWindow.LocalizeStatusDetail("Searching notes: my query");
        Assert.Equal("正在搜索笔记：my query", result);
    }

    // ── LocalizeError ──

    [Theory]
    [InlineData("API key is empty", "API Key 为空，请先在设置中配置模型服务。")]
    [InlineData("401 Unauthorized", "认证失败（401），请检查 API Key 是否正确。")]
    [InlineData("model not found", "指定的模型不存在，请在设置中检查模型名称。")]
    [InlineData("Connection refused", "连接被拒绝，后端服务可能未启动。")]
    [InlineData("502 Bad Gateway", "网关错误（502），服务可能正在重启。")]
    public void LocalizeError_KnownErrors_Translated(string input, string expected)
    {
        Assert.Equal(expected, MainWindow.LocalizeError(input));
    }

    [Fact]
    public void LocalizeError_UnknownError_Preserved()
    {
        var msg = "Something completely unexpected happened";
        Assert.Equal(msg, MainWindow.LocalizeError(msg));
    }

    [Fact]
    public void LocalizeError_EmptyString_ReturnsEmpty()
    {
        Assert.Equal("", MainWindow.LocalizeError(""));
    }
}
