using Xunit;
using System.Text.Json;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests;

public class NoteModelsTests
{
    [Fact]
    public void NoteMeta_PropertiesPreserved()
    {
        var meta = new NoteMeta(
            Id: "note-001",
            Title: "Getting Started",
            Tags: new List<string> { "guide", "beginner" },
            Keywords: new List<string> { "setup", "install" },
            Platform: "obsidian",
            Board: "main",
            Kernel: "default",
            Status: "active",
            CreatedAt: "2025-01-01T00:00:00Z",
            UpdatedAt: "2025-06-01T00:00:00Z",
            Source: "manual",
            Path: "/vault/getting-started.md",
            Summary: "A guide to getting started with VaultPilot");

        Assert.Equal("note-001", meta.Id);
        Assert.Equal("Getting Started", meta.Title);
        Assert.Equal(2, meta.Tags.Count);
        Assert.Contains("guide", meta.Tags);
        Assert.Equal("obsidian", meta.Platform);
        Assert.Equal("active", meta.Status);
    }

    [Fact]
    public void NoteMeta_JsonRoundTrip()
    {
        var meta = new NoteMeta(
            Id: "note-002",
            Title: "API Reference",
            Tags: new List<string> { "api", "reference" },
            Keywords: new List<string> { "rest", "endpoints" },
            Platform: "obsidian",
            Board: "main",
            Kernel: "default",
            Status: "published",
            CreatedAt: "2025-03-15T00:00:00Z",
            UpdatedAt: "2025-06-01T00:00:00Z",
            Source: "import",
            Path: "/vault/api-reference.md",
            Summary: "Complete API reference documentation");

        var json = JsonSerializer.Serialize(meta);
        var deserialized = JsonSerializer.Deserialize<NoteMeta>(json);

        Assert.NotNull(deserialized);
        Assert.Equal(meta.Id, deserialized.Id);
        Assert.Equal(meta.Title, deserialized.Title);
        Assert.Equal(meta.Tags.Count, deserialized.Tags.Count);
        Assert.Equal(meta.Path, deserialized.Path);
    }

    [Fact]
    public void NoteDocument_ContainsMetaAndBody()
    {
        var meta = new NoteMeta(
            Id: "note-003",
            Title: "Test Note",
            Tags: Array.Empty<string>(),
            Keywords: Array.Empty<string>(),
            Platform: "obsidian",
            Board: "main",
            Kernel: "default",
            Status: "draft",
            CreatedAt: "2025-06-01T00:00:00Z",
            UpdatedAt: "2025-06-01T00:00:00Z",
            Source: "manual",
            Path: "/vault/test.md",
            Summary: "A test note");

        var doc = new NoteDocument(Meta: meta, Body: "# Test\n\nThis is the body content.");

        Assert.Equal(meta, doc.Meta);
        Assert.Contains("# Test", doc.Body);
        Assert.Contains("body content", doc.Body);
    }
}
