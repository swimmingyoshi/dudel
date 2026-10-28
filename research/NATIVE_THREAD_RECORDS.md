# Native heap growth from per-call threads

2026-10-17. Follow-up to `WGPU_DEFERRED_DESTROY.md`, which removed the retained
Rust component of repeated-use DX12 growth. Generated native-window evidence on
the RTX 3060; not physical input or release certification.

## Locating the remainder

The optional `allocation-profile` build now records, at every native checkpoint,
a fixed-storage census of process memory (`src/region_census.rs`): committed
private/mapped/image bytes, private reservations bucketed by span, and
`HeapSummary` for each process heap. At each round's final idle it also walks
the default heap (`HeapWalk` under `HeapLock`), keying busy blocks by their first
word when it points into a loaded image (usually a vtable) and otherwise by exact
size. `DUDEL_HEAP_SAMPLE_SIZE` copies up to 256 blocks of one size for inspection.

Twelve-round DX12 session, rounds 4-12 at matching idle:

- Mapped, image and large private reservations were constant. Only the default
  process heap grew: +42 MiB committed, +26 MiB allocated, while live Rust
  requests stayed exactly flat.
- The heap walk attributed the whole +26 MiB to blocks of exactly **184 bytes**,
  +18,414 per round. They carry no vtable at offset 0.
- Sampled blocks hold a pointer into `nvwgf2umx.dll` (NVIDIA's user-mode D3D
  driver), a consecutive kernel-handle pair `(n, n+4)` and list links.
- Native growth was ~1.5 KiB per MiB of display upload in every phase, i.e.
  proportional to display composition work, not to frames.

`examples/dx12_upload_probe.rs` repeats single operations on a headless DX12
device and counts 184-byte blocks. Texture/buffer writes, submits, GPU copies and
texture/buffer creation added none; **each short-lived thread spawn/join added
exactly one** (+200 for 200). The NVIDIA driver keeps a per-thread record that is
not released when the thread exits.

`Document::composite_region` started up to 16 scoped threads per call for
regions of at least 65,536 pixels, and display batches composite tiles in row
strips, so routine refreshes created ~18,000 threads per round. Mask sampling
also used per-call scoped threads.

## Fix

`src/parallel.rs` provides persistent band workers (`available_parallelism - 1`,
at most 15) with a blocking `run(count, job)` in which the caller also works.
Panics are returned per index; every started job finishes before return; helpers
that start after the caller has finished skip the call, so nested calls from pool
threads cannot deadlock. Compositing and mask tile generation use it; their
error/panic behavior is unchanged.

## Result

Same workload and profiled configuration (`allocation-profile`), before
(`target/qa/heap-walk/dudel.exe`) and after (`target/qa/band-pool/dudel.exe`):

| Rounds 4-12, final idle | Before | After |
| --- | ---: | ---: |
| 184-byte driver blocks added per round | 18,414 | ~1,450 |
| Default-heap allocated drift, MiB | +25.94 | +2.15 |
| Native (heap allocated - Rust live) at final idle, round 12, MiB | 51.9 | 16.5 |
| Final private / resident, MiB | 577.9 / 299.6 | 548.4 / 271.7 |
| Display worker milliseconds, median per completed batch | 5.53 | 5.22 |

Private commitment drift is now dominated by heap commit that is not
decommitted (+20 MiB committed for +2 MiB allocated) and varies between rounds.
About 1,450 threads per round remain from per-job worker spawns (display
batches, history settlement, previews, save/load); each still leaves one record.
Moving those onto long-lived workers is the next step; this is not a memory-gate
pass. Worker timing comes from an instrumented build and is indicative only.

Raw reports stay local; heap-walk sidecars, summaries and captures:
`benchmarks/{region-census,heap-walk,band-pool}-dx12-sustained-100.*`.
Normal (764) and all-feature (773) suites pass with one high-memory test ignored;
warnings-denied Clippy, formatting and the optimized build pass.
