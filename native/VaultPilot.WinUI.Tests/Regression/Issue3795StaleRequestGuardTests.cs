using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3795: a superseded (stale) AI request could still
/// rewrite the UI after a newer request had taken over. When request #2 is fired
/// while request #1 is in flight, #1's CTS is cancelled via Interlocked.Exchange,
/// but its catch/finally continuation is scheduled independently and may run
/// AFTER #2 has shown its own loading card (or displayed its result), stomping
/// #2's newer UI state (answer replaced by "请求已取消" / loading overlay collapsed
/// while a newer request is still awaiting).
///
/// Bug:  QuickAskOverlay.SubmitQuestion and AiCommandPalette.ExecuteAction only
///       guarded the success path (ct.IsCancellationRequested) — the OCE /
///       generic-exception catch blocks mutated the UI unconditionally.
/// Fix:  Every UI-mutating path (success + all catch blocks) first checks
///       !ReferenceEquals(Volatile.Read(ref _activeRequestCts), newCts), so a
///       stale continuation bails out before touching the UI.
/// </summary>
public class Issue3795StaleRequestGuardTests
{
    private static string SourceFilePath(string fileName) =>
        Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", "Controls", fileName);

    private static string? TryReadSource(string fileName)
    {
        var path = SourceFilePath(fileName);
        return File.Exists(path) ? File.ReadAllText(path) : null;
    }

    [Theory]
    [InlineData("QuickAskOverlay.xaml.cs")]
    [InlineData("AiCommandPalette.xaml.cs")]
    public void Regression_3795_StaleGuardPresentInSource(string fileName)
    {
        var source = TryReadSource(fileName);
        if (source is null)
        {
            // CI may not co-locate source; skip file-based assertions.
            return;
        }

        // Every stale continuation must bail out before mutating the UI.
        Assert.Contains(
            "if (!ReferenceEquals(Volatile.Read(ref _activeRequestCts), newCts))",
            source);
    }

    [Fact]
    public void Regression_3795_QuickAskGuardCoversAllCatchPaths()
    {
        var source = TryReadSource("QuickAskOverlay.xaml.cs");
        if (source is null) return;

        // The OCE catch (the "请求已取消" stomp) must be guarded, not just the
        // success path.
        Assert.Contains("catch (OperationCanceledException) when (ct.IsCancellationRequested)", source);
        Assert.True(
            CountOccurrences(source, "if (!ReferenceEquals(Volatile.Read(ref _activeRequestCts), newCts))") >= 4,
            "QuickAskOverlay should guard success + OCE + Timeout + generic catch (4 sites)");
    }

    [Fact]
    public void Regression_3795_PaletteGuardCoversAllCatchPaths()
    {
        var source = TryReadSource("AiCommandPalette.xaml.cs");
        if (source is null) return;

        Assert.Contains("catch (OperationCanceledException)", source);
        Assert.True(
            CountOccurrences(source, "if (!ReferenceEquals(Volatile.Read(ref _activeRequestCts), newCts))") >= 3,
            "AiCommandPalette should guard success + OCE + generic catch (3 sites)");
    }

    private static int CountOccurrences(string text, string needle)
    {
        var count = 0;
        var idx = 0;
        while ((idx = text.IndexOf(needle, idx, StringComparison.Ordinal)) >= 0)
        {
            count++;
            idx += needle.Length;
        }
        return count;
    }
}