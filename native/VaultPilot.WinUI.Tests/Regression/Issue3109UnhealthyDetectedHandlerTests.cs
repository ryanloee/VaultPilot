using Xunit;
using System.IO;
using System.Reflection;
using System.Text.Json;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3109: WinUI silently dropped the
/// <c>UnhealthyDetected</c> agent event.
///
/// Bug: PR #3105 introduced the <c>SessionHealthTracker</c>, which emits an
/// <c>AgentEvent::UnhealthyDetected</c> when the agent falls into a loop
/// (same tool + args 4× in a row) or otherwise misbehaves. The CLI and agent
/// sidecar both serialise this as a <c>{stage: "unhealthyDetected", detail, suggestion, timestamp}</c>
/// payload. The WinUI <c>BackendClient</c> correctly parsed it and fired
/// <c>AgentStatusReceived</c>, but <c>MainWindow.HandleAgentEvent</c> had no
/// <c>"unhealthyDetected"</c> case in its switch — the event hit the default
/// fall-through and disappeared. Windows users never saw loop warnings.
///
/// Fix: add an explicit <c>unhealthyDetected</c> case in
/// <c>MainWindow.AgentMode.cs</c> that surfaces the warning via
/// <c>AppendMessage</c> + <c>UpdateStatusBar("warning", ...)</c>, and extend
/// <c>AgentStatusEvent</c> to preserve the <c>Suggestion</c> field that the
/// sidecar already sends.
///
/// These are source-structure + model assertions (HandleAgentEvent needs a
/// live UI thread, which is unavailable in headless CI).
/// </summary>
public class Issue3109UnhealthyDetectedHandlerTests
{
    private static string? ResolveSource(string relative)
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", relative);
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }

    [Fact]
    public void Regression_3109_AgentStatusEvent_PreservesSuggestion()
    {
        // The model MUST expose a Suggestion field so the sidecar payload's
        // recovery guidance survives deserialisation. Without it, JsonPropertyName
        // matching silently drops the property.
        var prop = typeof(AgentStatusEvent).GetProperty(
            "Suggestion",
            BindingFlags.Instance | BindingFlags.Public);
        Assert.NotNull(prop);
        // Must be nullable string — the field is only populated for unhealthyDetected.
        Assert.Equal(typeof(string), prop!.PropertyType);
    }

    [Fact]
    public void Regression_3109_AgentStatusEvent_Suggestion_RoundTripsThroughJson()
    {
        // Simulate the exact payload the agent sidecar emits (see
        // src/bin/vaultpilot-agent.rs L1013-L1018).
        var payload = """{"stage":"unhealthyDetected","detail":"Repetition detected","suggestion":"Reset context","timestamp":"2026-07-19T00:00:00Z"}""";
        var opts = new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
        var evt = JsonSerializer.Deserialize<AgentStatusEvent>(payload, opts);
        Assert.NotNull(evt);
        Assert.Equal("unhealthyDetected", evt!.Stage);
        Assert.Equal("Repetition detected", evt.Detail);
        Assert.Equal("Reset context", evt.Suggestion);
    }

    [Fact]
    public void Regression_3109_HandleAgentEvent_HasUnhealthyDetectedCase()
    {
        var sourcePath = ResolveSource("MainWindow.AgentMode.cs");
        if (sourcePath is null)
        {
            // Source not co-located in this build layout — skip structural check.
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // The switch in HandleAgentEvent must include a case for the
        // unhealthyDetected stage. The original bug was that the stage was
        // absent and events fell through silently.
        Assert.Contains("case \"unhealthyDetected\":", source);

        // The case must surface the warning to the user (AppendMessage) and
        // reflect it on the status bar (UpdateStatusBar with warning level).
        // Assert that, within the unhealthyDetected case body, both calls appear.
        var caseIdx = source.IndexOf("case \"unhealthyDetected\":", StringComparison.Ordinal);
        Assert.True(caseIdx >= 0);
        // Slice from the case to the next case/break boundary (~600 chars is plenty).
        var slice = source.Substring(caseIdx, Math.Min(800, source.Length - caseIdx));
        Assert.Contains("AppendMessage", slice);
        Assert.Contains("UpdateStatusBar(\"warning\"", slice);
        // The suggestion field MUST be referenced — otherwise the model change
        // would be dead and the user would lose the recovery guidance.
        Assert.Contains("Suggestion", slice);
    }

    [Fact]
    public void Regression_3109_HandleAgentEvent_MethodStillExists()
    {
        // Guard against accidental rename of the handler — the bug fix relies
        // on this being the single dispatch point for AgentStatusEvent.
        var method = typeof(MainWindow).GetMethod(
            "HandleAgentEvent",
            BindingFlags.NonPublic | BindingFlags.Instance,
            null,
            new[] { typeof(AgentStatusEvent) },
            null);
        Assert.NotNull(method);
    }
}
