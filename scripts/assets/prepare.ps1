# prepare.ps1 - Download, verify and stage third-party assets pinned in assets.lock.json.
#
# The Git repository does not carry third-party binaries (mihomo core, wintun
# driver). This script materializes them from the pinned upstream URLs:
#
#   download -> verify archive SHA256 -> extract -> verify extracted SHA256 -> stage
#
# Staged files land in <repo>/build/assets/staging/ and are consumed by
# scripts/windows/build-portable.ps1. Everything is cached: an already-staged
# file with a matching hash is never re-downloaded.
#
# Usage (from anywhere):
#   pwsh scripts/assets/prepare.ps1
#   pwsh scripts/assets/prepare.ps1 -Proxy http://127.0.0.1:7890
#
# Exit code 0 = all assets staged and verified.
# ASCII-only; Windows PowerShell 5.1 compatible.

[CmdletBinding()]
param(
    # Optional HTTP proxy for downloads, e.g. http://127.0.0.1:7890
    [string]$Proxy = ""
)

$ErrorActionPreference = "Stop"

$repo     = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$lockPath = Join-Path $repo "assets.lock.json"
$cacheDir = Join-Path $repo "build\assets\cache"
$stageDir = Join-Path $repo "build\assets\staging"

if (-not (Test-Path $lockPath)) {
    throw "Missing asset lock file: $lockPath"
}

$lock = Get-Content $lockPath -Raw -Encoding UTF8 | ConvertFrom-Json
New-Item -ItemType Directory -Force $cacheDir | Out-Null
New-Item -ItemType Directory -Force $stageDir | Out-Null

function Get-SHA256 {
    param([string]$Path)
    return (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLower()
}

function Invoke-Download {
    param([string]$Url, [string]$Destination)
    # curl.exe ships with Windows 10+ and GitHub runners, and supports resume.
    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($curl) {
        # NB: $args is a reserved automatic variable in PowerShell — do not rename.
        $curlArgs = @("-L", "--fail", "--retry", "3", "--connect-timeout", "30", "-o", $Destination)
        if ($Proxy -ne "") { $curlArgs += @("-x", $Proxy) }
        & $curl.Source @curlArgs $Url
        if ($LASTEXITCODE -ne 0) {
            throw "Download failed (curl exit $LASTEXITCODE): $Url"
        }
    } else {
        try {
            $wc = New-Object System.Net.WebClient
            if ($Proxy -ne "") {
                $wc.Proxy = New-Object System.Net.WebProxy($Proxy)
            }
            $wc.DownloadFile($Url, $Destination)
        } finally {
            if ($wc) { $wc.Dispose() }
        }
    }
}

$failed = 0
foreach ($asset in $lock.assets) {
    $name = $asset.name
    $outPath = Join-Path $stageDir ($asset.out -replace "/", "\")
    $outHash = $asset.extracted_sha256.ToLower()

    Write-Host "==> $name $($asset.version)"

    # 1. Already staged and verified -> done.
    if ((Test-Path $outPath) -and ((Get-SHA256 $outPath) -eq $outHash)) {
        Write-Host "  PASS  staged: $outPath"
        continue
    }

    # 2. Cache the archive; reuse it when its hash matches the lock.
    $archiveName = ($asset.url -split "/")[-1]
    $archivePath = Join-Path $cacheDir $archiveName
    $haveArchive = (Test-Path $archivePath) -and ((Get-SHA256 $archivePath) -eq $asset.sha256.ToLower())
    if (-not $haveArchive) {
        Write-Host "  downloading: $($asset.url)"
        Invoke-Download -Url $asset.url -Destination $archivePath
        $actual = Get-SHA256 $archivePath
        if ($actual -ne $asset.sha256.ToLower()) {
            $failed++
            Write-Host "  FAIL  archive hash mismatch for $name`n    expected: $($asset.sha256)`n    actual:   $actual" -ForegroundColor Red
            continue
        }
    }

    # 3. Extract the pinned member and re-verify the extracted artifact itself.
    $extractDir = Join-Path $cacheDir ("extract_" + $name)
    if (Test-Path $extractDir) { Remove-Item -Recurse -Force $extractDir }
    Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force
    $memberPath = Join-Path $extractDir ($asset.extract -replace "/", "\")
    if (-not (Test-Path $memberPath)) {
        $failed++
        Write-Host "  FAIL  member not found in archive: $($asset.extract)" -ForegroundColor Red
        continue
    }
    $memberHash = Get-SHA256 $memberPath
    if ($memberHash -ne $outHash) {
        $failed++
        Write-Host "  FAIL  extracted hash mismatch for $name`n    expected: $outHash`n    actual:   $memberHash" -ForegroundColor Red
        continue
    }

    New-Item -ItemType Directory -Force (Split-Path -Parent $outPath) | Out-Null
    Copy-Item $memberPath $outPath -Force
    Write-Host "  PASS  staged: $outPath"
}

Write-Host ""
if ($failed -gt 0) {
    Write-Host "$failed asset(s) failed verification. Do NOT weaken the lock; fix the source or pin a new verified version." -ForegroundColor Red
    exit 1
}
Write-Host "All assets staged and verified under $stageDir"
exit 0
