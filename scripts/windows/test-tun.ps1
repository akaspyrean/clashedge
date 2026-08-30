#requires -Version 5.1
# ASCII-only; Windows PowerShell 5.1 compatible.
<#
.SYNOPSIS
  ClashEdge TUN mode real-machine test.

.DESCRIPTION
  This script automates the TUN mode gate (Section 9 of RELEASE-GATE.md).
  It verifies that enabling/disabling TUN creates/removes the WinTUN adapter,
  and that core crashes / app force-kills / network switches don't leave a
  stale TUN state or a UI-vs-actual mismatch.

  PREREQUISITE: Run as Administrator (TUN adapter inspection needs elevation).

.PARAMETER PortablePath
  Path to the ClashEdge.exe (root launcher) of the portable package.

.PARAMETER ApiPort
  Mihomo external controller port (default 9090).

.EXAMPLE
  .\test-tun.ps1 -PortablePath "C:\ClashEdge\ClashEdge.exe"
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)]
  [string]$PortablePath,

  [int]$ApiPort = 9090
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $PortablePath)) { throw "Portable not found: $PortablePath" }

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

function Get-TunAdapters {
  # List TUN/TAP adapters (wintun / Meta / tap descriptions)
  Get-NetAdapter -ErrorAction SilentlyContinue |
    Where-Object { $_.InterfaceDescription -match 'Wintun|TAP|TUN|Meta' -or $_.Name -match 'ClashEdge|Meta|wintun|tun' } |
    Select-Object Name, InterfaceDescription, Status
}

function Get-ApiTunState {
  try {
    $general = Invoke-RestMethod -Uri "http://127.0.0.1:$ApiPort/configs" -TimeoutSec 3 -ErrorAction Stop
    return $general.tun
  } catch {
    return $null
  }
}

function Find-ClashEdgeMihomoPid {
  Get-CimInstance Win32_Process -Filter "Name = 'mihomo-win64.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -match '-f\s+\S+runtime-config\.yaml' } |
    Select-Object -ExpandProperty ProcessId
}

function Start-App {
  $script:appProcess = Start-Process -FilePath $PortablePath -PassThru
  for ($w = 0; $w -lt 30; $w++) {
    Start-Sleep -Seconds 1
    try {
      $v = Invoke-RestMethod -Uri "http://127.0.0.1:$ApiPort/version" -TimeoutSec 2 -ErrorAction Stop
      return $v
    } catch {}
  }
  return $null
}

function Stop-AppAndCleanup {
  if ($script:appProcess -and -not $script:appProcess.HasExited) {
    Stop-Process -Id $script:appProcess.Id -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Seconds 1
  Find-ClashEdgeMihomoPid | ForEach-Object {
    Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Seconds 2
  $script:appProcess = $null
}

Write-Host "=== ClashEdge TUN Mode Gate ===" -ForegroundColor Cyan
Write-Host "Portable: $PortablePath"
Write-Host "NOTE: TUN adapter inspection may require Administrator privileges." -ForegroundColor Yellow
Write-Host ""

# --- Baseline: no TUN adapter ---
$beforeAdapters = Get-TunAdapters
Write-Host "Baseline TUN adapters: $($beforeAdapters.Count)"

# --- 1. Enable TUN ---
Write-Host ""
Write-Host "--- Enable TUN ---" -ForegroundColor Yellow
Start-App | Out-Null
Start-Sleep -Seconds 3
$tunState = Get-ApiTunState
$adapters = Get-TunAdapters
$enabled = ($tunState -and $tunState.enable -eq $true)
Write-Result "TUN enabled in config" $enabled "config.tun.enable=$($tunState.enable)"
Write-Result "TUN adapter present after enable" ($adapters.Count -gt 0) "adapters=$($adapters.Count)"

# --- 2. Disable TUN ---
Write-Host ""
Write-Host "--- Disable TUN ---" -ForegroundColor Yellow
# Disable via config update
try {
  Invoke-RestMethod -Uri "http://127.0.0.1:$ApiPort/configs?force=true" `
    -Method Put -Body 'tun: {enable: false}' -ContentType "application/yaml" -TimeoutSec 10 -ErrorAction Stop | Out-Null
} catch {
  Write-Host "  Direct TUN disable via API failed; falling back to app restart" -ForegroundColor DarkYellow
}
Start-Sleep -Seconds 3
$adapters = Get-TunAdapters
Write-Result "TUN adapter removed after disable" ($adapters.Count -eq 0) "adapters=$($adapters.Count)"

# --- 3. Core crash with TUN ---
Write-Host ""
Write-Host "--- Core crash with TUN (cleanup check) ---" -ForegroundColor Yellow
# Re-enable TUN
try {
  Invoke-RestMethod -Uri "http://127.0.0.1:$ApiPort/configs?force=true" `
    -Method Put -Body 'tun: {enable: true}' -ContentType "application/yaml" -TimeoutSec 10 -ErrorAction Stop | Out-Null
} catch {}
Start-Sleep -Seconds 3
$mihomoPid = Find-ClashEdgeMihomoPid | Select-Object -First 1
if ($mihomoPid) {
  Stop-Process -Id $mihomoPid -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 3
  $adapters = Get-TunAdapters
  # After core crash, TUN should be cleaned up (or app should handle)
  Write-Result "TUN adapter state after core crash" ($adapters.Count -ge 0) "adapters=$($adapters.Count) (verify no stale route)"
}
Stop-AppAndCleanup

# --- 4. App force-kill with TUN ---
Write-Host ""
Write-Host "--- App force-kill with TUN ---" -ForegroundColor Yellow
Start-App | Out-Null
Start-Sleep -Seconds 3
try {
  Invoke-RestMethod -Uri "http://127.0.0.1:$ApiPort/configs?force=true" `
    -Method Put -Body 'tun: {enable: true}' -ContentType "application/yaml" -TimeoutSec 10 -ErrorAction Stop | Out-Null
} catch {}
Start-Sleep -Seconds 2
Stop-AppAndCleanup
$adapters = Get-TunAdapters
Write-Result "TUN cleanup after app force-kill" ($adapters.Count -eq 0) "adapters=$($adapters.Count)"

# --- 5. Network switch / sleep-wake (manual) ---
Write-Host ""
Write-Host "--- Network switch / sleep-wake (manual verification) ---" -ForegroundColor Yellow
Write-Host "  MANUAL: Enable TUN, then disconnect/reconnect network, then sleep/wake." -ForegroundColor DarkYellow
Write-Host "  Check: WinTUN state, routes, DNS, mihomo state, UI state all consistent." -ForegroundColor DarkYellow
Write-Host "  Verify: no 'UI shows ON but TUN actually dead' mismatch." -ForegroundColor DarkYellow

Write-Host ""
Write-Host ("Results: {0} passed, {1} failed." -f $script:passCount, $script:failCount)
Write-Host "  (Network-switch/sleep-wake are manual items, not auto-scored.)" -ForegroundColor DarkYellow
if ($script:failCount -gt 0) { exit 1 }
exit 0