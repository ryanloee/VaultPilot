using Xunit;
using VaultPilot.WinUI.Views;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3133: the WinUI SettingsDialog provider-type
/// round-trip was not idempotent for custom / future provider names.
///
/// Root cause: the load path used substring matching (Contains("anthropic") /
/// Contains("ollama")) to pick a ComboBox index, and the save path always emitted
/// a canonical literal for that index. Any saved value containing those
/// substrings (e.g. "anthropic-compatible", "my-ollama-fork") was silently
/// rewritten to the literal "anthropic" / "ollama" on the first save, destroying
/// the exact provider identifier. The validate path (TryBuildProviderConfig) had
/// its own third, inconsistent notion of the provider set.
///
/// Fix: a single canonical-set helper (IsCanonicalProviderType) is used by all
/// three paths. The verbatim loaded string is preserved in _loadedProviderType;
/// when it is a custom / future name it is emitted verbatim on save, so the
/// round-trip is idempotent.
/// </summary>
public class Issue3133ProviderTypeRoundTripTests
{
    private const string ValidKey = "sk-test-123";
    private const string ValidUrl = "http://localhost:11434";
    private const string ValidModel = "llama3";
    private const string ValidTimeout = "30000";
    private const string ValidContext = "128000";

    [Fact]
    public void Regression_3133_CustomAnthropicName_PreservedVerbatim()
    {
        // A custom provider whose name merely contains "anthropic" must not be
        // coerced to the literal "anthropic".
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "anthropic-compatible", "Custom", out var cfg);
        Assert.True(ok);
        Assert.Equal("anthropic-compatible", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_3133_CustomOllamaName_PreservedVerbatim()
    {
        // A custom provider whose name merely contains "ollama" must not be
        // coerced to the literal "ollama".
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "my-ollama-fork", "Custom", out var cfg);
        Assert.True(ok);
        Assert.Equal("my-ollama-fork", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_3133_FutureProviderName_PreservedVerbatim()
    {
        // A genuinely unknown / future provider name must survive the round-trip
        // instead of being silently rewritten to "openai".
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "some-future-provider", "Custom", out var cfg);
        Assert.True(ok);
        Assert.Equal("some-future-provider", cfg!.ProviderType);
    }

    [Fact]
    public void Regression_3133_CanonicalNames_NormalizedCaseInsensitively()
    {
        // Canonical names are still lower-cased / normalized as before.
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "Anthropic", "Claude", out var cfg);
        Assert.True(ok);
        Assert.Equal("anthropic", cfg!.ProviderType);

        ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, "OLLAMA", "Local", out var cfg2);
        Assert.True(ok);
        Assert.Equal("ollama", cfg2!.ProviderType);
    }

    [Fact]
    public void Regression_3133_NullType_DefaultsToOpenAi()
    {
        // A null provider type has no verbatim value to preserve, so the safe
        // default (openai) still applies.
        var ok = SettingsDialog.TryBuildProviderConfig(
            ValidKey, ValidUrl, ValidModel, ValidTimeout, ValidContext,
            null, null, "x", out var cfg);
        Assert.True(ok);
        Assert.Equal("openai", cfg!.ProviderType);
    }
}
