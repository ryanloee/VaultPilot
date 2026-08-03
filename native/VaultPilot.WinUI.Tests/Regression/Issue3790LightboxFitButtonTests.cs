using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3790: Lightbox fit-to-screen zoom button.
///
/// Obsidian 1.13.3 added explicit zoom controls and click-drag pan for
/// large images. VaultPilot WinUI already had pan, zoom, and reset, but
/// was missing an explicit "fit to screen" button in the top-bar controls.
///
/// This test verifies the fit button is present in the source code and
/// that zoom controls include all four operations: zoom in, zoom out,
/// 100% reset, and fit-to-screen.
/// </summary>
public class Issue3790LightboxFitZoomTests
{
    [Fact]
    public void Regression_3790_HasFitButton()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.ImageLightbox.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // The fit button must be declared (should have been added alongside
        // the existing zoom controls).
        Assert.Contains("var fitBtn = MakeLightboxIconButton", code);

        // The fit button must be added to the controlsStack (rendered).
        Assert.Contains("controlsStack.Children.Add(fitBtn);", code);
    }

    [Fact]
    public void Regression_3790_ZoomControlsIncludeAllOperations()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.ImageLightbox.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // All four zoom control buttons must exist:
        // - Zoom in (放大)
        // - Zoom out (缩小)
        // - 100% / reset (1:1)
        // - Fit to screen (适应屏幕) — added in #3790
        Assert.Contains("Lightbox_ZoomIn", code);
        Assert.Contains("Lightbox_ZoomOut", code);
        Assert.Contains("Lightbox_ResetZoom", code);
        Assert.Contains("适应屏幕", code); // tooltip for fit button
    }

    [Fact]
    public void Regression_3790_PanAlreadyPresent()
    {
        // Pan on pointer drag when zoomed was already implemented — verify
        // it is still intact (no regression from #3790 changes).
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.ImageLightbox.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // Pan tracking state variables must exist.
        Assert.Contains("_lightboxPanning", code);
        Assert.Contains("_lightboxPanLast", code);

        // Pointer event handlers for pan must be wired up.
        Assert.Contains("Lightbox_OnPointerPressed", code);
        Assert.Contains("Lightbox_OnPointerMoved", code);
        Assert.Contains("Lightbox_OnPointerReleased", code);
    }

    private static string ResolveSourcePath(string projectName, string relativePath)
    {
        var baseDir = AppContext.BaseDirectory;
        var dir = new DirectoryInfo(baseDir);
        while (dir is not null && !dir.GetFiles("*.sln").Any())
            dir = dir.Parent;
        if (dir is null)
            return string.Empty;
        return Path.Combine(dir.FullName, projectName, relativePath);
    }
}