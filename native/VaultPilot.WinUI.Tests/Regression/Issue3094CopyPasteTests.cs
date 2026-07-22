using Xunit;
using VaultPilot.WinUI.Views;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression tests for issue #3094: WinUI file browser copy/paste (Ctrl+C/Ctrl+V).
/// The core logic for note duplication is in NotesView.CreateDuplicateMeta, which
/// generates a new NoteMeta with a fresh GUID ID, "(副本)" title suffix, reset
/// path, and current timestamps while preserving all other metadata.
/// </summary>
public class Issue3094CopyPasteTests
{
    private static NoteMeta MakeSourceNote()
    {
        return new NoteMeta
        {
            Id = "original-note-id",
            Title = "项目计划",
            Tags = new[] { "工作", "计划" },
            Keywords = new[] { "计划", "项目" },
            Platform = "obsidian",
            Board = "main",
            Kernel = "v1",
            Status = "active",
            CreatedAt = "2026-01-01T00:00:00Z",
            UpdatedAt = "2026-01-02T00:00:00Z",
            Source = "manual",
            Path = "notes/project-plan.md",
            Summary = "这是一个项目计划笔记",
        };
    }

    [Fact]
    public void Regression_3094_DuplicateHasFreshId()
    {
        var source = MakeSourceNote();
        var dup = NotesView.CreateDuplicateMeta(source);

        Assert.NotEmpty(dup.Id);
        Assert.NotEqual(source.Id, dup.Id);
    }

    [Fact]
    public void Regression_3094_DuplicateHasCopySuffix()
    {
        var source = MakeSourceNote();
        var dup = NotesView.CreateDuplicateMeta(source);

        Assert.Equal("项目计划 (副本)", dup.Title);
    }

    [Fact]
    public void Regression_3094_DuplicatePreservesTagsAndKeywords()
    {
        var source = MakeSourceNote();
        var dup = NotesView.CreateDuplicateMeta(source);

        Assert.Equal(source.Tags, dup.Tags);
        Assert.Equal(source.Keywords, dup.Keywords);
    }

    [Fact]
    public void Regression_3094_DuplicatePreservesMetadata()
    {
        var source = MakeSourceNote();
        var dup = NotesView.CreateDuplicateMeta(source);

        Assert.Equal(source.Platform, dup.Platform);
        Assert.Equal(source.Board, dup.Board);
        Assert.Equal(source.Kernel, dup.Kernel);
        Assert.Equal(source.Status, dup.Status);
        Assert.Equal(source.Source, dup.Source);
        Assert.Equal(source.Summary, dup.Summary);
    }

    [Fact]
    public void Regression_3094_DuplicateResetsPath()
    {
        var source = MakeSourceNote();
        var dup = NotesView.CreateDuplicateMeta(source);

        Assert.Equal(string.Empty, dup.Path);
    }

    [Fact]
    public void Regression_3094_DuplicateHasCurrentTimestamps()
    {
        var source = MakeSourceNote();
        var before = DateTimeOffset.UtcNow.AddSeconds(-1);

        var dup = NotesView.CreateDuplicateMeta(source);

        var after = DateTimeOffset.UtcNow.AddSeconds(1);
        Assert.True(dup.CreatedAt.Length > 0, "CreatedAt should be set");
        Assert.True(dup.UpdatedAt.Length > 0, "UpdatedAt should be set");
        Assert.NotEqual(source.CreatedAt, dup.CreatedAt);
        Assert.NotEqual(source.UpdatedAt, dup.UpdatedAt);
    }

    [Fact]
    public void Regression_3094_MultipleDuplicatesHaveUniqueIds()
    {
        var source = MakeSourceNote();
        var ids = new System.Collections.Generic.HashSet<string>();

        for (int i = 0; i < 10; i++)
        {
            var dup = NotesView.CreateDuplicateMeta(source);
            Assert.True(ids.Add(dup.Id), $"Duplicate ID collision at iteration {i}");
        }
    }
}
