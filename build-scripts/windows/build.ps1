# SPDX-License-Identifier: MPL-2.0
param([switch]$Debug, [switch]$Test, [switch]$Check)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$targetRoot = Join-Path $repoRoot 'target'
$temporaryRoot = Join-Path $repoRoot '.build-temp'
$ledgerPath = Join-Path $temporaryRoot 'build-owned.json'
$env:CARGO_TARGET_DIR = $targetRoot

function Invoke-Cargo([string[]]$Arguments) {
    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) { throw "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
}
function Get-GeneratedSnapshot {
    $snapshot = @{}
    function Read-OutputDirectory([string]$Directory) {
        foreach ($item in @(Get-ChildItem -LiteralPath $Directory -Force)) {
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing reparse point in build output: $($item.FullName)" }
            if ($item.PSIsContainer) { Read-OutputDirectory $item.FullName }
            else { $snapshot[$item.FullName] = "$($item.Length):$($item.LastWriteTimeUtc.Ticks)" }
        }
    }
    foreach ($root in @($targetRoot, (Join-Path $repoRoot 'dist\windows'))) {
        if (Test-Path -LiteralPath $root) {
            Assert-LocalOutput $root
            Read-OutputDirectory $root
        }
    }
    return $snapshot
}
function Assert-LocalOutput([string]$Path) {
    $resolved=[IO.Path]::GetFullPath($Path)
    if (-not $resolved.StartsWith($repoRoot + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw "Output escapes repository: $resolved" }
    $cursor=$resolved
    while ($cursor -and $cursor.Length -ge $repoRoot.Length) {
        if ((Test-Path -LiteralPath $cursor) -and ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Refusing output through a junction/symlink: $resolved" }
        $cursor=Split-Path -Parent $cursor
    }
}
function Write-DependencyNotices([string]$Destination) {
    $raw = & cargo metadata --locked --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect locked dependency licenses' }
    $metadata = ($raw -join "`n") | ConvertFrom-Json
    $text = New-Object Text.StringBuilder
    [void]$text.AppendLine('THIRD-PARTY NOTICES - Fullscreen Night Light')
    [void]$text.AppendLine('Project license: MPL-2.0. Dependencies retain their own licenses.')
    foreach ($package in ($metadata.packages | Sort-Object name, version)) {
        if ($package.name -eq 'fullscreen-night-light') { continue }
        $expression = ([string]$package.license).Replace('(', '').Replace(')', '')
        if (-not $expression -or $expression -notmatch '^(MIT|Unlicense|Apache-2\.0|Unicode-3\.0|BSD-2-Clause|BSD-3-Clause|ISC|Zlib|MPL-2\.0)( (OR|AND) (MIT|Unlicense|Apache-2\.0|Unicode-3\.0|BSD-2-Clause|BSD-3-Clause|ISC|Zlib|MPL-2\.0))*$') {
            throw "Dependency license requires review: $($package.name) $($package.license)"
        }
        [void]$text.AppendLine("`n============================================================")
        [void]$text.AppendLine("$($package.name) $($package.version) - $($package.license)")
        [void]$text.AppendLine([string]$package.repository)
        $directory = Split-Path -Parent $package.manifest_path
        $licenses = @(Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '^(LICENSE|COPYING|COPYRIGHT|UNLICENSE)' })
        if ($licenses.Count -eq 0) { throw "No license text found for $($package.name); review before packaging" }
        foreach ($license in $licenses) {
            [void]$text.AppendLine("--- $($license.Name) ---")
            [void]$text.AppendLine([IO.File]::ReadAllText($license.FullName))
        }
    }
    [IO.File]::WriteAllText($Destination, $text.ToString(), (New-Object Text.UTF8Encoding($false)))
}

$before = @{}
$owned = @{}
$enteredBuild = $false
try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Windows is required' }
    foreach ($command in @('rustup', 'rustc', 'cargo')) {
        if (-not (Get-Command $command -ErrorAction SilentlyContinue)) { throw "$command not found. Install Rust using rustup, then restart PowerShell." }
    }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Install Visual Studio Build Tools with Desktop development with C++ and a Windows SDK.' }
    $visualStudio = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $visualStudio) { throw 'MSVC x64 C++ build tools not found.' }
    Push-Location $repoRoot
    try {
        foreach ($path in @($targetRoot,$temporaryRoot,$ledgerPath,(Join-Path $repoRoot 'dist\windows\FullscreenNightLight'))) { Assert-LocalOutput $path }
        & rustc --version
        if ($LASTEXITCODE -ne 0) { throw 'Pinned Rust toolchain is unavailable. Run rustup toolchain install 1.97.1 --component rustfmt --component clippy --target x86_64-pc-windows-msvc.' }
        New-Item -ItemType Directory -Path $temporaryRoot -Force | Out-Null
        if (Test-Path -LiteralPath $ledgerPath) {
            $recordedEntries = Get-Content -LiteralPath $ledgerPath -Raw | ConvertFrom-Json
            foreach ($entry in $recordedEntries) { $owned[$entry.path] = $entry.signature }
        }
        $before = Get-GeneratedSnapshot
        $enteredBuild = $true
        if ($Check) {
            Invoke-Cargo -Arguments @('fmt', '--check')
            Invoke-Cargo -Arguments @('clippy', '--locked', '--all-targets', '--target', 'x86_64-pc-windows-msvc', '--', '-D', 'warnings')
        }
        if ($Test) { Invoke-Cargo -Arguments @('test', '--locked', '--target', 'x86_64-pc-windows-msvc') }
        $arguments = @('build', '--locked', '--target', 'x86_64-pc-windows-msvc')
        $profile = 'debug'
        if (-not $Debug) { $arguments += '--release'; $profile = 'release' }
        Invoke-Cargo $arguments
        $distribution = Join-Path $repoRoot 'dist\windows\FullscreenNightLight'
        New-Item -ItemType Directory -Path $distribution -Force | Out-Null
        $executable = Join-Path $targetRoot "x86_64-pc-windows-msvc\$profile\FullscreenNightLight.exe"
        Copy-Item -LiteralPath $executable -Destination (Join-Path $distribution 'FullscreenNightLight.exe') -Force
        foreach ($name in @('README.md', 'LICENSE', 'config.example.toml')) {
            Copy-Item -LiteralPath (Join-Path $repoRoot $name) -Destination (Join-Path $distribution $name) -Force
        }
        Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\MANUAL-TESTS.md') -Destination (Join-Path $distribution 'MANUAL-TESTS.md') -Force
        $documentation = Join-Path $distribution 'docs'
        New-Item -ItemType Directory -Path $documentation -Force | Out-Null
        foreach ($name in @('MANUAL-TESTS.md', 'VALIDATION.md', 'IMPLEMENTATION.md')) {
            Copy-Item -LiteralPath (Join-Path $repoRoot "docs\$name") -Destination (Join-Path $documentation $name) -Force
        }
        Write-DependencyNotices (Join-Path $distribution 'THIRD-PARTY-NOTICES.txt')
        $zip = Join-Path $repoRoot 'dist\windows\FullscreenNightLight-portable.zip'
        $packageFiles = @('FullscreenNightLight.exe', 'README.md', 'LICENSE', 'config.example.toml', 'MANUAL-TESTS.md', 'THIRD-PARTY-NOTICES.txt') | ForEach-Object { Join-Path $distribution $_ }
        $packageFiles += @('MANUAL-TESTS.md', 'VALIDATION.md', 'IMPLEMENTATION.md') | ForEach-Object { Join-Path $documentation $_ }
        # Preserve relative links while packaging only explicitly named files.
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        Add-Type -AssemblyName System.IO.Compression
        $zipStream = [IO.File]::Open($zip, [IO.FileMode]::Create)
        try {
            $archive = New-Object IO.Compression.ZipArchive($zipStream, [IO.Compression.ZipArchiveMode]::Create, $true)
            try {
                foreach ($file in $packageFiles) {
                    $entryName = $file.Substring($distribution.Length + 1).Replace('\', '/')
                    [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $file, $entryName) | Out-Null
                }
            } finally { $archive.Dispose() }
        } finally { $zipStream.Dispose() }
        Write-Host "Executable: $(Join-Path $distribution 'FullscreenNightLight.exe')"
        Write-Host "Portable ZIP: $zip"
    } finally { Pop-Location }
} catch {
    Write-Error -Message "Build failed: $($_.Exception.Message)" -ErrorAction Continue
    exit 1
} finally {
    if ($enteredBuild) {
        $after = Get-GeneratedSnapshot
        foreach ($path in @($after.Keys)) {
            if (-not $before.ContainsKey($path) -or $before[$path] -ne $after[$path]) { $owned[$path] = $after[$path] }
        }
        $entries = @($owned.Keys | Sort-Object | ForEach-Object { [PSCustomObject]@{ path = $_; signature = $owned[$_] } })
        $json = ConvertTo-Json -InputObject $entries -Depth 3
        [IO.File]::WriteAllText($ledgerPath, $json, (New-Object Text.UTF8Encoding($false)))
    }
}
