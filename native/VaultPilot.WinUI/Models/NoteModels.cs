namespace VaultPilot.WinUI.Models;

public sealed record NoteMeta(
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
    string Summary);

public sealed record NoteDocument(NoteMeta Meta, string Body);
