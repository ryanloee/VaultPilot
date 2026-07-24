using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

/// <summary>
/// A single snapshot of a note at a point in time. Maps to Rust <c>NoteSnapshot</c>.
/// </summary>
public sealed record NoteSnapshot
{
    public string Id { get; init; } = string.Empty;
    public string NoteId { get; init; } = string.Empty;
    public string Body { get; init; } = string.Empty;
    public string Frontmatter { get; init; } = string.Empty;
    public string Source { get; init; } = string.Empty;
    public string CreatedAt { get; init; } = string.Empty;

    /// <summary>
    /// Human-readable summary of the snapshot for list display.
    /// </summary>
    [JsonIgnore]
    public string DisplayText
    {
        get
        {
            var date = CreatedAt.Length >= 10 ? CreatedAt[..10] : CreatedAt;
            var source = Source switch
            {
                "agent" => "🤖 Agent",
                "user" => "👤 用户",
                "sync" => "🔄 同步",
                _ => Source,
            };
            return $"{date}  {source}";
        }
    }

    [JsonConstructor]
    public NoteSnapshot() { }

    public NoteSnapshot(string Id, string NoteId, string Body, string Frontmatter, string Source, string CreatedAt)
    {
        this.Id = Id ?? string.Empty;
        this.NoteId = NoteId ?? string.Empty;
        this.Body = Body ?? string.Empty;
        this.Frontmatter = Frontmatter ?? string.Empty;
        this.Source = Source ?? string.Empty;
        this.CreatedAt = CreatedAt ?? string.Empty;
    }
}

/// <summary>
/// A single line in a diff hunk. Maps to Rust <c>DiffLine</c>.
/// </summary>
public sealed record DiffLine
{
    public string Context { get; init; } = string.Empty;
    public string Delete { get; init; } = string.Empty;
    public string Insert { get; init; } = string.Empty;

    [JsonIgnore]
    public string DisplayText => Context.Length > 0 ? Context : Delete.Length > 0 ? Delete : Insert;

    [JsonIgnore]
    public DiffLineKind Kind => Context.Length > 0 ? DiffLineKind.Context
        : Delete.Length > 0 ? DiffLineKind.Delete
        : DiffLineKind.Insert;

    [JsonConstructor]
    public DiffLine() { }
}

public enum DiffLineKind
{
    Context,
    Delete,
    Insert,
}

/// <summary>
/// A hunk of changes. Maps to Rust <c>DiffHunk</c>.
/// </summary>
public sealed record DiffHunk
{
    public int OldStart { get; init; }
    public int OldCount { get; init; }
    public int NewStart { get; init; }
    public int NewCount { get; init; }
    public IReadOnlyList<DiffLine> Lines { get; init; } = Array.Empty<DiffLine>();

    [JsonConstructor]
    public DiffHunk() { }
}

/// <summary>
/// Complete diff result. Maps to Rust <c>DiffResult</c>.
/// </summary>
public sealed record DiffResult
{
    public IReadOnlyList<DiffHunk> Hunks { get; init; } = Array.Empty<DiffHunk>();
    public int Additions { get; init; }
    public int Deletions { get; init; }

    [JsonConstructor]
    public DiffResult() { }
}
