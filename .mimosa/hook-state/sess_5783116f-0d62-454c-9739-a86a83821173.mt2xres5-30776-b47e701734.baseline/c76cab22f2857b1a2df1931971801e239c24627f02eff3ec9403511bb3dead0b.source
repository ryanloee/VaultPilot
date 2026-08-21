//! Thumbnail generation and caching for vault assets (#3371).
//!
//! Generates small preview thumbnails for images stored in the vault,
//! caches them in `<vault_root>/.vp-thumbnails/<sha256>.jpg`, and
//! provides an HTTP endpoint for serving them to the WinUI / mobile
//! Asset Picker UI.
//!
//! ## Architecture
//!
//! ```text
//! Asset Picker UI (WinUI / Mobile)
//!     │  GET /api/vault/thumbnails/{path}
//!     ▼
//! HTTP Bridge  ←─  get_or_create_thumbnail(source, vault_dir)
//!     │
//!     ├── Cache HIT  →  return cached .jpg
//!     └── Cache MISS →  decode → resize → encode JPEG → cache → return
//! ```

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Default thumbnail width in pixels.
const THUMB_WIDTH: u32 = 300;

/// Subdirectory inside the vault root where thumbnails are cached.
const THUMB_CACHE_DIR: &str = ".vp-thumbnails";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Return the path to a cached thumbnail for `source_path`, generating it
/// on demand if it does not already exist.
///
/// The thumbnail is written as a JPEG file at
/// `{vault_root}/.vp-thumbnails/{content-hash}.jpg`.
pub fn get_or_create_thumbnail(
    vault_dir: &Path,
    source_path: &Path,
    width: Option<u32>,
) -> Result<PathBuf> {
    let cache_dir = vault_dir.join(THUMB_CACHE_DIR);
    std::fs::create_dir_all(&cache_dir).context("Failed to create thumbnail cache directory")?;

    let cache_key = content_hash(source_path)?;
    let thumb_path = cache_dir.join(format!("{cache_key}.jpg"));

    if thumb_path.exists() {
        return Ok(thumb_path);
    }

    let w = width.unwrap_or(THUMB_WIDTH);
    generate_thumbnail(source_path, &thumb_path, w)?;

    Ok(thumb_path)
}

/// Determine the cache path for `source_path` without generating the
/// thumbnail (useful for checking existence or logging).
pub fn thumbnail_cache_path(vault_dir: &Path, source_path: &Path) -> Result<PathBuf> {
    let cache_dir = vault_dir.join(THUMB_CACHE_DIR);
    let cache_key = content_hash(source_path)?;
    Ok(cache_dir.join(format!("{cache_key}.jpg")))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute a content-based hash of the file at `path` to use as cache key.
///
/// SHA-256 is used so that the cache invalidates automatically when the
/// source image changes.
fn content_hash(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file =
        std::fs::File::open(path).context("Failed to open file for thumbnail cache-key hashing")?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .context("Failed to read file for thumbnail cache-key hashing")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Decode `source`, resize it to `width` (maintaining aspect ratio), and
/// write a JPEG to `dest`.
fn generate_thumbnail(source: &Path, dest: &Path, width: u32) -> Result<()> {
    let img = image::ImageReader::open(source)
        .context("Failed to open source image for thumbnail")?
        .decode()
        .context("Failed to decode source image for thumbnail")?;

    let (orig_w, orig_h) = (img.width(), img.height());

    // Clamp to original dimensions: never upscale thumbnails.
    let target_w = width.min(orig_w);

    // Maintain aspect ratio.
    let target_h = if target_w == 0 {
        1
    } else {
        ((orig_h as u64 * target_w as u64) / orig_w as u64).max(1) as u32
    };

    let thumb = img.resize_exact(target_w, target_h, image::imageops::FilterType::Lanczos3);

    // .jpg extension triggers JPEG encoding with default quality (~75).
    thumb.save(dest).context("Failed to save thumbnail")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU16, Ordering};

    static TEST_COUNTER: AtomicU16 = AtomicU16::new(0);

    /// Helper: create a temporary directory unique to this test process.
    fn test_dir(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("vp-thumb-test-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create test dir");
        base
    }

    /// Helper: create a small test PNG and return its path.
    fn create_test_image(dir: &Path) -> PathBuf {
        let idx = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = dir.join(format!("test-{idx}.png"));

        // Create a small checkerboard image (8×8, two colours).
        let mut img = image::RgbImage::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                let pixel = if (x + y) % 2 == 0 {
                    image::Rgb([255u8, 0, 0]) // red
                } else {
                    image::Rgb([0, 255, 0]) // green
                };
                img.put_pixel(x, y, pixel);
            }
        }
        img.save(&path).expect("save test image");
        path
    }

    #[test]
    fn test_content_hash_is_stable() {
        let dir = test_dir("hash_stable");
        let path = create_test_image(&dir);

        let hash1 = content_hash(&path).unwrap();
        let hash2 = content_hash(&path).unwrap();
        assert_eq!(hash1, hash2, "hash should be deterministic");
        assert!(!hash1.is_empty(), "hash should not be empty");
    }

    #[test]
    fn test_generate_thumbnail_creates_file() {
        let dir = test_dir("gen_thumb");
        let src = create_test_image(&dir);
        let dest = dir.join("thumb.jpg");

        assert!(!dest.exists());
        generate_thumbnail(&src, &dest, 100).unwrap();
        assert!(dest.exists(), "thumbnail should have been created");

        // Verify it's a valid JPEG by re-opening it.
        let reloaded = image::ImageReader::open(&dest).unwrap().decode().unwrap();
        // original is 8×8, width=100 but clamped to min(100,8)=8
        assert_eq!(reloaded.width(), 8, "width should be clamped to original");
        assert_eq!(reloaded.height(), 8, "height should maintain aspect ratio");
    }

    #[test]
    fn test_generate_thumbnail_does_not_upscale() {
        let dir = test_dir("no_upscale");
        // Create a larger image: 200×100
        let path = dir.join("wide.png");
        let img = image::RgbImage::new(200, 100);
        img.save(&path).unwrap();

        let dest = dir.join("thumb2.jpg");
        generate_thumbnail(&path, &dest, 500).unwrap();

        let reloaded = image::ImageReader::open(&dest).unwrap().decode().unwrap();
        // Should NOT upscale: width clamped to min(500, 200) = 200
        assert_eq!(reloaded.width(), 200, "should not upscale beyond original");
        assert_eq!(reloaded.height(), 100, "should maintain aspect ratio");
    }

    #[test]
    fn test_get_or_create_thumbnail_caches() {
        let dir = test_dir("cache_test");
        let vault_dir = dir.join("vault");
        fs::create_dir_all(&vault_dir).unwrap();
        let src = create_test_image(&vault_dir);

        // First call — cache miss (generate).
        let thumb1 = get_or_create_thumbnail(&vault_dir, &src, None).unwrap();
        assert!(thumb1.exists(), "thumbnail should exist after generation");

        // Second call — cache hit (should return same path without regenerating).
        let thumb2 = get_or_create_thumbnail(&vault_dir, &src, None).unwrap();
        assert_eq!(thumb1, thumb2, "cache path should be identical");

        // Verify cache directory was created.
        let cache_dir = vault_dir.join(THUMB_CACHE_DIR);
        assert!(cache_dir.is_dir(), "cache directory should exist");
    }

    #[test]
    fn test_thumbnail_cache_path_no_generation() {
        let dir = test_dir("cache_path");
        let vault_dir = dir.join("vault");
        fs::create_dir_all(&vault_dir).unwrap();
        let src = create_test_image(&vault_dir);

        // cache_path should succeed even without generating the thumbnail.
        let cache_path = thumbnail_cache_path(&vault_dir, &src).unwrap();
        assert!(cache_path.to_string_lossy().ends_with(".jpg"));
        // File should NOT exist yet — we only computed the path.
        assert!(!cache_path.exists());
    }
}
