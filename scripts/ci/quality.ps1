# quality.ps1 - Single source of truth for ClashEdge quality gate.
#
# Both CI (ci.yml) and Release (release.yml) call this script to ensure
# "any commit that reaches a release tag passes the same fmt/clippy/test/
# audit/build checks." There is exactly one quality gate, not two.
#
# Usage (from repo root):
#   pwsh scripts/ci/quality.ps1
#   pwsh scripts/ci/quality.ps1 -SkipCargoAudit   # if cargo-audit is pre-installed
#
# Exit code 0 = all checks passed; non-zero = at least one failed.
# ASCII-only; Windows PowerShell 5.1 compatible.

[CmdletBinding()]
param(
  [switch]$SkipCargoAudit
)

$ErrorActionPreference = 'Stop'

$repoRoot   = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$scaff      = Join-Path $repoRoot 'apps\windows'
$srcTauri  = Join-Path $scaff 'src-tauri'

$script:passCount = 0
$script:failCount = 0

function Invoke-Step {
  param([string]$Name, [scriptblock]$Action)
  Write-Host ""
  Write-Host "==> $Name" -ForegroundColor Cyan
  & $Action
  if ($LASTEXITCODE -ne 0) {
    Write-Host "FAIL  $Name (exit $LASTEXITCODE)" -ForegroundColor Red
    $script:failCount++
    return
  }
  Write-Host "PASS  $Name" -ForegroundColor Green
  $script:passCount++
}

Invoke-Step 'npm ci' {
  Set-Location $scaff
  npm ci
  Set-Location $repoRoot
}

Invoke-Step 'cargo fmt --check' {
  Set-Location $srcTauri
  cargo fmt --check
  Set-Location $repoRoot
}

Invoke-Step 'cargo clippy (-D warnings)' {
  Set-Location $srcTauri
  cargo clippy --all-targets -- -D warnings
  Set-Location $repoRoot
}

Invoke-Step 'cargo test' {
  Set-Location $srcTauri
  cargo test --all-targets
  Set-Location $repoRoot
}

if (-not $SkipCargoAudit) {
  Invoke-Step 'cargo audit (known vulnerabilities)' {
    Set-Location $srcTauri
    cargo install cargo-audit --locked 2>$null
    cargo audit
    Set-Location $repoRoot
  }
}

Invoke-Step 'npm audit (high / critical)' {
  Set-Location $scaff
  npm audit --audit-level=high
  Set-Location $repoRoot
}

Invoke-Step 'npm test (Vitest unit + component tests)' {
  Set-Location $scaff
  npm test
  Set-Location $repoRoot
}

Invoke-Step 'npm run build' {
  Set-Location $scaff
  npm run build
  Set-Location $repoRoot
}

Write-Host ""
Write-Host ("Results: {0} passed, {1} failed." -f $script:passCount, $script:failCount)
if ($script:failCount -gt 0) { exit 1 }
exit 0
