using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

public sealed record ImportResult
{
    public ulong Imported { get; init; }
    public ulong Skipped { get; init; }
    public IReadOnlyList<string> Errors { get; init; } = Array.Empty<string>();

    [JsonConstructor]
    public ImportResult() { }

    public ImportResult(ulong Imported, ulong Skipped, IReadOnlyList<string> Errors)
    {
        this.Imported = Imported;
        this.Skipped = Skipped;
        this.Errors = Errors ?? Array.Empty<string>();
    }
}

public sealed record IndexStats
{
    public ulong Scanned { get; init; }
    public ulong Indexed { get; init; }
    public ulong Removed { get; init; }

    [JsonConstructor]
    public IndexStats() { }

    public IndexStats(ulong Scanned, ulong Indexed, ulong Removed)
    {
        this.Scanned = Scanned;
        this.Indexed = Indexed;
        this.Removed = Removed;
    }
}
