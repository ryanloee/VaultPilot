using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

public sealed record ChatAttachment(string Path, string Name);

public sealed record ConversationTurn(string Role, string Text);

public sealed record ChatTurn(
    string Id,
    string Role,
    string Text,
    IReadOnlyList<AnswerCitation>? Citations,
    NoteMeta? SavedNote,
    ThinkingTrace? ThinkingTrace,
    IReadOnlyList<ChatAttachment>? Attachments,
    string? CreatedAt);

public sealed record ConversationSummary(
    string Text,
    string GeneratedAt,
    ulong CoveredTurnCount,
    ulong CompressionCount);

public sealed record ChatSession
{
    public string Id { get; init; } = string.Empty;
    public string Title { get; init; } = string.Empty;
    public IReadOnlyList<ChatTurn> Turns { get; init; } = Array.Empty<ChatTurn>();
    public ConversationSummary? Summary { get; init; }
    public string CreatedAt { get; init; } = string.Empty;
    public string UpdatedAt { get; init; } = string.Empty;

    [JsonConstructor]
    public ChatSession() { }

    /// <summary>
    /// Positional constructor for backward compatibility with existing code.
    /// </summary>
    public ChatSession(
        string Id,
        string Title,
        IReadOnlyList<ChatTurn> Turns,
        ConversationSummary? Summary,
        string CreatedAt,
        string UpdatedAt)
    {
        this.Id = Id ?? string.Empty;
        this.Title = Title ?? string.Empty;
        this.Turns = Turns ?? Array.Empty<ChatTurn>();
        this.Summary = Summary;
        this.CreatedAt = CreatedAt ?? string.Empty;
        this.UpdatedAt = UpdatedAt ?? string.Empty;
    }
}

public sealed record ChatState
{
    public string CurrentSessionId { get; init; } = string.Empty;
    public IReadOnlyList<ChatSession> Sessions { get; init; } = Array.Empty<ChatSession>();

    [JsonConstructor]
    public ChatState() { }

    /// <summary>
    /// Positional constructor for backward compatibility with existing code.
    /// </summary>
    public ChatState(
        string CurrentSessionId,
        IReadOnlyList<ChatSession> Sessions)
    {
        this.CurrentSessionId = CurrentSessionId ?? string.Empty;
        this.Sessions = Sessions ?? Array.Empty<ChatSession>();
    }
}
