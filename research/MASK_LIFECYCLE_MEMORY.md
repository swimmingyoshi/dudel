# Mask application lifecycle memory check

2026-10-16. Follow-up to the higher resident snapshots in
`MASK_TRANSFORM_PERFORMANCE.md`. This is a bounded same-process CPU lifecycle
check, not native GUI or long-session release acceptance.

## Method

Run the optimized executable with `--soak-mask-transforms NEW_REPORT.json`.
Each of 64 cycles creates 2048x2048 dense, fractional-selection and sparse mask
fixtures. Each fixture runs Move, nearest integer affine and bilinear fractional
translations, with both ready previews and pending final requests. Independent
full-canvas byte/selection oracles, artwork-sharing/flag checks and one-action
Undo/Redo must pass: 18 operations per cycle, 1,152 per process. Source, result,
history and completed worker ownership is released before each post-cycle sample.
Only small scalar summaries accumulate in the report.

The Windows process-memory query now uses PROCESS_MEMORY_COUNTERS_EX to report
working-set bytes and private committed bytes. Neither is an isolated live-heap
measurement; snapshots do not capture peaks. The probe does not trim the heap or
evict the working set. It uses the actual application acquisition/publication/
history paths without a native window, display compositing, GPU presentation,
OS clipboard, saving/recovery, input drivers or pen input. It creates a fresh app
for each operation, so it cannot detect retained state unique to a persistent
GUI session or growing user undo history. Existing reports are never replaced.

## Results

| Run | Seconds | Verified operations | Post-cycle resident range, MiB | Post-cycle private committed range, MiB |
| --- | ---: | ---: | ---: | ---: |
| 1 | 118.9 | 1,152 | 35.77-45.57 | 2.60-13.38 |
| 2 | 128.7 | 1,152 | 10.68-20.61 | 2.61-13.84 |

Median snapshots after cycles 9-16 versus 57-64 (descriptive samples, not a
predefined acceptance threshold):

- Run 1: resident 36.42 -> 42.53 MiB; private committed 3.26 -> 10.41 MiB.
- Run 2: resident 11.37 -> 11.85 MiB; private committed 3.88 -> 3.72 MiB.

Both invocations completed without oracle/history failures: **2,304 verified
operations**. Private committed memory fluctuated; run 1 ended above its early
warmed level, while run 2 returned near that level after larger intermediate
values. The evidence does not show a reproducible monotonic increase across
these bounded lifecycles. It also does not establish a stable native-session
plateau, exclude small leaks, or identify allocator retention as the cause.
The two resident ranges differ substantially, illustrating why resident memory
alone is insufficient to infer live retained ownership. No heap trimming or
working-set eviction was requested by the probe.

Each retained report is about 14.5 kB. Previous performance reports sampled while
results/history remained live; these post-cycle samples are taken after release.
They are different lifetimes and do not establish a reduction from that baseline.

## Reproduction and verification

- Reports: `benchmarks/mask-lifecycle-soak-release-{1,2}.json`.
- Progress logs: `target/qa/mask-lifecycle-soak-{1,2}.log`.
- Release binary/frozen probe SHA-256:
  `AF6B85D34BABE494E82AE213D94537E738A4AF48CC864A01717EDB741EFE8A5E`.
  Lifecycle-checkpoint candidate (historical): `target/release/dudel.exe`; frozen probe:
  `target/qa/dudel-mask-soak.exe`.
- Run twice with distinct new report paths. The command exits before opening a
  window/loading preferences. Existing-report refusal was verified with an
  unchanged destination hash and exit 1; missing output arguments also fail.
- Full suite: **725 passed, zero failed, one existing high-memory test ignored**.
  The existing independent-oracle regression now also exercises a small soak
  cycle. Warnings-denied all-target Clippy, formatting, whitespace and release
  build pass. Logs: `target/qa/mask-soak-{tests,clippy,release}.log`.

## Remaining acceptance

Measure a persistent native session that repeatedly edits, displays, applies,
undoes and redoes supported layered artwork, then saves/reopens and idles. Track
resident/private memory together with tracked live/history/display allocations
and frame scheduling; growing undo history must be distinguished from released
ownership. Use allocation profiling if retained private memory grows across
comparable idle checkpoints. This lifecycle check does not close that gate.
Dense fractional-transform latency, native display/GPU timing, physical XP-Pen,
OS clipboard, supported interop and packaging checks also remain open.


## Follow-up native evidence

`NATIVE_MASK_SESSION.md` records short persistent OpenGL window checks at 100/200%
application zoom with real display updates, accumulating/released history and
asynchronous save/reopen. Both final runs passed. History release recovers about
60 MiB resident memory, but the initial private-commit increase remains unexplained.
These short sequences do not close sustained-session, GPU-time or hardware gates.
The follow-up report identifies the newer candidate; the frozen binaries and
measurements recorded above remain historical evidence.
