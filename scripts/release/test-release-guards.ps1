# test-release-guards.ps1 - Unit tests for scripts/release/Protect-ReleaseTag.ps1.
#
# ASCII-only; Windows PowerShell 5.1 compatible. No network access and no gh/git
# calls are ever made: every case runs the guard script in a child
# powershell.exe (5.1) process with -WhatIfOnly and injected mock inputs
# (-ReleaseViewJson / -TagCommitJson), then asserts the exit code and the
# GUARD_DECISION line.
#
# Cases:
#   (a) no existing release + tag commit == build commit     -> CREATE (pass)
#   (b) published (non-draft) release exists                 -> REJECT
#   (c) draft, targetCommitish and tag commit match          -> DELETE_RECREATE_DRAFT (pass)
#   (d) draft but targetCommitish differs from build commit  -> REJECT
#   (e) tag points at a different commit (tag moved)         -> REJECT
#   (f) release targetCommitish is a branch name, not sha    -> REJECT
#   (g) defense in depth: published release AND moved tag    -> REJECT

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$guardScript = Join-Path $PSScriptRoot 'Protect-ReleaseTag.ps1'
if (-not (Test-Path -LiteralPath $guardScript)) {
  throw "Guard script not found: $guardScript"
}

$script:passCount = 0
$script:failCount = 0

function Invoke-GuardCase {
  param(
    [string]$Name,
    [int]$ExpectedExitCode,
    [string]$ExpectedDecision,
    [string]$Tag = 'v1.0.7',
    [string]$Sha = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    [AllowEmptyString()][string]$ReleaseViewJson,
    [AllowEmptyString()][string]$TagCommitJson
  )

  # JSON on a command line is a quoting minefield; a per-case driver script
  # avoids it entirely (repo paths contain spaces, so quote them too).
  # The guard script exits via `exit 1` which only sets $LASTEXITCODE in the
  # calling session; the driver must forward it so the child process exit code
  # (asserted below) reflects the guard decision.
  $cmd = "& '$guardScript' -Tag '$Tag' -Sha '$Sha' -WhatIfOnly"
  if ($PSBoundParameters.ContainsKey('ReleaseViewJson')) {
    $cmd += " -ReleaseViewJson '" + ($ReleaseViewJson -replace "'", "''") + "'"
  }
  if ($PSBoundParameters.ContainsKey('TagCommitJson')) {
    $cmd += " -TagCommitJson '" + ($TagCommitJson -replace "'", "''") + "'"
  }
  $cmd += "`n exit `$LASTEXITCODE"
  $driver = Join-Path ([System.IO.Path]::GetTempPath()) (
    'clashedge-guard-case-' + [Guid]::NewGuid().ToString('N') + '.ps1')
  Set-Content -Path $driver -Value $cmd -Encoding ASCII

  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try {
    # Child process = real exit-code semantics + exercises the guard script
    # under Windows PowerShell 5.1 exactly as CI would.
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $driver 2>&1
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $prevEap
    Remove-Item -LiteralPath $driver -Force -ErrorAction SilentlyContinue
  }

  $decision = ''
  foreach ($line in @($output)) {
    $text = "$line"
    if ($text -match 'GUARD_DECISION=(\S+)') { $decision = $Matches[1] }
  }

  $problems = @()
  if ($exitCode -ne $ExpectedExitCode) {
    $problems += ("exit code: expected {0}, got {1}" -f $ExpectedExitCode, $exitCode)
  }
  if ($decision -ne $ExpectedDecision) {
    $problems += ("decision: expected '{0}', got '{1}'" -f $ExpectedDecision, $decision)
  }

  if ($problems.Count -eq 0) {
    $script:passCount++
    Write-Host ("PASS  {0}" -f $Name)
    return $true
  }

  $script:failCount++
  Write-Host ("FAIL  {0}  ({1})" -f $Name, ($problems -join '; '))
  foreach ($line in @($output)) { Write-Host ("      | {0}" -f "$line") }
  return $false
}

$shaA = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
$shaB = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'

# (a) no existing release; tag points at the build commit -> proceed to create.
Invoke-GuardCase `
  -Name '(a) no existing release + tag commit == build commit -> CREATE' `
  -ExpectedExitCode 0 -ExpectedDecision 'CREATE' `
  -Sha $shaA `
  -ReleaseViewJson '' `
  -TagCommitJson ('{"ref":"refs/tags/v1.0.7","object":{"sha":"' + $shaA + '","type":"commit"}}') | Out-Null

# (b) release exists and is already published -> reject, never overwrite.
Invoke-GuardCase `
  -Name '(b) published (non-draft) release exists -> REJECT' `
  -ExpectedExitCode 1 -ExpectedDecision 'REJECT' `
  -Sha $shaA `
  -ReleaseViewJson ('{"isDraft":false,"targetCommitish":"' + $shaA + '"}') `
  -TagCommitJson ('{"object":{"sha":"' + $shaA + '","type":"commit"}}') | Out-Null

# (c) unpublished draft of the exact same commit -> delete+recreate allowed.
Invoke-GuardCase `
  -Name '(c) draft with matching commit -> DELETE_RECREATE_DRAFT' `
  -ExpectedExitCode 0 -ExpectedDecision 'DELETE_RECREATE_DRAFT' `
  -Sha $shaA `
  -ReleaseViewJson ('{"isDraft":true,"targetCommitish":"' + $shaA + '"}') `
  -TagCommitJson ('{"object":{"sha":"' + $shaA + '","type":"commit"}}') | Out-Null

# (d) draft exists but belongs to a different commit -> reject.
Invoke-GuardCase `
  -Name '(d) draft with different targetCommitish -> REJECT' `
  -ExpectedExitCode 1 -ExpectedDecision 'REJECT' `
  -Sha $shaA `
  -ReleaseViewJson ('{"isDraft":true,"targetCommitish":"' + $shaB + '"}') `
  -TagCommitJson ('{"object":{"sha":"' + $shaA + '","type":"commit"}}') | Out-Null

# (e) tag was moved: refs/tags/<tag> now points at a different commit.
Invoke-GuardCase `
  -Name '(e) tag moved to a different commit -> REJECT' `
  -ExpectedExitCode 1 -ExpectedDecision 'REJECT' `
  -Sha $shaA `
  -ReleaseViewJson '' `
  -TagCommitJson ('{"object":{"sha":"' + $shaB + '","type":"commit"}}') | Out-Null

# (f) draft targetCommitish is a branch name (not the built commit) -> reject.
Invoke-GuardCase `
  -Name '(f) release targetCommitish mismatch (branch name) -> REJECT' `
  -ExpectedExitCode 1 -ExpectedDecision 'REJECT' `
  -Sha $shaA `
  -ReleaseViewJson '{"isDraft":true,"targetCommitish":"main"}' `
  -TagCommitJson ('{"object":{"sha":"' + $shaA + '","type":"commit"}}') | Out-Null

# (g) defense in depth: published release AND moved tag -> still reject.
Invoke-GuardCase `
  -Name '(g) published release AND moved tag -> REJECT' `
  -ExpectedExitCode 1 -ExpectedDecision 'REJECT' `
  -Sha $shaA `
  -ReleaseViewJson ('{"isDraft":false,"targetCommitish":"' + $shaB + '"}') `
  -TagCommitJson ('{"object":{"sha":"' + $shaB + '","type":"commit"}}') | Out-Null

Write-Host ''
Write-Host ("Results: {0} passed, {1} failed." -f $script:passCount, $script:failCount)
if ($script:failCount -gt 0) { exit 1 }
exit 0
