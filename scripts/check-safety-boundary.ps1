<#
.SYNOPSIS
    ADR-012 safety boundary check (CI-enforced on windows-latest).

.DESCRIPTION
    Only reclaim-action may mutate the filesystem (ADR-012, DECISIONS.md).
    This script scans the src/ of every reclaim-* crate EXCEPT
    reclaim-action for raw delete/rename/move APIs, so a "quick delete"
    added anywhere else fails the build instead of quietly bypassing the
    safety core (the denylist veto + journal + quarantine move).

    reclaim-store's SQLite writes via rusqlite are application state (the
    quarantine manifest, audit log, tool-detection cache), not user-file
    mutation. They're plain SQL statements — they never match these
    patterns, so reclaim-store isn't special-cased; it's scanned like
    every other non-action crate and simply passes on its own merits.

.EXAMPLE
    pwsh -File scripts/check-safety-boundary.ps1
#>

$ErrorActionPreference = 'Stop'

$patterns = @(
    'remove_file'
    'remove_dir_all'
    'remove_dir'
    '::rename('
    'DeleteFile'
    'MoveFile'
    'RemoveDirectory'
    'SHFileOperation'
    'IFileOperation'
)

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

$crateDirs = Get-ChildItem -Path $repoRoot -Directory -Filter 'reclaim-*' |
    Where-Object { $_.Name -ne 'reclaim-action' } |
    Sort-Object Name

$violations = @()

foreach ($crate in $crateDirs) {
    $srcDir = Join-Path $crate.FullName 'src'
    if (-not (Test-Path $srcDir)) {
        continue
    }

    $files = Get-ChildItem -Path $srcDir -Filter '*.rs' -Recurse -File
    if (-not $files) {
        continue
    }

    $found = Select-String -Path $files.FullName -Pattern $patterns -SimpleMatch
    if ($found) {
        $violations += $found
    }
}

if ($violations) {
    Write-Host 'ADR-012 safety boundary violation: filesystem-mutation API found outside reclaim-action:' -ForegroundColor Red
    foreach ($match in $violations) {
        $relative = $match.Path.Substring($repoRoot.Length).TrimStart('\', '/')
        Write-Host ("  {0}:{1}: {2}" -f $relative, $match.LineNumber, $match.Line.Trim())
    }
    Write-Host ''
    Write-Host 'Only reclaim-action may mutate the filesystem (ADR-012). Move this code there, or route it through reclaim-action.' -ForegroundColor Red
    exit 1
}

Write-Host 'ADR-012 safety boundary check passed: no filesystem-mutation APIs found outside reclaim-action.' -ForegroundColor Green
exit 0
