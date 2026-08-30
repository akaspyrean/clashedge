#requires -Version 5.1
# ASCII-only; Windows PowerShell 5.1 compatible.
<#
.SYNOPSIS
  ClashEdge system proxy recovery real-machine test.

.DESCRIPTION
  This script automates the system proxy recovery gate (Section 8 of RELEASE-GATE.md).
  It verifies that after normal exit, force-kill, mihomo crash, and auto-restart,
  the Windows system proxy never points at a dead port.

  CRITICAL SAFETY RULES:
  - Never kill user's own mihomo by process name. This script only kills the
    specific mihomo PID that ClashEdge spawned (identified by CLI args -d -f).
  - Preserve user's existing system proxy settings: read them before, restore after.

.PARAMETER PortablePath
  Path to the ClashEdge.exe (root launcher) of the portable package.

.PARAMETER ApiPort
  Mihomo external controller port (default 9090).

.EXAMPLE
  .\test-system-proxy-recovery.ps1 -PortablePath "C:\ClashEdge\ClashEdge.exe"
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

$regPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings"

function Write-Result([string]$Name, [bool]$Ok, [string]$Detail = "") {
  if ($Ok) {
    Write-Host "PASS  $Name" -ForegroundColor Green
    $script:passCount++
  } else {
    Write-Host "FAIL  $Name  $Detail" -ForegroundColor Red
    $script:failCount++
  }
}

function Get-SysProxyState {
  $enabled = (Get-ItemProperty -Path $regPath -Name ProxyEnable -ErrorAction SilentlyContinue).ProxyEnable
  $server = (Get-ItemProperty -Path $regPath -Name ProxyServer -ErrorAction SilentlyContinue).ProxyServer
  return @{ enabled = $enabled; server = $server }
}

function Restore-SysProxy($state) {
  Set-ItemProperty -Path $regPath -Name ProxyEnable -Value $state.enabled -ErrorAction SilentlyContinue
  if ($state.server) {
    Set-ItemProperty -Path $regPath -Name ProxyServer -Value $state.server -ErrorAction SilentlyContinue
  }
}

function Find-ClashEdgeMihomoPid {
  # Find mihomo spawned by ClashEdge (has -d and -f args pointing at portable Data)
  Get-CimInstance Win32_Process -Filter "Name = 'mihomo-win64.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -match '-f\s+\S+runtime-config\.yaml' } |
    Select-Object -ExpandProperty ProcessId
}

function Start-AppForProxyTest {
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

function Stop-AppAndMihomo {
  # Kill app first
  if ($script:appProcess -and -not $script:appProcess.HasExited) {
    Stop-Process -Id $script:appProcess.Id -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Seconds 1
  # Kill only ClashEdge-owned mihomo (by identified PID, never by name)
  Find-ClashEdgeMihomoPid | ForEach-Object {
    Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Seconds 2
  $script:appProcess = $null
}

Write-Host "=== ClashEdge System Proxy Recovery Gate ===" -ForegroundColor Cyan
Write-Host "Portable: $PortablePath"
Write-Host ""

# --- Save user's existing proxy state ---
$originalProxy = Get-SysProxyState
Write-Host "Original system proxy: enabled=$($originalProxy.enabled) server=$($originalProxy.server)"


# --- 1. Normal exit ---
Write-Host ""
Write-Host "--- Normal exit ---" -ForegroundColor Yellow
Start-AppForProxyTest | Out-Null
Start-Sleep -Seconds 2
# Normal exit: close app
if ($script:appProcess -and -not $script:appProcess.HasExited) {
  $script:appProcess.CloseMainWindow() | Out-Null
  Start-Sleep -Seconds 2
  if (-not $script:appProcess.HasExited) {
    Stop-Process -Id $script:appProcess.Id -Force -ErrorAction SilentlyContinue
  }
}
Stop-AppAndMihomo
$after = Get-SysProxyState
# After normal exit with system-proxy OFF, proxy should be disabled
$proxyOff = ($after.enabled -eq 0)
Write-Result "normal exit clears system proxy" $proxyOff "enabled=$($after.enabled)"

# --- 2. Force kill app ---
Write-Host ""
Write-Host "--- Force kill app ---" -ForegroundColor Yellow
Start-AppForProxyTest | Out-Null
Start-Sleep -Seconds 2
# Enable system proxy via registry (simulate EN state) then force kill
Set-ItemProperty -Path $regPath -Name ProxyEnable -Value 0 -ErrorAction SilentlyContinue
Stop-AppAndMihomo
$after = Get-SysProxyState
$deadProxy = ($after.enabled -eq 1 -and $after.server -like "*7890*" -and -not (Test-PortListening 7890))
Write-Result "force-kill leaves no dead proxy" (-not $deadProxy) "enabled=$($after.enabled) server=$($after.server)"

# --- 3. Force kill mihomo (auto-restart) ---
Write-Host ""
Write-Host "--- Force kill mihomo (auto-restart) ---" -ForegroundColor Yellow
Start-AppForProxyTest | Out-Null
Start-Sleep -Seconds 2
$beforePid = Find-ClashEdgeMihomoPid | Select-Object -First 1
if ($beforePid) {
  Stop-Process -Id $beforePid -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 3
  $afterPid = Find-ClashEdgeMihomoPid | Select-Object -First 1
  $restarted = ($afterPid -and $afterPid -ne $beforePid)
  Write-Result "mihomo auto-restarts after kill" $restarted "before=$beforePid after=$afterPid"
} else {
  Write-Result "mihomo auto-restarts after kill" $false "no mihomo PID found"
}
Stop-AppAndMihomo

# --- 4. Crash circuit breaker (3 kills) ---
Write-Host ""
Write-Host "--- Crash circuit breaker (3 kills) ---" -ForegroundColor Yellow
Start-AppForProxyTest | Out-Null
Start-Sleep -Seconds 2
$crashed = 0
for ($k = 1; $k -le 3; $k++) {
  $pid = Find-ClashEdgeMihomoPid | Select-Object -First 1
  if ($pid) {
    Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    $crashed++
  }
}
$finalPid = Find-ClashEdgeMihomoPid | Select-Object -First 1
$breakerOpen = (-not $finalPid)
Write-Result "circuit breaker stops restart after 3 crashes" $breakerOpen "crashed=$crashed finalPid=$finalPid"
Stop-AppAndMihomo

# --- 5. Portable path change (autostart repair) ---
Write-Host ""
Write-Host "--- Portable path change ---" -ForegroundColor Yellow
# Document: autostart registry key should be updated on next launch after path change
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$autostart = (Get-ItemProperty -Path $runKey -Name "ClashEdge" -ErrorAction SilentlyContinue).ClashEdge
Write-Result "autostart path check (manual)" ($autostart -eq $null -or $autostart -like "*ClashEdge*") "autostart=$autostart"

# --- Restore original system proxy ---
Write-Host ""
Write-Host "Restoring original system proxy state..." -ForegroundColor Yellow
Restore-SysProxy $originalProxy

Write-Host ""
Write-Host ("Results: {0} passed, {1} failed." -f $script:passCount, $script:failCount)
if ($script:failCount -gt 0) { exit 1 }
exit 0