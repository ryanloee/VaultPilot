# Regenerate Android launcher icons so the circular logo fills the icon:
#   - legacy ic_launcher / ic_launcher_round: full-bleed circle, transparent corners
#   - adaptive ic_launcher_foreground: circle at 65% of canvas (inside the
#     66.7% visible zone so the rim survives launcher masks)
#   - adaptive background color: #0A2854 (the logo's own dark navy fill), so
#     any mask-shape corner blends into the circle instead of showing white
#
# Usage: drop a NEW transparent-background source at
#   mipmap-xxxhdpi/ic_launcher_foreground.png (circle anywhere on the
#   canvas), then run this script — the circle bounds are auto-detected
#   via an alpha scan, so re-running is idempotent and works after the
#   foreground has already been regenerated.
Add-Type -AssemblyName System.Drawing
$root = 'D:\code\VaultPilot\desktop\src-tauri\icons\android'

$src = [System.Drawing.Bitmap]::FromFile("$root\mipmap-xxxhdpi\ic_launcher_foreground.png")
$cx = [int]($src.Width / 2)
$cy = [int]($src.Height / 2)
# Auto-detect the opaque circle's bounding box: scan inward from each edge
# along the center row/column for the first non-transparent pixel.
$left = 0
while ($left -lt $src.Width -and $src.GetPixel($left, $cy).A -lt 16) { $left++ }
$right = $src.Width - 1
while ($right -ge 0 -and $src.GetPixel($right, $cy).A -lt 16) { $right-- }
$top = 0
while ($top -lt $src.Height -and $src.GetPixel($cx, $top).A -lt 16) { $top++ }
$bottom = $src.Height - 1
while ($bottom -ge 0 -and $src.GetPixel($cx, $bottom).A -lt 16) { $bottom-- }
if ($right -le $left -or $bottom -le $top) {
    throw "no opaque circle found in the source foreground PNG"
}
# Use the smaller dimension and re-center on the detected bounds so the
# crop is square and concentric with the circle.
$cropSide = [Math]::Min($right - $left + 1, $bottom - $top + 1)
$srcOff = [int](($left + $right) / 2 - $cropSide / 2)
$srcOffY = [int](($top + $bottom) / 2 - $cropSide / 2)
Write-Host "detected circle: side $cropSide at ($srcOff,$srcOffY) on $($src.Width)x$($src.Height)"
$crop = New-Object System.Drawing.Bitmap($cropSide, $cropSide)
$cg = [System.Drawing.Graphics]::FromImage($crop)
$cg.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$cg.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$cg.DrawImage($src, (New-Object System.Drawing.Rectangle(0, 0, $cropSide, $cropSide)),
    (New-Object System.Drawing.Rectangle($srcOff, $srcOffY, $cropSide, $cropSide)),
    [System.Drawing.GraphicsUnit]::Pixel)
$cg.Dispose()
$src.Dispose()

function Save-CircleIcon([int]$canvasPx, [string]$path, [double]$fillRatio) {
    $dia = [int][Math]::Round($canvasPx * $fillRatio)
    $bmp = New-Object System.Drawing.Bitmap($canvasPx, $canvasPx)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $off = [int](($canvasPx - $dia) / 2)
    $g.DrawImage($crop, (New-Object System.Drawing.Rectangle($off, $off, $dia, $dia)),
        (New-Object System.Drawing.Rectangle(0, 0, $cropSide, $cropSide)),
        [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "wrote $path (${canvasPx}px, circle ${dia}px)"
}

$legacy = @{ 'mipmap-mdpi' = 48; 'mipmap-hdpi' = 72; 'mipmap-xhdpi' = 96; 'mipmap-xxhdpi' = 144; 'mipmap-xxxhdpi' = 192 }
$fg     = @{ 'mipmap-mdpi' = 108; 'mipmap-hdpi' = 162; 'mipmap-xhdpi' = 216; 'mipmap-xxhdpi' = 324; 'mipmap-xxxhdpi' = 432 }

foreach ($dir in $legacy.Keys) {
    $px = $legacy[$dir]
    Save-CircleIcon $px "$root\$dir\ic_launcher.png" 1.0
    Save-CircleIcon $px "$root\$dir\ic_launcher_round.png" 1.0
}
foreach ($dir in $fg.Keys) {
    Save-CircleIcon $fg[$dir] "$root\$dir\ic_launcher_foreground.png" 0.65
}

# Adaptive background: the logo's own dark navy fill, replacing white.
$bgXml = @"
<?xml version="1.0" encoding="utf-8"?>
<resources>
  <color name="ic_launcher_background">#0A2854</color>
</resources>
"@
[System.IO.File]::WriteAllText("$root\values\ic_launcher_background.xml", $bgXml)
Write-Host "wrote $root\values\ic_launcher_background.xml (#0A2854)"
$crop.Dispose()

