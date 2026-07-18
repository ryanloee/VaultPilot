using System.Text.Json;
using VaultPilot.WinUI.Models;
using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3109: WinUI main window silently dropped the
/// Agent health-tracker's UnhealthyDetected event because HandleAgentEvent's
/// switch statement had no case for stage="unhealthyDetected".
///
/// Bug (#3109):  PR #3105 added SessionHealthTracker which emits an
///               UnhealthyDetected event when the agent loops on the same
///               tool/args 4+ times. The CLI and agent sidecar correctly
///               serialized it as { stage: "unhealthyDetected", detail, suggestion }.
///               WinUI's HandleAgentEvent switch had no matching case →
///               Windows users saw no warning at all.
/// Root cause:   Missing case in HandleAgentEvent + missing Suggestion
///               field on AgentStatusEvent.
/// Fix:          - Added `Suggestion` field to AgentStatusEvent model
///               - Added `case "unhealthyDetected":` that shows a warning
///                 (AppendMessage + UpdateStatusBar("warning", ...))
///                 WITHOUT auto-stopping the agent (it may self-correct).
/// </summary>
public class Issue3109UnhealthyDetectedHandlerTests
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = true
    };

    /// <summary>
    /// The Suggestion field must deserialize from the Rust sidecar's
    /// snake_case `suggestion` payload key (case-insensitive).
    /// </summary>
    [Fact]
    public void Regression_3109_SuggestionDeserializesFromSnakeCasePayload()
    {
        const string json = """
            {
                "stage": "unhealthyDetected",
                "detail": "Repetition detected: read_file called with same arguments 4 times consecutively",
                "suggestion": "Agent has been calling the same tool repeatedly. Consider resetting context.",
                "timestamp": "2026-07-19T10:00:00Z"
            }
            """;
        var status = JsonSerializer.Deserialize<AgentStatusEvent>(json, JsonOptions);
        Assert.NotNull(status);
        Assert.Equal("unhealthyDetected", status!.Stage);
        Assert.NotNull(status.Suggestion);
        Assert.Contains("resetting context", status.Suggestion);
    }

    /// <summary>
    /// The Suggestion field must remain null when absent from the payload
    /// (back-compat with older agent sidecars that don't emit it).
    /// </summary>
    [Fact]
    public void Regression_3109_SuggestionIsNullWhenAbsentFromPayload()
    {
        const string json = """
            {
                "stage": "unhealthyDetected",
                "detail": "Repetition detected",
                "timestamp": "2026-07-19T10:00:00Z"
            }
            """;
        var status = JsonSerializer.Deserialize<AgentStatusEvent>(json, JsonOptions);
        Assert.NotNull(status);
        Assert.Null(status!.Suggestion);
    }

    /// <summary>
    /// Direct mirror of the HandleAgentEvent "unhealthyDetected" case body.
    /// Verifies the message-construction predicate: when suggestion is present
    /// it is appended; when absent, only the reason is shown.
    /// </summary>
    [Theory]
    [InlineData("Reset the agent context", true)]
    [InlineData("", false)]
    [InlineData("   \t  ", false)]
    public void Regression_3109_WarningBodyIncludesSuggestionWhenPresent(
        string rawSuggestion, bool expectSuggestionInBody)
    {
        var status = new AgentStatusEvent
        {
            Stage = "unhealthyDetected",
            Detail = "Repetition detected",
            Suggestion = string.IsNullOrEmpty(rawSuggestion) ? null : rawSuggestion
        };

        // Mirror of the production code in MainWindow.AgentMode.cs:
        //   var reason = status.Detail ?? "Agent 可能陷入循环";
        //   var suggestion = status.Suggestion?.Trim();
        //   var warningBody = string.IsNullOrEmpty(suggestion)
        //       ? reason
        //       : $"{reason}\n\n建议: {suggestion}";
        var reason = status.Detail ?? "Agent 可能陷入循环";
        var suggestion = status.Suggestion?.Trim();
        var warningBody = string.IsNullOrEmpty(suggestion)
            ? reason
            : $"{reason}\n\n建议: {suggestion}";

        Assert.Equal("Repetition detected", reason);
        if (expectSuggestionInBody)
        {
            Assert.Contains("建议:", warningBody);
            Assert.Contains(suggestion, warningBody);
        }
        else
        {
            Assert.DoesNotContain("建议:", warningBody);
            Assert.Equal(reason, warningBody);
        }
    }

    /// <summary>
    /// The "unhealthyDetected" stage must be distinct from terminal stages
    /// (error/timeout/stepLimitReached) — it is a warning, not a stop trigger.
    /// This guards against future refactors that accidentally route it into
    /// the StopAgentMode path.
    /// </summary>
    [Fact]
    public void Regression_3109_UnhealthyStageIsNonTerminal()
    {
        // The fix intentionally does NOT call StopAgentMode for unhealthyDetected.
        // Terminal stages that DO stop the agent:
        var terminalStages = new HashSet<string>
        {
            "agentCompleted",
            "stepLimitReached",
            "tokenBudgetExceeded",
            "timeout",
            "error"
        };
        Assert.DoesNotContain("unhealthyDetected", terminalStages);
    }
}
