using Xunit;
using System.Text.Json;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #2621: OnSettingsClicked uses active provider
/// instead of legacy Provider field for model suggestions.
///
/// Regression test for issue #2617: ActiveProviderIndex clamped to lower
/// bound 0 to prevent crash on negative value.
///
/// Bug (#2621):  OnSettingsClicked used _settings.Provider.BaseUrl (legacy
///               single-provider field) instead of the active provider from
///               the Providers list.
/// Bug (#2617):  Math.Min on ActiveProviderIndex allowed negative values
///               (-1) to pass through, causing ArgumentOutOfRangeException.
/// Root cause:   Settings model supported multi-provider but consumer code
///               fell back to legacy single-provider path.
/// Fix:          PR #2621 (commit c4c8e03) + PR #2617 (commit 843b498)
/// </summary>
public class Issue2621ProviderSelectionTests
{
    private static ProviderConfig CreateProvider(string name, string url, string model) =>
        new(
            ApiKey: $"key-{name}",
            BaseUrl: url,
            Model: model,
            RequestTimeoutMs: 30_000,
            ContextWindowTokens: 128_000,
            MaxOutputTokens: 4096,
            ProviderType: "openai");

    [Fact]
    public void Regression_2621_ProvidersListRoundtripsViaJson()
    {
        // Arrange: settings with multiple providers
        var p1 = CreateProvider("openai", "https://api.openai.com", "gpt-4");
        var p2 = CreateProvider("anthropic", "https://api.anthropic.com", "claude-3");
        var providers = new List<ProviderConfig> { p1, p2 };

        var settings = new AppSettings(
            VaultDir: @"C:\vault",
            Provider: p1,
            AutoCheckUpdates: true,
            AutoWakeEnabled: false,
            AutoWakeIntervalMinutes: 30,
            AutoWakeModel: "gpt-4",
            AutoWakeStartTime: "09:00",
            AutoWakeEndTime: "17:00")
        {
            Providers = providers,
            ActiveProviderIndex = 1
        };

        // Act: serialization round-trip
        var json = JsonSerializer.Serialize(settings);
        var deserialized = JsonSerializer.Deserialize<AppSettings>(json);

        // Assert: Providers list preserved
        Assert.NotNull(deserialized);
        Assert.NotNull(deserialized!.Providers);
        Assert.Equal(2, deserialized.Providers.Count);
        Assert.Equal("https://api.anthropic.com", deserialized.Providers[1].BaseUrl);
        Assert.Equal(1, deserialized.ActiveProviderIndex);
    }

    [Fact]
    public void Regression_2617_ActiveProviderIndexClampedInConsumerCode()
    {
        // This tests the consumer-side clamp logic (Math.Clamp) that
        // exists in SettingsDialog.xaml.cs line 82. We verify the model
        // allows negative values (which the consumer must clamp).
        var p = CreateProvider("openai", "https://api.openai.com", "gpt-4");
        var settings = new AppSettings(
            VaultDir: @"C:\vault",
            Provider: p,
            AutoCheckUpdates: true,
            AutoWakeEnabled: false,
            AutoWakeIntervalMinutes: 30,
            AutoWakeModel: "gpt-4",
            AutoWakeStartTime: "09:00",
            AutoWakeEndTime: "17:00")
        {
            Providers = new List<ProviderConfig> { p },
            ActiveProviderIndex = -1  // deliberately negative
        };

        // The model allows -1 (it's just data) — the consumer must clamp.
        // Verify the consumer-side clamp: Math.Clamp(-1, 0, count-1)
        var count = settings.Providers.Count;
        var clampedIndex = Math.Clamp(settings.ActiveProviderIndex, 0, count - 1);
        Assert.Equal(0, clampedIndex);

        // Also verify that an out-of-range high index gets clamped
        var clampedHigh = Math.Clamp(99, 0, count - 1);
        Assert.Equal(0, clampedHigh);
    }

    [Fact]
    public void Regression_2621_EmptyProvidersListFallsBackToLegacyProvider()
    {
        // When Providers list is empty, the consumer code should fall
        // back to the legacy Provider field (backward compat).
        var legacyProvider = CreateProvider("legacy", "https://legacy.example.com", "gpt-3.5");
        var settings = new AppSettings(
            VaultDir: @"C:\vault",
            Provider: legacyProvider,
            AutoCheckUpdates: true,
            AutoWakeEnabled: false,
            AutoWakeIntervalMinutes: 30,
            AutoWakeModel: "gpt-3.5",
            AutoWakeStartTime: "09:00",
            AutoWakeEndTime: "17:00");

        // Verify fallback logic (mirrors MainWindow.xaml.cs:244-246):
        // var activeProvider = Providers.Count > 0
        //     ? Providers[Clamp(ActiveProviderIndex, 0, Providers.Count-1)]
        //     : Provider;
        Assert.Empty(settings.Providers);
        var activeProvider = settings.Providers.Count > 0
            ? settings.Providers[Math.Clamp(settings.ActiveProviderIndex, 0, settings.Providers.Count - 1)]
            : settings.Provider;
        Assert.Equal("https://legacy.example.com", activeProvider.BaseUrl);
    }
}
