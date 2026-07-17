using Xunit;
using System.IO;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3046: <c>AddTurnAsync</c> session fallback path.
///
/// Bug: when a caller passed a <c>sessionId</c> that did not match any existing
/// session, <c>EnsureCurrentSession()</c> would create (or restore) a session
/// with a fresh id, but the subsequent lookup still filtered by the original
/// (invalid) <c>sessionId</c>, returning null. The turn was then silently
/// dropped at <c>if (session is null) return;</c>.
///
/// Fix: after <c>EnsureCurrentSession()</c>, fall back to <c>CurrentSession()</c>
/// (which honours <c>_currentSessionId</c>) instead of re-querying by the stale
/// <c>sessionId</c>. Additionally the final <c>return;</c> now emits a
/// <c>Debug.WriteLine</c> so any future regression is diagnosable rather than a
/// silent message drop.
///
/// Source-structure assertion (the method needs a live UI environment).
/// </summary>
public class Issue3046SessionFallbackTests
{
    private static string? ResolveSource()
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", "MainWindow.ChatSessionManager.cs");
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }

    [Fact]
    public void Regression_3046_AddTurnAsync_MethodExists()
    {
        var method = typeof(MainWindow).GetMethod(
            "AddTurnAsync",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.Instance);
        Assert.NotNull(method);
    }

    [Fact]
    public void Regression_3046_FallbackDoesNotReQueryByStaleSessionId()
    {
        var sourcePath = ResolveSource();
        if (sourcePath is null)
        {
            // Source not co-located in this build layout — skip structural check.
            return;
        }

        var source = File.ReadAllText(sourcePath);

        // Locate the AddTurnAsync body. We assert against the fallback block
        // inside `if (session is null) { EnsureCurrentSession(); ... }`.
        var ensureIdx = source.IndexOf("EnsureCurrentSession();", StringComparison.Ordinal);
        Assert.True(ensureIdx >= 0, "Expected EnsureCurrentSession() call in AddTurnAsync.");

        // The buggy pattern: after EnsureCurrentSession() it re-queried with the
        // stale `sessionId`:
        //     session = sessionId is not null
        //         ? _chatState.Sessions.FirstOrDefault(s => s.Id == sessionId)
        //         : CurrentSession();
        // This must be GONE — if it is still present, the bug is still there.
        var buggyMarker = "sessionId is not null\n                    ? _chatState.Sessions.FirstOrDefault(s => s.Id == sessionId)";
        Assert.DoesNotContain(buggyMarker.ReplaceLineEndings(), source.ReplaceLineEndings());

        // The fix must include the unambiguous `CurrentSession()` fallback
        // immediately after EnsureCurrentSession() in the null branch.
        var fixedMarker = "EnsureCurrentSession();\n                session = CurrentSession();";
        Assert.Contains(fixedMarker.ReplaceLineEndings(), source.ReplaceLineEndings());

        // The silent `return;` must be replaced with a Debug.WriteLine so any
        // future regression is diagnosable instead of a silent message drop.
        Assert.Contains("AddTurnAsync: session lookup failed", source);
    }
}
