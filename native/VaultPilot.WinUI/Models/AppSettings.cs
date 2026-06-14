namespace VaultPilot.WinUI.Models;

public sealed record ProviderConfig(
    string ApiKey,
    string BaseUrl,
    string Model,
    ulong RequestTimeoutMs,
    ulong? ContextWindowTokens,
    uint? MaxOutputTokens,
    string? ProviderType)
{
    public override string ToString() =>
        $"ProviderConfig {{ ApiKey = [REDACTED], BaseUrl = {BaseUrl}, Model = {Model}, RequestTimeoutMs = {RequestTimeoutMs}, ContextWindowTokens = {ContextWindowTokens}, MaxOutputTokens = {MaxOutputTokens}, ProviderType = {ProviderType} }}";
}

public sealed record AppSettings(
    string VaultDir,
    ProviderConfig Provider,
    bool AutoCheckUpdates,
    bool AutoWakeEnabled,
    ulong AutoWakeIntervalMinutes,
    string AutoWakeModel,
    string AutoWakeStartTime,
    string AutoWakeEndTime);
