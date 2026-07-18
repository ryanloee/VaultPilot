using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

public sealed record AnswerCitation
{
    public string NoteId { get; init; } = string.Empty;
    public string Title { get; init; } = string.Empty;
    public string Path { get; init; } = string.Empty;
    public string Snippet { get; init; } = string.Empty;

    [JsonConstructor]
    public AnswerCitation() { }

    public AnswerCitation(string NoteId, string Title, string Path, string Snippet)
    {
        this.NoteId = NoteId ?? string.Empty;
        this.Title = Title ?? string.Empty;
        this.Path = Path ?? string.Empty;
        this.Snippet = Snippet ?? string.Empty;
    }
}

public sealed record ThinkingTraceStep
{
    public string Title { get; init; } = string.Empty;
    public string Detail { get; init; } = string.Empty;

    [JsonConstructor]
    public ThinkingTraceStep() { }

    public ThinkingTraceStep(string Title, string Detail)
    {
        this.Title = Title ?? string.Empty;
        this.Detail = Detail ?? string.Empty;
    }
}

public sealed record ThinkingTrace
{
    public string Summary { get; init; } = string.Empty;
    public IReadOnlyList<ThinkingTraceStep> Steps { get; init; } = Array.Empty<ThinkingTraceStep>();

    [JsonConstructor]
    public ThinkingTrace() { }

    public ThinkingTrace(string Summary, IReadOnlyList<ThinkingTraceStep> Steps)
    {
        this.Summary = Summary ?? string.Empty;
        this.Steps = Steps ?? Array.Empty<ThinkingTraceStep>();
    }
}

public sealed record ContextStatus
{
    public string Model { get; init; } = string.Empty;
    public ulong ContextWindowTokens { get; init; }
    public ulong LiveTokens { get; init; }
    public ulong ThresholdTokens { get; init; }
    public byte ThresholdPercent { get; init; }
    public double UsagePercent { get; init; }
    public string Source { get; init; } = string.Empty;
    public bool Precise { get; init; }
    public ulong? LastRequestInputTokens { get; init; }
    public ulong? LastRequestOutputTokens { get; init; }

    [JsonConstructor]
    public ContextStatus() { }

    public ContextStatus(
        string Model,
        ulong ContextWindowTokens,
        ulong LiveTokens,
        ulong ThresholdTokens,
        byte ThresholdPercent,
        double UsagePercent,
        string Source,
        bool Precise,
        ulong? LastRequestInputTokens,
        ulong? LastRequestOutputTokens)
    {
        this.Model = Model ?? string.Empty;
        this.ContextWindowTokens = ContextWindowTokens;
        this.LiveTokens = LiveTokens;
        this.ThresholdTokens = ThresholdTokens;
        this.ThresholdPercent = ThresholdPercent;
        this.UsagePercent = UsagePercent;
        this.Source = Source ?? string.Empty;
        this.Precise = Precise;
        this.LastRequestInputTokens = LastRequestInputTokens;
        this.LastRequestOutputTokens = LastRequestOutputTokens;
    }
}

public sealed record GroundedAnswer
{
    public string Answer { get; init; } = string.Empty;
    public IReadOnlyList<AnswerCitation> Citations { get; init; } = Array.Empty<AnswerCitation>();
    public NoteMeta? SavedNote { get; init; }
    public ThinkingTrace? ThinkingTrace { get; init; }
    public ContextStatus? ContextStatus { get; init; }
    public ulong UsedContextCount { get; init; }

    [JsonConstructor]
    public GroundedAnswer() { }

    public GroundedAnswer(
        string Answer,
        IReadOnlyList<AnswerCitation> Citations,
        NoteMeta? SavedNote,
        ThinkingTrace? ThinkingTrace,
        ContextStatus? ContextStatus,
        ulong UsedContextCount)
    {
        this.Answer = Answer ?? string.Empty;
        this.Citations = Citations ?? Array.Empty<AnswerCitation>();
        this.SavedNote = SavedNote;
        this.ThinkingTrace = ThinkingTrace;
        this.ContextStatus = ContextStatus;
        this.UsedContextCount = UsedContextCount;
    }
}

public sealed record AgentStatusEvent
{
    public string Stage { get; init; } = string.Empty;
    public string Detail { get; init; } = string.Empty;
    public string Timestamp { get; init; } = string.Empty;

    // Agent Mode fields (populated for agent events)
    public int? Step { get; init; }
    public string? Tool { get; init; }
    public string? Args { get; init; }
    public string? ResultPreview { get; init; }
    public bool? IsError { get; init; }
    public int? StepsUsed { get; init; }
    public ulong? TokensUsed { get; init; }
    // #3109: Agent health-tracker emits a remediation hint alongside
    // stage="unhealthyDetected". Surfaced to the user so they can decide
    // whether to reset the agent context.
    public string? Suggestion { get; init; }

    [JsonConstructor]
    public AgentStatusEvent() { }

    public AgentStatusEvent(string Stage, string Detail, string Timestamp)
    {
        this.Stage = Stage ?? string.Empty;
        this.Detail = Detail ?? string.Empty;
        this.Timestamp = Timestamp ?? string.Empty;
    }
}
