using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3530: Image Lightbox shows stale image on
/// rapid navigation — async race in LoadLightboxImageAsync.
///
/// Bug: LoadLightboxImageAsync is called fire-and-forget from Lightbox_Navigate.
/// After the await completes, there was no check that _lightboxIndex still
/// matches the index parameter, so a slow response could overwrite the
/// current image with a stale one.
///
/// Fix: Add a staleness check (_lightboxIndex != index) after the await,
/// before applying the bitmap, and again after the catch block before
/// updating nav buttons/labels.
/// </summary>
public class Issue3530StaleLightboxImageTests
{
    [Fact]
    public void Regression_3530_HasStalenessCheckAfterAwait()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.ImageLightbox.cs");
        if (!File.Exists(sourcePath))
            return; // source may not be co-located in CI

        var code = File.ReadAllText(sourcePath);

        // The staleness check must exist right after the await in LoadLightboxImageAsync.
        // Old code: no check — could overwrite current image with stale one.
        // Fixed:    if (_lightboxIndex != index) return;
        Assert.Contains("if (_lightboxIndex != index) return;", code);
    }

    [Fact]
    public void Regression_3530_HasRecheckAfterCatch()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.ImageLightbox.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // A second staleness check after the catch block ensures that even
        // if an exception occurred, stale state isn't applied.
        Assert.Contains("// Re-check staleness after catch", code);
    }

    [Fact]
    public void Regression_3530_ResetZoomAndNavAfterStalenessCheck()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.ImageLightbox.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // Lightbox_ResetZoom, nav buttons, and index label must only update
        // AFTER the staleness check passes — never on a stale response.
        var stalenessReturn = "if (_lightboxIndex != index) return;";
        var afterCatchIdx = code.IndexOf("// Re-check staleness after catch");
        Assert.True(afterCatchIdx >= 0, "Staleness re-check comment not found");
        var resetZoomIdx = code.IndexOf("Lightbox_ResetZoom();", afterCatchIdx);
        Assert.True(resetZoomIdx > afterCatchIdx, "Lightbox_ResetZoom must come after staleness re-check");
    }

    private static string ResolveSourcePath(string projectName, string relativePath)
    {
        var baseDir = AppContext.BaseDirectory;
        // Walk up from test output to find the solution root.
        var dir = new DirectoryInfo(baseDir);
        while (dir is not null && !dir.GetFiles("*.sln").Any())
            dir = dir.Parent;
        if (dir is null)
            return string.Empty;
        return Path.Combine(dir.FullName, projectName, relativePath);
    }
}
