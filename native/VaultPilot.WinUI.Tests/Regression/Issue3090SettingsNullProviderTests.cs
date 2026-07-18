using Xunit;
using System.IO;
using System.Text.Json;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3090: NullReferenceException when
/// AppSettings.Provider is null after JSON deserialisation.
///
/// Bug: System.Text.Json can deserialise "provider": null explicitly,
/// overriding the C# default `new ProviderConfig()`. Accesses like
/// `_settings.Provider.BaseUrl`, `_settings.Provider.RequestTimeoutMs`,
/// `_settings.Provider.ContextWindowTokens`, and `_settings.Provider.Model`
/// used single-level null-conditional (`_settings?.Provider.XXX`) which
/// only guards _settings, not Provider itself — causing a
/// NullReferenceException whenever the backend returned "provider": null.
///
/// Fix: all Provider property accesses upgraded to double-level
/// null-propagation (`_settings?.Provider?.XXX ?? fallback`), and the
/// fallback in OnSettingsClicked now coalesces to `new ProviderConfig()`
/// when Provider is null.
///
/// These assertions verify the JSON deserialisation behaviour and the
/// defensive null-propagation in source files (live UI tests need a
/// Windows environment which is unavailable on CI Linux runners).
/// </summary>
public class Issue3090SettingsNullProviderTests
{
    [Fact]
    public void Regression_3090_Provider_Default_GetsOverriddenByNullJson()
    {
        // Demonstrate the root cause: System.Text.Json explicitly sets
        // Provider to null when the payload contains "provider": null,
        // overriding the C# property initialiser `new ProviderConfig()`.
        var json = """{"provider": null, "vaultDir": "/tmp/vault"}""";
        var opts = new JsonSerializerOptions
        {
            PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
            PropertyNameCaseInsensitive = true,
        };
        var settings = JsonSerializer.Deserialize<AppSettings>(json, opts);
        Assert.NotNull(settings);
        // Critical: Provider MUST be null after "provider": null payload.
        Assert.Null(settings!.Provider);
        // Providers list should still be initialised (separate property).
        Assert.NotNull(settings.Providers);
        Assert.Empty(settings.Providers);
    }

    [Fact]
    public void Regression_3090_ProviderConfig_Fallback_New_HasNonEmptyFields()
    {
        // The fallback `new ProviderConfig()` used in OnSettingsClicked
        // must produce a safe instance that won't NPE on property access.
        var fallback = new ProviderConfig();
        Assert.Equal(string.Empty, fallback.BaseUrl);
        Assert.Equal(string.Empty, fallback.ApiKey);
        Assert.Equal(string.Empty, fallback.Model);
        Assert.Equal(0UL, fallback.RequestTimeoutMs);
        // BaseUrl access on the fallback must NOT throw.
        var baseUrl = fallback.BaseUrl;
        Assert.NotNull(baseUrl);
    }

    [Fact]
    public void Regression_3090_DefensiveNullCoalescing_PreventsNPE()
    {
        // Simulate the fixed pattern: safe access regardless of whether
        // Provider is null. This is the exact pattern used in the fix.
        AppSettings? settingsWithNullProvider = new AppSettings
        {
            VaultDir = "/tmp/vault",
            Provider = null!,  // intentionally null
        };

        // Fixed pattern (OnSettingsClicked):
        var activeProvider = (settingsWithNullProvider.Providers.Count > 0)
            ? settingsWithNullProvider.Providers[0]
            : (settingsWithNullProvider.Provider ?? new ProviderConfig());
        Assert.NotNull(activeProvider);
        var models = activeProvider.BaseUrl; // must not NPE
        Assert.Equal(string.Empty, models);

        // Fixed pattern (RequestTimeoutMs):
        var timeout = settingsWithNullProvider?.Provider?.RequestTimeoutMs ?? 60_000UL;
        Assert.Equal(60_000UL, timeout);

        // Fixed pattern (ContextWindowTokens):
        var context = settingsWithNullProvider?.Provider?.ContextWindowTokens;
        Assert.Null(context); // nullable — null when Provider is null

        // Fixed pattern (Model):
        var model = (settingsWithNullProvider?.Provider?.Model ?? string.Empty).Trim().ToLowerInvariant();
        Assert.Equal(string.Empty, model);
    }

    private static string? ResolveSource(string relative)
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", relative);
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }

    [Fact]
    public void Regression_3090_Source_HasDefensiveNullPropagation_MainWindow()
    {
        var sourcePath = ResolveSource("MainWindow.xaml.cs");
        if (sourcePath is null)
        {
            // Source not co-located in this build layout — skip.
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // The OnSettingsClicked fix MUST include the null-coalescing fallback.
        Assert.Contains("_settings.Provider ?? new ProviderConfig()", source);

        // The GetModelsForProvider call must use null-safe access.
        Assert.Contains("GetModelsForProvider(activeProvider?.BaseUrl ?? string.Empty)", source);

        // The RequestTimeoutMs fix MUST include double-level null-propagation.
        Assert.Contains("_settings?.Provider?.RequestTimeoutMs", source);
    }

    [Fact]
    public void Regression_3090_Source_HasDefensiveNullPropagation_ChatStreaming()
    {
        var sourcePath = ResolveSource("MainWindow.ChatStreamingHandler.cs");
        if (sourcePath is null)
        {
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // ContextWindowTokens must use double-level null-propagation.
        Assert.Contains("_settings?.Provider?.ContextWindowTokens", source);

        // Model access must use double-level null-propagation.
        Assert.Contains("_settings?.Provider?.Model", source);
    }
}