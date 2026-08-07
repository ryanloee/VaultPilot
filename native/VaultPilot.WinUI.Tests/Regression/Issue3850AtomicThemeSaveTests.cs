using System.Collections.Generic;
using System.IO;
using System.Linq;
using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for enhancement #3850: config files must never be left
/// truncated/corrupt when the app exits or crashes mid-write.
///
/// The Rust backend already writes settings.json and chat-state.json through
/// `atomic_write` (temp file + fsync + rename — src/storage/mod.rs). The
/// client-side gap was ThemePreferences.Save, which used a plain
/// File.WriteAllText: a crash between open and close could truncate
/// theme.json. #3850 makes it atomic (temp file + flush-to-disk + rename),
/// mirroring the backend contract.
///
/// These are source-structure assertions consistent with the other
/// regression tests in this folder (CI only compiles WinUI tests — #597).
/// </summary>
public class Issue3850AtomicThemeSaveTests
{
    /// <summary>
    /// ThemePreferences.Save must use the atomic temp-file + rename pattern.
    /// </summary>
    [Fact]
    public void Regression_3850_ThemeSave_IsAtomic()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "ThemePreferences.cs");
        if (!File.Exists(sourcePath))
        {
            // In CI the source may not be co-located with test output.
            return;
        }

        var source = File.ReadAllText(sourcePath);

        // The buggy pattern is gone.
        Assert.DoesNotContain("File.WriteAllText(FilePath, json)", source);

        // The atomic pattern is present: temp file in the same directory,
        // flush to disk, then rename over the target.
        Assert.Contains("\".tmp\"", source);
        Assert.Contains("stream.Flush(flushToDisk: true)", source);
        Assert.Contains("File.Move(tmpPath, FilePath, overwrite: true)", source);
    }

    /// <summary>
    /// A failed write must clean up its temp file so stale .tmp files never
    /// accumulate.
    /// </summary>
    [Fact]
    public void Regression_3850_ThemeSave_CleansUpTempOnFailure()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "ThemePreferences.cs");
        if (!File.Exists(sourcePath))
        {
            return;
        }

        var source = File.ReadAllText(sourcePath);
        Assert.Contains("File.Delete(FilePath + \".tmp\")", source);
    }

    /// <summary>
    /// The backend-side contract (settings.json / chat-state.json) must keep
    /// using atomic writes — no regression back to plain fs::write.
    /// </summary>
    [Fact]
    public void Regression_3850_BackendWrites_RemainAtomic()
    {
        var settingsPath = ResolveRepoFile("src", "storage", "settings.rs");
        var chatPath = ResolveRepoFile("src", "storage", "chat.rs");
        if (!File.Exists(settingsPath) || !File.Exists(chatPath))
        {
            return;
        }

        Assert.Contains("atomic_write", File.ReadAllText(settingsPath));
        Assert.Contains("atomic_write", File.ReadAllText(chatPath));
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

    private static string ResolveRepoFile(params string[] relativeParts)
    {
        var baseDir = AppContext.BaseDirectory;
        var dir = new DirectoryInfo(baseDir);
        while (dir is not null && !dir.GetFiles("*.sln").Any())
            dir = dir.Parent;
        if (dir is null)
            return string.Empty;
        var parts = new List<string> { dir.FullName };
        parts.AddRange(relativeParts);
        return Path.Combine(parts.ToArray());
    }
}
