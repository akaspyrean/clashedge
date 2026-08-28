# sync-rules.ps1 - Materialize the shared rule set into the Android app assets.
#
# The built-in rules (direct/proxy/media/ai/ad) live in shared/rules (cross-platform).
# This script copies them into apps/android/app/src/main/assets/rules so the Android
# build bundles them. Run before assembling a debug/release APK.
#
#   Usage:
#     scripts/android/sync-rules.ps1
#
# Relocatable: derives the repo root from the script location (up 2 levels).

$ErrorActionPreference = "Stop"

$repo       = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$srcRules   = Join-Path $repo "shared\rules"
$dstAssets  = Join-Path $repo "apps\android\app\src\main\assets\rules"

if (-not (Test-Path $srcRules)) { throw "shared/rules not found: $srcRules" }
New-Item -ItemType Directory -Force $dstAssets | Out-Null

Get-ChildItem -LiteralPath $srcRules -Filter "*.yaml" -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $dstAssets -Force
    Write-Host ("  copied {0}" -f $_.Name)
}
Write-Host "Done. Android assets ready: $dstAssets"
