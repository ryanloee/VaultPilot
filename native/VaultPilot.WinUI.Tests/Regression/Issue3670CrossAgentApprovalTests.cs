using Xunit;
using System.IO;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3670: cross-session write approval routing.
///
/// When a write approval dialog is open and the user stops/restarts the agent,
/// the dialog's response was incorrectly routed to the new agent session because
/// the generation check was missing.
///
/// Fix: an <c>_agentGeneration</c> counter (Interlocked.Increment on every
/// Start/Stop) is captured at dialog-open time. When the user responds, the
/// current generation is re-read; if it differs, the response is silently
/// discarded with "Agent 会话已变更" rather than being routed to the wrong
/// session.
/// </summary>
public class Issue3670CrossSessionApprovalTests
{
    private static string? ResolveSource()
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", "MainWindow.AgentMode.cs");
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }

    [Fact]
    public void Regression_3670_AgentGenerationField_Exists()
    {
        var field = typeof(MainWindow).GetField(
            "_agentGeneration",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
        Assert.NotNull(field);
        Assert.Equal(typeof(long), field.FieldType);
    }

    [Fact]
    public void Regression_3670_GenerationBumpedInStartAgentMode()
    {
        var sourcePath = ResolveSource();
        if (sourcePath is null) return;

        var source = File.ReadAllText(sourcePath);

        // StartAgentMode must contain Interlocked.Increment(ref _agentGeneration)
        var startIdx = source.IndexOf("private void StartAgentMode(", System.StringComparison.Ordinal);
        Assert.True(startIdx >= 0, "Expected StartAgentMode method.");

        var stopIdx = source.IndexOf("private void StopAgentMode(", System.StringComparison.Ordinal);
        Assert.True(stopIdx >= 0, "Expected StopAgentMode method.");

        var startBody = source[startIdx..stopIdx];
        Assert.Contains("Interlocked.Increment(ref _agentGeneration)", startBody);
    }

    [Fact]
    public void Regression_3670_GenerationBumpedInStopAgentMode()
    {
        var sourcePath = ResolveSource();
        if (sourcePath is null) return;

        var source = File.ReadAllText(sourcePath);

        var stopIdx = source.IndexOf("private void StopAgentMode(", System.StringComparison.Ordinal);
        Assert.True(stopIdx >= 0, "Expected StopAgentMode method.");

        var nextMethodIdx = source.IndexOf("private async Task ExecuteAgentRequestAsync", System.StringComparison.Ordinal);
        Assert.True(nextMethodIdx >= 0, "Expected ExecuteAgentRequestAsync method.");

        var stopBody = source[stopIdx..nextMethodIdx];
        Assert.Contains("Interlocked.Increment(ref _agentGeneration)", stopBody);
    }

    [Fact]
    public void Regression_3670_GenerationCheckInShowWriteApprovalDialog()
    {
        var sourcePath = ResolveSource();
        if (sourcePath is null) return;

        var source = File.ReadAllText(sourcePath);

        var dialogIdx = source.IndexOf("ShowWriteApprovalDialog", System.StringComparison.Ordinal);
        Assert.True(dialogIdx >= 0, "Expected ShowWriteApprovalDialog method.");

        var parseArgsIdx = source.IndexOf("ParseWriteArgs", dialogIdx, System.StringComparison.Ordinal);
        var sentinelIdx = source.IndexOf("private static (string", dialogIdx, System.StringComparison.Ordinal);

        var dialogBody = source[dialogIdx..sentinelIdx];

        // Must capture generation at dialog-open time
        Assert.Contains("Interlocked.Read(ref _agentGeneration)", dialogBody);

        // Must check generation after dialog closes
        Assert.Contains("currentGeneration != dialogGeneration", dialogBody);

        // Must have the "Agent 会话已变更" message for stale dialogs
        Assert.Contains("Agent 会话已变更", dialogBody);
    }

    [Fact]
    public void Regression_3670_GenerationBumpedAgentCompleted()
    {
        var sourcePath = ResolveSource();
        if (sourcePath is null) return;

        var source = File.ReadAllText(sourcePath);

        // Find the agentCompleted case in HandleAgentEvent
        var completedIdx = source.IndexOf("case \"agentCompleted\"", System.StringComparison.Ordinal);
        Assert.True(completedIdx >= 0, "Expected agentCompleted case.");

        var nextCaseIdx = source.IndexOf("case \"stepLimitReached\"", System.StringComparison.Ordinal);
        Assert.True(nextCaseIdx >= 0, "Expected next case after agentCompleted.");

        var completedBody = source[completedIdx..nextCaseIdx];
        Assert.Contains("Interlocked.Increment(ref _agentGeneration)", completedBody);
    }
}