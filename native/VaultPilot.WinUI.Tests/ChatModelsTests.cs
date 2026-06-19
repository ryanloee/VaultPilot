using Xunit;
using System.Text.Json;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests;

public class ChatModelsTests
{
    [Fact]
    public void ChatTurn_RecordEquality()
    {
        var a = new ChatTurn(
            Id: "turn-1",
            Role: "user",
            Text: "Hello, world!",
            Citations: null,
            SavedNote: null,
            ThinkingTrace: null,
            Attachments: null,
            CreatedAt: "2025-01-01T00:00:00Z");

        var b = new ChatTurn(
            Id: "turn-1",
            Role: "user",
            Text: "Hello, world!",
            Citations: null,
            SavedNote: null,
            ThinkingTrace: null,
            Attachments: null,
            CreatedAt: "2025-01-01T00:00:00Z");

        Assert.Equal(a, b);
    }

    [Fact]
    public void ConversationTurn_RoleAndTextPreserved()
    {
        var turn = new ConversationTurn(Role: "assistant", Text: "I can help with that.");

        Assert.Equal("assistant", turn.Role);
        Assert.Equal("I can help with that.", turn.Text);
    }

    [Fact]
    public void ChatSession_JsonRoundTrip()
    {
        var session = new ChatSession(
            Id: "session-abc",
            Title: "Test Chat",
            Turns: new List<ChatTurn>
            {
                new("t1", "user", "Hi", null, null, null, null, null),
                new("t2", "assistant", "Hello!", null, null, null, null, null)
            },
            Summary: null,
            CreatedAt: "2025-06-01T10:00:00Z",
            UpdatedAt: "2025-06-01T10:05:00Z");

        var json = JsonSerializer.Serialize(session);
        var deserialized = JsonSerializer.Deserialize<ChatSession>(json);

        Assert.NotNull(deserialized);
        Assert.Equal(session.Id, deserialized.Id);
        Assert.Equal(session.Title, deserialized.Title);
        Assert.Equal(2, deserialized.Turns.Count);
        Assert.Equal("user", deserialized.Turns[0].Role);
        Assert.Equal("assistant", deserialized.Turns[1].Role);
    }

    [Fact]
    public void ChatAttachment_PropertiesPreserved()
    {
        var attachment = new ChatAttachment(
            Path: @"C:\docs\readme.md",
            Name: "readme.md");

        Assert.Equal(@"C:\docs\readme.md", attachment.Path);
        Assert.Equal("readme.md", attachment.Name);
    }

    [Fact]
    public void ConversationSummary_PropertiesPreserved()
    {
        var summary = new ConversationSummary(
            Text: "Discussion about API design",
            GeneratedAt: "2025-06-01T12:00:00Z",
            CoveredTurnCount: 10,
            CompressionCount: 2);

        Assert.Equal("Discussion about API design", summary.Text);
        Assert.Equal(10UL, summary.CoveredTurnCount);
        Assert.Equal(2UL, summary.CompressionCount);
    }

    [Fact]
    public void ChatState_EmptySessions()
    {
        var state = new ChatState(
            CurrentSessionId: "s1",
            Sessions: Array.Empty<ChatSession>());

        Assert.Equal("s1", state.CurrentSessionId);
        Assert.Empty(state.Sessions);
    }
}
