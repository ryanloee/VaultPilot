using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3843: MainWindow.xaml contained a dead
/// "AI Command Palette" overlay grid (`AiCommandPaletteHost`, Phase 2 of
/// #2188) that was never shown or wired to any code — dead UI. The
/// palette feature is actually implemented via KeyboardAccelerator
/// (Ctrl+Shift+P → OnAiCommandPaletteAccelerator), so the hidden overlay
/// grid was unreachable clutter.
///
/// Fix: remove the dead `AiCommandPaletteHost` grid block from
/// MainWindow.xaml (merged via PR #3852).
/// </summary>
public class Issue3843DeadAiCommandPaletteHostTests
{
    /// <summary>
    /// The dead overlay grid must not be referenced anywhere in
    /// MainWindow.xaml.
    /// </summary>
    [Fact]
    public void Regression_3843_XamlDoesNotReferenceAiCommandPaletteHost()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.xaml");
        if (!File.Exists(sourcePath))
        {
            // In CI the source may not be co-located with test output.
            return;
        }

        var xaml = File.ReadAllText(sourcePath);

        // The dead grid was removed entirely — neither the name nor the
        // "AI Command Palette overlay" comment may survive a regression.
        Assert.DoesNotContain("AiCommandPaletteHost", xaml);
        Assert.DoesNotContain("AI Command Palette overlay", xaml);
    }

    /// <summary>
    /// The real palette entry point (Ctrl+Shift+P accelerator) must still
    /// be present — removing the dead grid must not have broken the live
    /// feature.
    /// </summary>
    [Fact]
    public void Regression_3843_LivePaletteAcceleratorStillPresent()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.xaml");
        if (!File.Exists(sourcePath))
        {
            return;
        }

        var xaml = File.ReadAllText(sourcePath);

        // The live accelerator hooking OnAiCommandPaletteAccelerator is the
        // actual palette implementation — it must remain.
        Assert.Contains("OnAiCommandPaletteAccelerator", xaml);
    }
}
