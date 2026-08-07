using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

/// <summary>
/// Timing of a single startup phase, as reported by the backend agent's
/// <c>startupStats</c> JSON-RPC method (issue #3910).
/// The agent emits snake_case keys (<c>elapsed_ms</c>), so the property is
/// annotated with <see cref="JsonPropertyNameAttribute"/> while the client's
/// camelCase naming policy covers the rest.
/// </summary>
public sealed record StartupPhaseInfo
{
    public string Name { get; init; } = string.Empty;

    [JsonPropertyName("elapsed_ms")]
    public double ElapsedMs { get; init; }

    [JsonConstructor]
    public StartupPhaseInfo() { }

    public StartupPhaseInfo(string Name, double ElapsedMs)
    {
        this.Name = Name ?? string.Empty;
        this.ElapsedMs = ElapsedMs;
    }
}

/// <summary>
/// Result of the backend agent's <c>startupStats</c> JSON-RPC method
/// (issue #3910): one <see cref="StartupPhaseInfo"/> per measured startup
/// phase (config load, storage open, agent init, IPC ready, ...) plus the
/// total startup duration.
/// </summary>
public sealed record StartupStatsResponse
{
    public List<StartupPhaseInfo> Phases { get; init; } = new();

    [JsonPropertyName("total_ms")]
    public double TotalMs { get; init; }

    [JsonConstructor]
    public StartupStatsResponse() { }

    public StartupStatsResponse(List<StartupPhaseInfo> Phases, double TotalMs)
    {
        this.Phases = Phases ?? new List<StartupPhaseInfo>();
        this.TotalMs = TotalMs;
    }
}
