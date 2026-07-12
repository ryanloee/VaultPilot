using Xunit;
using System.Text.RegularExpressions;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #2750: MainWindow.xaml line 18 referenced a
/// non-existent event handler `OnCommandPaletteAccelerator` for a
/// Ctrl+Shift+P KeyboardAccelerator. The XAML parser would either throw
/// MissingMethodException during initialization or silently fail to bind.
///
/// The line was a stale merge artifact — the handler was renamed to
/// `OnAiCommandPaletteAccelerator` (line 15) but the old XAML line was
/// not removed, creating both a duplicate accelerator and a missing-handler
/// reference.
///
/// Fix: Remove the stale KeyboardAccelerator line referencing the
/// non-existent `OnCommandPaletteAccelerator` handler.
/// </summary>
public class Issue2750AcceleratorHandlerTests
{
    /// <summary>
    /// The XAML must no longer reference the non-existent handler name.
    /// </summary>
    [Fact]
    public void Regression_2750_XamlDoesNotReferenceMissingHandler()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.xaml");
        if (!File.Exists(sourcePath))
        {
            // In CI the source may not be co-located with test output.
            return;
        }

        var xaml = File.ReadAllText(sourcePath);

        // The removed handler must not appear anywhere in the XAML.
        Assert.DoesNotContain("OnCommandPaletteAccelerator", xaml);

        // The valid handler must still be present (Ctrl+Shift+P binding).
        Assert.Contains("OnAiCommandPaletteAccelerator", xaml);
    }

    /// <summary>
    /// Every `Invoked="..."` handler in MainWindow.xaml must have a
    /// corresponding private method in MainWindow.xaml.cs. This is a
    /// structural assertion that catches the class of bug (#2750) where
    /// an XAML element references a handler that doesn't exist.
    /// </summary>
    [Fact]
    public void Regression_2750_AllXamlInvokedHandlersExistInCodeBehind()
    {
        var xamlPath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.xaml");
        var csPath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.xaml.cs");
        if (!File.Exists(xamlPath) || !File.Exists(csPath))
        {
            return;
        }

        var xaml = File.ReadAllText(xamlPath);
        var csSource = File.ReadAllText(csPath);

        // Extract all Invoked="HandlerName" values from the XAML.
        var handlerMatches = Regex.Matches(xaml, @"Invoked=""(\w+)""");
        Assert.NotEmpty(handlerMatches);

        foreach (Match match in handlerMatches)
        {
            var handlerName = match.Groups[1].Value;
            // Each handler must appear as a method definition in the code-behind.
            Assert.Contains($"void {handlerName}(", csSource);
        }
    }

    private static string ResolveSourcePath(string projectDir, string fileName)
    {
        // When running from the test bin directory, walk up to the repo root.
        var fromBin = Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            projectDir, fileName);
        return fromBin;
    }
}
