[CmdletBinding()]
param(
    [ValidateSet('Quick', 'Full')][string]$Mode = 'Quick',
    [string[]]$TestFilter,
    [string]$ReleaseTargetDirectory,
    [switch]$SkipReleaseBuild
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $repository
try {
    function Invoke-Check {
        param([string]$Program, [string[]]$Arguments)
        Get-Command $Program -ErrorAction Stop | Out-Null
        # Windows PowerShell 5 treats redirected native stderr as ErrorRecords.
        # Cargo writes ordinary progress there; only its exit code is failure.
        $ErrorActionPreference = 'Continue'
        & $Program @Arguments 2>&1 | ForEach-Object { $_.ToString() }
        $nativeExit = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        if ($nativeExit -ne 0) { throw "$Program $($Arguments -join ' ') failed ($nativeExit)" }
    }
    # Turn on the shared pre-push format check for this clone.
    if ((Test-Path .git) -and -not (git config core.hooksPath)) { git config core.hooksPath .githooks }
    Invoke-Check cargo @('fmt', '--all', '--', '--check')
    if ($TestFilter) {
        if ($Mode -eq 'Full') { throw 'TestFilter is for Quick development checks; Full verifies the complete suites' }
        # libtest treats multiple filters as OR and runs each matching test once.
        # One Cargo invocation also avoids repeated build checks between filters.
        $filters = @($TestFilter | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
        Invoke-Check cargo (@('test', '--locked', '--') + $filters)
    } else {
        Invoke-Check cargo @('test', '--locked')
    }
    Invoke-Check cargo @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
    # The Explorer DLL has an independent workspace, so root Cargo checks miss it.
    $thumbnailManifest = 'shell/thumbnails/Cargo.toml'
    Invoke-Check cargo @('fmt', '--manifest-path', $thumbnailManifest, '--', '--check')
    Invoke-Check cargo @('test', '--locked', '--manifest-path', $thumbnailManifest)
    Invoke-Check cargo @('clippy', '--locked', '--manifest-path', $thumbnailManifest, '--all-targets', '--', '-D', 'warnings')
    Invoke-Check python @('-m', 'unittest', 'discover', '-s', 'research', '-p', 'test_*.py')
    # No personal paths or private research in tracked files (the repository is public).
    Invoke-Check python @('scripts/check_public_tree.py')
    if ($Mode -eq 'Full') {
        # The diagnostic features add tests of their own and change no other
        # test, so only those run again; Clippy below still compiles every
        # target with the features on.
        function Get-TestNames {
            param([string[]]$Features)
            @(Invoke-Check cargo (@('test', '--locked') + $Features + @('--', '--list', '--format', 'terse')) |
                Where-Object { $_ -match ': test$' } | ForEach-Object { $_ -replace ': test$', '' })
        }
        $plain = New-Object 'System.Collections.Generic.HashSet[string]'
        foreach ($name in (Get-TestNames @())) { [void]$plain.Add($name) }
        $diagnostic = @(Get-TestNames @('--all-features') | Where-Object { -not $plain.Contains($_) })
        if ($plain.Count -eq 0 -or $diagnostic.Count -eq 0) { throw 'The diagnostic-feature tests were not found' }
        Invoke-Check cargo (@('test', '--locked', '--all-features', '--', '--exact') + $diagnostic)
        Invoke-Check cargo @('clippy', '--locked', '--all-targets', '--all-features', '--', '-D', 'warnings')
        # Package.ps1 builds the release itself, with the flags it ships with;
        # a run that packages next skips this copy.
        if (-not $SkipReleaseBuild) {
            $releaseArguments = @('build', '--locked', '--release')
            if ($ReleaseTargetDirectory) { $releaseArguments += @('--target-dir', $ReleaseTargetDirectory) }
            Invoke-Check cargo $releaseArguments
            Invoke-Check cargo ($releaseArguments + @('--manifest-path', $thumbnailManifest))
        }
    }
} finally {
    Pop-Location
}
