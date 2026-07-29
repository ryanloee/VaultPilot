namespace VaultPilot.WinUI.Models;

/// <summary>
/// Data item for the virtualized ItemsRepeater message list (#3581).
/// Each instance represents one turn in the chat thread. ItemsRepeater
/// only materializes the visual tree for viewport-visible items;
/// off-screen items have no UI overhead.
/// </summary>
public sealed record MessageItem
{
    public string TurnId { get; init; } = string.Empty;
    public string Role { get; init; } = string.Empty;
    public string Text { get; init; } = string.Empty;
    public string Author { get; init; } = string.Empty;
    public string? CreatedAt { get; init; }
    public IReadOnlyList<AnswerCitation>? Citations { get; init; }
    public IReadOnlyList<ChatAttachment>? Attachments { get; init; }
    public ThinkingTrace? ThinkingTrace { get; init; }
    public NoteMeta? SavedNote { get; init; }
    public string Source { get; init; } = string.Empty;
}
