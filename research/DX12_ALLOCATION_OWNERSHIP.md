# DirectX 12 allocation ownership study

2026-10-17. Follow-up to the repeated-use process-memory growth in
`SUSTAINED_NATIVE_DX12.md`. Document correctness and ownership evidence remain
separate; this study does not certify a release or physical pen behavior.

## Rust request profile

The preserved optional-allocation build
`target/qa/dudel-fixed-checkpoints-profile.exe` (SHA-256
`AF1C238713F0A5B4673A1597EBF29730DBE829BF617F5AF113AFFFBB8ACBCD4A`) ran the same
twelve-round native workload on actual DX12/RTX 3060 at 1280x800/100%. It exited
zero after 406.868 seconds and 29,349 callbacks, with all 12 generated strokes,
192 byte-oracle transforms, 12 save/reopens and 288 checkpoints verified.

| Matching idle stage | Rust live-request drift, rounds 4-12, MiB | Process-private drift, MiB |
| --- | ---: | ---: |
| Reset fixture | +26.069 | +62.824 |
| History released, before save | +30.069 | +69.512 |
| Reopened, history released | +30.069 | +77.484 |

Final reopened idle held 108.583 MiB of live Rust requests. This reproduces the
larger process-memory finding with a retained Rust component, unlike the small
OpenGL Rust drift after fixing diagnostic checkpoint storage. Registered document/
display ownership and released history do not explain that component. Global
Rust accounting includes dependencies and UI/renderer bookkeeping; it does not
locate allocation call stacks or establish which owner is leaking/caching.

Raw report/capture: `benchmarks/sustained-native-dx12-profile-100.{json,png}`.
Checked summary: `benchmarks/sustained-native-dx12-profile-summary.json`.
Rust/process figure: `benchmarks/sustained-native-dx12-rust-memory.{png,svg}`.
Launch logs: `target/qa/sustained-native-dx12-profile-100.{stdout,stderr}.log`.
The optional hooks alter execution timing; use the normal-build report for its
recorded CPU timing, and do not infer a controlled latency comparison here.

## Optional GPU suballocator summary

The existing `allocation-profile` feature now also records supported wgpu device
allocator reports at native checkpoints. `gpu_profile.rs` copies allocated/
reserved bytes and allocation/block counts into fixed-size records. Report
vectors and names drop before live Rust requests are sampled. No report names,
resource lists or mutable JSON trees are retained across checkpoints. The
diagnostic session holds one additional cloned device handle while active.

This uses the pinned wgpu 27.0.1 API and wgpu-hal 27.0.4 DX12 implementation;
unsupported or absent devices produce null. Normal builds compile out the device
handle and report calls. Summary generation can allocate temporarily and take
the backend allocator lock, so it remains diagnostic overhead and may affect
heap backing/lifetime peaks. It does not poll/wait the GPU or force reclamation.

These are **GPU suballocator** values, not complete VRAM or driver allocation
totals. Ownership outside that allocator is excluded. GPU, Rust-request and
process counters are overlapping/incomplete views, not additive budgets.

Frozen probe executable: `target/qa/dudel-gpu-allocation-profile.exe`, SHA-256
`8652B3EE92D48869657F33FEA9BAACABEC950C6F785AB9B9C35D3D4F11781549`.
Its short DX12 workload exited zero with all 23 reportable GPU samples and the
usual correctness checks. Initial and reopened idle both reported 36,962,304
suballocated bytes, 335,544,320 reserved bytes, 148 allocations and two blocks.
Raw report/capture: `benchmarks/gpu-profile-dx12-short-100.{json,png}`; logs:
`target/qa/gpu-profile-dx12-short-100.{stdout,stderr}.log`.

The analyzer accepts historical reports without GPU data and emits optional GPU
series and matching-idle drift when available. A generated numeric fixture checked
unit conversion/counts without treating those synthetic values as app evidence.

## Source review boundary

The pinned egui-wgpu renderer removes freed textures after queue submission;
wgpu-core submission already performs nonblocking maintenance. No missing free
or absent maintenance call was established by this review. Do not add forced
GPU waits or change renderer defaults without an observed causal fix.

Upstream [multi-monitor/DPI report](https://github.com/emilk/egui/issues/4674)
concerns a different trigger/version. The broader
[backend discussion](https://github.com/emilk/egui/issues/7761) includes memory
and latency observations but does not establish the cause of this fixed-window
workload. The reviewed [v27.0.4 release](https://github.com/gfx-rs/wgpu/releases/tag/v27.0.4)
was not accepted as evidence of a matching fix. Dependencies were not changed.

## Sustained GPU-profile result

The new probe completed the same twelve-round DX12 workload with exit zero:
412.704 seconds, 29,765 callbacks, 12 generated strokes, 192 independently checked
mask edits and 12 save/reopens. All 288 GPU and Rust profile samples were available
in a fixed 69,120-byte checkpoint reserve (240-byte records). History returned to
zero at released/initial/final idle; registered snapshot/display ownership stayed
constant at matching tail checkpoints.

All **36** initial/before-save/final idle samples across all twelve rounds reported
exactly **35.25 MiB suballocated, 320 MiB reserved, 148 allocations, two blocks**.
This is stronger than comparing only the first/last GPU sample. Nevertheless:

| Matching idle stage | Rust live drift, rounds 4-12, MiB | Process-private drift, MiB | GPU suballocated/reserved drift, MiB |
| --- | ---: | ---: | ---: |
| Reset fixture | +26.071 | +60.008 | 0 / 0 |
| History released, before save | +30.069 | +69.871 | 0 / 0 |
| Reopened, history released | +30.070 | +64.625 | 0 / 0 |

Final reopened idle held 108.593 MiB live Rust requests versus 623.055 MiB private
commitment and 323.434 MiB resident. The independently reproduced Rust growth
does not track growing GPU suballocation counts or bytes. Next identify retained
Rust allocation call stacks/owners, then address the observed retention. The
non-Rust process remainder is still unassigned; neither stable suballocator
totals nor a retained Rust component identify a leak owner by themselves.

Raw report and inspected capture:
`benchmarks/gpu-profile-dx12-sustained-100.{json,png}`. Checked summary:
`benchmarks/gpu-profile-dx12-sustained-summary.json`. Launch logs:
`target/qa/gpu-profile-dx12-sustained-100.{stdout,stderr}.log`. The Rust/process
figure above was also visually inspected. No CPU/GPU timing claim follows from
the instrumented report-generation work.

The professional application goal and resource/hardware release gates remain
open. No renderer defaults, forced cleanup policy or dependency versions changed.

## Build and compatibility checks

Full suites: **763 profiling / 760 normal passed**, zero failed, one existing
high-memory test ignored in each. Default/all-feature all-target warnings-denied
Clippy, formatting, Git whitespace and optimized builds passed. Logs:
`target/qa/gpu-profile-{tests,clippy,release,normal-tests,normal-clippy,normal-release}.log`
and `target/qa/gpu-profile-whitespace.log`.

Native short modes also exited zero on profiled OpenGL and normal OpenGL. The
profiled OpenGL report keeps Rust samples but reports GPU allocation data as
unavailable (null); the normal report disables both profile flags and omits GPU
samples. This is not reported as measured zero GPU usage. All three short cases
(DX12 available, OpenGL unavailable, normal disabled) passed the analyzer's
cycle/checkpoint/fixed-storage checks. Summaries:
`benchmarks/gpu-profile-short-summary.json`; raw files/captures:
`benchmarks/gpu-profile-{dx12,gl,normal}-short-100.{json,png}`. Corresponding logs
use the same stems under `target/qa/`. All three captures were inspected.

Restored normal release and frozen `target/qa/dudel-gpu-profile-normal.exe`
SHA-256: `15CA2C30D15F7983FE07CBCB964487B762507E8AEF1B1802F979A6F8D2AB9DD6`.
The probe remains opt-in; ordinary drawing does not call the report API.

For the next call-stack study, WPR is installed but the current terminal is not
elevated. UMDH/GFlags were not found on PATH or in the checked SDK debugger paths.
A release PDB exists; freeze its matching executable/PDB pair when building the
next diagnostic instead of assuming that a later overwritten PDB matches an old
frozen binary. These observations do not block process-local instrumentation.
