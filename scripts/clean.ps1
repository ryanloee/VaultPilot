param(
    [switch]$IncludeReleaseAssets
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir

$targets = @(
    (Join-Path $repoRoot "target"),
    (Join-Path $repoRoot "artifacts"),
    (Join-Path $repoRoot "tmp-icons"),
    (Join-Path $repoRoot "native\VaultPilot.WinUI\bin"),
    (Join-Path $repoRoot "native\VaultPilot.WinUI\obj"),
    (Join-Path $repoRoot "packaging\windows\Output"),
    (Join-Path $repoRoot "installer\Output")
)

if ($IncludeReleaseAssets) {
    $targets += (Join-Path $repoRoot "release-assets")
}

$processes = Get-Process -ErrorAction SilentlyContinue | Where-Object {
    $_.Path -like (Join-Path $repoRoot "artifacts\*") -or
    $_.Path -like (Join-Path $repoRoot "target\*") -or
    $_.Path -like (Join-Path $repoRoot "native\VaultPilot.WinUI\bin\*")
}

if ($processes) {
    $processes | Stop-Process -Force -ErrorAction SilentlyContinue
}

foreach ($path in $targets) {
    if (Test-Path $path) {
        Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "Cleanup complete."
