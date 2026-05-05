namespace VaultPilot.WinUI.Models;

public sealed record ProviderConfig(
    string ApiKey,
    string BaseUrl,
    string Model,
    ulong RequestTimeoutMs,
    ulong? ContextWindowTokens);

public sealed record AppSettings(
    string VaultDir,
    ProviderConfig Provider,
    bool AutoCheckUpdates,
    bool AutoWakeEnabled,
    ulong AutoWakeIntervalMinutes,
    string AutoWakeModel,
    string AutoWakeStartTime,
    string AutoWakeEndTime);
