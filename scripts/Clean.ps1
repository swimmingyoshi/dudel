# Lists build output that can be rebuilt, and with -Apply removes it.
#
#   ./scripts/Clean.ps1            what would go and how much room it frees
#   ./scripts/Clean.ps1 -Apply     remove it
#   ./scripts/Clean.ps1 -Deep      also the debug build itself (a full rebuild follows)
#
# Nothing here is a release: every ZIP, tar.gz and checksum under artifacts
# stays, and so do release builds. Run it after each release.
[CmdletBinding()]
param([switch]$Apply, [switch]$Deep)

$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent

function Get-Size {
    param([string]$Path)
    $sum = (Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction SilentlyContinue |
        Measure-Object -Property Length -Sum).Sum
    if ($sum) { [long]$sum } else { 0L }
}

$found = New-Object System.Collections.Generic.List[object]
function Add-Target {
    param([string]$Relative, [string]$Why)
    $path = Join-Path $repository $Relative
    if (Test-Path -LiteralPath $path) {
        $found.Add([pscustomobject]@{ Path = $path; Relative = $Relative; Why = $Why })
    }
}

# Caches Cargo rebuilds as needed.
if (-not $Deep) {
    Add-Target 'target/debug/incremental' 'incremental cache, debug'
    Add-Target 'target/fast/incremental' 'incremental cache, fast profile'
}
Add-Target 'shell/thumbnails/target/debug' 'thumbnail library, debug build'
# Folders made for one check or one measurement.
foreach ($name in 'exp', 'profile-build', 'qa', 'review', 'wt', 'flycheck0') {
    Add-Target "target/$name" 'verification or measurement build'
}
if ($Deep) {
    Add-Target 'target/debug' 'debug build (rebuilds in full)'
    Add-Target 'target/fast' 'fast-profile build (rebuilds in full)'
}

# Package.ps1 leaves one metadata file per run; the newest is enough.
$metadata = @(Get-ChildItem -LiteralPath (Join-Path $repository 'target') -Filter 'package-metadata-*.json' -File -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTimeUtc -Descending)
foreach ($file in ($metadata | Select-Object -Skip 1)) {
    Add-Target "target/$($file.Name)" 'metadata of an earlier package run'
}

# Extracted source trees, only where the archive beside them is intact.
$releases = Join-Path $repository 'artifacts/releases'
if (Test-Path -LiteralPath $releases) {
    foreach ($folder in Get-ChildItem -LiteralPath $releases -Directory -Filter '*-source') {
        $zip = "$($folder.FullName).zip"
        $record = "$zip.sha256"
        if (-not (Test-Path -LiteralPath $zip) -or -not (Test-Path -LiteralPath $record)) { continue }
        $expected = ((Get-Content -LiteralPath $record -TotalCount 1) -split '\s+')[0]
        if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ieq $expected) {
            Add-Target "artifacts/releases/$($folder.Name)" 'extracted source; its ZIP matches its checksum'
        } else {
            Write-Warning "Kept $($folder.Name): its ZIP does not match its checksum."
        }
    }
}

if ($found.Count -eq 0) {
    Write-Output 'Nothing to clean.'
    return
}
$total = 0L
foreach ($target in $found) {
    $size = if (Test-Path -LiteralPath $target.Path -PathType Container) { Get-Size $target.Path } else { (Get-Item -LiteralPath $target.Path).Length }
    $total += $size
    Write-Output ('{0,9:N1} MB  {1}  ({2})' -f ($size / 1MB), $target.Relative, $target.Why)
    if ($Apply) { Remove-Item -LiteralPath $target.Path -Recurse -Force -Confirm:$false }
}
$verb = if ($Apply) { 'Freed' } else { 'Would free' }
Write-Output ('{0} {1:N1} GB.' -f $verb, ($total / 1GB))
if (-not $Apply) { Write-Output 'Run again with -Apply to remove these.' }
