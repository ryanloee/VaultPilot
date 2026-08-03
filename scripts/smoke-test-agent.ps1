<#
.SYNOPSIS
    Smoke test for the VaultPilot Rust backend agent (vaultpilot-agent.exe).
    Launches the agent, exercises the JSON-RPC protocol end-to-end, and
    verifies the process survives a heartbeat window without crashing.

.DESCRIPTION
    This script:
    1. Launches vaultpilot-agent.exe with an isolated LOCALAPPDATA/APPDATA/HOME
    2. Sends "ping" -> expects {"ok": true}
    3. Sends "getSettings" -> expects a settings object with vaultDir
    4. Sends "listNotes" -> expects an array
    5. Sends "ping" every 5s for $HeartbeatSeconds to prove the agent
       does not die under sustained request load
    6. Scans agent-crash.log for panic messages
    7. Returns exit code 0 on success, 1 on failure

    This is the CI gate that catches "backend disconnect" class regressions:
    agent won't start, JSON-RPC protocol breakage, startup crash, panics.

.PARAMETER ExePath
    Path to vaultpilot-agent.exe

.PARAMETER HeartbeatSeconds
    How long to keep pinging before declaring the agent healthy (default: 20)

.PARAMETER TimeoutSeconds
    Per-request response timeout in seconds (default: 15)
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath,

    [int]$HeartbeatSeconds = 20,

    [int]$TimeoutSeconds = 15
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $ExePath)) {
    Write-Error "Agent exe not found: $ExePath"
    exit 1
}

$exeDir = Split-Path -Parent $ExePath
$processName = [System.IO.Path]::GetFileNameWithoutExtension($ExePath)

# Isolated profile dirs so the smoke test never touches the runner's real
# vault data and can cleanly read agent-crash.log afterwards.
$fakeRoot = Join-Path $env:TEMP "vaultpilot-agent-smoke-$(Get-Random)"
$fakeAppData = Join-Path $fakeRoot "AppData"
$fakeHome = Join-Path $fakeRoot "Home"
New-Item -ItemType Directory -Force -Path $fakeAppData | Out-Null
New-Item -ItemType Directory -Force -Path $fakeHome | Out-Null

Write-Host "=== VaultPilot Agent Smoke Test ==="
Write-Host "Exe:       $ExePath"
Write-Host "Heartbeat: ${HeartbeatSeconds}s"
Write-Host "Isolated:  $fakeRoot"
Write-Host ""

# Kill leftover agent processes from previous smoke runs
Get-Process -Name $processName -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $ExePath
$psi.WorkingDirectory = $exeDir
$psi.UseShellExecute = $false
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true

foreach ($key in [System.Environment]::GetEnvironmentVariables().Keys) {
    $psi.EnvironmentVariables[$key] = [System.Environment]::GetEnvironmentVariable($key)
}
$psi.EnvironmentVariables["LOCALAPPDATA"] = $fakeAppData
$psi.EnvironmentVariables["APPDATA"] = $fakeAppData
$psi.EnvironmentVariables["HOME"] = $fakeHome
$psi.EnvironmentVariables["USERPROFILE"] = $fakeHome

Write-Host "Starting agent process..."
try {
    $process = [System.Diagnostics.Process]::Start($psi)
} catch {
    Write-Error "Failed to start agent: $_"
    exit 1
}
Write-Host "Agent started with PID: $($process.Id)"
Write-Host ""

# Reads one stdout line with a per-line wait budget. Returns $null on EOF
# (agent died) or when the budget elapses without a complete line.
# Reuses a single pending ReadLineAsync task across calls: .NET throws
# "The stream is currently in use by a previous operation" if a new async
# read is started while the previous one is still pending, which happened
# on every cold start (agent not ready -> first read stays pending).
$script:pendingStdoutRead = $null

function Read-ResponseLine {
    param([int]$WaitMs)

    if ($process.HasExited) {
        return $null
    }
    if ($null -eq $script:pendingStdoutRead) {
        $script:pendingStdoutRead = $process.StandardOutput.ReadLineAsync()
    }
    if ($script:pendingStdoutRead.Wait($WaitMs)) {
        $line = $script:pendingStdoutRead.Result
        $script:pendingStdoutRead = $null
        return $line
    }
    return $null
}

function Send-Request {
    param(
        [string]$Method,
        [object]$Params,
        [string]$RequestId,
        [int]$TimeoutSec
    )

    $payload = @{ id = $RequestId; method = $Method; params = $Params } | ConvertTo-Json -Compress
    try {
        $process.StandardInput.WriteLine($payload)
        $process.StandardInput.Flush()
    } catch {
        Write-Host "!!! FAILED to write '$Method' to agent stdin: $_ !!!"
        return $null
    }

    # Wait for the response line matching our request id (skip event lines).
    # A null line just means the 1s read window elapsed with no complete
    # line yet - keep waiting until the overall deadline unless the agent
    # process has exited.
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $line = Read-ResponseLine -WaitMs 1000
        if ($null -eq $line) {
            if ($process.HasExited) {
                Write-Host "!!! AGENT PROCESS EXITED (code $($process.ExitCode)) while waiting for '$Method' !!!"
                return $null
            }
            continue
        }
        $line = $line.Trim()
        if ($line.Length -eq 0) { continue }
        try {
            $obj = $line | ConvertFrom-Json
        } catch {
            Write-Host "Non-JSON stdout line (ignored): $line"
            continue
        }
        if ($obj.id -eq $RequestId) {
            return $obj
        }
        # Event lines (agentStatus) carry no matching id - ignore and keep reading
    }

    Write-Host "!!! TIMEOUT waiting for response to '$Method' (${TimeoutSec}s) !!!"
    return $null
}

function Assert-Request {
    param(
        [string]$Method,
        [object]$Params,
        [string]$What
    )

    $reqId = [guid]::NewGuid().ToString("N")
    $resp = Send-Request -Method $Method -Params $Params -RequestId $reqId -TimeoutSec $TimeoutSeconds
    if ($null -eq $resp) {
        throw "FAIL: $What - no response"
    }
    if ($resp.error) {
        throw "FAIL: $What - agent error: $($resp.error.message)"
    }
    Write-Host "  PASS: $Method"
    return $resp
}

# -- Test sequence --
$failures = @()

# 0. Readiness probe: the agent cold-starts storage/keychain init, which can
#    take several seconds on a fresh CI cache. Wait (with a bounded retry)
#    until it answers ping instead of failing the very first request.
Write-Host ""
Write-Host "Probing for agent readiness (up to 90s)..."
$ready = $false
$probeDeadline = (Get-Date).AddSeconds(90)
while ((Get-Date) -lt $probeDeadline) {
    $probeId = [guid]::NewGuid().ToString("N")
    $r = $null
    try {
        $r = Send-Request -Method "ping" -Params @{} -RequestId $probeId -TimeoutSec 10
    } catch {}
    if ($r -and $r.result.ok -eq $true) {
        $ready = $true
        Write-Host "Agent ready."
        break
    }
    Start-Sleep -Seconds 3
}
if (-not $ready) {
    $failures += "agent not ready within 90s (cold start exceeded window)"
}

# 1. ping
if ($ready) {
    try {
        $r = Assert-Request -Method "ping" -Params @{} -What "ping"
        if (-not ($r.result.ok -eq $true)) {
            throw "ping result not ok: $($r.result | ConvertTo-Json -Compress)"
        }
    } catch { $failures += $_.Exception.Message }
}

# 2. getSettings - proves storage init + settings plumbing
try {
    $r = Assert-Request -Method "getSettings" -Params @{} -What "getSettings"
    if (-not $r.result.vaultDir) {
        throw "getSettings missing vaultDir"
    }
} catch { $failures += $_.Exception.Message }

# 3. listNotes - proves storage layer responds
try {
    $r = Assert-Request -Method "listNotes" -Params @{ limit = 5 } -What "listNotes"
    if ($null -eq $r.result) {
        throw "listNotes returned null result"
    }
} catch { $failures += $_.Exception.Message }

# 4. Heartbeat - sustained request load must not kill the agent
Write-Host ""
Write-Host "Heartbeat: pinging every 5s for ${HeartbeatSeconds}s..."
$hbFailures = 0
$hbPings = 0
$hbDeadline = (Get-Date).AddSeconds($HeartbeatSeconds)
while ((Get-Date) -lt $hbDeadline) {
    Start-Sleep -Seconds 5
    $reqId = [guid]::NewGuid().ToString("N")
    $resp = Send-Request -Method "ping" -Params @{} -RequestId $reqId -TimeoutSec 10
    $hbPings++
    if ($null -eq $resp) {
        $hbFailures++
        Write-Host "  heartbeat ping FAILED"
    } else {
        $remaining = [math]::Round((New-TimeSpan -Start (Get-Date) -End $hbDeadline).TotalSeconds)
        Write-Host "  heartbeat ping ok (${remaining}s remaining)"
    }
}
if ($hbFailures -gt 0) {
    $failures += "Heartbeat: $hbFailures of $hbPings pings failed"
}

# -- Verify no panic in agent-crash.log --
Write-Host ""
Write-Host "Checking agent-crash.log..."
$crashLog = Join-Path $fakeAppData "com.local.vaultpilot\agent-crash.log"
if (Test-Path $crashLog) {
    $crashContent = Get-Content $crashLog -Raw
    Write-Host "--- agent-crash.log contents ---"
    Write-Host $crashContent
    Write-Host "--- end agent-crash.log ---"
    if ($crashContent -match "panic") {
        $failures += "agent-crash.log contains panic"
    }
} else {
    Write-Host "No agent-crash.log (no panics recorded)."
}

# -- Drain stderr (best-effort) and check for panic text --
Write-Host ""
Write-Host "Checking stderr..."
# IMPORTANT: the agent is a long-running JSON-RPC server that never exits on
# its own, so ReadToEnd() would block until EOF forever, hanging the whole CI
# job until GitHub's 6h timeout. Kill the process first so the stderr pipe
# closes, then drain whatever was buffered.
try { $process.Kill() } catch {}
try { $process.WaitForExit(5000) } catch {}
try {
    $stderrAll = $process.StandardError.ReadToEnd()
    if ($stderrAll) {
        Write-Host "--- stderr ---"
        Write-Host $stderrAll
        Write-Host "--- end stderr ---"
        if ($stderrAll -match "panic") {
            $failures += "stderr contains panic"
        }
    } else {
        Write-Host "No stderr output."
    }
} catch {
    Write-Host "Could not read stderr: $_"
}

# -- Cleanup --
try { $process.Kill() } catch {}
try { $process.WaitForExit(5000) } catch {}
Remove-Item -LiteralPath $fakeRoot -Recurse -Force -ErrorAction SilentlyContinue

Write-Host ""
if ($failures.Count -gt 0) {
    Write-Host "=== AGENT SMOKE TEST FAILED ==="
    foreach ($f in $failures) {
        Write-Host "  - $f"
    }
    exit 1
} else {
    Write-Host "=== AGENT SMOKE TEST PASSED ==="
    exit 0
}
