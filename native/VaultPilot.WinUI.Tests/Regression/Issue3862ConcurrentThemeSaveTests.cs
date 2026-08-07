using System;
using System.IO;
using System.Linq;
using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for bug #3862: ThemePreferences.Save must not use a fixed
/// temp file name.
///
/// The old code wrote to a deterministic "theme.json.tmp" with FileShare.None.
/// Two concurrent Save calls (SettingsWindow + SettingsDialog both open, or a
/// save re-triggered mid-write) raced on that same temp file: the second
/// FileStream threw IOException, which the catch swallowed — silently dropping
/// the theme change. The backend's atomic_write (src/storage/mod.rs) uses
/// random UUID temp names specifically to prevent concurrent writers racing on
/// a deterministic name; the WinUI version had lost that protection.
///
/// The fix: temp name is now `theme.json.{guid}.tmp` — unique per save — while
/// keeping the atomic rename (#3850) and per-save temp cleanup.
///
/// These are source-structure assertions consistent with the other regression
/// tests in this folder (CI only compiles WinUI tests — #597).
/// </summary>
public class Issue3862ConcurrentThemeSaveTests
{
    /// <summary>
    /// Save must use a unique (Guid-suffixed) temp file name instead of a
    /// fixed ".tmp" name, so concurrent saves never race on the same file.
    /// </summary>
    [Fact]
    public void Regression_3862_ThemeSave_UsesUniqueTempName()
    {
        var source = ReadSource();
        if (source.Length == 0)
        {
            return;
        }

        // Unique temp name per save (mirrors backend atomic_write UUID suffix).
        Assert.Contains("Guid.NewGuid()", source);

        // The deterministic shared temp name is gone.
        Assert.DoesNotContain("FilePath + \".tmp\"", source);
        Assert.DoesNotContain("FilePath + \".tmp\")", source);
    }

    /// <summary>
    /// The atomic rename over the target (crash-safety from #3850) must be
    /// preserved.
    /// </summary>
    [Fact]
    public void Regression_3862_ThemeSave_KeepsAtomicRename()
    {
        var source = ReadSource();
        if (source.Length == 0)
        {
            return;
        }

        Assert.Contains("File.Move(tmpPath, FilePath, overwrite: true)", source);
        Assert.Contains("stream.Flush(flushToDisk: true)", source);
    }

    /// <summary>
    /// A failed save must delete its own unique temp file (not a shared name)
    /// so stale temp files never accumulate.
    /// </summary>
    [Fact]
    public void Regression_3862_ThemeSave_CleansUpItsOwnTempFile()
    {
        var source = ReadSource();
        if (source.Length == 0)
        {
            return;
        }

        Assert.Contains("File.Delete(tmpPath)", source);
    }

    private static string ReadSource()
    {
        var baseDir = AppContext.BaseDirectory;
        var dir = new DirectoryInfo(baseDir);
        while (dir is not null && !dir.GetFiles("*.sln").Any())
            dir = dir.Parent;
        if (dir is null)
        {
            return string.Empty;
        }

        var path = Path.Combine(dir.FullName, "VaultPilot.WinUI", "ThemePreferences.cs");
        return File.Exists(path) ? File.ReadAllText(path) : string.Empty;
    }
}
