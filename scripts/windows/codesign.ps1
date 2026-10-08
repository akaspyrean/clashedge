# codesign.ps1 - Shared Authenticode helper (dot-source it).
#
#   . "$PSScriptRoot\codesign.ps1"
#   Invoke-CodeSign -Targets @($exe1, $exe2)
#
# Reads WINDOWS_CODESIGN_PFX_BASE64 / WINDOWS_CODESIGN_PFX_PASSWORD from the environment.
# Returns $true when signed, $false when no certificate is configured (unsigned build).
# The password is passed through an environment variable to signtool's /p only; the PFX is
# written to a temp file that is always removed.
#
# ASCII-only; Windows PowerShell 5.1 compatible.

function Invoke-CodeSign {
    param([string[]]$Targets)

    if (-not $env:WINDOWS_CODESIGN_PFX_BASE64) {
        Write-Host "  [WARN] WINDOWS_CODESIGN_PFX_BASE64 not set - binaries are NOT Authenticode-signed." -ForegroundColor DarkYellow
        return $false
    }
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -match '[\/]x64[\/]' } |
                Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $signtool) { throw "signtool.exe not found (Windows SDK required for code signing)" }

    $pfxPath = Join-Path ([System.IO.Path]::GetTempPath()) ("clashedge-codesign-" + [guid]::NewGuid().ToString("N") + ".pfx")
    try {
        [System.IO.File]::WriteAllBytes($pfxPath, [Convert]::FromBase64String($env:WINDOWS_CODESIGN_PFX_BASE64))
        foreach ($target in $Targets) {
            Write-Host "==> Authenticode signing $target"
            & $signtool.FullName sign /f $pfxPath /p $env:WINDOWS_CODESIGN_PFX_PASSWORD /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $target
            if ($LASTEXITCODE -ne 0) { throw "signtool failed for $target (exit $LASTEXITCODE)" }
            $sig = Get-AuthenticodeSignature -FilePath $target
            if ($sig.Status -ne "Valid") { throw "Authenticode verification failed for $target : $($sig.Status)" }
        }
    } finally {
        if (Test-Path $pfxPath) { Remove-Item $pfxPath -Force }
    }
    return $true
}
