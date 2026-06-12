using System.Text.Json;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests;

public class AiModelsTests
{
    [Fact]
    public void AnswerCitation_PropertiesPreserved()
    {
        var citation = new AnswerCitation(
            NoteId: "note-123",
            Title: "My Note",
            Path: "/vault/notes/my-note.md",
            Snippet: "This is a relevant snippet.");

        Assert.Equal("note-123", citation.NoteId);
        Assert.Equal("My Note", citation.Title);
        Assert.Equal("/vault/notes/my-note.md", citation.Path);
        Assert.Equal("This is a relevant snippet.", citation.Snippet);
    }

    [Fact]
    public void ThinkingTrace_WithSteps()
    {
        var trace = new ThinkingTrace(
            Summary: "Analyzed 3 documents",
            Steps: new List<ThinkingTraceStep>
            {
                new("Search", "Found relevant documents"),
                new("Analyze", "Extracted key information"),
                new("Synthesize", "Generated answer")
            });

        Assert.Equal("Analyzed 3 documents", trace.Summary);
        Assert.Equal(3, trace.Steps.Count);
        Assert.Equal("Search", trace.Steps[0].Title);
        Assert.Equal("Synthesize", trace.Steps[2].Title);
    }

    [Fact]
    public void ContextStatus_PropertiesPreserved()
    {
        var status = new ContextStatus(
            Model: "gpt-4",
            ContextWindowTokens: 128_000,
            LiveTokens: 50_000,
            ThresholdTokens: 100_000,
            ThresholdPercent: 80,
            UsagePercent: 39.06,
            Source: "tiktoken",
            Precise: true,
            LastRequestInputTokens: 1_000,
            LastRequestOutputTokens: 500);

        Assert.Equal("gpt-4", status.Model);
        Assert.Equal(128_000UL, status.ContextWindowTokens);
        Assert.Equal(50_000UL, status.LiveTokens);
        Assert.Equal(80, status.ThresholdPercent);
        Assert.True(status.Precise);
        Assert.Equal(1000UL, status.LastRequestInputTokens);
    }

    [Fact]
    public void GroundedAnswer_WithCitations_JsonRoundTrip()
    {
        var answer = new GroundedAnswer(
            Answer: "The answer is 42.",
            Citations: new List<AnswerCitation>
            {
                new("n1", "Guide", "/guide.md", "The answer section")
            },
            SavedNote: null,
            ThinkingTrace: null,
            ContextStatus: null,
            UsedContextCount: 1);

        var json = JsonSerializer.Serialize(answer);
        var deserialized = JsonSerializer.Deserialize<GroundedAnswer>(json);

        Assert.NotNull(deserialized);
        Assert.Equal(answer.Answer, deserialized.Answer);
        Assert.Single(deserialized.Citations);
        Assert.Equal("Guide", deserialized.Citations[0].Title);
        Assert.Equal(1UL, deserialized.UsedContextCount);
    }

    [Fact]
    public void AgentStatusEvent_PropertiesPreserved()
    {
        var evt = new AgentStatusEvent(
            Stage: "processing",
            Detail: "Analyzing vault contents",
            Timestamp: "2025-06-01T10:30:00Z");

        Assert.Equal("processing", evt.Stage);
        Assert.Equal("Analyzing vault contents", evt.Detail);
        Assert.Equal("2025-06-01T10:30:00Z", evt.Timestamp);
    }
}
