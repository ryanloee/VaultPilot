using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

public sealed record NoteMeta
{
    public string Id { get; init; } = string.Empty;
    public string Title { get; init; } = string.Empty;
    public IReadOnlyList<string> Tags { get; init; } = Array.Empty<string>();
    public IReadOnlyList<string> Keywords { get; init; } = Array.Empty<string>();
    public string Platform { get; init; } = string.Empty;
    public string Board { get; init; } = string.Empty;
    public string Kernel { get; init; } = string.Empty;
    public string Status { get; init; } = string.Empty;
    public string CreatedAt { get; init; } = string.Empty;
    public string UpdatedAt { get; init; } = string.Empty;
    public string Source { get; init; } = string.Empty;
    public string Path { get; init; } = string.Empty;
    public string Summary { get; init; } = string.Empty;

    [JsonConstructor]
    public NoteMeta() { }

    public NoteMeta(
        string Id,
        string Title,
        IReadOnlyList<string> Tags,
        IReadOnlyList<string> Keywords,
        string Platform,
        string Board,
        string Kernel,
        string Status,
        string CreatedAt,
        string UpdatedAt,
        string Source,
        string Path,
        string Summary)
    {
        this.Id = Id ?? string.Empty;
        this.Title = Title ?? string.Empty;
        this.Tags = Tags ?? Array.Empty<string>();
        this.Keywords = Keywords ?? Array.Empty<string>();
        this.Platform = Platform ?? string.Empty;
        this.Board = Board ?? string.Empty;
        this.Kernel = Kernel ?? string.Empty;
        this.Status = Status ?? string.Empty;
        this.CreatedAt = CreatedAt ?? string.Empty;
        this.UpdatedAt = UpdatedAt ?? string.Empty;
        this.Source = Source ?? string.Empty;
        this.Path = Path ?? string.Empty;
        this.Summary = Summary ?? string.Empty;
    }
}

public sealed record NoteDocument
{
    public NoteMeta Meta { get; init; } = new NoteMeta();
    public string Body { get; init; } = string.Empty;

    [JsonConstructor]
    public NoteDocument() { }

    public NoteDocument(NoteMeta Meta, string Body)
    {
        this.Meta = Meta ?? new NoteMeta();
        this.Body = Body ?? string.Empty;
    }
}

/// <summary>
/// A note recommended as related to the current note, with a relevance score.
/// </summary>
public sealed record RelatedNote
{
    public NoteMeta Meta { get; init; } = new NoteMeta();
    public long Score { get; init; }
    public string? Snippet { get; init; }

    [JsonConstructor]
    public RelatedNote() { }

    public RelatedNote(NoteMeta Meta, long Score, string? Snippet)
    {
        this.Meta = Meta ?? new NoteMeta();
        this.Score = Score;
        this.Snippet = Snippet;
    }
}
