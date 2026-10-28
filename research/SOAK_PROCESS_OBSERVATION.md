# Supplemental dense-soak observation, 2026-10-17

The running 30-round maximum masked/noisy session uses frozen executable
`5B97376259EFB878ED082EFB796E2F30B8C098238D91B516E62CD5F33C1E1D22`.
Its first rounds are slower than the standard sparse study. The native harness has
a total limit of `rounds * 120 + 600` seconds: 4,200 seconds for this run. Based on
the observed cadence, that limit may expire before round 30. This is a forecast,
not an observed timeout or justification to restart a live process.

`scripts/Observe-Soak.ps1` attaches read-only to the recorded isolated process.
It checks the QA directory, exact executable path and process creation time, then
holds that process handle and samples Windows memory/CPU counters every 30 seconds.
It neither starts nor terminates the application, and refuses existing output.
Live attachment, counter sanity, script syntax and output preservation were checked;
see `benchmarks/soak-observer-check.json`.

The external observer began late in the session. Lifetime peak counters retain
earlier peak information; sampled current memory does not reconstruct early
stage ownership or completed-job timing. Final activity between samples may be
missed. The native complete report remains necessary for the planned assessment.

Files are under `target/qa/maximum32-noise-job-peaks-20261017/`:

- `STATE.json`, native stdout/stderr and the eventual native report.
- `external-process-observations.jsonl`: initial observer records; PowerShell
  decorated the phase field as an object. Numeric counters remain available.
- `external-process-observations-v2.jsonl`: corrected plain-string phase field.
  Only the first observer process was stopped and replaced to correct that
  serialization; the native application was left running unchanged.

Next actions depend on the actual terminal result. If the harness limit expires,
preserve the failure and external observations, improve the diagnostic duration
and report preservation, then explicitly schedule a newly identified run. Do not
relax the product's memory/job budgets or treat partial observation as a pass.

## Prepared next diagnostic version

Working-tree changes now give Pattern/Noise diagnostics 300 seconds per round
plus 600 seconds, with a fixed frame allocation sized for that observation window
at 120 records/second. Sparse time/storage limits remain unchanged. The controller
waits another 300 seconds beyond the dense native limit. These are diagnostic
observation limits, not the product's save/open or memory acceptance thresholds.
The complete report records the chosen limit and `complete: true`.

On a native diagnostic error, the capture wrapper will attempt to preserve an
exclusive `.incomplete.json` containing the error, checkpoints, completed jobs and
frame count/capacity. It omits raw frame samples and does not create a completed
report. A new Rust regression checks preservation and refusal to overwrite it.
Construction for that test avoids starting the global allocation tracer.

The Python summary and both acceptance assessors reject `complete: false` before
using workload fields. Three new regressions exercise that boundary; all 21 Python
checks pass. Formatting passes. **Low-priority, single-job Rust compilation and
behavioral verification are in progress.** Their overlap is explicitly recorded
in the engineering run's `CONTENTION.json`; isolated frame/queue/job acceptance
requires a later verified candidate. None of these source changes alters the
running executable or supplies its missing final report.

Default Rust verification now passes (788 tests; zero failed, two ignored), including
the diagnostic limits and exclusive incomplete-report preservation regression.
All-feature verification passed (797 tests; zero failed, two ignored), as did both
warnings-denied Clippy configurations. The isolated release build remains in
progress under `target/qa/keyboard-diagnostic-verification-20261017/STATE.json`.
Its compiler priority was raised from below-normal to normal to finish verification;
the engineering soak's contention record preserves the timestamp and scope.

The release build completed and was frozen as
`FFCD27770363AB4F9352886BB621306FDBB822D4908C278D2E08D221260381A6`
(`benchmarks/keyboard-diagnostic-build.json`). A native one-round/four-layer smoke
passed and its screenshot was visually checked. A real report-publication conflict
then preserved two completed jobs and 23 checkpoints, retained the competing file
and was rejected by the summary reader. Results:
`benchmarks/keyboard-diagnostic-native-results.json`.

That probe also exposed a spurious `UI capture timed out` message after the native
error: a later close/repaint callback fell through into the ordinary screenshot
stage. The working tree now marks failed captures and closes without reentering
that stage, retaining isolated mode. A regression supplies a late screenshot and
an expired capture timer and checks that nothing is published. The native probe
now rejects the extra timeout message. **This follow-up is in verification** under
`target/qa/capture-abort-verification-20261017/STATE.json`; the earlier frozen build
does not contain that guard and is not the final candidate.

## Terminal result of the original dense run

The original process exited with code 1. It logged 24 completed rounds, 400 mask
edit settlements and 25 generated brush settlements, then `Timed out in RedoAll`.
No complete native report was produced. The external observer closed normally.
`benchmarks/maximum32-noise-incomplete-summary.json` records the failure, log hash,
observed counters and contention. It does not calculate passing acceptance targets.

The old error text conflated elapsed time with exhausting 300,000 frame records.
Observed process lifetime was 4,092.23 seconds, below the 4,200-second time limit,
so frame-storage exhaustion is the likely limiter. No internal final frame count
survived; this interpretation is explicitly an inference. The new diagnostic
separates those errors and reserves more frame storage for dense runs.

The capture-abort follow-up now passes 789 default / 798 all-feature tests, 21
Python checks, formatting, both Clippy configurations and release compilation.
Frozen build: `5C2554712F13117DEC605CEA14B5EDC573D3B480F8629BC5FB90F75A3317EAE9`,
identified in `benchmarks/capture-abort-build.json`. Its stricter native publication
conflict probe is running under `target/qa/native-diagnostic-failure-fixed-20261017`.
Normal native smoke and new full-length resource runs still need this build.

For the next long study, use controllers from the immutable matching source
extraction rather than editing the running controller's files. The old controller
kept executing its original loaded code, but Python's traceback displayed lines
from subsequently edited disk files; its terminal error/state remain authoritative.

The stricter native conflict probe passed on the capture-abort build: two jobs and
23 checkpoints preserved, conflict unchanged, incomplete summary rejected and no
spurious screenshot timeout. See benchmarks/native-diagnostic-failure-fixed-results.json.
The ordinary native smoke run for the same build is now running.
