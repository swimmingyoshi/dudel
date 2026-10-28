# Duration-controlled native workloads

The duration workload retains the existing mask/history/save-open protocol and
adds a measured minimum duration plus an optional three-brush schedule. Ordinary
application drawing behavior is unchanged.

## Configuration

`DUDEL_SOAK_MIN_SECONDS` is zero (legacy fixed-round behavior) or 60-21600 seconds.
`DUDEL_SOAK_BRUSHES` is `controlled` or `mixed`. The short native mask smoke ignores
these overrides. The Python runner exposes `--minimum-seconds` and `--brushes`.

With a duration floor, the requested round count is a minimum. The diagnostic
finishes only after a complete final-idle checkpoint satisfies both that count
and the elapsed duration observed from its first native callback. It can extend
to 240 rounds, with fixed checkpoint/job storage reserved at startup. Frame samples
use a fixed 64 KiB RAM buffer and spool to a temporary JSONL file independently of
callback rate. Version 3 streams every frame into the final JSON report without
materializing the frame array; checkpoint capacity/used bytes describe RAM staging.
Temporary disk use grows with recorded samples and is removed on session close.
Write, flush, corrupt-record or report-publication errors fail the diagnostic.
Exhausting another bound produces an explicitly incomplete report, not a shorter
passing workload. Reports retain requested
minimum rounds separately from actual rounds; fixture and checkpoint round counts
describe the actual completed workload.

Mixed mode cycles through unchanged built-in G-pen and Pencil presets and an
explicit generated QA textured stamp with a 2x2 image/grain fixture. It records
all settings and the round-index schedule. Mixed mode always reserves at least
three rounds so every configuration is exercised. This is scripted input, not
physical tablet or brush-library UI acceptance.

## Continuity and power

The Windows controller temporarily requests system/display activity on its own
thread and restores its previous flags on exit. This does not change the power
scheme or prevent explicit user sleep. See Microsoft's
[execution-state documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate).

Each native process is bracketed by
[GetTickCount64](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-gettickcount64)
and [QueryUnbiasedInterruptTime](https://learn.microsoft.com/en-us/windows/win32/api/realtimeapiset/nf-realtimeapiset-queryunbiasedinterrupttime).
Their elapsed difference detects system suspension even inside a long diagnostic
step. A difference above two seconds invalidates the run. No sleep was induced
in testing; mocked suspension is rejected and actual acquisition/restoration and
clock reads passed on the reference PC.

Duration requests also require a callback trace spanning the observed session
within one second. Unexplained callback gaps above two seconds are rejected after
subtracting the previous diagnostic driver, measured frame-log I/O and current
measured UI prefix. Buffered logging has its own reported overhead distribution;
it is not counted as application UI work. Version 3 requires a matching frame
count and logging-time fields; legacy reports remain readable.
This continuity check does not establish frame/queue performance or physical
latency. One earlier standard run's largest such residual was 455.84 ms; the
three-minute integration probe's maximum was 75.98 ms.

## Historical verification

Frozen executable `7F765F63EEA25BCDC2B625FF2A666ECC7F4B3E09DB674D3DD0B5A5C623B654D8`:
791 default and 800 all-feature tests pass, two large-memory tests ignored normally.
Formatting, both warnings-denied Clippy configurations and release compilation
pass. All 28 Python checks pass, including mode separation, duration/trace mismatch,
suspension rejection and restoration when work raises an exception.

The actual native probe requested two minimum rounds, four sparse layers, mixed
brushes and 180 observed seconds. It completed five rounds / 181.854 seconds,
80 mask edit settlements, five generated strokes and five save/open pairs. Its
clock checks detected no suspension and the execution request was restored.
Evidence: `benchmarks/duration-continuity-build.json` and
`benchmarks/duration-mixed-probe-results.json`. The final capture was generated;
visual review of this probe remains separate from prior native smoke captures.

`--probe-only` permits small semantic/instrumentation probes. It cannot request
resource acceptance, reports those targets as unevaluated and terminates with
`probe-completed`. The ordinary acceptance runner still requires at least 30
rounds and 20 layers. The small probe is not a two-hour or maximum-document pass.

## Buffered recorder follow-up (2026-10-18)

The prior maximum noisy run filled 1,152,000 in-memory frame slots after 7,585
observed seconds, with 28 complete rounds. Its 120-callback/second sizing assumption
was invalid; partial evidence remains incomplete. The new recorder replaces that
rate-derived limit with fixed buffered temporary storage and streaming report
publication. It retains the same workload minima, artwork comparisons and budgets.

Executable D41C0072...2173011A passes 856 default / 865 all-feature tests, both
Clippy configurations, release compilation and 32 Python checks. A native DX12
probe passes six rounds / 198.31 seconds, all 32,004 frame records, constant 64 KiB
staging and twelve completed save/open jobs. Logging p95 is 0.0064 ms; this is
recorder overhead, not pen or UI latency acceptance. Continuity and power-state
restoration pass and the capture was reviewed. Exact source/probe identities:
[recorder verification](../benchmarks/frame-log-fix-verification-20261018.json).
This small probe does not evaluate standard or maximum resource targets.

## Full study

After matching-source packaging/rebuild verification, run controllers from that
immutable source extraction and place outputs inside its `target/qa` directory.
Do not edit the running controller's source files or perform competing builds
during isolated timing collection. Example maximum-document study:

```powershell
python scripts/run_soak.py --executable PATH_TO_CANDIDATE_EXE --build PATH_TO_BUILD_JSON --output target/qa/maximum32-mixed-two-hours --runs 1 --rounds 30 --layers 32 --density noise --minimum-seconds 7200 --brushes mixed --require-job-peaks
```

Complete the three standard repeats separately on the same candidate. Existing
private-commitment and completed-job budgets are unchanged. Full sustained,
frame/queue, GPU, cold-cache and physical acceptance remain governed by
`DELIVERY_PLAN.md` and `RELEASE_CHECKLIST.md`.
