using Xunit;
using System.Diagnostics;
using System.IO;
using System.Reflection;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for enhancement #3604: verify that startup performance
/// instrumentation (StartupWatch + LogStartup) is present and wired correctly.
///
/// #3604 added:
/// 1. App.StartupWatch — a Stopwatch that starts at process entry
/// 2. LogStartup() — logs milestones with elapsed-ms prefix
/// 3. Startup milestones in MainWindow.OnLoaded
///
/// These are source-structure assertions consistent with the other regression
/// tests in this folder.
/// </summary>
public class Issue3604StartupPerformanceTests
{
    [Fact]
    public void Regression_3604_StartupWatch_Exists()
    {
        var field = typeof(App).GetField(
            "StartupWatch",
            BindingFlags.Public | BindingFlags.Static);
        Assert.NotNull(field);
    }

    [Fact]
    public void Regression_3604_StartupWatch_IsStopwatch()
    {
        var field = typeof(App).GetField(
            "StartupWatch",
            BindingFlags.Public | BindingFlags.Static);
        Assert.NotNull(field);
        Assert.Equal(typeof(Stopwatch), field!.FieldType);
    }

    [Fact]
    public void Regression_3604_LogStartup_Method_Exists()
    {
        var method = typeof(MainWindow).GetMethod(
            "LogStartup",
            BindingFlags.NonPublic | BindingFlags.Static);
        Assert.NotNull(method);
    }

    [Fact]
    public void Regression_3604_LogStartup_Accepts_String()
    {
        var method = typeof(MainWindow).GetMethod(
            "LogStartup",
            BindingFlags.NonPublic | BindingFlags.Static);
        Assert.NotNull(method);
        var parameters = method!.GetParameters();
        Assert.Single(parameters);
        Assert.Equal(typeof(string), parameters[0].ParameterType);
    }

    [Fact]
    public void Regression_3604_OnLoaded_Logs_Key_Milestones()
    {
        var source = ResolveSource();
        if (source is null) return;

        var content = File.ReadAllText(source);

        // Each milestone ensures the startup timeline captures these phases
        Assert.Contains("LogStartup(\"Window loaded\"", content);
        Assert.Contains("LogStartup(\"Backend process started\"", content);
        Assert.Contains("LogStartup(\"Ping ok\"", content);
        Assert.Contains("LogStartup(\"Startup complete\"", content);
    }

    [Fact]
    public void Regression_3604_OnLoaded_Uses_TaskWhenAll()
    {
        var source = ResolveSource();
        if (source is null) return;

        var content = File.ReadAllText(source);

        // #3604: parallelize independent backend calls at startup
        Assert.Contains("Task.WhenAll", content);
    }

    [Fact]
    public void Regression_3604_MessagesRepeater_Uses_Virtualized_ItemsSource()
    {
        var source = ResolveSource();
        if (source is null) return;

        var content = File.ReadAllText(source);

        // #3581/#3604: virtualized chat message list
        Assert.Contains("MessagesRepeater.ItemsSource = _messageItems", content);
    }

    private static string? ResolveSource()
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", "MainWindow.xaml.cs");
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }
}

/// <summary>
/// Workaround: the project uses <ImplicitUsings>disable</ImplicitUsings> and
/// App is internal (defined in App.xaml.cs). Reflection-based tests reference
/// the type by name, but the compiler sees no App type in global scope.
/// This alias provides a neutral name for the reflection get-field test.
/// </summary>
#pragma warning disable CA1050
internal static class AppStartup
{
    public static object GetStartupWatch()
    {
      var field = typeof(App).GetField(
            "StartupWatch",
            BindingFlags.Public | BindingFlags.Static);
      return field?.GetValue(null) ?? new object();
    }
}