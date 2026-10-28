# Persistent display workers: 2026-10-17

## Change and ownership

Baseline is `dcb5a73`, which already moved composite bands and mask sampling to
persistent workers. `NATIVE_THREAD_RECORDS.md` identified NVIDIA DX12 per-thread
records and left approximately 1,450 new short-lived threads per native round.

`canvas_textures.rs` now submits owned display-compositing jobs to
`display_workers.rs`: four persistent threads, at most 64 queued jobs, and a
nonblocking submission API. This pool is separate from the borrowed band pool,
because display jobs wait for band work. It is dedicated to display jobs; those
jobs must not synchronously wait for other display jobs.

Queue exhaustion or thread-start failure returns to the existing sliced display
fallback. Panics are caught per job and retain the existing display failure
handling. Completion is published only after the input closure has been consumed
and its captured source dropped, preserving snapshot-accounting semantics.
Canceled/retired jobs retain their existing cancellation and publication fences.
Dropping a result handle does not terminate a worker or block the UI.

## Reproduction and build identity

Frozen profile executable/PDB and detailed local build identity:
`target/qa/display-pool-20261017/`. Profile executable SHA-256:
`F3E21B812FC1A8B2822BA54E9C98239DA4CC286CD422B2A9B00A0B530E87A9B5`.
Normal executable SHA-256:
`22547ECEEF04CB0A4CB653653A6927F1147FAC004A22DFA38C52A44F4BCACD24`.
[Tracked source/environment identity](../benchmarks/display-pool-build.json)
includes every Rust input hash, Windows build, CPU, RAM, GPU and driver.

Set `DUDEL_RENDERER=dx12` in the child environment and run the frozen executable:

```text
--soak-native-drawing-session benchmarks/display-pool-dx12-sustained-100.json 1280 800 1
```

The isolated generated fixture is 2048 x 2048 with four layers and a dense mask.
The run completed in 411.17 seconds, exit zero: twelve generated brush strokes,
192 independently byte-checked mask edits, per-edit and complete-sequence Undo/
Redo, Cancel and twelve save/reopens. All 288 checkpoints passed existing coverage
and fixed-storage validation. Final native capture was inspected.

## Memory result

The same analysis validates both original band-pool and new display-pool reports.
Tail comparison excludes rounds 1-3 and uses final reopened/history-released idle.
The 184-byte bucket is a proxy for the previously identified native thread
records; size alone does not independently identify every block's owner.

| Matching tail measurement, rounds 4-12 | Band pool baseline | Display pool |
| --- | ---: | ---: |
| 184-byte block fitted increase / round | 1,450.65 | 42.68 |
| Default-heap busy-byte net increase, MiB | 2.146 | 0.185 |
| Process-private net increase, MiB | 19.707 | 11.719 |
| Process-private fitted increase, MiB/round | 1.758 | 1.692 |
| Rust live requested-byte net increase, MiB | 0.0064 | 0.0065 |
| Final private commitment, MiB | 548.355 | 541.578 |
| Final 184-byte bucket count | 17,439 | 547 |

GPU final-idle ownership remains constant throughout the new run: 35.25 MiB
suballocated, 320 MiB reserved, 148 allocations and two blocks. History is zero at
each released final idle. Live artwork/display snapshot accounting remains stable.

This removes approximately 97% of the remaining size-bucket growth. Private
commitment has not demonstrated plateau: its fitted tail slope still exceeds the
proposed 1 MiB/round target. Do not treat a smaller net drift as G2 acceptance.
Approximately 43 thread records/round remain across other job lifecycles; native
heap backing/fragmentation also needs longer observation. Do not force heap/GPU
cleanup or change renderer defaults based on this result.

Local verification/build/test activity overlapped the run. These are process-local
ownership observations from a single diagnostic session, not controlled speed,
frame-rate or physical pen-latency measurements. The fixture is four layers, not
the plan's standard 20-layer or maximum-dense acceptance workloads.

Summary generation:

```powershell
python research/analyze_native_retention.py benchmarks/band-pool-dx12-sustained-100.json benchmarks/display-pool-dx12-sustained-100.json --output NEW_RETENTION_SUMMARY.json
python research/analyze_native_session.py benchmarks/display-pool-dx12-sustained-100.json --output NEW_SESSION_SUMMARY.json
```

Tracked summaries: `benchmarks/display-pool-retention-summary.json` and
`benchmarks/display-pool-dx12-summary.json`. Raw session and heap walks stay local
under `benchmarks/` (gitignored); the capture is
`benchmarks/display-pool-dx12-sustained-100.png`.

## Validation and distribution work

- Default suite: 767 passed, zero failed, one high-memory test ignored normally.
- All-feature suite: 776 passed, zero failed, the same one ignored normally.
- Dedicated maximum-supported binary-project test: passed separately in 94.28
  seconds, covering 4096 x 1024 / 32 layers and project/recovery round trips.
- Formatting, default/all-feature all-target warnings-denied Clippy and optimized
  default/profile builds pass. The verification script completed successfully.
- Eight Python analysis tests pass, including malformed/partial heap evidence
  rejection and independently calculated retention arithmetic.
- Pool tests cover thread reuse, panic recovery, dropped result handles,
  nonblocking saturation/rejected-capture release and source release before ready.
  Existing cancellation/stale-display/snapshot tests run against the new jobs.
- Packaged normal executable launched in isolated OpenGL capture mode, exit zero;
  screenshot inspected. This is not a clean-machine or physical tablet result.
- Windows CI, a fail-fast local verification script, candidate ZIP/checksum
  packaging and dependency-metadata inventory are implemented locally. Hosted CI
  has not been run; no remote publication occurred. Public-release license,
  notices, signing/channel and clean-machine acceptance remain open.

Next: longer unprofiled memory runs with the acceptance fixtures, then remaining
per-job thread lifetimes if they cause an observed blocker. Execute the artwork-
safety failure matrix and physical XP-Pen workflow on the identified candidate.
