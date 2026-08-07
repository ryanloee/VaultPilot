using System.IO;
using System.Text.Json;
using Microsoft.UI.Xaml;

namespace VaultPilot.WinUI;

/// <summary>
/// Persists the user's theme preference (System/Light/Dark) to a local JSON
/// file under <c>LocalApplicationData/com.local.vaultpilot</c>.
///
/// This intentionally bypasses the Rust backend's <c>AppSettings</c> so the
/// theme toggle can ship without a backend schema change — the preference is
/// purely a client-side concern. The file is small, lazily created, and safe
/// to delete (defaults back to System).
///
/// #3850: saves are atomic (temp file + flush + rename) so an exit or crash
/// mid-write can never leave a truncated/corrupt theme.json — mirroring the
/// backend's atomic_write contract for settings/chat-state files.
/// </summary>
internal static class ThemePreferences
{
    private static readonly string Directory = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "com.local.vaultpilot");
    private static readonly string FilePath = Path.Combine(Directory, "theme.json");

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true
    };

    /// <summary>
    /// Loads the persisted theme mode. Returns <see cref="ElementTheme.Default"/>
    /// (follow system) when the file is missing or unreadable.
    /// </summary>
    public static ElementTheme Load()
    {
        try
        {
            if (!File.Exists(FilePath)) return ElementTheme.Default;
            var json = File.ReadAllText(FilePath);
            var doc = JsonSerializer.Deserialize<ThemeRecord>(json, JsonOptions);
            return doc?.Mode switch
            {
                "light" => ElementTheme.Light,
                "dark" => ElementTheme.Dark,
                _ => ElementTheme.Default
            };
        }
        catch
        {
            return ElementTheme.Default;
        }
    }

    /// <summary>
    /// Persists the theme mode. Creates the directory lazily and swallows
    /// IO failures (the theme still applies in-memory for the session).
    /// </summary>
    public static void Save(ElementTheme mode)
    {
        // #3862: unique temp name per save. A fixed ".tmp" name +
        // FileShare.None made concurrent Save calls (SettingsWindow and
        // SettingsDialog open at the same time, or a save re-triggered
        // mid-write) race on the same deterministic file: the second
        // FileStream threw IOException, which the catch swallowed — silently
        // dropping that theme change. The backend's atomic_write
        // (src/storage/mod.rs) uses random UUID suffixes precisely to prevent
        // concurrent writers racing on one temp name; mirror that here. The
        // rename over FilePath is still atomic on NTFS (same volume), so the
        // #3850 crash-safety contract is unchanged.
        var tmpPath = $"{FilePath}.{Guid.NewGuid():N}.tmp";
        try
        {
            System.IO.Directory.CreateDirectory(Directory);
            var record = new ThemeRecord
            {
                Mode = mode switch
                {
                    ElementTheme.Light => "light",
                    ElementTheme.Dark => "dark",
                    _ => "system"
                }
            };
            var json = JsonSerializer.Serialize(record, JsonOptions);

            // #3850: atomic write — write to a temp file in the same
            // directory, flush to disk, then rename over the target. A
            // process exit or crash mid-write leaves only a harmless temp
            // file, never a truncated theme.json (mirrors the backend's
            // atomic_write contract).
            using (var stream = new FileStream(
                tmpPath,
                FileMode.Create,
                FileAccess.Write,
                FileShare.None))
            using (var writer = new StreamWriter(stream, System.Text.Encoding.UTF8))
            {
                writer.Write(json);
                writer.Flush();
                stream.Flush(flushToDisk: true);
            }
            // File.Move with overwrite is atomic on NTFS (same volume).
            File.Move(tmpPath, FilePath, overwrite: true);
        }
        catch
        {
            // Best-effort persistence; in-memory theme still applies.
            // Clean up this save's temp file so a failed write never leaves
            // stale theme.json.*.tmp files behind.
            try
            {
                if (File.Exists(tmpPath))
                {
                    File.Delete(tmpPath);
                }
            }
            catch
            {
                // Ignore cleanup failures.
            }
        }
    }

    /// <summary>Converts the persisted string form to <see cref="ElementTheme"/>.</summary>
    public static ElementTheme FromString(string? mode) => mode switch
    {
        "light" => ElementTheme.Light,
        "dark" => ElementTheme.Dark,
        _ => ElementTheme.Default
    };

    /// <summary>Converts <see cref="ElementTheme"/> to the persisted string form.</summary>
    public static string ToStringValue(ElementTheme mode) => mode switch
    {
        ElementTheme.Light => "light",
        ElementTheme.Dark => "dark",
        _ => "system"
    };

    private sealed class ThemeRecord
    {
        public string Mode { get; set; } = "system";
    }
}
