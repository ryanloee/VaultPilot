using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

/// <summary>
/// Available AI quick action types for the global command palette.
/// </summary>
[JsonConverter(typeof(JsonStringEnumConverter))]
public enum AiActionType
{
    [JsonPropertyName("summarize")]
    Summarize,
    [JsonPropertyName("rewrite")]
    Rewrite,
    [JsonPropertyName("translate")]
    Translate,
    [JsonPropertyName("explain")]
    Explain,
    [JsonPropertyName("continueWriting")]
    ContinueWriting,
    [JsonPropertyName("extractTodos")]
    ExtractTodos,
    [JsonPropertyName("findRelatedNotes")]
    FindRelatedNotes,
    [JsonPropertyName("cleanUp")]
    CleanUp,
    [JsonPropertyName("generateOutline")]
    GenerateOutline,
    [JsonPropertyName("editNote")]
    EditNote,
    [JsonPropertyName("summarizeUrl")]
    SummarizeUrl,
    [JsonPropertyName("brainstorm")]
    Brainstorm,
    [JsonPropertyName("reviewNote")]
    ReviewNote,
    [JsonPropertyName("synthesizeWiki")]
    SynthesizeWiki,
    /// <summary>Workspace-wide Q&amp;A with inline citations (#3188).</summary>
    [JsonPropertyName("workspaceQuery")]
    WorkspaceQuery,
    /// <summary>Transcribe audio to text via OpenAI Whisper API (#3256).</summary>
    [JsonPropertyName("transcribeAudio")]
    TranscribeAudio,
    /// <summary>AI-powered context-aware note link suggestion (#3271).</summary>
    [JsonPropertyName("suggestLinks")]
    SuggestLinks,
    /// <summary>Ad-hoc multi-note synthesis (#3270).</summary>
    [JsonPropertyName("synthesizeNotes")]
    SynthesizeNotes,
    /// <summary>Export note content to a structured file format (XLSX/DOCX/HTML/PPTX).</summary>
    [JsonPropertyName("exportDocument")]
    ExportDocument,
    /// <summary>Generate interactive HTML/JS widget block (#3042).</summary>
    [JsonPropertyName("generateWidget")]
    GenerateWidget,
    /// <summary>AI meeting notes transcription with speaker diarization, summary, and action items (#3588).</summary>
    [JsonPropertyName("meetingNotes")]
    MeetingNotes,
}

/// <summary>
/// Request payload for executing an AI quick action.
/// </summary>
public sealed record AiActionRequest
{
    /// <summary>The action type to perform.</summary>
    [JsonPropertyName("action")]
    public AiActionType Action { get; init; }

    /// <summary>The text content to operate on.</summary>
    [JsonPropertyName("text")]
    public string Text { get; init; } = string.Empty;

    /// <summary>Target language for translation.</summary>
    [JsonPropertyName("targetLanguage")]
    public string? TargetLanguage { get; set; }

    /// <summary>Target tone for rewrite (formal, concise, vivid).</summary>
    [JsonPropertyName("tone")]
    public string? Tone { get; set; }

    /// <summary>Note ID for context (e.g., findRelatedNotes).</summary>
    [JsonPropertyName("noteId")]
    public string? NoteId { get; set; }

    /// <summary>Edit instruction for Composer (EditNote action).</summary>
    [JsonPropertyName("instruction")]
    public string? Instruction { get; set; }

    /// <summary>Model override.</summary>
    [JsonPropertyName("model")]
    public string? Model { get; init; }

    [JsonConstructor]
    public AiActionRequest() { }

    public AiActionRequest(AiActionType action, string text)
    {
        Action = action;
        Text = text ?? string.Empty;
    }
}

/// <summary>
/// Token usage statistics returned with an AI action result.
/// </summary>
public sealed record AiActionUsage
{
    [JsonPropertyName("promptTokens")]
    public ulong PromptTokens { get; init; }

    [JsonPropertyName("completionTokens")]
    public ulong CompletionTokens { get; init; }

    [JsonPropertyName("totalTokens")]
    public ulong TotalTokens { get; init; }

    [JsonConstructor]
    public AiActionUsage() { }
}

/// <summary>
/// Result of an executed AI quick action.
/// </summary>
public sealed record AiActionResult
{
    /// <summary>The resulting text after the action was applied.</summary>
    [JsonPropertyName("result")]
    public string Result { get; init; } = string.Empty;

    /// <summary>Token usage statistics.</summary>
    [JsonPropertyName("usage")]
    public AiActionUsage Usage { get; init; } = new();

    /// <summary>Error message if the action failed.</summary>
    [JsonPropertyName("error")]
    public string? Error { get; init; }

    /// <summary>True if the action completed successfully.</summary>
    public bool IsSuccess => string.IsNullOrEmpty(Error);

    [JsonConstructor]
    public AiActionResult() { }
}

/// <summary>
/// Action type metadata returned from listAiActions.
/// </summary>
public sealed record AiActionInfo
{
    /// <summary>Action type identifier.</summary>
    [JsonPropertyName("id")]
    public string Id { get; init; } = string.Empty;

    /// <summary>Human-readable Chinese label.</summary>
    [JsonPropertyName("label")]
    public string Label { get; init; } = string.Empty;

    /// <summary>Action type enum value.</summary>
    [JsonPropertyName("actionType")]
    public AiActionType ActionType { get; init; }

    [JsonConstructor]
    public AiActionInfo() { }
}

/// <summary>
/// Extension methods for <see cref="AiActionType"/>.
/// </summary>
public static class AiActionTypeExtensions
{
    /// <summary>
    /// Returns the human-readable Chinese label for an AI action type.
    /// </summary>
    public static string Label(this AiActionType type) => type switch
    {
        AiActionType.Summarize => "总结要点",
        AiActionType.Rewrite => "改写润色",
        AiActionType.Translate => "翻译",
        AiActionType.Explain => "解释说明",
        AiActionType.ContinueWriting => "续写",
        AiActionType.ExtractTodos => "提取待办",
        AiActionType.FindRelatedNotes => "关联笔记",
        AiActionType.CleanUp => "整理",
        AiActionType.GenerateOutline => "大纲生成",
        AiActionType.EditNote => "编辑笔记",
        AiActionType.SummarizeUrl => "链接摘要",
        AiActionType.Brainstorm => "头脑风暴",
        AiActionType.ReviewNote => "审阅笔记",
        AiActionType.SynthesizeWiki => "综合维基",
        AiActionType.WorkspaceQuery => "工作区问答",
        AiActionType.TranscribeAudio => "音频转写",
        AiActionType.SuggestLinks => "智能推荐链接",
        AiActionType.SynthesizeNotes => "笔记综合",
        AiActionType.ExportDocument => "导出文档",
        AiActionType.GenerateWidget => "交互组件",
        AiActionType.MeetingNotes => "会议笔记",
        _ => "未知操作"
    };
}
