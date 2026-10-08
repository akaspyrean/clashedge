# sync-rules.ps1 - Materialize the built-in rule set into the Android app assets.
#
# The built-in rules (direct/proxy/media/ai/ad) are pinned in assets.lock.json
# and staged by scripts/assets/prepare.ps1 into build/assets/staging/rules.
# This script copies them into apps/android/app/src/main/assets/rules so the
# Android build bundles them. Run prepare.ps1 before this script.
#
#   Usage:
#     scripts/assets/prepare.ps1
#     scripts/android/sync-rules.ps1
#
# Relocatable: derives the repo root from the script location (up 2 levels).

$ErrorActionPreference = "Stop"

$repo       = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$srcRules   = Join-Path $repo "build\assets\staging\rules"
$dstAssets  = Join-Path $repo "apps\android\app\src\main\assets\rules"

if (-not (Test-Path $srcRules)) { throw "staged rules not found: $srcRules (run scripts/assets/prepare.ps1 first)" }
New-Item -ItemType Directory -Force $dstAssets | Out-Null

Get-ChildItem -LiteralPath $srcRules -Filter "*.yaml" -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $dstAssets -Force
    Write-Host ("  copied {0}" -f $_.Name)
}
Write-Host "Done. Android assets ready: $dstAssets"
