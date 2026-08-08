using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

/// <summary>
/// A single crash-recovery snapshot (vault-EXTERNAL, src/recovery.rs) as
/// returned by the backend agent's <c>recoveryList</c> JSON-RPC method
/// (issue #3960). Cheap list form — deliberately has no <c>content</c> field;
/// fetch the full content via <c>recoveryShow</c>.
/// </summary>
public sealed record RecoverySnapshotInfo
{
    public string Id { get; init; } = string.Empty;
    public string NotePath { get; init; } = string.Empty;
    public string Title { get; init; } = string.Empty;
    public long ContentSize { get; init; }
    public string CreatedAt { get; init; } = string.Empty;

    [JsonConstructor]
    public RecoverySnapshotInfo() { }

    public RecoverySnapshotInfo(string Id, string NotePath, string Title, long ContentSize, string CreatedAt)
    {
        this.Id = Id ?? string.Empty;
        this.NotePath = NotePath ?? string.Empty;
        this.Title = Title ?? string.Empty;
        this.ContentSize = ContentSize;
        this.CreatedAt = CreatedAt ?? string.Empty;
    }
}

/// <summary>
/// Full crash-recovery snapshot content as returned by the backend agent's
/// <c>recoveryShow</c> JSON-RPC method (issue #3960).
/// </summary>
public sealed record RecoverySnapshotDetail
{
    public string Id { get; init; } = string.Empty;
    public string NotePath { get; init; } = string.Empty;
    public string Title { get; init; } = string.Empty;
    public string Content { get; init; } = string.Empty;
    public long ContentSize { get; init; }
    public string CreatedAt { get; init; } = string.Empty;

    [JsonConstructor]
    public RecoverySnapshotDetail() { }

    public RecoverySnapshotDetail(
        string Id,
        string NotePath,
        string Title,
        string Content,
        long ContentSize,
        string CreatedAt)
    {
        this.Id = Id ?? string.Empty;
        this.NotePath = NotePath ?? string.Empty;
        this.Title = Title ?? string.Empty;
        this.Content = Content ?? string.Empty;
        this.ContentSize = ContentSize;
        this.CreatedAt = CreatedAt ?? string.Empty;
    }
}

/// <summary>
/// Result of the backend agent's <c>recoveryRestore</c> JSON-RPC method
/// (issue #3960): writes the snapshot content back into the vault at its
/// original (vault-relative) note path.
/// </summary>
public sealed record RecoveryRestoreResult
{
    public bool Ok { get; init; }
    public string NotePath { get; init; } = string.Empty;
    public long BytesWritten { get; init; }

    [JsonConstructor]
    public RecoveryRestoreResult() { }

    public RecoveryRestoreResult(bool Ok, string NotePath, long BytesWritten)
    {
        this.Ok = Ok;
        this.NotePath = NotePath ?? string.Empty;
        this.BytesWritten = BytesWritten;
    }
}

/// <summary>
/// Result of the backend agent's <c>recoveryDelete</c> JSON-RPC method
/// (issue #3960).
/// </summary>
public sealed record RecoveryDeleteResult
{
    public bool Ok { get; init; }

    [JsonConstructor]
    public RecoveryDeleteResult() { }

    public RecoveryDeleteResult(bool Ok)
    {
        this.Ok = Ok;
    }
}
