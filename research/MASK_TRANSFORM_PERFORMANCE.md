# Mask transform CPU measurements

2026-10-16. This checkpoint measures the application ownership boundary and
worker-inclusive completion, not visible frame rate or physical pen latency.

## Workloads and verification

`dudel.exe --benchmark-mask-transforms NEW_REPORT.json` runs supported
1024x1024, 2048x2048 and 4096x1024 documents. Five profiles cover whole sparse
and dense masks, a 64x64 rectangular selection on a dense mask, a fractional
selection, and a folder mask. Each runs integer Move, nearest integer affine
translation and bilinear half-pixel translation, both as a ready preview and as
a final request handed to the background queue: 90 measurements per invocation.
Every source passes current document validation; project limits are unchanged.

The timer includes actual application source acquisition, frozen-source preview
publication and FIFO history settlement. Explicit worker waits sit outside
owner-call timers. Full-canvas integer-coordinate/four-tap scalar oracles verify
mask bytes and moved selection coverage; artwork allocation and mask metadata
must remain unchanged. Undo restores the source in one action and Redo restores
the result. Verification work is outside the reported preview/finish timers.
This probe excludes compositing, GPU/display work, OS input, frame scheduling and
physical tablet latency; it cannot support an FPS or input-latency claim.

## Changes

Selected integer moves visit the source rectangle plus disjoint destination
strips. They neither scan the surrounding canvas nor the gap between distant
regions. Exact rectangular selections retain rectangle metadata without a
temporary full-canvas byte bitmap; off-canvas empty selections remain explicit.
Soft selections retain their 8-bit coverage. Checked wide arithmetic handles
extreme integer offsets. Pure integer affine translations use the same exact
byte path for all three sampling modes.

Dense whole-mask integer moves copy source tile-row spans. Full sampled results
are built by tile rather than acquiring copy-on-write access per pixel. Both
retain the original default and identical tile allocations, preserving redraw
reuse and packed size. A near-limit regression uses 32 shared raster layers at
2048x2047: the moved default-43 mask must allocate only its white edge band,
not a dense white-default replacement that would exceed the document quota.

Generated selection bounds are accumulated while writing coverage. The same
full-canvas bitmap is retained for fractional selections, but surrounding zeros
are not rescanned. Clearing writes invalidate the bounds cache and use the
checked full scan. Rectangle detection examines only the enclosing bounds;
constructors ensure all non-zero coverage lies inside them. Empty selections,
overwrites, cancellation and scanned-reference equivalence are tested.

Mostly-white whole masks can move/flip from their stored tile bounds into a white
destination. Sparse affine transforms project those bounds with the existing
filter-support padding. This path is limited to affine mappings and small
projected work; other backgrounds, dense masks, large projected regions and mesh
warps retain the full-quality fallback. Large constant results retain compact
storage. Source/slot guards, cancellation and payload validation still precede
publication. No sampling quality was reduced.

An intermediate implementation accelerated selected moves but added per-pixel
region checks/counters to control cases. Measurements exposed a small whole-sparse
Move regression. Region decomposition and area accounting now happen outside the
pixel loop, and the sparse whole-mask path avoids scanning white background.
Intermediate reports are retained as `mask-transform-optimized-{1,2}.json`,
`mask-transform-final-{1,2}.json` and `mask-transform-release-{1,2}.json`.

## Memory interpretation

Mask payload bytes and selection capacity are storage counts, not total heap.
Windows resident values are process snapshots before/after measurements, not a
peak or isolated operation working set. Retained allocator/runtime pages and
other live allocations affect them. The rectangle path removes a temporary
coverage vector by construction; the resident snapshots do not establish a peak
memory reduction. Worker scratch and display/GPU allocations are not separately
attributed by this probe.

## Results

Two release observations before and two after, on this Windows environment.
These are local ranges, not confidence intervals or portable performance targets.
The table shows worker-inclusive ready-preview wall time on 2048x2048 documents;
owner admission/publication is included, while verification is excluded.

| Mask/selection | Operation | Before (ms) | After (ms) |
| --- | --- | ---: | ---: |

| Whole sparse mask | Move | 42.219-42.733 | 0.492-0.574 |
| Whole sparse mask | Nearest affine translation | 108.025-109.105 | 0.522-0.534 |
| Whole sparse mask | Bilinear half-pixel translation | 196.540-198.796 | 1.284-1.306 |
| Whole dense mask | Move | 75.425-75.776 | 2.677-2.942 |
| Whole dense mask | Nearest affine translation | 136.419-140.764 | 2.627-2.775 |
| Whole dense mask | Bilinear half-pixel translation | 254.907-269.979 | 193.350-199.564 |
| 64x64 rectangle on dense mask | Move | 91.102-107.751 | 0.690-0.764 |
| 64x64 rectangle on dense mask | Nearest affine translation | 11.627-12.701 | 0.628-0.635 |
| 64x64 rectangle on dense mask | Bilinear half-pixel translation | 8.758-10.139 | 1.038-1.107 |
| 64x64 fractional selection | Move | 94.150-115.826 | 0.973-1.017 |
| 64x64 fractional selection | Nearest affine translation | 5.953-6.538 | 1.040-1.073 |
| 64x64 fractional selection | Bilinear half-pixel translation | 5.387-6.501 | 1.543-2.242 |
| 64x64 rectangle on folder mask | Move | 87.538-107.345 | 0.625-0.766 |
| 64x64 rectangle on folder mask | Nearest affine translation | 9.958-12.626 | 0.618-0.856 |
| 64x64 rectangle on folder mask | Bilinear half-pixel translation | 7.582-10.355 | 1.096-1.173 |

Before: observed owner-call maximum **0.756 ms** across 180 application measurements; Undo/Redo maxima 0.696/0.306 ms. Process-resident snapshots ranged 11.3-38.8 MiB.

After: observed owner-call maximum **0.550 ms** across 180 application measurements; Undo/Redo maxima 0.472/0.280 ms. Process-resident snapshots ranged 11.4-62.2 MiB.

Resident snapshots are higher after the changes (up to 62.2 MiB versus 38.8
MiB). They are not isolated live-allocation or peak measurements; no memory
reduction or leak conclusion is supported. Session-level memory/allocator
behavior still needs a soak. Logical mask/selection storage and quota semantics
are checked separately by the fixtures and near-limit regression.

## Verification and identities

- Full suite: **725 passed, zero failed, one existing high-memory test ignored**.
- All-target Clippy with warnings denied, formatting, whitespace and optimized
  build pass. Logs: `target/qa/mask-performance-{tests,clippy,release}.log`.
- Reports: `target/qa/mask-transform-baseline-{1,2}.json` and
  `target/qa/mask-transform-verified-{1,2}.json`. Each invocation verified every
  reported coverage/selection pixel, unchanged artwork/flags and one-action
  Undo/Redo. Before executable SHA-256:
  `1421C329C5AA6FD5F7FB0F257882A720EABA858DF96DDBA322E5121E4C2D0229`.
  The before executable was not retained.
- Performance-checkpoint and frozen probe executable SHA-256:
  `7BD99B0E24B371C9041F755B15BE0EC505EE5FC568DDA287B341C711C122898E`.
  The frozen binary is `target/qa/dudel-mask-transform-final.exe`; rerun with
  `--benchmark-mask-transforms NEW_REPORT.json` without overwriting a report.
- Native mask-target scale/rotation captures at 100% and 200% were inspected:
  `target/qa/mask-performance-preview-{100,200}.png`. Canvas, navigator/thumbnail
  where visible, target and Apply/Cancel feedback agree. These are static visual
  checks and do not time presentation or input.

## Remaining work

Dense fractional transforms still take roughly 193-200 ms in the 2048-square
CPU probe. This remains a responsiveness issue, despite faster common moves and
sparse transforms. Measure native compositing/presentation and realistic layered
scenes before choosing further sampler or display-preview work. Preserve full
sampling quality, cancellation, final results and Undo throughout. Native frame,
GPU, sustained-session memory, physical XP-Pen and external clipboard checks
remain open; this checkpoint does not establish professional release readiness.


## Follow-up lifecycle memory evidence

`MASK_LIFECYCLE_MEMORY.md` records two 64-cycle same-process release checks with
resident and private committed memory after document/history release. All 2,304
operations passed independent byte/history verification. The two runs fluctuated
and did not show a reproducible monotonic increase; persistent native-session
memory acceptance remains open. These post-release snapshots cannot establish a
reduction from this report's live result/history snapshots. The follow-up report
identifies the newer instrumented candidate; this report's frozen performance
binary and measurements remain unchanged.


## Follow-up native evidence

`NATIVE_MASK_SESSION.md` records short persistent OpenGL window checks at 100/200%
application zoom with real display updates, accumulating/released history and
asynchronous save/reopen. Both final runs passed. History release recovers about
60 MiB resident memory, but the initial private-commit increase remains unexplained.
These short sequences do not close sustained-session, GPU-time or hardware gates.
The follow-up report identifies the newer candidate; the frozen binaries and
measurements recorded above remain historical evidence.


## Follow-up sampled-mask performance and guidance

`MASK_SAMPLER_OPTIMIZATION.md` records the newer candidate: 727 tests pass,
interior bilinear footprints share tile lookups, and large full-mask sampling uses
bounded children of the edit worker with joined cancellation/failure handling.
The two-run 2048-square dense fractional case improves from about 203 ms to
36-37 ms. Target-aware mask tool guidance is visible at 100/200%; both native
workflows pass again. Earlier measurements and frozen binaries above remain
checkpoint evidence. Full-rate interaction, sustained native memory and physical/
external interoperability gates are not closed by that improvement.
