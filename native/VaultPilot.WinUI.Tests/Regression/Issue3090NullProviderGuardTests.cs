using System.Text.Json;
using VaultPilot.WinUI.Models;
using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3090: NullReferenceException in
/// MainWindow.OnSettingsClicked / OnAutoWakeTimerTick when the backend
/// returns `"provider": null` in the settings JSON.
///
/// Bug (#3090):  In MainWindow.xaml.cs, the settings-load path accessed
///               `_settings.Provider.BaseUrl` (and `_settings?.Provider.RequestTimeoutMs`)
///               without null-checking Provider. While AppSettings.Provider
///               has a default `new ProviderConfig()`, System.Text.Json
///               deserialization can set the property to null when the payload
///               explicitly includes `"provider": null`. The outer try/catch
///               caught the NPE but forced an opaque "打开设置失败" dialog.
/// Root cause:   No defensive null-coalesce at the consumer side.
/// Fix:          Add `?? new ProviderConfig()` at both call sites in
///               MainWindow.xaml.cs (OnSettingsClicked + OnAutoWakeTimerTick).
///
/// This test verifies the *logic mirror* of the consumer-side fix without
/// requiring WinUI infrastructure (UI thread, XamlRoot, etc.). It mirrors
/// the predicate used in production code.
/// </summary>
public class Issue3090NullProviderGuardTests
{
    private const string SettingsJsonWithNullProvider = """
        {
            "vault_dir": "C:\\vault",
            "provider": null,
            "providers": [],
            "auto_check_updates": true,
            "auto_wake_enabled": false,
            "auto_wake_interval_minutes": 30,
            "auto_wake_model": "gpt-4",
            "auto_wake_start_time": "09:00",
            "auto_wake_end_time": "17:00"
        }
        """;

    private const string SettingsJsonWithNullProviderAndEmptyProviders = """
        {
            "vault_dir": "C:\\vault",
            "provider": null,
            "providers": null,
            "auto_check_updates": true,
            "auto_wake_enabled": false,
            "auto_wake_interval_minutes": 30,
            "auto_wake_model": null,
            "auto_wake_start_time": "09:00",
            "auto_wake_end_time": "17:00"
        }
        """;

    /// <summary>
    /// Deserializing a JSON payload that explicitly sets `provider: null`
    /// must yield AppSettings.Provider == null (mirrors System.Text.Json
    /// behavior). This is the precondition that motivates the fix.
    /// </summary>
    [Fact]
    public void Regression_3090_NullProviderInJsonProducesNullProviderField()
    {
        var settings = JsonSerializer.Deserialize<AppSettings>(SettingsJsonWithNullProvider);
        Assert.NotNull(settings);
        // System.Text.Json honors the explicit null in the payload, even
        // though the property has an init default. This is the root cause.
        Assert.Null(settings!.Provider);
    }

    /// <summary>
    /// Direct mirror of the OnSettingsClicked fix at MainWindow.xaml.cs:262-265:
    /// the consumer must coalesce null Provider to a default before accessing BaseUrl.
    /// Without the `?? new ProviderConfig()` guard, this would NPE in production.
    /// </summary>
    [Fact]
    public void Regression_3090_SettingsClickedFallbackHandlesNullProvider()
    {
        var settings = JsonSerializer.Deserialize<AppSettings>(SettingsJsonWithNullProvider)!;
        Assert.Null(settings.Provider);

        // Mirror of the post-fix code:
        //   var activeProvider = Providers.Count > 0
        //       ? Providers[Clamp(ActiveProviderIndex, 0, count-1)]
        //       : (Provider ?? new ProviderConfig());
        //   var models = GetModelsForProvider((activeProvider ?? new ProviderConfig()).BaseUrl);
        var activeProvider = settings.Providers.Count > 0
            ? settings.Providers[Math.Clamp(settings.ActiveProviderIndex, 0, settings.Providers.Count - 1)]
            : (settings.Provider ?? new ProviderConfig());

        // No NullReferenceException — activeProvider is non-null even when
        // Provider was explicitly null in the JSON payload.
        Assert.NotNull(activeProvider);
        var baseUrl = (activeProvider ?? new ProviderConfig()).BaseUrl;
        Assert.NotNull(baseUrl); // ProviderConfig.BaseUrl defaults to empty string
    }

    /// <summary>
    /// Direct mirror of the OnAutoWakeTimerTick fix at MainWindow.xaml.cs:942:
    /// the timeout must coalesce null Provider to a sane default (60s).
    /// </summary>
    [Fact]
    public void Regression_3090_AutoWakeTimeoutHandlesNullProvider()
    {
        var settings = JsonSerializer.Deserialize<AppSettings>(SettingsJsonWithNullProvider)!;
        Assert.Null(settings.Provider);

        // Mirror of the post-fix code:
        //   var timeoutMs = (_settings?.Provider?.RequestTimeoutMs ?? 60_000) + 30_000;
        var timeoutMs = (settings?.Provider?.RequestTimeoutMs ?? 60_000) + 30_000;
        Assert.Equal(90_000, timeoutMs); // 60s default + 30s buffer
    }

    /// <summary>
    /// Sanity: a non-null Provider payload continues to take precedence
    /// over the default (no behavior regression for the normal case).
    /// </summary>
    [Fact]
    public void Regression_3090_NonNullProviderStillUsedWhenPresent()
    {
        const string json = """
            {
                "vault_dir": "C:\\vault",
                "provider": {
                    "api_key": "k",
                    "base_url": "https://api.example.com",
                    "model": "gpt-4",
                    "request_timeout_ms": 120000,
                    "context_window_tokens": 128000,
                    "max_output_tokens": 4096,
                    "provider_type": "openai"
                },
                "providers": [],
                "auto_check_updates": true
            }
            """;
        var settings = JsonSerializer.Deserialize<AppSettings>(json)!;
        Assert.NotNull(settings.Provider);
        Assert.Equal("https://api.example.com", settings.Provider!.BaseUrl);

        var activeProvider = settings.Providers.Count > 0
            ? settings.Providers[Math.Clamp(settings.ActiveProviderIndex, 0, settings.Providers.Count - 1)]
            : (settings.Provider ?? new ProviderConfig());
        Assert.Equal("https://api.example.com", activeProvider.BaseUrl);

        var timeoutMs = (settings?.Provider?.RequestTimeoutMs ?? 60_000) + 30_000;
        Assert.Equal(150_000, timeoutMs); // 120s configured + 30s buffer
    }
}
