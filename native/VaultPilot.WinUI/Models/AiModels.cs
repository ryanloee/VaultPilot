namespace VaultPilot.WinUI.Models;

public sealed record AnswerCitation(
    string NoteId,
    string Title,
    string Path,
    string Snippet);

public sealed record ThinkingTraceStep(
    string Title,
    string Detail);

public sealed record ThinkingTrace(
    string Summary,
    IReadOnlyList<ThinkingTraceStep> Steps);

public sealed record ContextStatus(
    string Model,
    ulong ContextWindowTokens,
    ulong LiveTokens,
    ulong ThresholdTokens,
    byte ThresholdPercent,
    double UsagePercent,
    string Source,
    bool Precise,
    ulong? LastRequestInputTokens,
    ulong? LastRequestOutputTokens);

public sealed record GroundedAnswer(
    string Answer,
    IReadOnlyList<AnswerCitation> Citations,
    NoteMeta? SavedNote,
    ThinkingTrace? ThinkingTrace,
    ContextStatus? ContextStatus,
    ulong UsedContextCount);

public sealed record AgentStatusEvent(
    string Stage,
    string Detail,
    string Timestamp);
