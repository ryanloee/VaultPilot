using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

public sealed record ChatAttachment
{
    public string Path { get; init; } = string.Empty;
    public string Name { get; init; } = string.Empty;

    [JsonConstructor]
    public ChatAttachment() { }

    public ChatAttachment(string Path, string Name)
    {
        this.Path = Path ?? string.Empty;
        this.Name = Name ?? string.Empty;
    }
}

public sealed record ConversationTurn
{
    public string Role { get; init; } = string.Empty;
    public string Text { get; init; } = string.Empty;

    [JsonConstructor]
    public ConversationTurn() { }

    public ConversationTurn(string Role, string Text)
    {
        this.Role = Role ?? string.Empty;
        this.Text = Text ?? string.Empty;
    }
}

public sealed record ChatTurn
{
    public string Id { get; init; } = string.Empty;
    public string Role { get; init; } = string.Empty;
    public string Text { get; init; } = string.Empty;
    public IReadOnlyList<AnswerCitation>? Citations { get; init; }
    public NoteMeta? SavedNote { get; init; }
    public ThinkingTrace? ThinkingTrace { get; init; }
    public IReadOnlyList<ChatAttachment>? Attachments { get; init; }
    public string? CreatedAt { get; init; }

    [JsonConstructor]
    public ChatTurn() { }

    public ChatTurn(
        string Id,
        string Role,
        string Text,
        IReadOnlyList<AnswerCitation>? Citations,
        NoteMeta? SavedNote,
        ThinkingTrace? ThinkingTrace,
        IReadOnlyList<ChatAttachment>? Attachments,
        string? CreatedAt)
    {
        this.Id = Id ?? string.Empty;
        this.Role = Role ?? string.Empty;
        this.Text = Text ?? string.Empty;
        this.Citations = Citations;
        this.SavedNote = SavedNote;
        this.ThinkingTrace = ThinkingTrace;
        this.Attachments = Attachments;
        this.CreatedAt = CreatedAt;
    }
}

public sealed record ConversationSummary
{
    public string Text { get; init; } = string.Empty;
    public string GeneratedAt { get; init; } = string.Empty;
    public ulong CoveredTurnCount { get; init; }
    public ulong CompressionCount { get; init; }

    [JsonConstructor]
    public ConversationSummary() { }

    public ConversationSummary(string Text, string GeneratedAt, ulong CoveredTurnCount, ulong CompressionCount)
    {
        this.Text = Text ?? string.Empty;
        this.GeneratedAt = GeneratedAt ?? string.Empty;
        this.CoveredTurnCount = CoveredTurnCount;
        this.CompressionCount = CompressionCount;
    }
}

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
