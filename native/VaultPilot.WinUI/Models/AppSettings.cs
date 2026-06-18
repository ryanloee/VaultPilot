using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

public sealed record ProviderConfig
{
    public string ApiKey { get; init; } = string.Empty;
    public string BaseUrl { get; init; } = string.Empty;
    public string Model { get; init; } = string.Empty;
    public ulong RequestTimeoutMs { get; init; }
    public ulong? ContextWindowTokens { get; init; }
    public uint? MaxOutputTokens { get; init; }
    public string? ProviderType { get; init; }

    [JsonConstructor]
    public ProviderConfig() { }

    public ProviderConfig(
        string ApiKey,
        string BaseUrl,
        string Model,
        ulong RequestTimeoutMs,
        ulong? ContextWindowTokens,
        uint? MaxOutputTokens,
        string? ProviderType)
    {
        this.ApiKey = ApiKey ?? string.Empty;
        this.BaseUrl = BaseUrl ?? string.Empty;
        this.Model = Model ?? string.Empty;
        this.RequestTimeoutMs = RequestTimeoutMs;
        this.ContextWindowTokens = ContextWindowTokens;
        this.MaxOutputTokens = MaxOutputTokens;
        this.ProviderType = ProviderType;
    }

    public override string ToString() =>
        $"ProviderConfig {{ ApiKey = [REDACTED], BaseUrl = {BaseUrl}, Model = {Model}, RequestTimeoutMs = {RequestTimeoutMs}, ContextWindowTokens = {ContextWindowTokens}, MaxOutputTokens = {MaxOutputTokens}, ProviderType = {ProviderType} }}";
}

public sealed record AppSettings
{
    public string VaultDir { get; init; } = string.Empty;
    public ProviderConfig Provider { get; init; } = new ProviderConfig();
    public bool AutoCheckUpdates { get; init; } = true;
    public bool AutoWakeEnabled { get; init; }
    public ulong AutoWakeIntervalMinutes { get; init; }
    public string AutoWakeModel { get; init; } = string.Empty;
    public string AutoWakeStartTime { get; init; } = string.Empty;
    public string AutoWakeEndTime { get; init; } = string.Empty;

    [JsonConstructor]
    public AppSettings() { }

    public AppSettings(
        string VaultDir,
        ProviderConfig Provider,
        bool AutoCheckUpdates,
        bool AutoWakeEnabled,
        ulong AutoWakeIntervalMinutes,
        string AutoWakeModel,
        string AutoWakeStartTime,
        string AutoWakeEndTime)
    {
        this.VaultDir = VaultDir ?? string.Empty;
        this.Provider = Provider ?? new ProviderConfig();
        this.AutoCheckUpdates = AutoCheckUpdates;
        this.AutoWakeEnabled = AutoWakeEnabled;
        this.AutoWakeIntervalMinutes = AutoWakeIntervalMinutes;
        this.AutoWakeModel = AutoWakeModel ?? string.Empty;
        this.AutoWakeStartTime = AutoWakeStartTime ?? string.Empty;
        this.AutoWakeEndTime = AutoWakeEndTime ?? string.Empty;
    }
}
