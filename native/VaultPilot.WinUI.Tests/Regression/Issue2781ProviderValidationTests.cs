using Xunit;
using VaultPilot.WinUI.Views;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #2781: provider validation in SettingsDialog was
/// gated behind a _providerFieldsDirty flag that got reset whenever the user
/// switched providers, allowing illegal configs (empty API key, bad URL, etc.)
/// to be saved. Additionally, SaveCurrentProviderFields wrote illegal values
/// into _providers during a switch, where they survived into saved settings.
///
/// Fix: validation always runs (TryBuildProviderConfig), and
/// SaveCurrentProviderFields only persists valid configs. This test pins the
/// validation rules so they cannot silently regress.
/// </summary>
public class Issue2781ProviderValidationTests
{
    private const string ValidKey = "sk-test-123";
    private const string ValidUrl = "https://api.openai.com";
    private const string ValidModel = "gpt-4";
    private const string ValidTimeout = "30000";
    private const string ValidContext = "128000";

    [Fact]
    public void Regression_2781_ValidConfig_BuildsProvider()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            4096, "openai", "OpenAI", out var cfg);
        Assert.True(ok);
        Assert.NotNull(cfg);
        Assert.Equal(ValidKey, cfg!.ApiKey);
        Assert.Equal(ValidUrl, cfg.BaseUrl);
        Assert.Equal(ValidModel, cfg.Model);
        Assert.Equal("openai", cfg.ProviderType);
        Assert.Equal("OpenAI", cfg.Name);
    }

    [Fact]
    public void Regression_2781_AnthropicFlag_SetsProviderType()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "anthropic", "Claude", out var cfg);
        Assert.True(ok);
        Assert.NotNull(cfg);
        Assert.Equal("anthropic", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_2781_EmptyApiKey_Rejected()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            "", ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "openai", "x", out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }

    [Fact]
    public void Regression_2781_EmptyBaseUrl_Rejected()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, "", ValidModel, ValidTimeout, ValidContext,
            null, "openai", "x", out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }

    [Fact]
    public void Regression_2781_NonHttpUrl_Rejected()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, "ftp://example.com", ValidModel, ValidTimeout, ValidContext,
            null, "openai", "x", out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }

    [Fact]
    public void Regression_2781_EmptyModel_Rejected()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, "", ValidTimeout, ValidContext,
            null, "openai", "x", out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }

    [Fact]
    public void Regression_2781_TimeoutTooLow_Rejected()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, "500", ValidContext,
            null, "openai", "x", out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }

    [Fact]
    public void Regression_2781_TimeoutTooHigh_Rejected()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, "400000", ValidContext,
            null, "openai", "x", out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }

    [Fact]
    public void Regression_2781_ContextWindowTooLarge_Rejected()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, "3000000",
            null, "openai", "x", out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }

    [Fact]
    public void Regression_2781_ContextWindowNonNumeric_Rejected()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, "abc",
            null, "openai", "x", out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }

    [Fact]
    public void Regression_2781_EmptyContextWindow_Allowed()
    {
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, "",
            null, "openai", "x", out var cfg);
        Assert.True(ok);
        Assert.NotNull(cfg);
        Assert.Null(cfg!.ContextWindowTokens);
    }

    [Fact]
    public void Regression_2781_NullInputs_Rejected()
    {
        // Simulates controls returning null (e.g. untouched) — must not produce
        // a valid config that could slip past always-on validation.
        var ok = SettingsDialog.TryBuildProviderConfig(
            null, null, null, null, null, null, "openai", null, out var cfg);
        Assert.False(ok);
        Assert.Null(cfg);
    }
}
