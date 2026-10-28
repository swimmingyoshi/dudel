# Optional native Rust allocation profile

2026-10-16. Follow-up to `SUSTAINED_NATIVE_DRAWING.md` and its unexplained
process-memory changes at preview and reopened idle.

## Measurement

Build with `cargo build --offline --release --features allocation-profile`.
Copy that executable to a distinct diagnostic path before rebuilding the normal
release. Run either existing isolated native diagnostic with new output paths:

```text
dudel-allocation-profile.exe --soak-native-mask-session NEW_REPORT.json 1280 800 1
dudel-allocation-profile.exe --soak-native-drawing-session NEW_REPORT.json 1280 800 1
```

The optional feature installs a `System` global allocator wrapper that counts
successful Rust allocation requests. Without the feature, this module and its
allocator hooks are absent. Native reports identify the feature explicitly;
uninstrumented checkpoints have `rust_allocations: null`.

Hooks allocate nothing, acquire no locks, and perform no formatting. They use
relaxed atomic counters for live requested bytes, lifetime peak, successful
allocation/reallocation/deallocation counts, and cumulative acquired/released
requested extents. Successful reallocations account the full old/new extents;
failed allocations/reallocations leave the counters unchanged. Shrinking and
freeing reduce live bytes. Layout/alignment and memory operations delegate to
`System` unchanged. Tests check accounting balance, concurrent updates, zeroed
memory, alignment and preserved data across growth/shrink.

These are requested bytes across Rust code and dependencies using its global
allocator. Heap backing, size-class overhead, fragmentation, external C/driver
allocations and GPU memory are excluded. Counters are sampled independently
while workers may run; a checkpoint is not a stop-the-world heap census. The
diagnostic reference documents, frame reserve, report/checkpoint storage and
temporary registry work are included. Cumulative totals measure churn, not
retained memory. Lifetime peak does not localize a transient to one operation.
Do not add registered snapshots/display estimates to global live Rust bytes:
these are different, potentially overlapping views of ownership.

Atomic hooks alter execution timing. Profiled timings are not an uninstrumented
responsiveness baseline. Compare matching phases within a run; short mode is
one round without a brush stroke and cannot establish sustained stability.
Process-private commitment minus live requested Rust bytes does not identify
any particular external owner. A discrepancy warrants further heap/renderer
profiling, not a leak verdict or speculative release/allocator tuning.

`research/analyze_native_session.py` includes optional allocation checkpoint
series and matching-idle Rust live drift, and still parses historical reports.

## Evidence

Frozen profiled executable: `target/qa/dudel-allocation-profile.exe`, SHA-256
`5DD3D50A86CAABB4EF65AFD62BF5A27B3B6A908093FF89884041ACC306F32110`.

The two one-round OpenGL checks exited zero and completed all sixteen mask edits,
Cancel, Undo/Redo, history release and save/reopen. Reports and visually inspected
screenshots: `benchmarks/allocation-profile-short-{100,200}.{json,png}`. Launch
logs: `target/qa/allocation-profile-short-{100,200}.{stdout,stderr}.log`.

| Measurement | 100% | 200% |
| --- | ---: | ---: |
| Elapsed seconds | 32.105 | 31.869 |
| First cancellation: private commitment increase, MiB | 64.922 | 65.039 |
| First cancellation: live Rust request increase, KiB | 22.634 | 17.451 |
| Initial live Rust requests, MiB | 30.434 | 30.242 |
| Cancelled-idle live Rust requests, MiB | 30.456 | 30.259 |
| Before-save history-released live Rust requests, MiB | 33.915 | 33.717 |
| Reopened final-idle live Rust requests, MiB | 55.523 | 55.246 |
| Lifetime requested-byte peak by final idle, MiB | 93.946 | 93.530 |

The first-preview discrepancy reproduced without a similar retained Rust-buffer
increase. This narrows the investigation to heap backing/overhead or allocations
outside these hooks; it does not distinguish allocator behavior from UI/renderer/
driver ownership. Different absolute memory at the two scales is not a controlled
scaling conclusion. Reopened versus initial ownership also differs deliberately:
the diagnostic keeps the original baseline while decoding a new document.

The twelve-round OpenGL run at 100% also exited zero: 409.779 seconds, 29,364
callbacks, 12 generated strokes, 192 oracle-checked mask edits, and 12 save/reopens.
All 288 checkpoints were present. History peaked at 76.448 MiB and returned to
zero at every history-released/initial/final idle. Registered snapshots and
display allocations were exactly constant at matching checkpoints in rounds
4 through 12. Lifetime peak requested Rust bytes reached 136.951 MiB.

| Matching idle stage | Rust live drift, rounds 4 to 12, MiB | Process private drift, MiB |
| --- | ---: | ---: |
| Reset fixture | +0.693 | +3.781 |
| History released, before save | +0.693 | +4.516 |
| Reopened, history released | +0.693 | +2.250 |

Final reopened idle retained 65.903 MiB of Rust requests versus 296.996 MiB
process-private commitment and 153.316 MiB resident. The first cancellation
increased private commitment by 68.938 MiB and live Rust requests by 22.634 KiB.
The small live drift is identical at all three stages and is consistent with
growing diagnostic checkpoint metadata, but has not been independently assigned
to that owner. Frame storage was already reserved; used trace bytes increased
by 1.644 MiB over the tail, which is not additional requested Vec capacity.
Do not describe Rust live requests as perfectly constant or the private-memory
remainder as a measured driver allocation.

Report and inspected screenshot:
`benchmarks/allocation-profile-sustained-100.{json,png}`. Logs:
`target/qa/allocation-profile-sustained-100.{stdout,stderr}.log`.
`benchmarks/allocation-profile-summary.json` holds checked short/long allocation
series. The analyzer also accepted a historical unprofiled report and rejected
a generated report missing one allocation checkpoint.

Feature-enabled full suite: **762 passed, zero failed, one existing high-memory
test ignored**; all-target/all-feature warnings-denied Clippy and optimized
offline build passed. Logs: `target/qa/allocation-profile-{tests,clippy,release}.log`.

Restored default build: **759 passed, zero failed, one existing high-memory test
ignored**; default all-target warnings-denied Clippy, formatting, Git whitespace
and optimized build passed. Logs: `target/qa/allocation-normal-{tests,clippy,release}.log`
and `target/qa/allocation-profile-whitespace-result.log`. Normal release and frozen
`target/qa/dudel-allocation-normal.exe` SHA-256:
`F93D0159533BF17A2080A9900E3A981AE207772C67609E79E2152778FB06EE9E`.
The normal short native run also exited zero with all 23 checkpoints, sixteen
mask edits and save/reopen verified; its report explicitly disables allocation
profiling and all allocation checkpoint fields are null. Artifacts:
`benchmarks/allocation-normal-short-100.{json,png}`; logs:
`target/qa/allocation-normal-short-100.{stdout,stderr}.log`.

Follow-up `FIXED_NATIVE_CHECKPOINTS.md` replaces growing JSON records with a fixed
reserve, reducing the matching-idle Rust drift to 6.25 KiB, and adds matched short
OpenGL/DX12 checks. The memory gate remains open. Next profile heap backing/external
renderer ownership and sustained modern-renderer behavior; do not tune the
production allocator or force heap reclamation from these counters.
Physical XP-Pen input, actual clipboard exchange, renderer/GPU interaction,
supported CSP fixtures and packaging remain independent release gates.
