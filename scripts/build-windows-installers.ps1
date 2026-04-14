param(
    [string[]]$Platforms = @("x86", "x64"),
    [string]$Version,
    [string]$RepoUrl,
    [string]$GitHubToken,
    [switch]$FetchReleaseHistory
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
$cargoToml = Join-Path $repoRoot "Cargo.toml"
$projectFile = Join-Path $repoRoot "native\VaultPilot.WinUI\VaultPilot.WinUI.csproj"
$iconPath = Join-Path $repoRoot "native\VaultPilot.WinUI\icon.ico"
$artifactsRoot = Join-Path $repoRoot "artifacts\velopack"

function Get-VersionFromCargoToml {
    param([string]$Path)

    $line = Get-Content $Path | Where-Object { $_ -match '^version\s*=\s*".*"$' } | Select-Object -First 1
    if (-not $line) {
        throw "Unable to resolve version from Cargo.toml."
    }

    return [regex]::Match($line, '"([^"]+)"').Groups[1].Value
}

function Resolve-MSBuild {
    $command = Get-Command msbuild.exe -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    $candidates = @(
        "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\amd64\MSBuild.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\amd64\MSBuild.exe",
        "C:\Program Files (x86)\Microsoft Visual Studio\2022\Enterprise\MSBuild\Current\Bin\amd64\MSBuild.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\MSBuild\Current\Bin\amd64\MSBuild.exe",
        "C:\Program Files (x86)\Microsoft Visual Studio\2022\Community\MSBuild\Current\Bin\amd64\MSBuild.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\Community\MSBuild\Current\Bin\amd64\MSBuild.exe"
    )

    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            return $candidate
        }
    }

    throw "MSBuild.exe not found. Install Visual Studio Build Tools with Windows App SDK support."
}

function Resolve-Vpk {
    $command = Get-Command vpk -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    dotnet tool install --global vpk --version 0.0.1298 | Out-Null
    $command = Get-Command vpk -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    throw "vpk not found and automatic installation failed."
}

function Ensure-RustTarget {
    param([string]$Target)

    $installedTargets = & rustup target list --installed
    if ($installedTargets -notcontains $Target) {
        & rustup target add $Target
    }
}

function Get-BuildMetadata {
    param([string]$Platform)

    switch ($Platform) {
        "x86" {
            return @{
                RuntimeId = "win-x86"
                Channel = "win-x86"
                RustTarget = "i686-pc-windows-msvc"
            }
        }
        "x64" {
            return @{
                RuntimeId = "win-x64"
                Channel = "win-x64"
                RustTarget = "x86_64-pc-windows-msvc"
            }
        }
        default {
            throw "Unsupported platform '$Platform'. Use x86 or x64."
        }
    }
}

function Download-ReleaseHistory {
    param(
        [string]$Vpk,
        [string]$OutputDir,
        [string]$Channel
    )

    if ([string]::IsNullOrWhiteSpace($RepoUrl)) {
        return
    }

    $arguments = @(
        "download", "github",
        "--repoUrl", $RepoUrl,
        "--outputDir", $OutputDir,
        "--channel", $Channel
    )

    if (-not [string]::IsNullOrWhiteSpace($GitHubToken)) {
        $arguments += @("--token", $GitHubToken)
    }

    try {
        & $Vpk @arguments | Out-Host
    }
    catch {
        Write-Warning "Unable to download existing release history for channel '$Channel'. Continuing without delta history."
    }
}

function Remove-CurrentVersionRelease {
    param(
        [string]$PackageDir,
        [string]$Channel,
        [string]$Version
    )

    if (-not (Test-Path $PackageDir)) {
        return
    }

    $escapedVersion = [regex]::Escape($Version)
    $escapedChannel = [regex]::Escape($Channel)
    $packagePattern = "^VaultPilot-$escapedVersion-$escapedChannel-(full|delta)\.nupkg$"

    Get-ChildItem -LiteralPath $PackageDir -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match $packagePattern } |
        ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force }

    $releasesFile = Join-Path $PackageDir "RELEASES-$Channel"
    if (Test-Path $releasesFile) {
        $remainingLines = @(Get-Content $releasesFile | Where-Object {
            $_ -notmatch " VaultPilot-$escapedVersion-$escapedChannel-(full|delta)\.nupkg "
        })
        if ($remainingLines.Count -gt 0) {
            Set-Content -LiteralPath $releasesFile -Value $remainingLines
        }
        else {
            Remove-Item -LiteralPath $releasesFile -Force
        }
    }

    $releasesJsonFile = Join-Path $PackageDir "releases.$Channel.json"
    if (Test-Path $releasesJsonFile) {
        $releasesJson = Get-Content $releasesJsonFile -Raw | ConvertFrom-Json
        $remainingAssets = @($releasesJson.Assets | Where-Object { $_.Version -ne $Version })
        if ($remainingAssets.Count -gt 0) {
            @{ Assets = $remainingAssets } | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $releasesJsonFile
        }
        else {
            Remove-Item -LiteralPath $releasesJsonFile -Force
        }
    }

    $assetsJsonFile = Join-Path $PackageDir "assets.$Channel.json"
    if (Test-Path $assetsJsonFile) {
        $assetsJson = @(Get-Content $assetsJsonFile -Raw | ConvertFrom-Json)
        $remainingAssets = @($assetsJson | Where-Object {
            $_.RelativeFileName -notmatch $packagePattern
        })
        if ($remainingAssets.Count -gt 0) {
            $remainingAssets | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $assetsJsonFile
        }
        else {
            Remove-Item -LiteralPath $assetsJsonFile -Force
        }
    }
}

if (-not (Test-Path $cargoToml)) {
    throw "Cargo.toml not found at $cargoToml"
}

$resolvedVersion = if ($Version) { $Version } else { Get-VersionFromCargoToml -Path $cargoToml }
$msbuild = Resolve-MSBuild
$vpk = Resolve-Vpk
$Platforms = $Platforms | ForEach-Object { $_ -split "," } | ForEach-Object { $_.Trim() } | Where-Object { $_ }

New-Item -ItemType Directory -Force -Path $artifactsRoot | Out-Null

foreach ($platform in $Platforms) {
    $build = Get-BuildMetadata -Platform $platform
    Ensure-RustTarget -Target $build.RustTarget

    $publishDir = Join-Path $artifactsRoot "publish\$($build.RuntimeId)"
    $packageDir = Join-Path $artifactsRoot "packages\$($build.Channel)"
    if (Test-Path $publishDir) {
        Remove-Item -LiteralPath $publishDir -Recurse -Force
    }

    if (-not $FetchReleaseHistory -and (Test-Path $packageDir)) {
        Remove-Item -LiteralPath $packageDir -Recurse -Force
    }

    New-Item -ItemType Directory -Force -Path $publishDir, $packageDir | Out-Null

    if ($FetchReleaseHistory) {
        Download-ReleaseHistory -Vpk $vpk -OutputDir $packageDir -Channel $build.Channel
        Remove-CurrentVersionRelease -PackageDir $packageDir -Channel $build.Channel -Version $resolvedVersion
    }

    Write-Host "Publishing self-contained WinUI application for $platform..."
    & $msbuild $projectFile `
        /restore `
        /t:Publish `
        /p:Configuration=Release `
        /p:Platform=$platform `
        /p:RuntimeIdentifier=$($build.RuntimeId) `
        /p:SelfContained=true `
        /p:WindowsAppSDKSelfContained=true `
        /p:PublishDir=$publishDir
    if ($LASTEXITCODE -ne 0) {
        throw "MSBuild publish failed for $platform."
    }

    if (-not (Test-Path $publishDir)) {
        throw "Publish output directory not found for ${platform}: $publishDir"
    }

    Write-Host "Packing Velopack release for $platform..."
    & $vpk pack `
        --packId VaultPilot `
        --packVersion $resolvedVersion `
        --packDir $publishDir `
        --outputDir $packageDir `
        --mainExe VaultPilot.WinUI.exe `
        --runtime $($build.RuntimeId) `
        --channel $($build.Channel) `
        --packTitle VaultPilot `
        --packAuthors jy `
        --icon $iconPath
    if ($LASTEXITCODE -ne 0) {
        throw "Velopack packaging failed for $platform."
    }
}

Write-Host ""
Write-Host "Build complete. Packages:"
Get-ChildItem (Join-Path $artifactsRoot "packages") -Recurse |
    Where-Object { -not $_.PSIsContainer } |
    Sort-Object FullName |
    ForEach-Object { Write-Host " - $($_.FullName)" }
