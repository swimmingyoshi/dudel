[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$StateFolder,
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]*\.jsonl$')]
    [string]$OutputName = 'external-process-observations.jsonl'
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
$qaRoot = (Resolve-Path -LiteralPath (Join-Path $repository 'target/qa')).Path
$folder = (Resolve-Path -LiteralPath $StateFolder).Path
if (-not $folder.StartsWith($qaRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Observation must use an existing isolated target/qa soak folder.'
}
$state = Get-Content -LiteralPath (Join-Path $folder 'STATE.json') -Raw | ConvertFrom-Json
if ($state.status -ne 'running' -or -not $state.pid) { throw 'No running soak child is recorded.' }
$child = Get-Process -Id $state.pid -ErrorAction Stop
$expectedExecutable = (Resolve-Path -LiteralPath (Join-Path $folder 'dudel.exe')).Path
if ($child.Path -ine $expectedExecutable) { throw 'PID does not identify the isolated soak executable.' }
$launched = [DateTimeOffset]::Parse($state.launched_utc).UtcDateTime
if ([Math]::Abs(($child.StartTime.ToUniversalTime() - $launched).TotalSeconds) -gt 5) {
    throw 'Process creation time differs from the recorded soak launch.'
}
# Acquire this process handle once. Waiting never switches to a reused PID.
$null = $child.Handle
$output = Join-Path $folder $OutputName
$stream = [IO.File]::Open($output, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
$writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
try {
    $writer.WriteLine(([ordered]@{
        kind = 'scope'
        observed_from_utc = [DateTime]::UtcNow.ToString('o')
        pid = $child.Id
        process_started_utc = $child.StartTime.ToUniversalTime().ToString('o')
        executable = $expectedExecutable
        interval_seconds = 30
        scope = 'External Windows Process samples, beginning at observer attachment. Lifetime peak counters include earlier process activity; instantaneous samples do not reconstruct earlier stages, job timings, tracked ownership or final unsampled activity. No process restart or release acceptance.'
    } | ConvertTo-Json -Compress))
    $writer.Flush()
    Write-Output "Observing confirmed soak PID $($child.Id): $output"
    while (-not $child.HasExited) {
        try {
            $child.Refresh()
            $sample = [ordered]@{
                kind = 'sample'
                utc = [DateTime]::UtcNow.ToString('o')
                private_bytes = $child.PrivateMemorySize64
                peak_paged_bytes = $child.PeakPagedMemorySize64
                working_set_bytes = $child.WorkingSet64
                peak_working_set_bytes = $child.PeakWorkingSet64
                cpu_seconds = $child.TotalProcessorTime.TotalSeconds
                last_phase = [string](Get-Content -LiteralPath $state.stderr -Tail 1)
            }
            $writer.WriteLine(($sample | ConvertTo-Json -Compress))
            $writer.Flush()
        } catch {
            if (-not $child.HasExited) { throw }
        }
        if ($child.WaitForExit(30000)) { break }
    }
    $writer.WriteLine(([ordered]@{ kind = 'process-exited'; utc = [DateTime]::UtcNow.ToString('o') } | ConvertTo-Json -Compress))
    $writer.Flush()
} finally {
    $writer.Dispose()
    $child.Dispose()
}
