<#
.SYNOPSIS
    Smoke test for VaultPilot WinUI application.
    Launches the .exe on a CI runner and detects startup crashes
    (XamlParseException, missing DLLs, unhandled exceptions).

.DESCRIPTION
    This script:
    1. Launches VaultPilot.WinUI.exe
    2. Monitors the process for $TimeoutSeconds
    3. If the process exits within that window -> startup crash detected
    4. Checks crash.log for unhandled exceptions
    5. Returns exit code 0 on success, 1 on failure

.PARAMETER ExePath
    Path to VaultPilot.WinUI.exe

.PARAMETER TimeoutSeconds
    How long to wait before declaring the app healthy (default: 8)
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath,

    [int]$TimeoutSeconds = 8
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $ExePath)) {
    Write-Error "Exe not found: $ExePath"
    exit 1
}

$exeDir = Split-Path -Parent $ExePath
$processName = [System.IO.Path]::GetFileNameWithoutExtension($ExePath)

# Prepare isolated LOCALAPPDATA so we don't interfere with any existing install
# and can cleanly read crash logs
$fakeLocalAppData = Join-Path $env:TEMP "vaultpilot-smoke-$(Get-Random)"
New-Item -ItemType Directory -Force -Path $fakeLocalAppData | Out-Null

$logDir = Join-Path (Join-Path $fakeLocalAppData "com.local.vaultpilot") "logs"

Write-Host "=== VaultPilot WinUI Smoke Test ==="
Write-Host "Exe:      $ExePath"
Write-Host "Timeout:  ${TimeoutSeconds}s"
Write-Host "Fake AppData: $fakeLocalAppData"
Write-Host ""

# Kill any leftover VaultPilot processes
Get-Process -Name $processName -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# Launch with redirected LOCALAPPDATA via environment override
# We use Start-Process with a modified environment to isolate the crash log
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $ExePath
$psi.WorkingDirectory = $exeDir
$psi.UseShellExecute = $false
$psi.RedirectStandardError = $true
$psi.RedirectStandardOutput = $true

# Copy current environment and override LOCALAPPDATA
foreach ($key in [System.Environment]::GetEnvironmentVariables().Keys) {
    $psi.EnvironmentVariables[$key] = [System.Environment]::GetEnvironmentVariable($key)
}
$psi.EnvironmentVariables["LOCALAPPDATA"] = $fakeLocalAppData
$psi.EnvironmentVariables["USERPROFILE"] = $fakeLocalAppData  # some apps use this

Write-Host "Starting process..."
$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

try {
    $process = [System.Diagnostics.Process]::Start($psi)
} catch {
    Write-Error "Failed to start process: $_"
    exit 1
}

$processId = $process.Id
Write-Host "Process started with PID: $processId"

# Poll: if process exits within timeout -> crash
$crashed = $false
$exitCode = $null
$stderrOutput = ""

for ($i = 0; $i -lt $TimeoutSeconds; $i++) {
    Start-Sleep -Seconds 1

    if ($process.HasExited) {
        $crashed = $true
        $exitCode = $process.ExitCode
        $elapsed = $stopwatch.Elapsed.TotalSeconds
        Write-Host ""
        Write-Host "!!! STARTUP CRASH DETECTED !!!"
        Write-Host "Process exited after ${elapsed}s with exit code: $exitCode"

        # Read stderr
        try {
            $stderrOutput = $process.StandardError.ReadToEnd()
        } catch {}

        # Read stdout
        try {
            $stdoutOutput = $process.StandardOutput.ReadToEnd()
            if ($stdoutOutput) {
                Write-Host "stdout: $stdoutOutput"
            }
        } catch {}

        if ($stderrOutput) {
            Write-Host "stderr: $stderrOutput"
        }

        break
    }

    Write-Host "  ... ${i}s elapsed, process still running (PID $processId)"
}

if (-not $crashed) {
    # Process survived the timeout -> healthy
    Write-Host ""
    Write-Host "Process survived ${TimeoutSeconds}s -- app starts OK."

    # Kill it gracefully
    try {
        $process.Kill()
        $process.WaitForExit(5000)
    } catch {}

    $stopwatch.Stop()
}

# Now check crash logs regardless
Write-Host ""
Write-Host "Checking crash logs at: $logDir"
$crashLogPath = Join-Path $logDir "crash.log"

$logFailure = $false
if (Test-Path $crashLogPath) {
    $logContent = Get-Content $crashLogPath -Raw
    Write-Host "--- crash.log contents ---"
    Write-Host $logContent
    Write-Host "--- end crash.log ---"

    # Check for fatal patterns
    if ($logContent -match "XamlParseException" -or
        $logContent -match "TypeLoadException" -or
        $logContent -match "FileNotFoundException" -or
        $logContent -match "DllNotFoundException" -or
        $logContent -match "MissingMethodException") {
        Write-Host ""
        Write-Host "!!! FATAL STARTUP EXCEPTION FOUND IN CRASH LOG !!!"
        $logFailure = $true
    } else {
        Write-Host "Crash log exists but no fatal startup exceptions found."
    }
} else {
    Write-Host "No crash.log found (clean startup)."
}

# Cleanup
Remove-Item -LiteralPath $fakeLocalAppData -Recurse -Force -ErrorAction SilentlyContinue

# Final verdict
Write-Host ""
if ($crashed -or $logFailure) {
    Write-Host "=== SMOKE TEST FAILED ==="
    if ($crashed) {
        Write-Host "Reason: Process exited during startup (exit code $exitCode)"
    }
    if ($logFailure) {
        Write-Host "Reason: Fatal exception found in crash.log"
    }
    exit 1
} else {
    Write-Host "=== SMOKE TEST PASSED ==="
    exit 0
}
