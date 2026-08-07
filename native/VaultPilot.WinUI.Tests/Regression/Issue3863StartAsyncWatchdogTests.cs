using System;
using System.IO;
using System.Linq;
using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for bug #3863: the health check watchdog must be armed on
/// EVERY process start, not only from StartAsync.
///
/// Previously StartHealthCheck() was called only from StartAsync. If
/// StartAsync's 5s _reconnectLock wait timed out (reachable: TryReconnectAsync
/// can hold the lock ~9-10s during dispose+restart), StartAsync returned
/// without starting the process or the timer — and even if the backend was
/// later pulled up via SendAsync → EnsureConnectedAsync, no health check
/// watchdog ever ran, so a hung-but-alive backend (no EOF to trigger the
/// reconnect path) never recovered for the rest of the session.
///
/// The fix: StartProcessAsync (the success path every process start flows
/// through — initial start, TryReconnectAsync, EnsureConnectedAsync) now calls
/// StartHealthCheck() itself, guaranteeing a watchdog whenever the process is
/// up.
///
/// These are source-structure assertions consistent with the other regression
/// tests in this folder (CI only compiles WinUI tests — #597).
/// </summary>
public class Issue3863StartAsyncWatchdogTests
{
    private static readonly string SourceRelativePath = Path.Combine("Backend", "BackendClient.cs");

    /// <summary>
    /// StartHealthCheck must be invoked from within StartProcessAsync (between
    /// its declaration and the next method), so every process-start path arms
    /// the watchdog — not just StartAsync.
    /// </summary>
    [Fact]
    public void Regression_3863_WatchdogStartsWithProcess()
    {
        var source = ReadSource();
        if (source.Length == 0)
        {
            // In CI the source may not be co-located with test output.
            return;
        }

        var startProcessIdx = source.IndexOf(
            "private async Task StartProcessAsync", StringComparison.Ordinal);
        var nextMethodIdx = source.IndexOf(
            "private void StartHealthCheck", StringComparison.Ordinal);
        Assert.True(startProcessIdx >= 0, "StartProcessAsync method not found");
        Assert.True(nextMethodIdx > startProcessIdx, "StartHealthCheck definition not found");

        // The only call site must be inside StartProcessAsync's body.
        var callIdx = source.IndexOf("StartHealthCheck();", StringComparison.Ordinal);
        Assert.True(callIdx >= 0, "StartHealthCheck() call not found");
        Assert.True(callIdx > startProcessIdx && callIdx < nextMethodIdx,
            "StartHealthCheck() must be called from StartProcessAsync, not only StartAsync");

        // The watchdog must still exist as a method.
        Assert.Contains("private void StartHealthCheck()", source);
    }

    /// <summary>
    /// StartAsync must not swallow the watchdog on its 5s lock timeout — the
    /// recovery path is now owned by StartProcessAsync.
    /// </summary>
    [Fact]
    public void Regression_3863_StartAsync_NoLongerSoleWatchdogOwner()
    {
        var source = ReadSource();
        if (source.Length == 0)
        {
            return;
        }

        var startAsyncStart = source.IndexOf(
            "public async Task StartAsync", StringComparison.Ordinal);
        var startAsyncEnd = source.IndexOf(
            "private async Task StartProcessAsync", StringComparison.Ordinal);
        Assert.True(startAsyncStart >= 0 && startAsyncEnd > startAsyncStart,
            "StartAsync method not found");

        // StartHealthCheck() must NOT be called from StartAsync's body.
        var startAsyncBody = source[startAsyncStart..startAsyncEnd];
        Assert.DoesNotContain("StartHealthCheck();", startAsyncBody);
    }

    private static string ReadSource()
    {
        var baseDir = AppContext.BaseDirectory;
        var dir = new DirectoryInfo(baseDir);
        while (dir is not null && !dir.GetFiles("*.sln").Any())
            dir = dir.Parent;
        if (dir is null)
        {
            return string.Empty;
        }

        var path = Path.Combine(dir.FullName, "VaultPilot.WinUI", SourceRelativePath);
        return File.Exists(path) ? File.ReadAllText(path) : string.Empty;
    }
}
