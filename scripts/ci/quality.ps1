# quality.ps1 - Single source of truth for ClashEdge quality gate.
#
# Both CI (ci.yml) and Release (release.yml) call this script to ensure
# "any commit that reaches a release tag passes the same fmt/clippy/test/
# audit/build checks." There is exactly one quality gate, not two.
#
# Usage (from repo root):
#   pwsh scripts/ci/quality.ps1
#   pwsh scripts/ci/quality.ps1 -SkipCargoAudit    # if cargo-audit is unavailable
#   pwsh scripts/ci/quality.ps1 -SkipLauncher      # no .NET Framework csc.exe on this machine
#
# Exit code 0 = all checks passed; non-zero = at least one failed.
# ASCII-only; Windows PowerShell 5.1 compatible.

[CmdletBinding()]
param(
  [switch]$SkipCargoAudit,
  [switch]$SkipLauncher
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
  $global:LASTEXITCODE = 0
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
    # Install only when missing (a fresh `cargo install` on every run costs minutes).
    if (-not (Get-Command cargo-audit -ErrorAction SilentlyContinue)) {
      cargo install cargo-audit --locked
    }
    if ($LASTEXITCODE -eq 0) { cargo audit }
    Set-Location $repoRoot
  }
}

if (-not $SkipCargoAudit) {
  Invoke-Step 'cargo deny (licenses / bans / sources)' {
    Set-Location $srcTauri
    if (-not (Get-Command cargo-deny -ErrorAction SilentlyContinue)) {
      cargo install cargo-deny --locked
    }
    if ($LASTEXITCODE -eq 0) { cargo deny check licenses bans sources }
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

# The C# launcher used to be compiled only at release time, so a compile error or
# a recovery regression was discovered while cutting a release. Build it and run
# its fault-injection tests in the same gate as everything else.
if (-not $SkipLauncher) {
  Invoke-Step 'launcher compile + recovery tests' {
    $csc = Get-ChildItem "C:\Windows\Microsoft.NET\Framework64" -Filter "csc.exe" -Recurse -ErrorAction SilentlyContinue |
           Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $csc) { throw "csc.exe not found (use -SkipLauncher to skip on machines without .NET Framework)" }
    $outDir = Join-Path $repoRoot 'build\launcher-test'
    New-Item -ItemType Directory -Force $outDir | Out-Null
    $exe = Join-Path $outDir 'ClashEdge.exe'
    $src = Join-Path $repoRoot 'packaging\windows\launcher\ClashEdge.Launcher.cs'
    & $csc.FullName /nologo /target:winexe "/out:$exe" `
      /r:System.dll /r:System.Drawing.dll /r:System.Windows.Forms.dll `
      /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll $src
    if ($LASTEXITCODE -ne 0) { return }
    # winexe: Start-Process -Wait -PassThru is the only reliable way to get the exit code.
    $p = Start-Process -FilePath $exe -ArgumentList '--test-recovery' -Wait -PassThru -NoNewWindow
    $global:LASTEXITCODE = $p.ExitCode
  }
}

# Android is experimental and frozen (apps/android/README.md): it must never leak into the
# release chain until its prerequisites are done.
Invoke-Step 'android release-chain guard' {
  $hit = Select-String -Path (Join-Path $repoRoot '.github\workflows\release.yml') -Pattern 'apps/android|apps\\android' -SimpleMatch:$false -ErrorAction SilentlyContinue
  if ($hit) {
    Write-Host "release.yml references apps/android: $($hit[0].Line.Trim())" -ForegroundColor Red
    $global:LASTEXITCODE = 1
  }
}

Write-Host ""
Write-Host ("Results: {0} passed, {1} failed." -f $script:passCount, $script:failCount)
if ($script:failCount -gt 0) { exit 1 }
exit 0
