<#
.SYNOPSIS
    Builds the per-user Reclaim MSI stub (installer/reclaim.wxs).

.DESCRIPTION
    Release-builds dq.exe, then invokes the WiX v4 CLI to package it into
    dist\reclaim.msi. Requires the WiX v4 CLI on PATH:
        dotnet tool install --global wix

.EXAMPLE
    pwsh -File scripts/build-msi.ps1
#>

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

Push-Location $repoRoot
try {
    cargo build --release -p reclaim-cli
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }

    New-Item -ItemType Directory -Force -Path (Join-Path $repoRoot 'dist') | Out-Null

    wix build installer\reclaim.wxs -d DqExePath=target\release\dq.exe -o dist\reclaim.msi
    if ($LASTEXITCODE -ne 0) {
        throw "wix build failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

Write-Host 'Built dist\reclaim.msi' -ForegroundColor Green
