#requires -Version 5.1
<#
.SYNOPSIS
  Scan a staged ClashEdge portable tree for absolute-path leaks that would break
  relocatability (the portable must run from any drive / folder / machine).

.DESCRIPTION
  The portable derives everything at runtime from <PORTABLE ROOT>, so packaged
  files must NOT contain a hardcoded absolute path to the build machine.

  Returns non-zero on any leak.

.EXAMPLE
  .\scan_portable_paths.ps1 -Root C:\out\ClashEdge
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Root
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path $Root)) { throw "Root not found: $Root" }

# Leak patterns: build-machine absolute paths that must not be baked into runtime
# configuration/assets. Rust dependency panic locations under Cargo's immutable
# registry source are diagnostic metadata and do not affect relocatability; those
# prefixes are normalized before matching (same treatment for local and CI builds).
$DevSourcePatterns = @(
    "D:\\900 AIWork",
    "C:\\Users\\Fong"
)

$TextExt = @(
    ".json", ".toml", ".cfg", ".config", ".ini", ".conf", ".yaml", ".yml",
    ".xml", ".txt", ".md", ".log", ".env", ".rs", ".ps1", ".html", ".js", ".css", ".ts"
)

$Failures = [System.Collections.Generic.List[string]]::new()
$fileCount = 0

function Test-AbsolutePath([string]$line) {
    if ($line -notmatch '%[sSdD]%|%\{') {
        if ($line -match '(^|[^A-Za-z0-9/:])[A-Za-z]:[\\/]') {
            if ($line -match '%[sSxXoOdDeEfFgG]:') { return $false }
            return $true
        }
    }
    return $false
}

Get-ChildItem -Path $Root -Recurse -Force -File | ForEach-Object {
    $fileCount++
    $f = $_
    $rel = $f.FullName.Substring($Root.Length)

    # byte-level scan over the WHOLE file (binaries up to ~34MB are acceptable):
    # build-machine paths may sit anywhere in binary content, not just the first 4MB.
    $bytes = [System.IO.File]::ReadAllBytes($f.FullName)
    $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
    $asciiForDevScan = [regex]::Replace(
        $ascii,
        '[A-Za-z]:\\Users\\[^\\]+\\\.cargo\\registry\\src\\[^\\]+\\',
        '<cargo-registry>\\'
    )
    foreach ($p in $DevSourcePatterns) {
        if ($asciiForDevScan -match $p) {
            $Failures.Add("dev source path: $rel  (matched '$p')")
        }
    }

    $isText = [System.IO.Path]::GetExtension($f.Name).ToLower() -in $TextExt
    if (-not $isText) { return }

    $lines = [System.IO.File]::ReadLines($f.FullName)
    foreach ($line in $lines) {
        if ($line.Length -eq 0) { continue }
        if ($line -match '%\w+%') { continue }
        if (Test-AbsolutePath $line) {
            $trimmed = $line.Trim()
            $Failures.Add("absolute path: $rel :: $trimmed")
        }
    }
}

Write-Host ""
Write-Host "Scanned $fileCount file(s) under $Root" -ForegroundColor Cyan
if ($Failures.Count) {
    Write-Host ""
    Write-Host "ABSOLUTE-PATH LEAKS FOUND ($($Failures.Count)):" -ForegroundColor Red
    foreach ($x in $Failures) { Write-Host "  $x" -ForegroundColor Red }
    Write-Host "Portable is NOT fully relocatable. Fix the leaks, then rebuild." -ForegroundColor Red
    exit 1
}
Write-Host "OK: no absolute-path leaks. Tree is relocatable." -ForegroundColor Green
exit 0
