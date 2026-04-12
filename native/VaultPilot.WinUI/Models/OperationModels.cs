namespace VaultPilot.WinUI.Models;

public sealed record ImportResult(
    ulong Imported,
    ulong Skipped,
    IReadOnlyList<string> Errors);

public sealed record IndexStats(
    ulong Scanned,
    ulong Indexed,
    ulong Removed);
