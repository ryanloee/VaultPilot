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
}
