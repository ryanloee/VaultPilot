using Xunit;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests;

public class AiActionModelsTests
{
    [Fact]
    public void AiActionTypeLabel_AllTypesHaveLabels()
    {
        Assert.Equal("总结要点", AiActionType.Summarize.Label());
        Assert.Equal("改写润色", AiActionType.Rewrite.Label());
        Assert.Equal("翻译", AiActionType.Translate.Label());
        Assert.Equal("解释说明", AiActionType.Explain.Label());
        Assert.Equal("续写", AiActionType.ContinueWriting.Label());
        Assert.Equal("提取待办", AiActionType.ExtractTodos.Label());
        Assert.Equal("关联笔记", AiActionType.FindRelatedNotes.Label());
    }

    [Fact]
    public void AiActionTypeLabel_UnknownTypeReturnsFallback()
    {
        var unknown = (AiActionType)99;
        Assert.Equal("未知操作", unknown.Label());
    }

    [Fact]
    public void AiActionRequest_DefaultConstructorWorks()
    {
        var request = new AiActionRequest();
        Assert.Equal(default, request.Action);
        Assert.Equal(string.Empty, request.Text);
        Assert.Null(request.TargetLanguage);
        Assert.Null(request.Tone);
        Assert.Null(request.NoteId);
        Assert.Null(request.Model);
    }

    [Fact]
    public void AiActionRequest_ConvenienceConstructorSetsActionAndText()
    {
        var request = new AiActionRequest(AiActionType.Summarize, "hello world");
        Assert.Equal(AiActionType.Summarize, request.Action);
        Assert.Equal("hello world", request.Text);
    }

    [Fact]
    public void AiActionRequest_ConvenienceConstructorTreatsNullAsEmpty()
    {
        var request = new AiActionRequest(AiActionType.Rewrite, null!);
        Assert.Equal(string.Empty, request.Text);
    }

    [Fact]
    public void AiActionResult_IsSuccessTrueWhenNoError()
    {
        var result = new AiActionResult
        {
            Result = "done",
            Error = null
        };
        Assert.True(result.IsSuccess);
        Assert.Equal("done", result.Result);
    }

    [Fact]
    public void AiActionResult_IsSuccessFalseWhenError()
    {
        var result = new AiActionResult
        {
            Result = "partial",
            Error = "something went wrong"
        };
        Assert.False(result.IsSuccess);
        Assert.Equal("partial", result.Result);
        Assert.Equal("something went wrong", result.Error);
    }

    [Fact]
    public void AiActionUsage_PropertiesStored()
    {
        var usage = new AiActionUsage
        {
            PromptTokens = 100,
            CompletionTokens = 50,
            TotalTokens = 150
        };
        Assert.Equal(100UL, usage.PromptTokens);
        Assert.Equal(50UL, usage.CompletionTokens);
        Assert.Equal(150UL, usage.TotalTokens);
    }

    [Fact]
    public void AiActionInfo_PropertiesPreserved()
    {
        var info = new AiActionInfo
        {
            Id = "summarize",
            Label = "总结要点",
            ActionType = AiActionType.Summarize
        };
        Assert.Equal("summarize", info.Id);
        Assert.Equal("总结要点", info.Label);
        Assert.Equal(AiActionType.Summarize, info.ActionType);
    }

    [Fact]
    public void AiActionItem_ShortcutVisibility_CollapsedWhenEmpty()
    {
        var item = new AiActionItem
        {
            ActionType = AiActionType.Summarize,
            Label = "test",
            Description = "desc",
            IconGlyph = "\uE9D9",
            ShortcutHint = ""
        };
        Assert.Equal(Microsoft.UI.Xaml.Visibility.Collapsed, item.ShortcutVisibility);
    }

    [Fact]
    public void AiActionItem_ShortcutVisibility_VisibleWhenSet()
    {
        var item = new AiActionItem
        {
            ActionType = AiActionType.Summarize,
            Label = "test",
            Description = "desc",
            IconGlyph = "\uE9D9",
            ShortcutHint = "Ctrl+Shift+S"
        };
        Assert.Equal(Microsoft.UI.Xaml.Visibility.Visible, item.ShortcutVisibility);
    }
}
