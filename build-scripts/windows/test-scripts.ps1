# SPDX-License-Identifier: MPL-2.0
# Isolated cleanup acceptance. Does not delete real project outputs or AppData.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$fixture = Join-Path $repoRoot ('.build-temp\cleanup-test-' + [Guid]::NewGuid().ToString('N'))
if (-not $fixture.StartsWith($repoRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid fixture root' }
$scripts = Join-Path $fixture 'build-scripts\windows'
$output = Join-Path $fixture 'target'
$temporary = Join-Path $fixture '.build-temp'
New-Item -ItemType Directory -Path $scripts,$output,$temporary -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'clean.ps1') -Destination $scripts
$generated = Join-Path $output 'generated.bin'
$unknown = Join-Path $output 'user-sentinel.txt'
$modified = Join-Path $output 'modified.bin'
[IO.File]::WriteAllText($generated,'generated')
[IO.File]::WriteAllText($unknown,'preserve me')
[IO.File]::WriteAllText($modified,'before')
$entries = @($generated,$modified) | ForEach-Object { $f=Get-Item -LiteralPath $_; [PSCustomObject]@{path=$f.FullName;signature="$($f.Length):$($f.LastWriteTimeUtc.Ticks)"} }
$ledger = Join-Path $temporary 'build-owned.json'
[IO.File]::WriteAllText($ledger,(ConvertTo-Json -InputObject @($entries)))
[IO.File]::WriteAllText($modified,'modified by someone else')
$clean = Join-Path $scripts 'clean.ps1'
& powershell.exe -NoProfile -File $clean -WhatIf | Out-Null
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $generated)) { throw 'WhatIf changed output or failed' }
& powershell.exe -NoProfile -File $clean
if ($LASTEXITCODE -ne 0 -or (Test-Path -LiteralPath $generated)) { throw 'Recorded output was not removed' }
if (-not (Test-Path -LiteralPath $unknown) -or [IO.File]::ReadAllText($modified) -ne 'modified by someone else') { throw 'Unknown/modified output was damaged' }
& powershell.exe -NoProfile -File $clean | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Repeated cleanup failed' }
$outside = Join-Path $fixture 'outside.txt'
[IO.File]::WriteAllText($outside,'outside sentinel')
$f=Get-Item -LiteralPath $outside
[IO.File]::WriteAllText($ledger,(ConvertTo-Json -InputObject @([PSCustomObject]@{path=$outside;signature="$($f.Length):$($f.LastWriteTimeUtc.Ticks)"})))
$oldPreference=$ErrorActionPreference
try { $ErrorActionPreference='Continue'; $messages = & powershell.exe -NoProfile -File $clean 2>&1; $rejected=$LASTEXITCODE -ne 0 } finally { $ErrorActionPreference=$oldPreference }
if (-not $rejected -or [IO.File]::ReadAllText($outside) -ne 'outside sentinel') { throw 'Out-of-root ownership entry was not safely rejected' }
Write-Host "PASS: PowerShell 5.1 cleanup WhatIf, repeated cleanup, unknown/modified preservation and out-of-root rejection. Fixture retained at $fixture"
$global:LASTEXITCODE = 0
