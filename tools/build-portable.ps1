# build-portable.ps1 - Build + assemble the ClashEdge portable release
#                       (reference launcher layout, matching docs/ClashEdge-portable-0.8.5.zip)
#
#   Usage:
#     build-portable.ps1            assemble only (requires pre-built release exe)
#     build-portable.ps1 -Build     also run npm run tauri -- build --no-bundle first
#
#   Output:
#     <repo>/release/portable-out/                        assembled portable directory
#     <repo>/release/ClashEdge-portable-<ver>-win64.zip   single-file distributable archive
#
#   Output structure (replicates the reference package):
#     <out>/ClashEdge.exe                 C# launcher (sets CLASH_EDGE_DATA_DIR + envs, spawns inner app)
#     <out>/App/ClashEdge/ClashEdge.exe   Tauri app (the real application binary)
#     <out>/App/ClashEdge/sidecar/        mihomo core + TUN helpers
#       mihomo-win64.exe                  mihomo core (renamed from clash-edge-core.exe)
#       EnableLoopback.exe                loopback enabler
#       go-tun2socks.exe                  TUN helper
#       wintun.dll                        TUN driver
#     <out>/App/DefaultData/              default data (launcher copies missing files into Data/)
#     <out>/Data/                         user data (pre-seeded from DefaultData)
#     <out>/Other/Help/README.md
#
# The launcher reads the same sidecar name the reference package uses, so the app
# finds mihomo at App/ClashEdge/sidecar/mihomo-win64.exe via CLASH_EDGE_DATA_DIR.
#
# Prerequisites:
#   1) sidecar files are taken from portable-template (released resources, not in git).
#   2) .NET Framework csc.exe (C# 5) available under C:\Windows\Microsoft.NET\Framework64.
#
# NOTE: keep this file ASCII-only - Windows PowerShell 5.1 reads no-BOM .ps1 as ANSI,
#       and non-ASCII bytes corrupt parsing.

param(
    [switch]$Build,

    # P1-12 hardening (Phase 3 audit): sidecar .sha256 missing => HARD FAILURE.
    # Never mint a trusted hash for an unknown binary. The only escape is the
    # DEV-ONLY switch below for bootstrapping hashes of brand-new sidecars;
    # CI / release builds must NOT pass it.
    [switch]$GenerateMissingChecksum
)

$ErrorActionPreference = "Stop"

$repo   = Split-Path -Parent $PSScriptRoot
$scaff  = Join-Path $repo "tauri-scaffold"
$tpl    = Join-Path $repo "portable-template"
$releaseDir = Join-Path $repo "release"
$out        = Join-Path $releaseDir "portable-out"
New-Item -ItemType Directory -Force $releaseDir | Out-Null

if ($Build) {
    Write-Host "==> Running frontend + tauri release build (--no-bundle) ..."
    Push-Location $scaff
    try {
        npm run tauri -- build --no-bundle
        if ($LASTEXITCODE -ne 0) { throw "tauri build failed (exit $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

# The Tauri release binary (named ClashEdge.exe via [[bin]] in Cargo.toml).
# This is the INNER app - it lives at App/ClashEdge/ClashEdge.exe, not at the root.
$releaseExe = Join-Path $scaff "src-tauri\target\release\ClashEdge.exe"
if (-not (Test-Path $releaseExe)) {
    $releaseExeOld = Join-Path $scaff "src-tauri\target\release\clash-edge.exe"
    if (Test-Path $releaseExeOld) {
        $releaseExe = $releaseExeOld
    } else {
        throw "Missing release binary: $releaseExe (run: npm run tauri -- build --no-bundle)"
    }
}

# Sanity check: the release exe must be a real Tauri app (embeds the frontend,
# ~20 MB). A small launcher stub here would mean we assembled it wrong.
$MIN_RELEASE_EXE_BYTES = 5MB
$releaseExeLen = (Get-Item $releaseExe).Length
if ($releaseExeLen -lt $MIN_RELEASE_EXE_BYTES) {
    throw ("Release binary looks like a stale launcher stub ({0:N0} B < {1:N0} B): {2}`n" +
           "Build the real app first: npm run tauri -- build --no-bundle") -f `
        $releaseExeLen, $MIN_RELEASE_EXE_BYTES, $releaseExe
}

# --- Compile the C# launcher (root ClashEdge.exe) ----------------------------
$launcherSrc  = Join-Path $repo "tools\ClashEdge.Launcher.R8.2.cs"
if (-not (Test-Path $launcherSrc)) {
    throw "Missing launcher source: $launcherSrc"
}
$csc = Get-ChildItem "C:\Windows\Microsoft.NET\Framework64" -Filter "csc.exe" -Recurse |
       Sort-Object FullName -Descending | Select-Object -First 1
if (-not $csc) { throw "csc.exe not found under C:\Windows\Microsoft.NET\Framework64" }

$launcherOut = Join-Path $out "ClashEdge.exe"
# Root launcher icon = cat.ico (multi-size 16/24/32/48/64/128/256, same source as the
# inner Tauri app icon), so the root ClashEdge.exe shows the identical icon as
# App/ClashEdge/ClashEdge.exe at every shell zoom level.
$rootIco = Join-Path $scaff "src-tauri\icons\cat.ico"
if (-not (Test-Path $rootIco)) { throw "Missing launcher icon: $rootIco" }

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force (Split-Path $launcherOut) | Out-Null

Write-Host "==> Compiling C# launcher -> $launcherOut"
& $csc.FullName /nologo /target:winexe /out:"$launcherOut" /win32icon:"$rootIco" `
    /r:System.dll /r:System.Drawing.dll /r:System.Windows.Forms.dll `
    /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll "$launcherSrc"
if ($LASTEXITCODE -ne 0) { throw "csc launcher compile failed (exit $LASTEXITCODE)" }
if (-not (Test-Path $launcherOut)) { throw "Launcher was not produced: $launcherOut" }

# --- Layout ------------------------------------------------------------------
$appClashEdgeDir = Join-Path $out "App\ClashEdge"
$sidecarDir      = Join-Path $appClashEdgeDir "sidecar"
$srcSide         = Join-Path $tpl "App\ClashEdge\resources\static\files\win"

New-Item -ItemType Directory -Force $sidecarDir | Out-Null
New-Item -ItemType Directory -Force (Join-Path $out "Data") | Out-Null
New-Item -ItemType Directory -Force (Join-Path $out "App\DefaultData") | Out-Null

# 1. Inner Tauri app binary
Copy-Item $releaseExe (Join-Path $appClashEdgeDir "ClashEdge.exe")

# 2. Sidecars (mihomo core renamed to mihomo-win64.exe to match the reference package)
#    P1-12: 自动 SHA256 校验——每个 sidecar 源文件旁应有 {filename}.sha256。
#    存在则比对；缺失则 FAIL（Release 语义：绝不为未知二进制生成可信哈希）。
#    仅本地开发可传 -GenerateMissingChecksum 显式生成新 .sha256（需 git add）。
#    不匹配立即失败，防止二进制被替换/损坏。
$sidecars = @(
    @{ src = Join-Path $srcSide "x64\clash-edge-core.exe";   dst = "mihomo-win64.exe" },
    @{ src = Join-Path $srcSide "x64\go-tun2socks.exe";      dst = "go-tun2socks.exe" },
    @{ src = Join-Path $srcSide "common\EnableLoopback.exe"; dst = "EnableLoopback.exe" },
    @{ src = Join-Path $srcSide "x64\wintun.dll";            dst = "wintun.dll" }
)
foreach ($sc in $sidecars) {
    if (-not (Test-Path $sc.src)) {
        throw ("Missing sidecar source: {0} (needed for {1})`n" +
               "Portable sidecars live under portable-template/App/ClashEdge/resources/static/files/win/") -f `
            $sc.src, $sc.dst
    }
    $shaFile = $sc.src + ".sha256"
    $actual = (Get-FileHash $sc.src -Algorithm SHA256).Hash
    if (Test-Path $shaFile) {
        $expected = (Get-Content $shaFile -Raw).Trim()
        if ($actual -ne $expected) {
            throw ("SHA256 mismatch for {0}:`n  expected ({1}): {2}`n  actual:            {3}`n" +
                   "The sidecar binary has changed. Update the .sha256 file:") -f `
                $sc.dst, (Split-Path -Leaf $shaFile), $expected, $actual
        }
        Write-Host "  SHA256 OK: $($sc.dst)"
    } elseif ($GenerateMissingChecksum) {
        # DEV ONLY: bootstrap .sha256 for a brand-new sidecar (git add this file).
        Set-Content -Path $shaFile -Value $actual -Encoding ascii -NoNewline
        Write-Host ("  [DEV-ONLY] SHA256 file created: {0} (git add this file to lock the hash)") -f `
            (Split-Path -Leaf $shaFile)
    } else {
        throw ("Missing SHA256 checksum file: {0}`n" +
               "Refusing to package sidecar '{1}' without a locked hash.`n" +
               "If this is a NEW sidecar added intentionally, run once with " +
               "-GenerateMissingChecksum and commit the .sha256 file.") -f `
            (Split-Path -Leaf $shaFile), $sc.dst
    }
    $dstPath = Join-Path $sidecarDir $sc.dst
    Copy-Item $sc.src $dstPath
}

# 3. Default data + pre-seeded user Data/
Copy-Item -Recurse (Join-Path $tpl "App\DefaultData\*") (Join-Path $out "App\DefaultData")
Copy-Item -Recurse (Join-Path $tpl "App\DefaultData\*") (Join-Path $out "Data")

# 4. Help docs in Other/
Copy-Item -Recurse (Join-Path $tpl "Other") (Join-Path $out "Other")

Write-Host ""
Write-Host "Portable layout assembled: $out"
Get-ChildItem -Recurse $out -File | ForEach-Object {
    $rel = $_.FullName.Substring($out.Length).TrimStart('\')
    Write-Host ("  {0,12:N0} B  {1}" -f $_.Length, $rel)
}

# 5. Post-assembly validation: invariants the reference layout depends on.
$assertions = @(
    @{ path = Join-Path $out "ClashEdge.exe";                  label = "root C# launcher ClashEdge.exe" },
    @{ path = Join-Path $out "App\ClashEdge\ClashEdge.exe";    label = "inner Tauri app App/ClashEdge/ClashEdge.exe" },
    @{ path = Join-Path $sidecarDir "mihomo-win64.exe";        label = "mihomo core sidecar/mihomo-win64.exe" },
    @{ path = Join-Path $sidecarDir "go-tun2socks.exe";        label = "TUN helper sidecar/go-tun2socks.exe" },
    @{ path = Join-Path $sidecarDir "EnableLoopback.exe";      label = "loopback enabler sidecar/EnableLoopback.exe" },
    @{ path = Join-Path $sidecarDir "wintun.dll";              label = "TUN driver sidecar/wintun.dll" },
    @{ path = Join-Path $out "Data";                           label = "Data directory" },
    @{ path = Join-Path $out "App\DefaultData";                label = "App/DefaultData directory" },
    @{ path = Join-Path $out "Other\Help\README.md";           label = "Other/Help/README.md" }
)
foreach ($a in $assertions) {
    if (-not (Test-Path $a.path)) {
        throw "Post-assembly validation failed: missing $($a.label) at $($a.path)"
    }
}
# The packaged root launcher must be the C# launcher (small, < 1 MB), NOT the Tauri app.
$rootExeLen = (Get-Item $launcherOut).Length
if ($rootExeLen -ge $MIN_RELEASE_EXE_BYTES) {
    throw ("Post-assembly validation failed: root ClashEdge.exe is {0:N0} B - " +
           "expected the small C# launcher (< {1:N0} B), got the Tauri app. " +
           "Check the launcher compile step.") -f $rootExeLen, $MIN_RELEASE_EXE_BYTES
}
Write-Host "Post-assembly validation passed: $($assertions.Count) invariants OK."

# 5b. Absolute-path leak scan (portable must be fully relocatable)
Write-Host "==> Absolute-path leak scan ..." -ForegroundColor Cyan
$ScanScript = Join-Path $repo "tools\scan_portable_paths.ps1"
if (Test-Path $ScanScript) {
    & $ScanScript -Root $out
    if ($LASTEXITCODE -ne 0) {
        throw "Absolute-path leak scan FAILED — portable must be fully relocatable."
    }
} else {
    Write-Host "  [SKIP] scan script not found at $ScanScript" -ForegroundColor DarkYellow
}

# 6. Single-file distributable archive.
# Top-level folder "ClashEdge/": extracting the zip yields a ClashEdge directory
# (matching the user's expectation of a clean folder, not scattered root files).
# tauri.conf.json is JSON5 now (contains comments); PowerShell 5.1 ConvertFrom-Json
# cannot parse it, so extract the version field with a regex instead.
$confRaw = Get-Content (Join-Path $scaff "src-tauri\tauri.conf.json") -Encoding UTF8 -Raw
$verMatch = [regex]::Match($confRaw, '"version"\s*:\s*"([^"]+)"')
if (-not $verMatch.Success) { throw "Cannot extract version from tauri.conf.json" }
$zip = Join-Path $releaseDir ("ClashEdge-portable-{0}-win64.zip" -f $verMatch.Groups[1].Value)
Write-Host ""
Write-Host "==> Compressing $out -> $zip ..."
# Compress-Archive -Path <dir> embeds the directory itself as the zip root entry.
$zipRoot = Join-Path $releaseDir "ClashEdge"
if (Test-Path $zipRoot) { Remove-Item -Recurse -Force $zipRoot }
New-Item -ItemType Directory -Force $zipRoot | Out-Null
Copy-Item -Recurse -Force (Join-Path $out "*") $zipRoot
try {
    Compress-Archive -Path $zipRoot -DestinationPath $zip -Force
} finally {
    Remove-Item -Recurse -Force $zipRoot
}
$zipInfo = Get-Item $zip
Write-Host ("  {0:N1} MB  {1}" -f ($zipInfo.Length / 1MB), $zipInfo.Name)

# 7. Release integrity: SHA256 of the distributable archive
$sha = (Get-FileHash $zip -Algorithm SHA256).Hash
$shaFile = "$zip.sha256"
Set-Content -Path $shaFile -Value ($sha + "  " + $zipInfo.Name) -Encoding ascii
Write-Host ("  SHA256: {0}" -f $sha)
Write-Host ("  Checksum file: {0}" -f $shaFile)