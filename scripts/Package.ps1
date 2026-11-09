[CmdletBinding()]
param([switch]$IncludeSymbols, [switch]$Release)

$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $repository
try {
    function Invoke-Native {
        param([string]$Program, [string[]]$Arguments)
        # Windows PowerShell 5 turns redirected progress stderr into ErrorRecords.
        # Keep stdout separate for metadata; the exit code determines failure.
        $ErrorActionPreference = 'Continue'
        & $Program @Arguments
        $nativeExit = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        if ($nativeExit -ne 0) { throw "$Program failed ($nativeExit)" }
    }
    # Hash actual build inputs, including uncommitted/untracked Rust modules.
    function Get-SourceIdentity {
        $inputs = @('Cargo.toml', 'Cargo.lock', 'LICENSE', 'LICENSE-psd-tools', 'shell/thumbnails/Cargo.toml', 'shell/thumbnails/Cargo.lock') + @(Get-ChildItem src,examples,plugins,shell/thumbnails/src -File -Recurse | ForEach-Object {
            $_.FullName.Substring($repository.Length + 1).Replace('\', '/')
        })
        @($inputs | Sort-Object | ForEach-Object {
            [ordered]@{ path = $_; sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash }
        })
    }
    # Build settings (static CRT) live in .cargo/config.toml. The source archive prepends
    # this exact text to its generated vendoring config, so it is recorded rather than
    # hashed as an archive input.
    $cargoConfigPath = Join-Path $repository '.cargo/config.toml'
    $cargoConfigBefore = (Get-FileHash -LiteralPath $cargoConfigPath -Algorithm SHA256).Hash
    # Keep this machine's folders out of the shipped binaries: panic locations would
    # otherwise name the user profile through the Cargo/rustup homes and the checkout.
    # Passed with --config so the static-CRT flags in .cargo/config.toml still apply
    # (Cargo joins rustflags arrays; RUSTFLAGS would replace them).
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
    $sysroot = ([string](Invoke-Native rustc @('--print', 'sysroot'))).Trim()
    $remapping = [ordered]@{ $repository = '/dudel'; $cargoHome = '/cargo'; $sysroot = '/rustc' }
    if (@($remapping.Keys | Where-Object { $_ -match "'" }).Count) { throw 'Build paths cannot contain apostrophes.' }
    $remapFlags = @($remapping.Keys | ForEach-Object { "'--remap-path-prefix=$_=$($remapping[$_])'" }) -join ', '
    $buildConfig = @('--config', "target.x86_64-pc-windows-msvc.rustflags=[$remapFlags]")
    $before = Get-SourceIdentity
    Invoke-Native cargo (@('build', '--locked', '--release') + $buildConfig)
    # Explorer thumbnails for .dudel files; its own workspace and lockfile.
    Invoke-Native cargo (@('build', '--locked', '--release', '--manifest-path', (Join-Path $repository 'shell/thumbnails/Cargo.toml')) + $buildConfig)
    $after = Get-SourceIdentity
    if (($before | ConvertTo-Json -Compress) -cne ($after | ConvertTo-Json -Compress) -or
        (Get-FileHash -LiteralPath $cargoConfigPath -Algorithm SHA256).Hash -ne $cargoConfigBefore) {
        throw 'Source changed during build; rerun against a stable workspace.'
    }
    $metadataText = Invoke-Native cargo @('metadata', '--locked', '--format-version', '1')
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq 'dudel' -and $_.manifest_path -eq (Join-Path $repository 'Cargo.toml') }
    if (-not $package) { throw 'Root package not found in Cargo metadata.' }
    $revision = Invoke-Native git @('rev-parse', 'HEAD')
    $state = @(Invoke-Native git @('status', '--porcelain'))
    $timestamp = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-fff')
    $name = "dudel-$($package.version)-candidate-$timestamp"
    $releaseDecision = $null
    if ($Release) {
        $releaseDecision = Get-Content -LiteralPath (Join-Path $repository 'docs/release/RELEASE_ACCEPTANCE.json') -Raw | ConvertFrom-Json
        if ($releaseDecision.status -ne 'ready_for_publication' -or $releaseDecision.version -ne $package.version) {
            throw 'Release preparation is not accepted for this version.'
        }
        $expectedPaths = @($releaseDecision.artifacts | ForEach-Object { $_.path })
        if ($expectedPaths.Count -ne 2 -or $expectedPaths -notcontains 'dudel.exe' -or $expectedPaths -notcontains 'dudel_thumbnails.dll') {
            throw 'Release acceptance must identify exactly the app and thumbnail DLL.'
        }
        foreach ($artifact in $releaseDecision.artifacts) {
            $builtPath = if ($artifact.path -eq 'dudel.exe') {
                Join-Path $metadata.target_directory 'release/dudel.exe'
            } else {
                Join-Path $repository 'shell/thumbnails/target/release/dudel_thumbnails.dll'
            }
            if ((Get-FileHash -LiteralPath $builtPath -Algorithm SHA256).Hash -cne $artifact.sha256) {
                throw "Release binary differs from accepted evidence: $($artifact.path)"
            }
        }
        $name = "dudel-$($package.version)-windows-x64"
    }
    $releases = Join-Path $repository 'artifacts/releases'
    New-Item -ItemType Directory -Path $releases -Force | Out-Null
    $stage = Join-Path $releases $name
    if (Test-Path -LiteralPath $stage) { throw "Package already exists: $stage" }
    New-Item -ItemType Directory -Path $stage | Out-Null
    $executable = Join-Path $metadata.target_directory 'release/dudel.exe'
    # Clean Windows installs lack the Visual C++ redistributable; the CRT must be linked statically.
    Invoke-Native python @((Join-Path $PSScriptRoot 'check_pe_imports.py'), $executable)
    Copy-Item -LiteralPath $executable -Destination (Join-Path $stage 'dudel.exe')
    $thumbnails = Join-Path $repository 'shell/thumbnails/target/release/dudel_thumbnails.dll'
    Invoke-Native python @((Join-Path $PSScriptRoot 'check_pe_imports.py'), $thumbnails)
    Invoke-Native python @((Join-Path $PSScriptRoot 'check_public_tree.py'), '--binary', $executable, $thumbnails)
    Copy-Item -LiteralPath $thumbnails -Destination (Join-Path $stage 'dudel_thumbnails.dll')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'RunPerformanceStudy.ps1') -Destination $stage
    $studyTools = @('RunPerformanceStudy.ps1')
    foreach ($relative in @('scripts/run_soak.py', 'research/analyze_native_session.py', 'research/assess_soak_memory.py', 'research/assess_soak_jobs.py')) {
        $destination = Join-Path $stage ('diagnostics/' + $relative)
        New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
        Copy-Item -LiteralPath (Join-Path $repository $relative) -Destination $destination
        $studyTools += 'diagnostics/' + $relative
    }
    @($studyTools | ForEach-Object { [ordered]@{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $stage $_)).Hash } }) |
        ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $stage 'STUDY_TOOLS.json') -Encoding utf8
    if ($IncludeSymbols) {
        Copy-Item -LiteralPath (Join-Path $metadata.target_directory 'release/dudel.pdb') -Destination $stage
    }
    # User documents keep their repository paths so their links still work;
    # package_docs.py adds any further document they link to.
    foreach ($document in @('RELEASE_NOTES.md', 'docs/QUICK_START.md', 'docs/FORMAT_SUPPORT.md', 'docs/EXPORT_OPTIONS.md', 'docs/CREATOR_FEATURES.md', 'docs/PLUGIN_DEVELOPMENT.md', 'docs/DISTRIBUTION.md', 'docs/TESTER_GUIDE.md')) {
        $destination = Join-Path $stage $document
        New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
        Copy-Item -LiteralPath $document -Destination $destination
    }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'RELEASE_README.md') -Destination (Join-Path $stage 'README.md')
    foreach ($license in @(Get-ChildItem -File -Filter 'LICENSE*')) {
        Copy-Item -LiteralPath $license.FullName -Destination $stage
    }
    $provenance = [ordered]@{
        version = $package.version
        revision = $revision
        working_tree = $state
        built_utc = [DateTime]::UtcNow.ToString('o')
        cargo = (Invoke-Native cargo @('--version'))
        rustc = (Invoke-Native rustc @('--version'))
        features = 'default (profiling disabled)'
        # Targets only; the local folders they replace are deliberately not recorded.
        path_remapping = @($remapping.Values)
        executable_sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
        artifacts = @(@('dudel.exe', 'dudel_thumbnails.dll') | ForEach-Object {
            $artifactPath = Join-Path $stage $_
            [ordered]@{
                path = $_
                sha256 = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash
                imports = @((Invoke-Native python @((Join-Path $PSScriptRoot 'check_pe_imports.py'), $artifactPath)) | ForEach-Object { [string]$_ } | Where-Object { $_ -notmatch '^Visual C\+\+ runtime imports:' })
            }
        })
        source_inputs = $after
        # Plain strings: Get-Content attaches provider metadata that ConvertTo-Json would expand.
        cargo_config = [ordered]@{ path = '.cargo/config.toml'; sha256 = $cargoConfigBefore; text = [string][IO.File]::ReadAllText($cargoConfigPath) }
        executable_imports = @((Invoke-Native python @((Join-Path $PSScriptRoot 'check_pe_imports.py'), $executable)) | ForEach-Object { [string]$_ } | Where-Object { $_ -notmatch '^Visual C\+\+ runtime imports:' })
        acceptance = 'Development candidate; see docs/dev/RELEASING.md. Not release-certified.'
    }
    if ($Release) {
        $provenance.acceptance = 'Prepared Windows release with recorded evidence and limits; see docs/release/RELEASE_DECISION.md. Not yet published.'
        $provenance['release_decision'] = $releaseDecision
    }
    $provenance | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $stage 'BUILD.json') -Encoding utf8
    $metadataFile = Join-Path $metadata.target_directory "package-metadata-$timestamp.json"
    $metadataText | Set-Content -LiteralPath $metadataFile -Encoding utf8
    Invoke-Native python @((Join-Path $PSScriptRoot 'package_source.py'), '--repository', $repository, '--destination', "$stage-source", '--binary', $stage, '--metadata', $metadataFile)
    # License expressions identify review work; they do not replace license texts/notices.
    @($metadata.packages | Sort-Object name, version | Select-Object name, version, license, source) |
        ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $stage 'DEPENDENCIES.json') -Encoding utf8
    $zip = "$stage.zip"
    if (Test-Path -LiteralPath $zip) { throw "Archive already exists: $zip" }
    Invoke-Native python @((Join-Path $PSScriptRoot 'zip_directory.py'), $stage, $zip)
    # Final gate on everything published: no personal paths or private research.
    Invoke-Native python @((Join-Path $PSScriptRoot 'check_public_tree.py'), '--archive', $zip, "$stage-source.zip")
    "$((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash)  $name.zip" |
        Set-Content -LiteralPath "$zip.sha256" -Encoding ascii
    Write-Output "Package: $zip"
    if ($Release) { Write-Output 'Prepared release; publication has not been performed.' }
    else { Write-Output 'Development candidate; consult the release decision and evidence before publication.' }
} finally {
    Pop-Location
}
