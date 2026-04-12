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

public sealed record ChatSession(
    string Id,
    string Title,
    IReadOnlyList<ChatTurn> Turns,
    ConversationSummary? Summary,
    string CreatedAt,
    string UpdatedAt);

public sealed record ChatState(
    string CurrentSessionId,
    IReadOnlyList<ChatSession> Sessions);
