using Xunit;
using System.Reflection;
using VaultPilot.WinUI.Backend;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression tests for issue #3858: the _reconnectInProgress idempotency
/// guard was wrapped in `if (!forceRestart)` while ALL 7 background
/// fire-and-forget trigger points (EOF, health check, SendAsync write-fail /
/// timeout, power resume) funneled through TryReconnectWithRetryAsync(),
/// which hardcoded forceRestart: true — so the guard never ran and concurrent
/// triggers still queued on _reconnectLock and sequentially executed a full
/// dispose+restart, each one killing the just-restarted healthy process
/// (restart churn).
///
/// Fix:
///   1. TryReconnectWithRetryAsync now takes forceRestart = false by default
///      and passes it through — background triggers keep the default.
///   2. TryReconnectAsync collapses via the `IsConnected && !forceRestart`
///      check, both before and (crucially) AFTER acquiring _reconnectLock:
///      a trigger that was queued while another reconnect ran finds the
///      fresh healthy process and backs off instead of killing it.
///   3. Only the manual ReconnectAsync (user Reconnect button) passes
///      forceRestart: true (fresh restart even of a healthy process is the
///      user's explicit intent).
/// </summary>
public class Issue3858ReconnectGuardTests
{
    private static string? TryGetSource()
    {
        var sourcePath = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", "Backend", "BackendClient.cs");
        // Fall back to the repo-relative path if the build output path doesn't resolve
        if (!File.Exists(sourcePath))
        {
            // In CI the test runs from the test bin directory; the source
            // may not be co-located. Skip source assertion in that case.
            return null;
        }
        return File.ReadAllText(sourcePath);
    }

    [Fact]
    public void Regression_3858_TryReconnectWithRetryAsync_HasForceRestartDefaultFalse()
    {
        var method = typeof(BackendClient).GetMethod(
            "TryReconnectWithRetryAsync",
            BindingFlags.NonPublic | BindingFlags.Instance);
        Assert.NotNull(method);

        var forceRestartParam = method!.GetParameters()
            .FirstOrDefault(p => p.Name == "forceRestart");
        Assert.NotNull(forceRestartParam);
        Assert.Equal(typeof(bool), forceRestartParam!.ParameterType);
        Assert.Equal(false, forceRestartParam.DefaultValue);
    }

    [Fact]
    public void Regression_3858_BackgroundTriggers_DoNotForceRestart()
    {
        var source = TryGetSource();
        if (source is null) return;

        // The retry wrapper must pass forceRestart through (not hardcode true).
        Assert.Contains(
            "TryReconnectAsync(forceRestart: forceRestart",
            source);

        // The old hardcoded forceRestart: true inside the retry wrapper must
        // be gone (that was what bypassed the guard for all background
        // triggers).
        Assert.DoesNotContain(
            "TryReconnectAsync(forceRestart: true",
            source);
    }

    [Fact]
    public void Regression_3858_ManualReconnect_StillForcesRestart()
    {
        var source = TryGetSource();
        if (source is null) return;

        // The manual Reconnect button must keep forceRestart: true (a user
        // clicking Reconnect wants a fresh process even when one is alive).
        Assert.Contains(
            "TryReconnectWithRetryAsync(cancellationToken: cancellationToken, forceRestart: true)",
            source);
    }

    [Fact]
    public void Regression_3858_CollapseCheck_PresentBeforeAndInsideLock()
    {
        var source = TryGetSource();
        if (source is null) return;

        // The collapse check `IsConnected && !forceRestart` must exist both
        // before the lock (fast path) and after it (the critical one: a
        // trigger queued behind an in-flight reconnect must collapse onto
        // the fresh process instead of killing it).
        int count = 0;
        int index = 0;
        while ((index = source.IndexOf("IsConnected && !forceRestart", index, StringComparison.Ordinal)) >= 0)
        {
            count++;
            index += "IsConnected && !forceRestart".Length;
        }
        Assert.True(count >= 2,
            $"Expected at least 2 collapse checks (pre-lock + in-lock), found {count}.");

        // The in-lock check must be positioned after the lock acquisition.
        var lockIndex = source.IndexOf("_reconnectLock.WaitAsync", StringComparison.Ordinal);
        var lastCheckIndex = source.LastIndexOf("IsConnected && !forceRestart", StringComparison.Ordinal);
        Assert.True(lockIndex >= 0);
        Assert.True(lastCheckIndex > lockIndex,
            "The in-lock collapse check must appear after the lock acquisition.");
    }

    [Fact]
    public void Regression_3858_TryReconnectAsync_SignaturePreserved()
    {
        var method = typeof(BackendClient).GetMethod(
            "TryReconnectAsync",
            BindingFlags.NonPublic | BindingFlags.Instance);
        Assert.NotNull(method);
        Assert.True(method!.ReturnType == typeof(System.Threading.Tasks.Task<bool>),
            "TryReconnectAsync should return Task<bool>");
    }
}
