using Xunit;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests;

/// <summary>
/// Regression tests for #3595: AiActionType enum members ExportDocument,
/// GenerateWidget, and MeetingNotes must be handled in all switch expressions.
/// </summary>
public class Issue3595AiActionTypeExhaustivenessTests
{
    [Fact]
    public void ExportDocument_HasLabel()
    {
        Assert.Equal("导出文档", AiActionType.ExportDocument.Label());
    }

    [Fact]
    public void GenerateWidget_HasLabel()
    {
        Assert.Equal("交互组件", AiActionType.GenerateWidget.Label());
    }

    [Fact]
    public void MeetingNotes_HasLabel()
    {
        Assert.Equal("会议笔记", AiActionType.MeetingNotes.Label());
    }

    [Fact]
    public void AllActionTypes_HaveDistinctLabels()
    {
        // Verify that every enum member has a non-fallback label and no duplicates.
        var labels = new HashSet<string>();
        foreach (AiActionType type in Enum.GetValues<AiActionType>())
        {
            var label = type.Label();
            Assert.NotEqual("未知操作", label);
            Assert.True(labels.Add(label), $"Duplicate label for {type}: '{label}'");
        }
    }

    [Fact]
    public void AllActionTypes_HaveJsonPropertyName()
    {
        // Verify every enum member has a JsonPropertyName attribute.
        foreach (AiActionType type in Enum.GetValues<AiActionType>())
        {
            var field = typeof(AiActionType).GetField(type.ToString());
            Assert.NotNull(field);
            var attr = field!.GetCustomAttributes(
                typeof(System.Text.Json.Serialization.JsonPropertyNameAttribute), false);
            Assert.Single(attr);
        }
    }

    [Fact]
    public void MeetingNotes_IsInEnum()
    {
        // Verify MeetingNotes is in the enum (previously missing — #3588).
        Assert.Contains(AiActionType.MeetingNotes, Enum.GetValues<AiActionType>());
        Assert.Equal("meetingNotes", typeof(AiActionType)
            .GetField(nameof(AiActionType.MeetingNotes))!
            .GetCustomAttributes(typeof(System.Text.Json.Serialization.JsonPropertyNameAttribute), false)
            .Cast<System.Text.Json.Serialization.JsonNamePropertyAttribute>()
            .First().Name);
    }

    [Fact]
    public void ExportDocument_IsInEnum()
    {
        Assert.Contains(AiActionType.ExportDocument, Enum.GetValues<AiActionType>());
    }

    [Fact]
    public void GenerateWidget_IsInEnum()
    {
        Assert.Contains(AiActionType.GenerateWidget, Enum.GetValues<AiActionType>());
    }
}