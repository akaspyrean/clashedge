# Validate-ReleaseTrigger.ps1 - Release trigger guard (Release workflow safety gate).
#
# Goal: reject two unsafe release triggers before any external side effect:
#   1. workflow_dispatch from a branch (github.ref is a branch, not a tag) — FAIL.
#      A manual dispatch with a branch ref would otherwise create a release named
#      after the branch and publish it as if it were a versioned release.
#   2. tag name does not match the version declared in tauri.conf.json — FAIL.
#      Release ZIP / manifest / updater all key off the version; a mismatch means
#      the published artifacts and the updater path disagree.
#
# Decision matrix (exit code 1 on every REJECT, ASCII-safe output for CI):
#   -Ref not like 'refs/tags/v*'            -> REJECT (trigger from branch/dispatch)
#   -Tag != 'v' + -Version                  -> REJECT (tag/version mismatch)
#   otherwise                                -> PASS
#
# Test seams (unit-tested by scripts/release/test-release-triggers.ps1):
#   This script is pure: it takes explicit parameters and never touches git/gh/registry.
#
# ASCII-only; Windows PowerShell 5.1 compatible.

[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [AllowEmptyString()]
  [string]$Ref,

  [Parameter(Mandatory = $true)]
  [ValidatePattern('^v')]
  [string]$Tag,

  [Parameter(Mandatory = $true)]
  [string]$Version
)

$ErrorActionPreference = 'Stop'

if ($Ref -notlike 'refs/tags/v*') {
  Write-Host ("::error::Release workflow must be triggered by pushing a 'v*' tag, not by manual workflow_dispatch (ref: {0})" -f $Ref)
  exit 1
}

if ($Tag -ne "v$Version") {
  Write-Host ("::error::tag '{0}' != 'v{1}' from tauri.conf.json - bump the config before tagging" -f $Tag, $Version)
  exit 1
}

Write-Host "Release trigger OK: tag '$Tag' on '$Ref' matches config version '$Version'."
exit 0
