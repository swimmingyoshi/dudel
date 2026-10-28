# Mask sampling latency and target-aware tool guidance

2026-10-16. Follow-up to `NATIVE_MASK_SESSION.md`. Dense mask sampling was still
roughly 200 ms despite responsive UI updates. This checkpoint reduces sampled
mask completion latency while preserving coverage, sampling arithmetic and history.

## Implementation

Bilinear sampling converts the footprint's integer origin once. Interior 2x2
footprints share a tile-directory lookup and stride instead of performing four
independent floating-coordinate lookups. Canvas edges retain white coverage;
tile seams and partial edge tiles use correct neighboring tiles. The original
multiplication/addition order, rounding and clamping are retained. Nearest and
bicubic sampling behavior is unchanged. Inlining exposes coordinate mapping,
mask lookup and sampling to the optimizer; inlining alone was insufficient.

Large whole-mask sampled output uses independent tile batches. At one million
pixels and above, the policy chooses up to four sampling workers, bounded by
reported parallelism minus one and roughly one worker per 512K pixels. These
are children of the existing background edit worker; other display/OS work is
scheduled separately. Small sampled masks, selections and integer Move retain
their existing scheduling. Thread creation failure computes that batch locally.
Every launched child is joined before return, including cancellation/errors;
private results publish only after successful completion and quota validation.
Unchanged tile allocations and the original default coverage are preserved;
uniform results retain compact storage. Canvas/layer/payload limits are unchanged.

Tool guidance now follows the captured artwork/mask target. Move's toolbar,
sidebar and rail explain selected coverage versus the whole mask. Mask paint,
eraser, fill, picker and lasso rail hints describe conceal/reveal/coverage rather
than artwork RGB operations. Artwork guidance retains its existing text.

## CPU comparison

Two before/after pairs were interleaved with no concurrent agent build/test or
native diagnostic. Each invocation runs the existing 90-case application probe
at supported 1024-square, 2048-square and 4096x1024 sizes, including ready and
pending-final-request phases. All 360 measurements passed independent coverage/
selection byte oracles, unchanged artwork/flags and one-action Undo/Redo.

Values below are 2048-square ready-preview **wall milliseconds**, including
worker completion and application publication, excluding independent verification.
They are not total CPU consumption, GPU time, frame rate or input latency.
Parallel work uses more cores; a single-core machine uses the serial path.

| Profile | Operation | Before, two-run range | After, two-run range |
| --- | --- | ---: | ---: |
| Whole sparse mask | Move | 0.515-0.607 | 0.473-0.518 |
| Whole sparse mask | Nearest integer affine | 0.524-0.946 | 0.549-0.714 |
| Whole sparse mask | Bilinear fractional affine | 1.276-1.286 | 1.210-1.269 |
| Whole dense mask | Move | 2.812-3.335 | 3.129-3.697 |
| Whole dense mask | Nearest integer affine | 3.328-3.598 | 2.895-3.219 |
| Whole dense mask | Bilinear fractional affine | 203.353-203.578 | 36.381-36.868 |
| 64x64 rectangle | Move | 0.679-0.689 | 0.668-0.732 |
| 64x64 rectangle | Nearest integer affine | 0.666-0.681 | 0.876-1.005 |
| 64x64 rectangle | Bilinear fractional affine | 1.112-1.161 | 1.113-1.121 |
| 64x64 fractional selection | Move | 1.007-1.372 | 1.088-1.169 |
| 64x64 fractional selection | Nearest integer affine | 1.095-1.153 | 1.196-1.358 |
| 64x64 fractional selection | Bilinear fractional affine | 1.536-1.693 | 1.389-1.625 |
| Folder-mask rectangle | Move | 0.717-0.724 | 0.645-0.711 |
| Folder-mask rectangle | Nearest integer affine | 0.671-0.681 | 0.674-0.934 |
| Folder-mask rectangle | Bilinear fractional affine | 1.207-1.282 | 1.142-1.406 |

The dense fractional case improves from **203.35-203.58 ms to 36.38-36.87 ms**,
about 5.5x faster in this fixture. Small control cases vary: several integer
or selected cases are slightly slower in individual runs, so this does not
establish a universal speedup. The measured workload does not certify arbitrary
rotation, perspective, mesh, bicubic, dense selections or 32-layer responsiveness.

Across 180 application measurements on each side, the observed maximum owner
call was 0.725 ms before and 1.333 ms after; Undo maxima 0.602/0.495 ms and Redo
maxima 0.335/0.319 ms. Whole-process resident snapshots ranged 11.38-37.15 MiB
before and 11.43-53.79 MiB after. These are snapshots, not peaks or live-allocation
counts. No memory-reduction or leak-freedom claim follows; thread/allocator/
runtime overhead is not included in logical tile payload counts.

## Native integration and UI

Both rebuilt short native workflows passed at 100% and 200% application zoom:
clean Cancel, sixteen mask translations with independent byte checks, per-edit
and full-sequence Undo/Redo, history release, and asynchronous save/reopen with
fresh mask identity. The same four-layer 2048-square fixture and OpenGL renderer
from `NATIVE_MASK_SESSION.md` were used. Diagnostic driver/oracles remain separate
from UI-prefix timing, and callback intervals include their overhead.

| Zoom | Seconds | Callback samples | UI prefix median, ms | p95, ms | Maximum, ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| 100% | 32.64 | 2314 | 2.023 | 3.089 | 5.485 |
| 200% | 32.26 | 2285 | 1.833 | 2.729 | 5.571 |

Final screenshots were inspected at both scales. The new Move toolbar guidance
fits, the 100% sidebar describes mask coverage, and the transformed mask,
stationary artwork, target and saved feedback agree. At 200% the automatic
compact layout retains the Mask target and uses status ellipsis.

Native tracked history again reaches 60.44 MiB and returns to zero after release;
resident snapshots drop from 234.35 to 173.70 MiB at 100%, and 179.36 to 117.01
MiB at 200%, before save/reopen. First-preview private commitment still rises
by roughly 65-67 MiB without equivalent tracked document ownership. This remains
unexplained and requires profiling; these short sequences do not close the
sustained native-session memory gate. The fixed diagnostic baseline ownership
and distinct decoded artwork after reopening are tracked as in the earlier report.

## Verification and reproduction

- **727 passed, zero failed, one existing high-memory test ignored**. Two added
  regressions compare bilinear sampling against independent dense scalar storage
  over sparse/dense/default masks, 1-pixel canvases, tile/canvas boundaries,
  fractional coordinates, rounding ties and non-finite input; and verify serial/
  parallel partial-tile output, shared unchanged tiles, cancellation, worker error
  and worker panic with joined/discarded private results.
- Warnings-denied all-target Clippy, formatting, whitespace and optimized build
  pass. Logs: `target/qa/mask-sampler-{tests,clippy,release}.log`.
- Before reports: `benchmarks/mask-sampler-before-release-{1,2}.json`.
  Before executable SHA-256:
  `D18205F57DF818BFC52FC672C17A939DD9BB365C2C89600C3E198B11D8A1126D`;
  retained as `target/qa/dudel-native-session.exe`.
- After reports: `benchmarks/mask-sampler-after-release-{1,2}.json`.
  Sampler-checkpoint release and identical frozen probe SHA-256:
  `AA95700D4A4A14F877E03E60CDC54905E65DF9962050BF1D350F22EEA50A0003`.
  Frozen executable: `target/qa/dudel-mask-sampler-parallel.exe`.
- Repeat CPU probes with `--benchmark-mask-transforms NEW_REPORT.json` on the
  retained executables. Reports are not overwritten. Intermediate inline/tile
  probes remain under `target/qa/mask-sampler-{before,after,taps}-*.json`.
- Native reports: `benchmarks/mask-sampler-native-release-{100,200}.json`.
  Images/logs: `target/qa/mask-sampler-native-{100,200}.{png,log}`. Reproduce with
  `--soak-native-mask-session NEW_REPORT.json 1280 800 1` (or final scale `2`).

## Remaining acceptance

A 36-37 ms result for this dense fractional case still exceeds a 60 Hz frame
interval; it does not establish full-rate transform interaction. Measure broader
geometry/selection workloads before choosing further CPU/GPU preview work, while
preserving final sampling and cancellation. Profile native private commitment
over longer comparable idle checkpoints, and complete real drawing, physical
XP-Pen, external clipboard, supported interop and packaging checks. The full
professional application goal stays active.


## Later brush-workflow candidate

`BRUSH_CREATION_WORKFLOW.md` identifies the newer brush-creation/UI candidate
with 732 tests passing. This sampler report's retained binary and measured data
remain checkpoint evidence; no newer timing claim is inferred from that build.
