# test-release-triggers.ps1 - Unit tests for scripts/release/Validate-ReleaseTrigger.ps1.
#
# ASCII-only; Windows PowerShell 5.1 compatible. No network access and no gh/git
# calls are ever made: every case runs the trigger guard in a child powershell.exe
# (5.1) process with explicit parameters, then asserts the exit code.
#
# Cases:
#   (a) valid tag push ref + matching tag/version          -> PASS (exit 0)
#   (b) workflow_dispatch from branch ref                  -> FAIL (exit 1)
#   (c) workflow_dispatch without any ref (empty)          -> FAIL (exit 1)
#   (d) tag name does not match version                    -> FAIL (exit 1)
#   (e) version has no 'v' prefix but tag does             -> FAIL (exit 1)

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$scriptPath = Join-Path $PSScriptRoot 'Validate-ReleaseTrigger.ps1'
if (-not (Test-Path -LiteralPath $scriptPath)) {
  throw "Trigger guard script not found: $scriptPath"
}

$script:passCount = 0
$script:failCount = 0

function Invoke-TriggerCase {
  param(
    [string]$Name,
    [int]$ExpectedExitCode,
    [string]$Ref,
    [string]$Tag,
    [string]$Version
  )

  # The guard script exits via `exit 1` which only sets $LASTEXITCODE in the
  # calling session; the driver must forward it so the child process exit code
  # (asserted below) reflects the guard decision.
  $cmd = "& '$scriptPath' -Ref '$Ref' -Tag '$Tag' -Version '$Version'`n exit `$LASTEXITCODE"
  $driver = Join-Path ([System.IO.Path]::GetTempPath()) (
    'clashedge-trigger-case-' + [Guid]::NewGuid().ToString('N') + '.ps1')
  Set-Content -Path $driver -Value $cmd -Encoding ASCII

  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try {
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $driver 2>&1
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $prevEap
    Remove-Item -LiteralPath $driver -Force -ErrorAction SilentlyContinue
  }

  if ($exitCode -eq $ExpectedExitCode) {
    $script:passCount++
    Write-Host ("PASS  {0}" -f $Name)
    return $true
  }

  $script:failCount++
  Write-Host ("FAIL  {0}  (exit code: expected {1}, got {2})" -f $Name, $ExpectedExitCode, $exitCode)
  foreach ($line in @($output)) { Write-Host ("      | {0}" -f "$line") }
  return $false
}

# (a) valid tag push: ref = refs/tags/v1.0.8, tag v1.0.8, version 1.0.8 -> pass.
Invoke-TriggerCase `
  -Name '(a) tag push + matching version -> PASS' `
  -ExpectedExitCode 0 `
  -Ref 'refs/tags/v1.0.8' -Tag 'v1.0.8' -Version '1.0.8' | Out-Null

# (b) workflow_dispatch from branch (ref = refs/heads/main) -> reject.
Invoke-TriggerCase `
  -Name '(b) workflow_dispatch from branch -> FAIL' `
  -ExpectedExitCode 1 `
  -Ref 'refs/heads/main' -Tag 'v1.0.8' -Version '1.0.8' | Out-Null

# (c) workflow_dispatch without ref (empty string) -> reject.
Invoke-TriggerCase `
  -Name '(c) workflow_dispatch without ref -> FAIL' `
  -ExpectedExitCode 1 `
  -Ref '' -Tag 'v1.0.8' -Version '1.0.8' | Out-Null

# (d) tag does not match version -> reject.
Invoke-TriggerCase `
  -Name '(d) tag/version mismatch -> FAIL' `
  -ExpectedExitCode 1 `
  -Ref 'refs/tags/v1.0.8' -Tag 'v1.0.8' -Version '1.0.9' | Out-Null

# (e) version missing v prefix while tag has it -> reject.
Invoke-TriggerCase `
  -Name '(e) version prefix mismatch -> FAIL' `
  -ExpectedExitCode 1 `
  -Ref 'refs/tags/v1.0.8' -Tag 'v1.0.8' -Version 'v1.0.8' | Out-Null

Write-Host ''
Write-Host ("Results: {0} passed, {1} failed." -f $script:passCount, $script:failCount)
if ($script:failCount -gt 0) { exit 1 }
exit 0
