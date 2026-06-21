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

    // ── MessageV2 tests (#1239) ──────────────────────────────────────

    [Fact]
    public void MessageV2_DefaultValues()
    {
        var msg = new MessageV2();
        Assert.Equal(string.Empty, msg.Id);
        Assert.Equal(MessageV2Role.User, msg.Role);
        Assert.Equal(string.Empty, msg.Content);
        Assert.Empty(msg.Attachments);
        Assert.Equal(string.Empty, msg.Metadata.Model);
        Assert.Equal(0UL, msg.Metadata.Tokens);
        Assert.Empty(msg.Extensions);
        Assert.Empty(msg.Validate());
    }

    [Fact]
    public void MessageV2_JsonRoundTrip_TextOnly()
    {
        var msg = new MessageV2
        {
            Id = "550e8400-e29b-41d4-a716-446655440000",
            Role = MessageV2Role.User,
            Content = "Hello **world**",
        };

        var json = JsonSerializer.Serialize(msg);
        var parsed = JsonSerializer.Deserialize<MessageV2>(json);

        Assert.NotNull(parsed);
        Assert.Equal(msg.Id, parsed.Id);
        Assert.Equal(MessageV2Role.User, parsed.Role);
        Assert.Equal("Hello **world**", parsed.Content);
        Assert.Empty(parsed.Attachments);
    }

    [Fact]
    public void MessageV2_JsonRoundTrip_WithAttachment()
    {
        var msg = new MessageV2
        {
            Id = "a1",
            Role = MessageV2Role.Assistant,
            Content = "Here is the image:",
            Attachments = new List<MessageV2Attachment>
            {
                new()
                {
                    Type = MessageV2AttachmentType.Image,
                    Url = "local://vault/images/chart.png",
                    Mime = "image/png",
                },
            },
            Metadata = new MessageV2Metadata
            {
                Model = "deepseek-v4",
                Tokens = 42,
            },
        };

        var json = JsonSerializer.Serialize(msg);
        var parsed = JsonSerializer.Deserialize<MessageV2>(json);

        Assert.NotNull(parsed);
        Assert.Single(parsed.Attachments);
        Assert.Equal(MessageV2AttachmentType.Image, parsed.Attachments[0].Type);
        Assert.Equal("local://vault/images/chart.png", parsed.Attachments[0].Url);
        Assert.Equal("deepseek-v4", parsed.Metadata.Model);
        Assert.Equal(42UL, parsed.Metadata.Tokens);
    }

    [Fact]
    public void MessageV2_JsonRoundTrip_SystemRole()
    {
        var msg = new MessageV2
        {
            Role = MessageV2Role.System,
            Content = "You are a helpful assistant.",
        };

        var json = JsonSerializer.Serialize(msg);
        var parsed = JsonSerializer.Deserialize<MessageV2>(json);

        Assert.NotNull(parsed);
        Assert.Equal(MessageV2Role.System, parsed.Role);
    }

    [Fact]
    public void MessageV2Attachment_RejectsNonLocalUrl()
    {
        var bad = new MessageV2Attachment { Url = "https://evil.com/payload" };
        Assert.NotNull(bad.ValidateUrl());
        Assert.Contains("local://", bad.ValidateUrl()!);

        var good = new MessageV2Attachment { Url = "local://vault/doc.pdf" };
        Assert.Null(good.ValidateUrl());
    }

    [Fact]
    public void MessageV2_Validate_CatchesBadAttachmentUrls()
    {
        var msg = new MessageV2
        {
            Attachments = new List<MessageV2Attachment>
            {
                new() { Url = "local://ok.png" },
                new() { Url = "/etc/passwd" },
            },
        };

        var errors = msg.Validate();
        Assert.Single(errors);
        Assert.Contains("local://", errors[0]);
    }

    [Fact]
    public void MessageV2_JsonRoundTrip_WithExtensions()
    {
        var msg = new MessageV2
        {
            Id = "ext1",
            Role = MessageV2Role.User,
            Content = "test",
            Extensions = new Dictionary<string, JsonElement>
            {
                ["plugin_x"] = JsonSerializer.SerializeToElement(new { enabled = true }),
            },
        };

        var json = JsonSerializer.Serialize(msg);
        var parsed = JsonSerializer.Deserialize<MessageV2>(json);

        Assert.NotNull(parsed);
        Assert.Single(parsed.Extensions);
        Assert.True(parsed.Extensions.ContainsKey("plugin_x"));
    }
}
