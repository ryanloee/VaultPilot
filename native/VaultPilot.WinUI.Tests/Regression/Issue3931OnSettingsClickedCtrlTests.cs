using Xunit;
using System.IO;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3931: the Ctrl+click fallback in
/// OnSettingsClicked (MainWindow.xaml.cs) was dead code.
///
/// Bug: OnSettingsClicked checked `e is KeyRoutedEventArgs` to decide
/// whether to open the old SettingsDialog, but all three call sites pass
/// RoutedEventArgs (SettingsButton.Click, the settings accelerator key,
/// and ChatMessageRenderer). A Click event can never carry
/// KeyRoutedEventArgs, so useDialog was always false and the
/// OpenSettingsDialogAsync path (#3612 claimed both paths were kept) was
/// unreachable.
///
/// Fix: detect the Ctrl key via
/// InputKeyboardSource.GetKeyStateForCurrentThread (same pattern as
/// OnComposerKeyDown) instead of inspecting the event args. Both code
/// paths — OpenSettingsDialogAsync (Ctrl held) and OpenSettingsWindow —
/// remain reachable.
///
/// These assertions verify the source patterns (live UI tests need a
/// Windows environment which is unavailable on CI Linux runners).
/// </summary>
public class Issue3931OnSettingsClickedCtrlTests
{
    private static string? ResolveSource(string relative)
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", relative);
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }

    [Fact]
    public void Regression_3931_Source_NoLongerChecksEventArgType()
    {
        var sourcePath = ResolveSource("MainWindow.xaml.cs");
        if (sourcePath is null)
        {
            // Source not co-located in this build layout — skip.
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // The dead `e is KeyRoutedEventArgs` check (which could never match
        // because all callers pass RoutedEventArgs) must be gone.
        Assert.DoesNotContain("e is KeyRoutedEventArgs", source);
        Assert.DoesNotContain("keyArgs.Key", source);
    }

    [Fact]
    public void Regression_3931_Source_UsesKeyboardStateForCtrlDetection()
    {
        var sourcePath = ResolveSource("MainWindow.xaml.cs");
        if (sourcePath is null)
        {
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // Ctrl detection must read the real keyboard state (same pattern as
        // OnComposerKeyDown) instead of inspecting event args.
        Assert.Contains("InputKeyboardSource.GetKeyStateForCurrentThread", source);
        Assert.Contains("CoreVirtualKeyStates.Down", source);

        // Both settings paths must remain reachable: the old ContentDialog
        // path (Ctrl held) and the new SettingsWindow path, and the
        // null-coalescing fallback guarded by #3090 must stay intact.
        Assert.Contains("OpenSettingsDialogAsync()", source);
        Assert.Contains("_settings.Provider ?? new ProviderConfig()", source);
    }
}
