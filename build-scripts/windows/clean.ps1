# SPDX-License-Identifier: MPL-2.0
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Low')]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$ledgerPath = Join-Path $repoRoot '.build-temp\build-owned.json'
$allowed = @('target', 'dist\windows') | ForEach-Object { [IO.Path]::GetFullPath((Join-Path $repoRoot $_)) + [IO.Path]::DirectorySeparatorChar }
try {
    if (-not (Test-Path -LiteralPath $ledgerPath)) { Write-Host 'No build ownership record; no files removed.'; exit 0 }
    $remaining = @()
    $parents = @{}
    $recordedEntries = Get-Content -LiteralPath $ledgerPath -Raw | ConvertFrom-Json
    foreach ($entry in $recordedEntries) {
        $path = [IO.Path]::GetFullPath($entry.path)
        if (-not ($allowed | Where-Object { $path.StartsWith($_, [StringComparison]::OrdinalIgnoreCase) })) { throw "Refusing path outside project output directories: $path" }
        $cursor = Split-Path -Parent $path
        while ($cursor -and $cursor.Length -ge $repoRoot.Length) {
            if ((Test-Path -LiteralPath $cursor) -and ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Refusing output behind a junction or symlink: $path" }
            if ($cursor -ne $repoRoot) { $parents[$cursor] = $true }
            $cursor = Split-Path -Parent $cursor
        }
        if (-not (Test-Path -LiteralPath $path)) { continue }
        $file = Get-Item -LiteralPath $path -Force
        if ($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Expected a regular generated file: $path" }
        if ("$($file.Length):$($file.LastWriteTimeUtc.Ticks)" -ne $entry.signature) { Write-Warning "Preserving modified output: $path"; $remaining += $entry; continue }
        if ($PSCmdlet.ShouldProcess($path, 'Remove recorded build output')) { Remove-Item -LiteralPath $path -Force } else { $remaining += $entry }
    }
    foreach ($directory in ($parents.Keys | Sort-Object { $_.Length } -Descending)) {
        if ((Test-Path -LiteralPath $directory) -and @(Get-ChildItem -LiteralPath $directory -Force).Count -eq 0 -and $PSCmdlet.ShouldProcess($directory, 'Remove empty output directory')) { Remove-Item -LiteralPath $directory }
    }
    if (-not $WhatIfPreference) {
        if ($remaining.Count -eq 0) {
            Remove-Item -LiteralPath $ledgerPath -Force
            $temporaryRoot = Split-Path -Parent $ledgerPath
            if (@(Get-ChildItem -LiteralPath $temporaryRoot -Force).Count -eq 0) { Remove-Item -LiteralPath $temporaryRoot }
        }
        else { [IO.File]::WriteAllText($ledgerPath, (ConvertTo-Json -InputObject $remaining -Depth 3), (New-Object Text.UTF8Encoding($false))) }
    }
    Write-Host 'Cleanup finished. Unrecorded and modified files were preserved.'
} catch { Write-Error -Message "Cleanup stopped: $($_.Exception.Message)" -ErrorAction Continue; exit 1 }
