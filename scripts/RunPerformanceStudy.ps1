[CmdletBinding()]
param(
    [string]$Candidate,
    [ValidateSet('Full', 'Standard', 'Maximum')][string]$Mode = 'Full',
    [switch]$CheckOnly
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Candidate)) { $Candidate = $PSScriptRoot }
if ($env:OS -ne 'Windows_NT') { throw 'This study measures the Windows DX12 build.' }
$candidateRoot = (Resolve-Path -LiteralPath $Candidate).Path
$executable = Join-Path $candidateRoot 'dudel.exe'
$buildFile = Join-Path $candidateRoot 'BUILD.json'
$build = Get-Content -LiteralPath $buildFile -Raw | ConvertFrom-Json
if ((Get-FileHash -LiteralPath $executable).Hash -ne $build.executable_sha256) {
    throw 'The executable does not match BUILD.json. Extract an unchanged candidate.'
}
$diagnostics = Join-Path $candidateRoot 'diagnostics'
foreach ($item in (Get-Content -LiteralPath (Join-Path $candidateRoot 'STUDY_TOOLS.json') -Raw | ConvertFrom-Json)) {
    $path = [IO.Path]::GetFullPath((Join-Path $candidateRoot $item.path))
    if (-not $path.StartsWith($candidateRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Invalid study tool path.'
    }
    if ((Get-FileHash -LiteralPath $path).Hash -ne $item.sha256) { throw "Study tool changed: $($item.path)" }
}
$python = (Get-Command python -ErrorAction Stop).Source
& $python -c 'import sys; assert sys.version_info >= (3, 11), "Python 3.11 or newer is required"'
if ($LASTEXITCODE -ne 0) { throw 'Install Python 3.11 or newer, with python available in PATH.' }
& $python (Join-Path $diagnostics 'scripts/run_soak.py') --help | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'The packaged study modules could not load.' }
$editors = @(Get-Process dudel -ErrorAction SilentlyContinue)
if ($CheckOnly) {
    Write-Output "Candidate and study tools validated. Other Dudel processes: $($editors.Count). No study started."
    return
}
if ($editors.Count) { throw 'Save your work and close all Dudel windows before starting the study. No windows were closed.' }
$drive = [IO.DriveInfo]::new([IO.Path]::GetPathRoot($candidateRoot))
if ($drive.AvailableFreeSpace -lt 8GB) { throw 'The study needs at least 8 GiB of free space for reports and disposable projects.' }
$runRoot = Join-Path $diagnostics ('target/qa/study-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Path $runRoot | Out-Null
$hardware = [ordered]@{
    captured_utc = [DateTime]::UtcNow.ToString('o')
    os = @(Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber)
    cpu = @(Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors)
    ram_bytes = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
    gpu = @(Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion)
}
$hardware | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $runRoot 'hardware.json') -Encoding utf8
$state = [ordered]@{ status = 'running'; mode = $Mode; executable_sha256 = $build.executable_sha256; started_utc = [DateTime]::UtcNow.ToString('o'); release_accepted = $false }
$stateFile = Join-Path $runRoot 'STUDY.json'
function Write-StudyState { $state | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $stateFile -Encoding utf8 }
function Invoke-Study {
    param([string]$Name, [string[]]$Workload)
    $state.stage = $Name
    Write-StudyState
    $arguments = @((Join-Path $diagnostics 'scripts/run_soak.py'), '--executable', $executable, '--build', $buildFile,
        '--output', (Join-Path $runRoot $Name), '--rounds', '30', '--brushes', 'mixed', '--require-job-peaks') + $Workload
    # Cargo/Python progress on stderr is not a PowerShell failure; inspect exit.
    $ErrorActionPreference = 'Continue'
    & $python @arguments 2>&1 | Tee-Object -FilePath (Join-Path $runRoot "$Name.log") | ForEach-Object { $_.ToString() }
    $code = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($code -ne 0) { throw "$Name stopped ($code). Keep the reports; it will not restart automatically." }
}
Write-Output "Reports: $runRoot"
Write-Output 'Leave the PC idle and plugged in. Full mode usually takes four hours or more. Ctrl+C stops the owned test; reports remain.'
try {
    Write-StudyState
    if ($Mode -in @('Full', 'Standard')) { Invoke-Study 'standard20' @('--runs', '3', '--layers', '20', '--density', 'sparse') }
    if ($Mode -in @('Full', 'Maximum')) { Invoke-Study 'maximum32-two-hours' @('--runs', '1', '--layers', '32', '--density', 'noise', '--minimum-seconds', '7200') }
    $state.status = 'passed'
} catch {
    $state.status = 'stopped'
    $state.error = $_.Exception.Message
    throw
} finally {
    $state.ended_utc = [DateTime]::UtcNow.ToString('o')
    Write-StudyState
    Write-Output "Study status: $($state.status). Reports: $runRoot"
}
