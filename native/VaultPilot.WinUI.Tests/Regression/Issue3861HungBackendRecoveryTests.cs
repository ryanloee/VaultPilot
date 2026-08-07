using System;
using System.IO;
using System.Linq;
using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for bug #3861: the health check must distinguish a "busy"
/// backend from a "hung" one.
///
/// After #3859's busy-protection, a ping timeout was always attributed to
/// busy-ness whenever the process was alive: the catch block skipped
/// reconnection, _consecutiveHealthCheckFailures never accumulated, degraded
/// mode never engaged, and an idle-but-hung backend (agent main loop
/// deadlock) never recovered — every request hung for the full 180s timeout.
///
/// The fix: skip the ping entirely while requests are in flight (serial agent
/// loop), and in the catch block only skip reconnection when other requests
/// are still in flight (busy). An idle ping timeout — nothing in _pending —
/// means alive-but-hung and escalates to TryReconnectWithRetryAsync.
///
/// These are source-structure assertions consistent with the other regression
/// tests in this folder (CI only compiles WinUI tests — #597).
/// </summary>
public class Issue3861HungBackendRecoveryTests
{
    private static readonly string SourceRelativePath = Path.Combine("Backend", "BackendClient.cs");

    /// <summary>
    /// The tick must not send a ping while a request is in flight — a ping
    /// queued behind a long askWithAi would time out and must never be allowed
    /// to kill the busy-but-healthy process.
    /// </summary>
    [Fact]
    public void Regression_3861_HealthCheck_SkipsPingWhileRequestInFlight()
    {
        var source = ReadSource();
        if (source.Length == 0)
        {
            // In CI the source may not be co-located with test output.
            return;
        }
        var tickStart = source.IndexOf("private async void OnHealthCheckTick", StringComparison.Ordinal);
        var tickEnd = source.IndexOf("private void OnConsecutiveHealthCheckFailure", StringComparison.Ordinal);
        Assert.True(tickStart >= 0 && tickEnd > tickStart, "OnHealthCheckTick method not found");

        var method = source[tickStart..tickEnd];
        Assert.Contains("_pending.IsEmpty", method);
        Assert.Contains("SendAsync(\"ping\"", method);
    }

    /// <summary>
    /// The catch block must only skip reconnection when other requests are in
    /// flight. An idle ping timeout (empty _pending) is a hung backend and
    /// must still escalate to TryReconnectWithRetryAsync.
    /// </summary>
    [Fact]
    public void Regression_3861_IdlePingTimeout_EscalatesToReconnect()
    {
        var source = ReadSource();
        if (source.Length == 0)
        {
            // In CI the source may not be co-located with test output.
            return;
        }
        var tickStart = source.IndexOf("private async void OnHealthCheckTick", StringComparison.Ordinal);
        var tickEnd = source.IndexOf("private void OnConsecutiveHealthCheckFailure", StringComparison.Ordinal);
        Assert.True(tickStart >= 0 && tickEnd > tickStart, "OnHealthCheckTick method not found");

        var method = source[tickStart..tickEnd];

        // The busy guard: skip reconnect only when requests are in flight.
        Assert.Contains("&& !_pending.IsEmpty", method);

        // The hung path: an idle ping timeout must still attempt a reconnect.
        Assert.Contains("TryReconnectWithRetryAsync", method);
        Assert.Contains("OnConsecutiveHealthCheckFailure", method);
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
