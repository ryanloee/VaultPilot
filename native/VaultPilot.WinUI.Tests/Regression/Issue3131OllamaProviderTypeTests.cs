using Xunit;
using VaultPilot.WinUI.Views;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3131: the WinUI SettingsDialog silently rewrote
/// any provider with a non-OpenAI / non-Anthropic providerType (e.g. "ollama")
/// to "openai" on save, because the old TryBuildProviderConfig only accepted a
/// bool isAnthropic and defaulted everything else to openai.
///
/// Fix: TryBuildProviderConfig now round-trips the actual provider type string
/// (openai / anthropic / ollama, falling back to openai for unknown values) and
/// the dialog maps the Ollama ComboBox item (SelectedIndex 2) to "ollama".
/// </summary>
public class Issue3131OllamaProviderTypeTests
{
    private const string ValidKey = "sk-test-123";
    private const string ValidUrl = "http://localhost:11434";
    private const string ValidModel = "llama3";
    private const string ValidTimeout = "30000";
    private const string ValidContext = "128000";

    [Fact]
    public void Regression_3131_OllamaType_Preserved()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "ollama", "Local", out var cfg);
        Assert.True(ok);
        Assert.NotNull(cfg);
        Assert.Equal("ollama", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_3131_OllamaType_FullString_Preserved()
    {
        // settings.json may store the providerType verbatim.
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "Ollama", "Local", out var cfg);
        Assert.True(ok);
        Assert.Equal("ollama", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_3131_OpenAiType_Unchanged()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "openai", "OpenAI", out var cfg);
        Assert.True(ok);
        Assert.Equal("openai", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_3131_AnthropicType_Unchanged()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "anthropic", "Claude", out var cfg);
        Assert.True(ok);
        Assert.Equal("anthropic", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_3131_UnknownType_FallsBackToOpenAi()
    {
        // Safety: unknown provider types must not produce garbage; default openai.
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "some-future-provider", "x", out var cfg);
        Assert.True(ok);
        Assert.Equal("openai", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_3131_NullType_DefaultsToOpenAi()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, null, "x", out var cfg);
        Assert.True(ok);
        Assert.Equal("openai", cfg!.ProviderType);
    }
}
