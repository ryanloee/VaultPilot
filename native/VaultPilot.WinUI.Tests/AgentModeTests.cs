using Xunit;

namespace VaultPilot.WinUI.Tests;

public class AgentModeTests
{
    [Fact]
    public void TruncateString_Short_ReturnsOriginal()
    {
        // Use reflection to call private static method
        var method = typeof(MainWindow).GetMethod("TruncateString",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Assert.NotNull(method);

        var result = method.Invoke(null, new object[] { "hello", 10 });
        Assert.Equal("hello", result);
    }

    [Fact]
    public void TruncateString_Long_TruncatesWithEllipsis()
    {
        var method = typeof(MainWindow).GetMethod("TruncateString",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Assert.NotNull(method);

        var result = method.Invoke(null, new object[] { "hello world", 5 });
        Assert.Equal("hello…", result);
    }

    [Fact]
    public void TruncateString_ExactBoundary_ReturnsOriginal()
    {
        var method = typeof(MainWindow).GetMethod("TruncateString",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Assert.NotNull(method);

        var result = method.Invoke(null, new object[] { "12345", 5 });
        Assert.Equal("12345", result);
    }

    [Fact]
    public void TruncateString_Empty_ReturnsEmpty()
    {
        var method = typeof(MainWindow).GetMethod("TruncateString",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Assert.NotNull(method);

        var result = method.Invoke(null, new object[] { "", 10 });
        Assert.Equal("", result);
    }

    [Fact]
    public void ParseWriteArgs_WriteNote_ShowsFilePathAndPreview()
    {
        var method = typeof(MainWindow).GetMethod("ParseWriteArgs",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Assert.NotNull(method);

        var args = """{"path":"notes/test.md","content":"# Hello\nWorld"}""";
        var result = method.Invoke(null, new object[] { "write_note", args });
        var desc = ((string, string))result;

        Assert.Contains("notes/test.md", desc.Item1);
        Assert.Contains("# Hello", desc.Item2);
    }

    [Fact]
    public void ParseWriteArgs_DeleteNote_ShowsDeleteDescription()
    {
        var method = typeof(MainWindow).GetMethod("ParseWriteArgs",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Assert.NotNull(method);

        var args = """{"path":"notes/old.md"}""";
        var result = method.Invoke(null, new object[] { "delete_note", args });
        var desc = ((string, string))result;

        Assert.Contains("删除", desc.Item1);
        Assert.Contains("notes/old.md", desc.Item1);
    }

    [Fact]
    public void ParseWriteArgs_InvalidJson_FallbackToRawArgs()
    {
        var method = typeof(MainWindow).GetMethod("ParseWriteArgs",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Assert.NotNull(method);

        var result = method.Invoke(null, new object[] { "write_note", "not-json" });
        var desc = ((string, string))result;

        Assert.Contains("write_note", desc.Item1);
        Assert.Contains("not-json", desc.Item2);
    }

    [Fact]
    public void ParseWriteArgs_LongContent_TruncatesAt50Lines()
    {
        var method = typeof(MainWindow).GetMethod("ParseWriteArgs",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Assert.NotNull(method);

        var lines = string.Join("\\n", Enumerable.Range(1, 60).Select(i => $"Line {i}"));
        var args = $$"""{"path":"notes/long.md","content":"{{lines}}"}""";
        var result = method.Invoke(null, new object[] { "save_note", args });
        var desc = ((string, string))result;

        Assert.Contains("共 60 行", desc.Item2);
        Assert.DoesNotContain("Line 60", desc.Item2);
    }
}
