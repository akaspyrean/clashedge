# build-portable.ps1 - Build + assemble the ClashEdge portable release
#                       (reference launcher layout, matching docs/ClashEdge-portable-0.8.5.zip)
#
#   Usage:
#     build-portable.ps1            assemble only (requires pre-built release exe)
#     build-portable.ps1 -Build     also run the local Tauri CLI build first
#
#   Output:
#     <repo>/release/portable-out/                        assembled portable directory
#     <repo>/release/ClashEdge-portable-win64.zip         single-file distributable archive (stable name)
#
#   Output structure (replicates the reference package):
#     <out>/ClashEdge.exe                 C# launcher (sets CLASH_EDGE_DATA_DIR + envs, spawns inner app)
#     <out>/App/ClashEdge/ClashEdge.exe   Tauri app (the real application binary)
#     <out>/App/ClashEdge/sidecar/        mihomo core + TUN driver
#       mihomo-win64.exe                  mihomo core (renamed from clash-edge-core.exe)
#       wintun.dll                        TUN driver
#     <out>/App/DefaultData/              default data (launcher copies missing files into Data/)
#     <out>/Data/                         user data (pre-seeded from DefaultData)
#     <out>/Other/Help/README.md
#
# The launcher reads the same sidecar name the reference package uses, so the app
# finds mihomo at App/ClashEdge/sidecar/mihomo-win64.exe via CLASH_EDGE_DATA_DIR.
#
# Prerequisites:
#   1) run scripts/assets/prepare.ps1 once: it downloads the pinned mihomo core,
#      wintun.dll and built-in rule sets (see assets.lock.json) into
#      build/assets/staging/.
#   2) .NET Framework csc.exe (C# 5) available under C:\Windows\Microsoft.NET\Framework64.
#
# NOTE: keep this file ASCII-only - Windows PowerShell 5.1 reads no-BOM .ps1 as ANSI,
#       and non-ASCII bytes corrupt parsing.

param(
    [switch]$Build
)

$ErrorActionPreference = "Stop"

# Repo root = up 2 from this script (script lives at scripts/windows/), so the
# script remains correct no matter which machine / drive / folder holds the repo.
$repo   = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$scaff  = Join-Path $repo "apps\windows"
$tpl    = Join-Path $repo "packaging\windows\portable"
$releaseDir = Join-Path $repo "release"
$out        = Join-Path $releaseDir "portable-out"
New-Item -ItemType Directory -Force $releaseDir | Out-Null

if ($Build) {
    Write-Host "==> Running frontend + tauri release build (--no-bundle) ..."
    Push-Location $scaff
    try {
        node node_modules/@tauri-apps/cli/tauri.js build --no-bundle
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
        throw "Missing release binary: $releaseExe (run the local Tauri CLI build first)"
    }
}

# Sanity check: the release exe must be a real Tauri app (embeds the frontend,
# ~20 MB). A small launcher stub here would mean we assembled it wrong.
$MIN_RELEASE_EXE_BYTES = 5MB
$releaseExeLen = (Get-Item $releaseExe).Length
if ($releaseExeLen -lt $MIN_RELEASE_EXE_BYTES) {
    throw ("Release binary looks like a stale launcher stub ({0:N0} B < {1:N0} B): {2}`n" +
           "Build the real app first with the local Tauri CLI") -f `
        $releaseExeLen, $MIN_RELEASE_EXE_BYTES, $releaseExe
}

# --- Compile the C# launcher (root ClashEdge.exe) ----------------------------
$launcherSrc  = Join-Path $repo "packaging\windows\launcher\ClashEdge.Launcher.cs"
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
$assets          = Join-Path $repo "build\assets\staging"

New-Item -ItemType Directory -Force $sidecarDir | Out-Null
New-Item -ItemType Directory -Force (Join-Path $out "Data") | Out-Null
New-Item -ItemType Directory -Force (Join-Path $out "App\DefaultData") | Out-Null

# 1. Inner Tauri app binary
Copy-Item $releaseExe (Join-Path $appClashEdgeDir "ClashEdge.exe")

# 2. Sidecars, staged by scripts/assets/prepare.ps1 (hashes pinned in assets.lock.json).
#    mihomo core is renamed to mihomo-win64.exe to match the reference package.
$sidecars = @(
    @{ src = Join-Path $assets "mihomo\clash-edge-core.exe"; dst = "mihomo-win64.exe" },
    @{ src = Join-Path $assets "wintun\wintun.dll";          dst = "wintun.dll" }
)
foreach ($sc in $sidecars) {
    if (-not (Test-Path $sc.src)) {
        throw ("Missing staged sidecar: {0} (needed for {1})`n" +
               "Run scripts/assets/prepare.ps1 first.") -f $sc.src, $sc.dst
    }
    Copy-Item $sc.src (Join-Path $sidecarDir $sc.dst)
}

# 3. Default data + pre-seeded user Data/
#    Built-in rule sets come from the staged assets (pinned in assets.lock.json).
#    wintun.dll comes from staging too (it ships in both sidecar/ and DefaultData).
$stagedRules = Join-Path $assets "rules"
$stagedWintun = Join-Path $assets "wintun\wintun.dll"
Copy-Item -Recurse (Join-Path $tpl "App\DefaultData\*") (Join-Path $out "App\DefaultData")
Copy-Item -Recurse (Join-Path $tpl "App\DefaultData\*") (Join-Path $out "Data")
Copy-Item $stagedWintun (Join-Path $out "App\DefaultData\wintun.dll")
Copy-Item $stagedWintun (Join-Path $out "Data\wintun.dll")
if (Test-Path $stagedRules) {
    Copy-Item -Recurse (Join-Path $stagedRules "*") (Join-Path $out "App\DefaultData\rules")
    Copy-Item -Recurse (Join-Path $stagedRules "*") (Join-Path $out "Data\rules")
}

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
    @{ path = Join-Path $sidecarDir "wintun.dll";              label = "TUN driver sidecar/wintun.dll" },
    @{ path = Join-Path $out "App\DefaultData\wintun.dll";     label = "TUN driver App/DefaultData/wintun.dll" },
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
$ScanScript = Join-Path $repo "scripts\windows\scan-portable-paths.ps1"
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
#
# 稳定文件名：不带版本号——与 scripts/release/make-update-manifest.py 的 ZIP_ASSET
# ("ClashEdge-portable-win64.zip") 保持一致，Portable Updater 按固定名下载；
# 版本信息以 portable-manifest.json 与应用「设置→关于」为准。
$zip = Join-Path $releaseDir "ClashEdge-portable-win64.zip"
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
