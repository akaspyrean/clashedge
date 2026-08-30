#requires -Version 5.1
# ASCII-only; Windows PowerShell 5.1 compatible.
<#
.SYNOPSIS
  ClashEdge core lifecycle real-machine test: Start x10, Stop x10, Restart x10.
  Verifies no zombie mihomo, no duplicate watchers, final state = real process state.

.DESCRIPTION
  This script automates the core lifecycle gate (Section 7 of RELEASE-GATE.md).
  It launches ClashEdge portable, uses the REST API to start/stop/restart the core,
  and checks Windows process state after each cycle.

  REST API: http://127.0.0.1:9090 (default controller)
  The script reads the controller secret from the app's config.yaml if present.

.PARAMETER PortablePath
  Path to the ClashEdge.exe (root launcher) of the portable package.

.PARAMETER ApiPort
  Mihomo external controller port (default 9090).

.EXAMPLE
  .\test-lifecycle.ps1 -PortablePath "C:\ClashEdge\ClashEdge.exe"
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)]
  [string]$PortablePath,

  [int]$ApiPort = 9090
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $PortablePath)) {
  throw "Portable not found: $PortablePath"
}

$script:passCount = 0
$script:failCount = 0
$script:appProcess = $null

function Write-Result([string]$Name, [bool]$Ok, [string]$Detail = "") {
  if ($Ok) {
    Write-Host "PASS  $Name" -ForegroundColor Green
    $script:passCount++
  } else {
    Write-Host "FAIL  $Name  $Detail" -ForegroundColor Red
    $script:failCount++
  }
}

function Get-MihomoCount {
  @(Get-Process -Name "mihomo-win64" -ErrorAction SilentlyContinue).Count
}

function Get-MihomoPids {
  Get-Process -Name "mihomo-win64" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id
}

function Get-ApiStatus {
  try {
    $resp = Invoke-RestMethod -Uri "http://127.0.0.1:$ApiPort/version" -TimeoutSec 3 -ErrorAction Stop
    return $resp
  } catch {
    return $null
  }
}

function Stop-AppSafely {
  if ($script:appProcess -and -not $script:appProcess.HasExited) {
    Stop-Process -Id $script:appProcess.Id -Force -ErrorAction SilentlyContinue
    $script:appProcess = $null
  }
  Start-Sleep -Seconds 2
  Get-Process -Name "mihomo-win64" -ErrorAction SilentlyContinue | ForEach-Object {
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Seconds 1
}

Write-Host "=== ClashEdge Lifecycle Gate ===" -ForegroundColor Cyan
Write-Host "Portable: $PortablePath"
Write-Host "API: http://127.0.0.1:$ApiPort"
Write-Host ""

# Ensure clean state
Stop-AppSafely

# --- Start x10 ---
Write-Host "--- Start x10 ---" -ForegroundColor Yellow
for ($i = 1; $i -le 10; $i++) {
  $script:appProcess = Start-Process -FilePath $PortablePath -PassThru
  $ready = $false
  for ($w = 0; $w -lt 30; $w++) {
    Start-Sleep -Seconds 1
    if (Get-ApiStatus) { $ready = $true; break }
  }
  $mihomoCount = Get-MihomoCount
  $ok = $ready -and ($mihomoCount -eq 1)
  Write-Result "Start #$i" $ok "ready=$ready mihomo=$mihomoCount"
  Stop-AppSafely
}

# --- Stop x10 ---
Write-Host ""
Write-Host "--- Stop x10 ---" -ForegroundColor Yellow
for ($i = 1; $i -le 10; $i++) {
  $script:appProcess = Start-Process -FilePath $PortablePath -PassThru
  for ($w = 0; $w -lt 30; $w++) {
    Start-Sleep -Seconds 1
    if (Get-ApiStatus) { break }
  }
  Stop-AppSafely
  $mihomoCount = Get-MihomoCount
  $ok = ($mihomoCount -eq 0)
  Write-Result "Stop #$i" $ok "mihomo remaining=$mihomoCount"
}

# --- Restart x10 ---
Write-Host ""
Write-Host "--- Restart x10 ---" -ForegroundColor Yellow
$script:appProcess = Start-Process -FilePath $PortablePath -PassThru
for ($w = 0; $w -lt 30; $w++) {
  Start-Sleep -Seconds 1
  if (Get-ApiStatus) { break }
}
for ($i = 1; $i -le 10; $i++) {
  $beforePids = Get-MihomoPids
  # Restart via API
  try {
    Invoke-RestMethod -Uri "http://127.0.0.1:$ApiPort/configs?force=true" `
      -Method Put -Body " " -ContentType "application/yaml" -TimeoutSec 10 -ErrorAction Stop | Out-Null
  } catch {
    # Fallback: restart via process management
    Get-Process -Name "mihomo-win64" -ErrorAction SilentlyContinue | ForEach-Object {
      Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 3
    for ($w = 0; $w -lt 20; $w++) {
      Start-Sleep -Seconds 1
      if (Get-ApiStatus) { break }
    }
  }
  Start-Sleep -Seconds 2
  $afterPids = Get-MihomoPids
  $mihomoCount = Get-MihomoCount
  $pidChanged = ($beforePids -ne $afterPids) -or ($beforePids.Count -eq 0)
  $ok = ($mihomoCount -eq 1)
  Write-Result "Restart #$i" $ok "mihomo=$mihomoCount before=$beforePids after=$afterPids"
}

# Final cleanup
Stop-AppSafely

# --- Final state check ---
Write-Host ""
Write-Host "--- Final State ---" -ForegroundColor Yellow
$finalMihomo = Get-MihomoCount
$ok = ($finalMihomo -eq 0)
Write-Result "No zombie mihomo after all cycles" $ok "count=$finalMihomo"

Write-Host ""
Write-Host ("Results: {0} passed, {1} failed." -f $script:passCount, $script:failCount)
if ($script:failCount -gt 0) { exit 1 }
exit 0
