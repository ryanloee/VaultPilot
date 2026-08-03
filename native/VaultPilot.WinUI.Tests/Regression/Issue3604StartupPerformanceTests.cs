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

        var content = File.ReadAllText(source);

        // #3604: parallelize independent backend calls at startup
        Assert.Contains("Task.WhenAll", content);
    }

    [Fact]
    public void Regression_3604_MessagesRepeater_Uses_Virtualized_ItemsSource()
    {
        var source = ResolveSource();

        var content = File.ReadAllText(source);

        // #3581/#3604: virtualized chat message list
        Assert.Contains("MessagesRepeater.ItemsSource = _messageItems", content);
    }

    /// <summary>
    /// Resolves native/VaultPilot.WinUI/MainWindow.xaml.cs relative to the test
    /// assembly, walking up the directory tree until the file is found.
    ///
    /// CI builds the test project with /p:Platform=x64, so the output lands in
    /// bin/x64/Debug|Release/net8.0-windows10.0.19041.0/ — a fixed number of
    /// ".." hops from there never reaches native/. Walking up until the file is
    /// found makes the tests work in any output layout (with or without an
    /// x64/RID subdirectory).
    /// </summary>
    private static string ResolveSource()
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(
                directory.FullName, "VaultPilot.WinUI", "MainWindow.xaml.cs");
            if (File.Exists(candidate))
            {
                return Path.GetFullPath(candidate);
            }
            directory = directory.Parent;
        }

        // Fail loudly instead of silently passing: a wrong path must never turn
        // these source-structure tests into no-ops (see #3793).
        Assert.Fail(
            "Could not locate VaultPilot.WinUI/MainWindow.xaml.cs from test " +
            $"output directory '{AppContext.BaseDirectory}'. " +
            "The test project output layout may have changed; update ResolveSource().");
        return string.Empty; // unreachable — Assert.Fail throws
    }
}
