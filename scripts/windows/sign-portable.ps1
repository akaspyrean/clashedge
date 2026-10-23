# sign-portable.ps1 - Authenticode-sign the binaries inside an already built portable ZIP.
#
# Release CI builds the ZIP in an unprivileged job (third-party build code runs there) and
# signs it here, in the publish job that is the only one holding signing secrets.
# Re-packs with the same layout ("ClashEdge/" top-level folder) and rewrites <zip>.sha256.
#
# Fail-closed by default: without a configured certificate the script exits non-zero.
# Local builds that intentionally want an unsigned ZIP may pass -AllowUnsigned.
#
#   pwsh scripts/windows/sign-portable.ps1 -Zip release\ClashEdge-portable-win64.zip
#   pwsh scripts/windows/sign-portable.ps1 -Zip ... -AllowUnsigned   # local dev only
#
# ASCII-only; Windows PowerShell 5.1 compatible.

param(
    [Parameter(Mandatory = $true)][string]$Zip,
    [switch]$AllowUnsigned
)

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\codesign.ps1"

if (-not (Test-Path $Zip)) { throw "ZIP not found: $Zip" }
$Zip = (Resolve-Path $Zip).Path

if (-not $env:WINDOWS_CODESIGN_PFX_BASE64) {
    if ($AllowUnsigned) {
        Write-Host "  [WARN] No signing certificate configured - ZIP left unsigned (-AllowUnsigned)." -ForegroundColor DarkYellow
        exit 0
    }
    throw "No signing certificate configured (WINDOWS_CODESIGN_PFX_BASE64 is empty). Release builds must be Authenticode-signed; pass -AllowUnsigned only for local dev."
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("clashedge-sign-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force $work | Out-Null
try {
    Expand-Archive -Path $Zip -DestinationPath $work -Force
    $top = Get-ChildItem $work -Directory | Select-Object -First 1
    if (-not $top) { throw "Unexpected ZIP layout (no top-level folder)" }
    $launcher = Join-Path $top.FullName "ClashEdge.exe"
    $inner    = Join-Path $top.FullName "App\ClashEdge\ClashEdge.exe"
    foreach ($f in @($launcher, $inner)) { if (-not (Test-Path $f)) { throw "Missing in ZIP: $f" } }

    [void](Invoke-CodeSign -Targets @($launcher, $inner))

    Remove-Item $Zip -Force
    Compress-Archive -Path $top.FullName -DestinationPath $Zip -Force
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

$sha = (Get-FileHash $Zip -Algorithm SHA256).Hash
Set-Content -Path "$Zip.sha256" -Value ($sha + "  " + (Split-Path $Zip -Leaf)) -Encoding ascii
Write-Host "Signed and re-packed: $Zip (SHA256 $sha)"
