# Persistent native mask session: workflow, UI timing and memory

2026-10-16. A short scripted native-window integration check follows the earlier
headless CPU/lifecycle probes. This is implemented, byte-verified and visually
checked evidence; it does not establish sustained-session or hardware acceptance.
The later repeated drawing/resource study is recorded in `SUSTAINED_NATIVE_DRAWING.md`.

## Workload and boundaries

`dudel.exe --soak-native-mask-session NEW_REPORT.json [WIDTH HEIGHT SCALE]`
opens a visible isolated native window with a generated supported 2048x2048
four-layer document: one dense masked raster and three sparse artwork layers.
It loads no user artwork/preferences, ignores external input during the scripted
session, suppresses preference/window persistence, and uses a temporary project
file. The tested renderer is OpenGL. The companion final screenshot uses the
report stem with `.png`; existing report/screenshot paths are rejected at startup.
Missing paths and non-JSON report names fail. Diagnostic failure without a
verified report returns failure. An isolated renderer fallback no longer writes
the user's global display settings.

One native app remains alive throughout each sequence:

1. Warm the normal canvas/navigator/thumbnails and idle for two seconds.
2. Begin a full-quality bilinear half-pixel mask translation and Cancel. The
   exact clean source and empty history must be restored.
3. Apply sixteen translations of [17.5, -8.5]. Each completed mask is compared
   against the independent full-canvas four-tap oracle; all artwork allocations
   remain shared. Each edit is undone and redone, checking exact snapshots.
   History accumulates across the sixteen edits.
4. Undo all sixteen actions and compare the initial document; redo all sixteen
   and compare the final document, also checking stack exhaustion.
5. Release history, idle for two seconds, and sample memory. Save asynchronously
   through the app, reopen through its actual background loader, require fresh
   mask identity, and compare artwork and coverage/flags. A failed load leaving
   the old document in place cannot satisfy that identity guard.
6. Idle again, sample memory and capture the native UI before closing only the
   diagnostic window. The temporary project is removed with the session.

The app-side settled condition requires three consecutive callbacks with no
pending texture/render work or dirty canvas, matching thumbnail revision and a
ready affine preview where applicable. This is not a GPU fence or proof of the
instant a pixel appeared on the monitor.

UI timing ends immediately before the diagnostic driver. Independent byte
oracles, assertions, command driving and report writing are timed separately.
Callback intervals include that diagnostic work, OS scheduling and renderer
activity; they are not GPU execution times, FPS or pen-to-display latency.
The diagnostic driver itself reached 104-121 ms during verification in these
runs, so callback intervals must not be presented as ordinary user frame times.

## Native results

Both final runs completed Cancel, all sixteen byte-verified edits, per-edit and
complete-sequence Undo/Redo, and asynchronous save/reopen. Each report contains
2,496 native callback samples and 23 settled memory checkpoints.

| Application zoom | Viewport, points | Pixels per point | Seconds | UI prefix median, ms | UI prefix p95, ms | UI prefix max, ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 100% | 1280x800 | 1 | 34.62 | 1.810 | 2.811 | 5.069 |
| 200% | 640x400 | 2 | 34.58 | 1.632 | 2.532 | 4.699 |

For active Preview/Applied/Undone/Redone/UndoAll/RedoAll callbacks specifically,
p95 UI-prefix times were 2.869 ms at 100% and 2.593 ms at 200%. These results
support responsive CPU UI updates in this fixture, not prompt completion of a
dense fractional transform. The earlier 193-200 ms CPU worker issue remains.

## Memory evidence

All numbers below are MiB process snapshots, not peaks or isolated live heaps.
History/display allocation estimates exclude allocator/driver overhead. The fixed
diagnostic reference document is explicitly included in tracked allocations.
Frame samples use an unchanged preallocated 0.5 MiB vector in both runs.

| Zoom | Checkpoint | Resident | Private committed | Tracked history |
| --- | --- | ---: | ---: | ---: |
| 100% | initial-idle | 189.09 | 228.86 | 0.00 |
| 100% | cancel-restored | 190.71 | 297.14 | 0.00 |
| 100% | all-redone | 254.98 | 361.66 | 60.44 |
| 100% | history-released-before-save | 195.21 | 308.66 | 0.00 |
| 100% | final-idle-history-released | 217.11 | 323.93 | 0.00 |
| 200% | initial-idle | 110.41 | 174.75 | 0.00 |
| 200% | cancel-restored | 111.96 | 243.92 | 0.00 |
| 200% | all-redone | 177.33 | 308.23 | 60.44 |
| 200% | history-released-before-save | 116.57 | 255.34 | 0.00 |
| 200% | final-idle-history-released | 138.88 | 269.71 | 0.00 |

The accumulating edits retain about 60.44 MiB of history. Releasing history before
save reduces resident memory by about 60 MiB in each run and tracked history to
zero. At that checkpoint tracked allocations beyond the live document are about
4.03 MiB, including the diagnostic initial-mask reference. This is expected
retained test ownership rather than an undisclosed live-history leak.

Entering and canceling the first preview increases private committed memory by
about 68-69 MiB while tracked additional document/history allocations remain
zero and resident memory changes by about 1.6 MiB. Its cause is not established;
allocator/renderer/driver attribution would require profiling. After history
release, private commitment remains above the initial idle. Do not declare this
memory gate closed or claim the unexplained commitment was recovered.

After reopening, tracked additional allocations are about 25.53 MiB: the fixed
initial reference now owns artwork separately from the decoded document, whereas
it shared the original artwork before reopening. Final resident/private snapshots
therefore cannot be interpreted as a same-ownership comparison with initial idle.
Initial process memory differs substantially between runs; DPI is not an
established cause of the process-memory differences.

## Visual and build verification

The final 100% and 200% screenshots were inspected. Mask focus, saved feedback,
the same transformed coverage and stationary artwork are visible. At 100% the
navigator and layer/mask thumbnails agree with the canvas. At 200% responsive
panels hide automatically, the Mask target remains directly accessible, and the
status text uses ellipsis. Physical pen behavior and OS clipboard were not tested.

Artifacts:

- Durable reports: `benchmarks/native-mask-session-release-{100,200}.json`.
- Captures/logs: `target/qa/native-mask-session-final-{100,200}.{png,log}`.
- Native-checkpoint release and identical frozen native probe SHA-256:
  `D18205F57DF818BFC52FC672C17A939DD9BB365C2C89600C3E198B11D8A1126D`.
  Frozen executable: `target/qa/dudel-native-session.exe`.
- Full suite: **725 passed, zero failed, one existing high-memory test ignored**.
  Warnings-denied all-target Clippy, formatting, whitespace and optimized build
  pass. Logs: `target/qa/native-session-{tests,clippy,release}.log`.
- CLI refusal checks passed for an existing report, missing report path and
  non-JSON output name. Earlier native exploratory runs are superseded by the
  final reports above, which require fresh identity after reopen.

Local inventory: Windows 11 Pro build 26200; AMD Ryzen 5 3600X (6 cores/12 logical
processors); installed NVIDIA GeForce RTX 3060, driver 32.0.15.9579. The app reports
OpenGL; inventory alone does not identify all driver allocations or measure GPU
execution. Inventory record: `target/qa/native-session-environment.json`.

## Remaining work

These are roughly 35-second native sequences, not hour-long drawing sessions.
Repeat comparable idle checkpoints over sustained native use and profile the
unexplained initial private commitment. Extend coverage to realistic brush,
selection, save/recovery and clipboard workloads and other supported renderers.
The later `MASK_SAMPLER_OPTIMIZATION.md` checkpoint reduces the measured dense
fractional case to 36-37 ms and delivers target-aware Move/painting guidance.
Broader geometry responsiveness and full-rate interaction remain unverified.
Physical XP-Pen, external interchange and packaging gates remain open. The full
professional drawing-application goal remains active.


## Follow-up sampler build

`MASK_SAMPLER_OPTIMIZATION.md` identifies the newer candidate and repeats the
native workflow at both scales with 727 tests passing. This report's original
frozen native binary and measurements remain historical evidence. Its private-
commit finding and sustained-session/hardware limitations remain open.
