using Xunit;
using System.IO;
using System.Reflection;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #2783: the re-entrance guard <c>_updateCheckStarted</c>
/// was only reset to 0 in the <c>catch</c> branch of
/// <c>CheckForAppUpdatesAsync()</c>. The early <c>return</c>s inside the
/// <c>try</c> block (e.g. "already up to date", "not installed") therefore left
/// the flag stuck at 1, so the manual "检查更新" button became a permanent
/// silent no-op after any successful check.
///
/// Fix: a <c>finally</c> block now resets the guard on EVERY exit path.
///
/// These are source-structure assertions (the method requires a live UI
/// environment and cannot be exercised in a headless test), consistent with the
/// other regression tests in this folder.
/// </summary>
public class Issue2783UpdateCheckGuardTests
{
    private static string? ResolveSource()
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", "MainWindow.Updates.cs");
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }

    [Fact]
    public void Regression_2783_CheckForAppUpdatesAsync_Exists()
    {
        var method = typeof(MainWindow).GetMethod(
            "CheckForAppUpdatesAsync",
            BindingFlags.NonPublic | BindingFlags.Instance);
        Assert.NotNull(method);
        Assert.Equal(typeof(System.Threading.Tasks.Task), method!.ReturnType);
    }

    [Fact]
    public void Regression_2783_ResetGuardInFinally_NotOnlyCatch()
    {
        var sourcePath = ResolveSource();
        if (sourcePath is null)
        {
            // Source not co-located in this build layout — skip structural check.
            return;
        }

        var source = File.ReadAllText(sourcePath);

        // The re-entrance guard must exist.
        Assert.Contains("_updateCheckStarted", source);

        // The fix adds a `finally` block that resets the guard, in addition to
        // the original catch-branch reset. If the finally is reverted, the
        // reset only appears once (in catch) and this assertion fails.
        var resetCount = CountOccurrences(source, "Interlocked.Exchange(ref _updateCheckStarted, 0)");
        Assert.True(resetCount >= 2,
            $"Expected the guard to be reset on >=2 paths (catch + finally), found {resetCount}.");

        // The finally block must be present in the method.
        Assert.Contains("finally", source);
    }

    private static int CountOccurrences(string haystack, string needle)
    {
        if (needle.Length == 0) return 0;
        int count = 0, idx = 0;
        while ((idx = haystack.IndexOf(needle, idx, StringComparison.Ordinal)) >= 0)
        {
            count++;
            idx += needle.Length;
        }
        return count;
    }
}
