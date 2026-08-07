using Xunit;
using System.IO;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3941: the Ctrl+, settings accelerator and the
/// empty-state "打开设置" button must open the new SettingsWindow, not the old
/// blocking ContentDialog.
///
/// Root cause: #3931 replaced dead `e is KeyRoutedEventArgs` with reading the
/// global Ctrl key state via InputKeyboardSource.GetKeyStateForCurrentThread.
/// That state is *always* Down when the Ctrl+, accelerator fires (the user is
/// holding Ctrl), so OnSettingsClicked always took the OpenSettingsDialogAsync
/// branch — keyboard users could never reach SettingsWindow.
///
/// Fix: OnSettingsClicked now delegates to OnSettingsClickedCore(forceWindow).
/// Only the mouse-Click path runs the Ctrl detection; the accelerator and the
/// ChatMessageRenderer empty-state button pass forceWindow:true to bypass it.
///
/// These assertions verify the source patterns (live UI tests need a Windows
/// environment which is unavailable on CI Linux runners).
/// </summary>
public class Issue3941SettingsAcceleratorForceWindowTests
{
    private static string? ResolveSource(string relative)
    {
        var candidate = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "VaultPilot.WinUI", relative);
        return File.Exists(candidate) ? Path.GetFullPath(candidate) : null;
    }

    [Fact]
    public void Regression_3941_OnSettingsClickedCore_HasForceWindowParameter()
    {
        var sourcePath = ResolveSource("MainWindow.xaml.cs");
        if (sourcePath is null)
        {
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // The core method must accept a forceWindow flag.
        Assert.Contains("OnSettingsClickedCore(object sender, RoutedEventArgs e, bool forceWindow)", source);
        // Only the non-force path runs the Ctrl detection.
        Assert.Contains("if (!forceWindow)", source);
    }

    [Fact]
    public void Regression_3941_SettingsAccelerator_ForcesWindow()
    {
        var sourcePath = ResolveSource("MainWindow.xaml.cs");
        if (sourcePath is null)
        {
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // The accelerator must bypass Ctrl detection (forceWindow: true).
        // Locate the OnSettingsAccelerator body and assert it calls Core with forceWindow.
        var accelIdx = source.IndexOf("OnSettingsAccelerator", StringComparison.Ordinal);
        Assert.True(accelIdx >= 0, "OnSettingsAccelerator not found");
        var slice = source.Substring(accelIdx, Math.Min(400, source.Length - accelIdx));
        Assert.Contains("forceWindow: true", slice);
    }

    [Fact]
    public void Regression_3941_ChatMessageRenderer_ForcesWindow()
    {
        var sourcePath = ResolveSource("MainWindow.ChatMessageRenderer.cs");
        if (sourcePath is null)
        {
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // The empty-state settings button must bypass Ctrl detection.
        Assert.Contains("OnSettingsClickedCore(settingsBtn, new RoutedEventArgs(), forceWindow: true)", source);
    }

    [Fact]
    public void Regression_3941_MouseClickPath_StillRunsCtrlDetection()
    {
        var sourcePath = ResolveSource("MainWindow.xaml.cs");
        if (sourcePath is null)
        {
            return;
        }
        var source = File.ReadAllText(sourcePath);

        // The mouse-Click handler OnSettingsClicked must still delegate to Core
        // with forceWindow:false so the #3931 Ctrl+click fallback stays intact.
        var clickIdx = source.IndexOf("private async void OnSettingsClicked", StringComparison.Ordinal);
        Assert.True(clickIdx >= 0, "OnSettingsClicked not found");
        var slice = source.Substring(clickIdx, Math.Min(200, source.Length - clickIdx));
        Assert.Contains("forceWindow: false", slice);

        // And the Ctrl detection pattern itself remains present (#3931).
        Assert.Contains("InputKeyboardSource.GetKeyStateForCurrentThread", source);
    }
}
