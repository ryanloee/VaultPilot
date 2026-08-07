using System.IO;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Text.Json;
using Microsoft.UI.Xaml.Media.Imaging;

namespace VaultPilot.WinUI;

/// <summary>
/// Persists the user's custom avatar image to a local file under
/// <c>LocalApplicationData/com.local.vaultpilot</c> and exposes it for
/// the sidebar brand mark and chat message avatars.
///
/// Mirrors <see cref="ThemePreferences"/>: purely client-side, bypasses the
/// Rust backend's AppSettings (no schema change / no 77 test literals to
/// update), lazy-created, safe to delete (falls back to the letter avatar).
///
/// Storage: copies the chosen image to <c>avatar.png</c> so the original
/// file can move/delete without breaking the UI; the copy is what gets loaded.
/// </summary>
internal static class AvatarPreferences
{
    private static readonly string Directory = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "com.local.vaultpilot");
    private static readonly string AvatarPath = Path.Combine(Directory, "avatar.png");

    /// <summary>
    /// Path of the persisted avatar image, or null when none is set.
    /// </summary>
    public static string? AvatarFilePath =>
        File.Exists(AvatarPath) ? AvatarPath : null;

    /// <summary>
    /// Loads the avatar as a <see cref="BitmapImage"/> ready for an
    /// <c>Image.Source</c>, or null when no avatar is configured.
    /// </summary>
    public static BitmapImage? LoadBitmap()
    {
        var path = AvatarFilePath;
        if (path is null)
        {
            return null;
        }

        try
        {
            var bitmap = new BitmapImage();
            using var stream = File.OpenRead(path);
            bitmap.SetSource(stream.AsRandomAccessStream());
            return bitmap;
        }
        catch
        {
            return null; // corrupt/unreadable avatar — fall back to letter
        }
    }

    /// <summary>
    /// Saves a new avatar by copying <paramref name="sourcePath"/> into the
    /// app data directory. Swallows IO failures (avatar simply doesn't change).
    /// Returns the persisted path, or null on failure.
    /// </summary>
    public static string? Save(string sourcePath)
    {
        try
        {
            System.IO.Directory.CreateDirectory(Directory);

            // Copy to a temp name first, then atomic-replace over avatar.png
            // so a crash mid-copy never leaves a truncated avatar.
            var tmpPath = $"{AvatarPath}.{Guid.NewGuid():N}.tmp";
            File.Copy(sourcePath, tmpPath, overwrite: true);
            File.Move(tmpPath, AvatarPath, overwrite: true);
            return AvatarPath;
        }
        catch
        {
            try
            {
                if (File.Exists(AvatarPath))
                {
                    File.Delete(AvatarPath);
                }
            }
            catch
            {
                // Ignore cleanup failures.
            }
            return null;
        }
    }

    /// <summary>
    /// Removes the persisted avatar, reverting to the letter fallback.
    /// </summary>
    public static void Clear()
    {
        try
        {
            if (File.Exists(AvatarPath))
            {
                File.Delete(AvatarPath);
            }
        }
        catch
        {
            // Best-effort.
        }
    }
}
