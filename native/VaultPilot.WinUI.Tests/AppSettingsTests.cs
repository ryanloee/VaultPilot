using Xunit;
using System.Text.Json;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests;

public class AppSettingsTests
{
    private static ProviderConfig CreateTestProvider() =>
        new(
            ApiKey: "test-key-123",
            BaseUrl: "https://api.example.com",
            Model: "gpt-4",
            RequestTimeoutMs: 30_000,
            ContextWindowTokens: 128_000,
            MaxOutputTokens: 4096,
            ProviderType: "openai");

    private static AppSettings CreateTestSettings() =>
        new(
            VaultDir: @"C:\Users\test\vault",
            Provider: CreateTestProvider(),
            AutoCheckUpdates: true,
            AutoWakeEnabled: false,
            AutoWakeIntervalMinutes: 30,
            AutoWakeModel: "gpt-4",
            AutoWakeStartTime: "09:00",
            AutoWakeEndTime: "17:00");

    [Fact]
    public void AppSettings_RecordEquality_SameValues_AreEqual()
    {
        var a = CreateTestSettings();
        var b = CreateTestSettings();

        Assert.Equal(a, b);
        Assert.Equal(a.GetHashCode(), b.GetHashCode());
    }

    [Fact]
    public void AppSettings_RecordEquality_DifferentValues_AreNotEqual()
    {
        var a = CreateTestSettings();
        var b = a with { VaultDir = @"C:\other\path" };

        Assert.NotEqual(a, b);
    }

    [Fact]
    public void AppSettings_Properties_ArePreserved()
    {
        var settings = CreateTestSettings();

        Assert.Equal(@"C:\Users\test\vault", settings.VaultDir);
        Assert.True(settings.AutoCheckUpdates);
        Assert.False(settings.AutoWakeEnabled);
        Assert.Equal(30UL, settings.AutoWakeIntervalMinutes);
        Assert.Equal("gpt-4", settings.AutoWakeModel);
        Assert.Equal("09:00", settings.AutoWakeStartTime);
        Assert.Equal("17:00", settings.AutoWakeEndTime);
    }

    [Fact]
    public void AppSettings_JsonRoundTrip_PreservesValues()
    {
        var original = CreateTestSettings();
        var json = JsonSerializer.Serialize(original);
        var deserialized = JsonSerializer.Deserialize<AppSettings>(json);

        Assert.NotNull(deserialized);
        Assert.Equal(original, deserialized);
    }

    [Fact]
    public void AppSettings_JsonRoundTrip_PreservesProvider()
    {
        var original = CreateTestSettings();
        var json = JsonSerializer.Serialize(original);
        var deserialized = JsonSerializer.Deserialize<AppSettings>(json);

        Assert.NotNull(deserialized);
        Assert.Equal(original.Provider.ApiKey, deserialized.Provider.ApiKey);
        Assert.Equal(original.Provider.BaseUrl, deserialized.Provider.BaseUrl);
        Assert.Equal(original.Provider.Model, deserialized.Provider.Model);
        Assert.Equal(original.Provider.RequestTimeoutMs, deserialized.Provider.RequestTimeoutMs);
        Assert.Equal(original.Provider.ContextWindowTokens, deserialized.Provider.ContextWindowTokens);
        Assert.Equal(original.Provider.MaxOutputTokens, deserialized.Provider.MaxOutputTokens);
        Assert.Equal(original.Provider.ProviderType, deserialized.Provider.ProviderType);
    }

    [Fact]
    public void AppSettings_NullableContextWindow_Allowed()
    {
        var provider = new ProviderConfig(
            ApiKey: "key",
            BaseUrl: "https://api.example.com",
            Model: "gpt-3.5-turbo",
            RequestTimeoutMs: 15_000,
            ContextWindowTokens: null,
            MaxOutputTokens: null,
            ProviderType: null);

        Assert.Null(provider.ContextWindowTokens);
        Assert.Null(provider.MaxOutputTokens);
        Assert.Null(provider.ProviderType);
    }
}
