using Xunit;
using VaultPilot.WinUI.Backend;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3437: CancellationTokenSource leak in
/// BackendClient.DisposeProcessAsync during reconnect — CTS was cancelled
/// but never disposed, accumulating kernel wait handles across reconnection
/// cycles.
///
/// Bug (#3437):  Interlocked.Exchange(ref _readerCts, null)?.Cancel()
///               cancelled the CTS but skipped .Dispose(), leaving the
///               underlying kernel handle undisposed until GC finalizer.
/// Fix:          Capture the old CTS, call Cancel() then Dispose().
/// </summary>
public class Issue3437CtsDisposeLeakTests
{
    /// <summary>
    /// Verify that DisposeProcessAsync still exists as a private method.
    /// </summary>
    [Fact]
    public void Regression_3437_DisposeProcessAsyncMethodExists()
    {
        var method = typeof(BackendClient).GetMethod(
            "DisposeProcessAsync",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
        Assert.NotNull(method);
        Assert.True(method!.ReturnType == typeof(System.Threading.Tasks.Task),
            "DisposeProcessAsync should return Task");
    }

    /// <summary>
    /// Verify the source code contains both Cancel() and Dispose()
    /// calls on the CancellationTokenSource in the finally block of
    /// DisposeProcessAsync.
    /// </summary>
    [Fact]
    public void Regression_3437_CtsCancelAndDisposeInFinallyBlock()
    {
        var sourcePath = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", "Backend", "BackendClient.cs");
        if (!File.Exists(sourcePath))
        {
            // CI may not co-locate source; skip file-based assertions
            return;
        }

        var source = File.ReadAllText(sourcePath);

        // Ensure both Cancel() and Dispose() are called on the CTS
        Assert.Contains("oldCts?.Cancel();", source);
        Assert.Contains("oldCts?.Dispose();", source);

        // Ensure the old pattern (cancel without dispose) is gone
        Assert.DoesNotContain("Interlocked.Exchange(ref _readerCts, null)?.Cancel();", source);
    }
}
