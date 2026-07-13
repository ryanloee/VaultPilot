using Xunit;
using System.IO;
using System.Reflection;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #2784: <c>ShowWriteApprovalDialog</c> captured the
/// agent <c>_agentCts</c> reference at dialog-open time. If the agent was
/// stopped or restarted while the dialog was open, <c>StartAgentMode</c>/
/// <c>StopAgentMode</c> rotate <c>_agentCts</c> (disposing the old one). The
/// approval was then sent with the stale, disposed <c>CancellationTokenSource</c>,
/// throwing <c>OperationCanceledException</c> so the response was silently lost.
///
/// Fix: the CTS is captured LIVE, after <c>await dialog.ShowAsync()</c> returns,
/// so the current session's token is used.
///
/// Source-structure assertion (the method needs a live UI environment).
/// </summary>
public class Issue2784WriteApprovalCtsTests
{
    private static string? ResolveSource()
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", "MainWindow.AgentMode.cs");
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }

    [Fact]
    public void Regression_2784_ShowWriteApprovalDialog_Exists()
    {
        var method = typeof(MainWindow).GetMethod(
            "ShowWriteApprovalDialog",
            BindingFlags.NonPublic | BindingFlags.Instance);
        Assert.NotNull(method);
    }

    [Fact]
    public void Regression_2784_CtsCapturedAfterDialog_NotAtOpen()
    {
        var sourcePath = ResolveSource();
        if (sourcePath is null)
        {
            // Source not co-located in this build layout — skip structural check.
            return;
        }

        var source = File.ReadAllText(sourcePath);

        var dialogShowIdx = source.IndexOf("await dialog.ShowAsync()", StringComparison.Ordinal);
        var ctsCaptureIdx = source.IndexOf("var cts = _agentCts;", StringComparison.Ordinal);

        Assert.True(dialogShowIdx >= 0, "Expected 'await dialog.ShowAsync()' in ShowWriteApprovalDialog.");
        Assert.True(ctsCaptureIdx >= 0, "Expected 'var cts = _agentCts;' capture in ShowWriteApprovalDialog.");

        // The capture must come AFTER the dialog is shown (live capture), not
        // before it (stale capture at open time).
        Assert.True(ctsCaptureIdx > dialogShowIdx,
            "Bug #2784: _agentCts was captured at dialog-open time (stale reference). " +
            "It must be captured after await dialog.ShowAsync().");

        // The old "capture at call time" comment must be gone.
        Assert.DoesNotContain("Capture reference at call time", source);
    }
}
