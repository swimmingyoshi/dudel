# Drawing performance and workflow audit — 2026-10-22

Audited source baseline: `a3170d045df6bdb43288aaeb5a2f4bd464a4e4b7`.
This change implements measured brush and large-canvas optimizations with a focused UI cleanup.
It retains the supported 4096-pixel side limit and existing document budgets.

Release evidence review, 2026-10-28: the owner confirms repeated working Artist 12
use and working Explorer previews. The sustained study passed, and current
viewport/timelapse/dab/fill/document sources match their later focused benchmark
records. [Evidence reuse](benchmarks/release-evidence-reuse-20261028.json) supersedes
blanket retest requests. Profiling suggestions below are specific research
questions, not a standing request for another unattended or artist session.

## Implemented

- **Batch raster writes by row span.** Previously, `paint_dab_rows` resolved
  tile ownership through `Arc::make_mut` for each covered pixel. Coverage now
  uses fixed 4 KiB scratch and writes contiguous covered spans, resolving
  ownership once per tile segment. Zero-coverage gaps remain untouched, retaining
  sparse allocation behavior. Pixel order, blending arithmetic, stroke ceilings
  and row-level cooperative scheduling remain intact. Masks and short flat
  rows retain direct writes, where batching cannot amortize tile ownership.
- **Clip to selection bounds before raster evaluation.** Rectangular exclusions
  no longer perform per-pixel geometry work. An entirely excluded slice returns
  before creating a copy-on-write writer. Soft selection coverage still applies
  to both delivery and stroke opacity ceiling.
- **Remove per-frame brush clones in both preview callers.** Preview workers
  still own a copy when a render actually starts. Idle UI frames borrow the
  current brush, including its image/grain assets and dynamics curves.
- **Simplify brush properties.** Basic exposes Size, Opacity, Stabilizer and
  Smooth/Rope follow. Flow, Hardness and pressure switches share one expandable
  section. Tip, Texture and Advanced remain available; Studio retains the full
  editor and test pad. Existing presets and shortcuts retain their behavior.
- **Shorten canvas creation.** Sketch (1024 square), Square (2048 square), 4K
  (3840 × 2160), orientation swap, custom dimensions and a dense-layer memory
  estimate share one form. Enter on a preset or option edits that option;
  Enter in a dimension field creates the canvas. Cancel retains the document.

## Follow-up: timelapse and display memory

The follow-up replaces the full-resolution timelapse intermediate with bands
covering 64 output rows. It downsamples directly into RGB, using the existing
thumbnail window boundaries and integer rounding. Already-small canvases retain
one parallel composite and copy RGB directly, avoiding band-scheduling overhead. Repeat-frame hashes and exported frame pixels retain their meaning.
Tests compare against the original image-library thumbnail, including odd and
one-pixel dimensions, masks, opacity and blending.

Uniform display tiles now use a one-pixel texture. The first nonuniform edit
reconstructs the padded background and replaces that texture; later edits use
normal partial uploads. Full refreshes compact tiles again when they become
uniform. This applies to white paper, solid colors and transparent tiles. The
canvas and navigator still publish complete generations together. Padding and
mipmaps on nonuniform tiles retain their existing behavior.

This is a sparse-artwork optimization, not a general viewport residency budget.
The first stroke into a compact tile uploads a full 320-square tile once. A
canvas whose tiles are all nonuniform can still use the original texture budget.
Texture byte counts describe RGBA payload, not driver allocation or process RAM.

## Findings and next priorities

| Priority | Evidence in current source | Recommended next change |
| --- | --- | --- |
| High | Viewport resolution now retains detail near the visible area and quarter-resolution tiles elsewhere. The canvas and navigator share a complete generation. Very large/high-DPI viewports can still require full detail across much of the document. | Measure physical GPU allocation and sustained pan/zoom before adding a strict cross-project cap or replacing the tiled overview with one coarse navigator texture. |
| Medium | Timelapse uses bounded bands, waits for settled drawing and now observes a 150 ms quiet period before ordinary captures. A running worker can still overlap a subsequent stroke. | Measure recording on/off during sustained physical drawing before adding cooperative background-worker priority or trading exact frame pixels for a reduced-resolution composite. |
| Medium | `DabPaint` and tail replay are cooperative at row/work-unit boundaries. A deadline is checked between units, so it is an allowance, not a guaranteed latency cap. | Profile per-phase p50/p95/p99 under physical pen input, dense 4K layers, large textured brushes and symmetry. Use those results to decide whether row chunks or prepared coverage need further caching. |
| Medium | `workspace.rs`, `tool_panel.rs` and `brush_studio.rs` share many controls, but brush access remains spread across the options bar, properties and Studio. | Make the options bar the common drawing path, properties the contextual adjustment path, and Studio the preset-authoring path. Factor remaining duplicate control code around the existing shared section helpers; keep tool shortcuts and explicit panel choices. |
| Medium | Some benchmark descriptions still call 4096 square unsupported, while `document::MAX_SIDE` and validation accept it; README also described a single document despite tabs. | README is corrected here. Update older benchmark scope text when revising those workloads; distinguish sparse 20-layer fixtures from densely painted multi-layer acceptance. |

The existing engine already has sparse copy-on-write raster tiles, dirty-region
display uploads, prefix compositing caches, bounded stroke queues, background
file work, cached/virtualized brush previews and compressed history. Replacing
these wholesale would create substantial correctness risk. The next large-canvas
gain should be measured in display residency and full-frame background work.

Increasing the canvas limit is a separate change: validate storage formats,
mask/raster payload accounting, undo, export, recovery, resize and GPU residency
at the new size. Twenty mostly empty 4K layers do not establish dense-layer
memory or physical tablet performance.

## Validation and measurement

Release queue results on an AMD Ryzen 5 3600X, Rust 1.97.1, Windows x64:

| Canvas/storage | Brush | Before live-row CPU ms | After live-row CPU ms | Reduction |
| --- | --- | ---: | ---: | ---: |
| 512 square, sparse tiled | 300 px | 338.04 | 281.81 | 16.6% |
| 2048 square, dense flat | 300 px | 367.03 | 304.78 | 17.0% |
| 2048 square, dense tiled | 300 px | 341.54 | 297.50 | 12.9% |
| 2048 square, dense flat | 18 px | 12.68 | 12.97 | -2.3% |

These are medians of three alternating trials per version, summing the live-row
phase across each workload; they are not per-frame times. End-to-end queue drain
improved 9.1–15.4% for these large-brush cases. The small flat case's drain moved
from 33.90 to 34.01 ms, so there is no established improvement there. Small tiled
live-row results ranged from 0.4% to 4.9% lower. No uniform speedup is claimed.

The first brush/UI pass passed **287 focused Rust tests** (one large-memory test
ignored), Clippy for all targets, formatting and **36 Python tests**. Added
regressions compare span writes against scalar writes across multiple tile edges
with shared snapshots, and exercise keyboard preset/orientation changes without
accidental form submission. Existing tests cover exact undo/redo, taper,
selection, mask and compact-layout behavior.

Release engine and queue comparisons, exact tapered-stroke checksums, focused
regressions and native UI captures are recorded in the accompanying benchmark
summary, [brush-ux-audit-20261022.json](benchmarks/brush-ux-audit-20261022.json),
including executable/source hashes and reproduction commands.
Timings describe CPU workloads on this machine, not GPU frame rate or
pen-to-display latency. Three queue trials per version are descriptive medians,
not a statistical confidence claim. The broad engine workload has one baseline
and two final trials and includes sparse 4096-square lifecycle checks. All six
tapered-stroke output hashes agree across all three reports. The legacy large
soft brush varied from 51.44 to 65.21 ms after the change versus 57.57 ms in the
baseline; this does not establish an improvement for that individual workload.
Both final runs are retained instead of selecting only the faster result.

Native captures are available locally as
`artifacts/brush-audit/final-brush-basic.png` and
`artifacts/brush-audit/final-new-canvas.png`, showing the brush properties at
1280 × 800 and New Canvas at 960 × 640. Existing workflow
tests also exercise smaller layouts. Physical tablet feel and sustained native
GPU behavior still need a hands-on pass.

## Follow-up measurements and final verification

Three alternating full-frame/banded release trials in the same final test binary:

| Timelapse source | Original median CPU ms | Final median CPU ms | Source RGBA buffer before / after |
| --- | ---: | ---: | ---: |
| 512 square, blank | 2.30 | 2.23 | 1 MiB / 1 MiB |
| 2048 square, one dab | 67.83 | 45.06 | 16 MiB / 0.80 MiB |
| 4096 square, one dab | 144.92 | 113.04 | 64 MiB / 3.20 MiB |
| 4096 square, dense layer | 240.68 | 210.44 | 64 MiB / 3.20 MiB |

All output RGB bytes matched the original thumbnail path. These descriptive
medians show 12.6-22.0% lower CPU time in the two 4K cases; the small case is
approximately unchanged. Buffer counts exclude the RGB output, source snapshots,
compositor scratch and allocator overhead. The small-canvas single-composite
path avoids the slowdown observed with unconditional banding.

A blank 4096-square display uses **1 KiB** base RGBA texture payload, down from
**100 MiB**. A 300-pixel central dab expands four padded tiles, bringing base
payload to **1.56 MiB**, or **2.08 MiB** including estimated mip levels. Clearing
and refreshing returns it to 1 KiB. Metadata remains about 146 KiB for 256 tiles;
texture handles and driver allocation are additional costs. This probe measures
CPU-side payload accounting, not actual GPU commitment.

The existing 48-case display benchmark passed on both frozen builds, including
exact padded pixels, coherent canvas/navigator publication, retained snapshots
and upload byte accounting. In its 2048-square, single-layer case, synchronous
cold preparation moved from 68.88 to 58.19 ms and background owner CPU from
22.72 to 10.16 ms. The first synchronous dirty edit moved from 0.97 to 1.46 ms
because compact tiles need expansion. These are single runs with no statistical
speedup claim; all cases and their timings are retained in
[canvas-memory-audit-20261022.json](benchmarks/canvas-memory-audit-20261022.json).

Final matching-source verification passed **894 normal Rust tests** (five
explicit probes/large fixtures ignored), **two release performance probes**,
Clippy for all targets, formatting, and **36 Python tests**. The release
application was rebuilt. Native 1280 x 800 captures were inspected on OpenGL
and DirectX 12 / NVIDIA GeForce RTX 3060; local captures are
`artifacts/canvas-audit/opengl.png` and `artifacts/canvas-audit/dx12.png`.
The benchmark summary records executable/source/capture hashes and commands.
Fully nonuniform display residency and physical pen latency remain the next
profiling targets.

## Direct upload preparation - 2026-10-23

Display tiles now pack shared compositor output directly into premultiplied
upload pixels. This removes the intermediate padded RGBA copy from background
batches, synchronous full refreshes and shared dirty-rectangle updates. Edge
gutters reuse converted pixels/rows. Uniform tiles are recognized before a
full-tile allocation. Texture layout, filtering, resident budgets and atomic
canvas/navigator publication are unchanged.

For a nonuniform 320-square tile, local packing allocations fall from 800 KiB
to 400 KiB, excluding the shared composite and queued images. This reduces
preparation scratch; it does not halve the resident texture budget. Synchronous
conversion now falls within the composite timer, so compare total owner time
rather than isolated composite/upload phase timers against earlier reports.

All **895 normal Rust tests** passed (six explicit probes/large fixtures
ignored), along with Clippy for all targets, formatting and **36 Python tests**.
The new regression compares direct packing with the previous padded-RGBA path
for opaque, transparent and partial-alpha pixels, thin canvases, edge gutters
and partial dirty rectangles. Native OpenGL and DirectX 12 captures are
pixel-identical to the preceding build on the same renderer.

Five alternating-order pairs of 100 preparations in the same release binary:

| Upload packing case | Before median ms/tile | After median ms/tile | Reduction |
| --- | ---: | ---: | ---: |
| Dense opaque | 0.368 | 0.179 | 51.5% |
| Dense partial alpha | 0.487 | 0.322 | 33.9% |
| Partial-alpha canvas edge | 0.443 | 0.270 | 39.1% |
| Uniform full tile | 0.061 | 0.029 | 52.9% |
| 18-square dirty patch | 0.00104 | 0.00084 | 19.4% |

These isolate packing, not compositing, rendering or pen latency. The existing
48-case display benchmark also passed three alternating runs of each frozen
build. Whole-display timings were mixed: the 2048-square single-layer synchronous
cold median moved from 57.28 to 56.75 ms, while its background dirty-generation
owner total rose from 11.06 to 13.08 ms. Eight- and 32-layer dirty-generation
owner medians fell from 3.91 to 3.45 ms and 7.11 to 6.57 ms, respectively.
The unchanged worker/compositor times also varied; these runs do not establish
a general end-to-end speedup. All trials, allocation counts, hashes and commands
are retained in
[direct-upload-audit-20261023.json](benchmarks/direct-upload-audit-20261023.json).

## Applied efficiency roadmap - 2026-10-23

- **Viewport resolution.** On documents of at least 2048-square pixel area,
  visible tiles retain at least the screen sampling density (including DPI).
  A one-tile margin anticipates nearby pans. Offscreen tiles retain a quarter-size
  representation along each axis. A separate, consistently reduced navigator
  image preserves thin-detail coverage across both OpenGL and DirectX 12,
  independently of which main-view tiles are detailed. Its longest side has
  approximately 256 document samples. Resolution plans
  are frozen for each generation; the previous complete frame remains drawable
  until its replacement publishes. Pan, zoom, rotation and mirror changes request
  a successor instead of starving work already in progress. Constant tiles still
  use one texel. Small visible brush damage keeps the immediate partial-upload
  path; damage to reduced tiles recomposites and reduces them before replacement.
  Saved pixels and exports retain full resolution. Large preview windows use
  their own resolution plan, so their detail follows their own view and DPI.
  Small previews still share main-view textures when layer visibility agrees.
- **Latency diagnostics.** View > Performance now reports p50/p95/p99 for the
  last 256 UI updates and drawing-queue pumps, plus the longest observed queue
  unit by phase. Detailed unit timers run only while the window is open. Queue
  deadlines remain cooperative; this change does not promise a 2 ms hard cap or
  physical pen-to-display latency. Reset clears the timing samples.
- **Indexed compositing cache.** Prefix regions now use indexed lookup and
  constant-time removal instead of scanning/shifting the whole entry vector on
  every hit. Least-recently-used eviction remains bounded by the existing entry
  and 8 MiB limits, including index payload estimates. Hit, miss, bypass and
  eviction counts appear in Performance. Existing clipping, mask, nesting and
  snapshot invalidation rules remain intact.
- **Optional work yields to drawing.** Ordinary timelapse admission waits for a
  150 ms quiet period; explicit final-frame requests retain their existing settled
  capture behavior. Live brush previews defer starting jobs during drawing and
  discard superseded completed renders. Layer thumbnails defer during drawing
  and prepare at most two changed layers per update. Save/recovery scheduling is
  unchanged. This governs job admission, not OS thread priority or preemption of
  workers already running. Layer-thumbnail work is bounded, not viewport-virtualized.
- **Cached asset fingerprints.** Tip/grain byte hashes are cached lazily. Mutable
  access invalidates the cache, and serialized brush bytes remain unchanged.
  Tests cover mutation, cloning and wire-format round trips.
- **Contextual brush editing.** Properties and Studio share size/opacity and
  flow/hardness controls. Opening Studio from Properties selects the current
  Basic/Tip/Texture/Advanced section, reducing navigation while retaining the
  existing toolbar and panel choices.

The viewport policy bounds retained detail by the visible working area, with
6.64 MiB base RGBA for a dense 4K fit view, including its independent navigator
overview (6.25 MiB canvas + 0.39 MiB navigator). It is
not a fixed global VRAM cap: large/high-DPI windows, previous generations,
additional preview views and the most recently parked project can add memory.
Consolidating navigator tiles into one texture, physical GPU accounting, visibility-first
layer thumbnails and preemption of running background workers remain future
work. Brush work-unit splitting should follow the new phase/latency evidence;
no arbitrary stroke resampling or dropped input was introduced.


Final source verification passed **899 normal Rust tests**, **908 tests with all
features**, Clippy for all targets in both configurations, formatting, and
**36 Python tests**. Seven explicit probes/large fixtures remain excluded from
normal runs. The release application was rebuilt. Native 1280 x 800 fit and
rotated/mirrored 100% captures were inspected on OpenGL and DirectX 12 / NVIDIA
GeForce RTX 3060. The independent navigator image fixes a visible mixed-detail
artifact found during those checks. Pixel regressions independently verify
premultiplied reductions, immediate visible edits, offscreen edits, transparency
and atomic publication.


Dense 4096-square artwork, 800 x 600 viewport, one final release run:

| View | Retained base RGBA, including navigator | Peak with previous generation |
| --- | ---: | ---: |
| Fit | 6.64 MiB | 6.64 MiB |
| 100% detail | 19.82 MiB | 19.97 MiB |
| Pan, rotate and mirror | 18.73 MiB | 33.65 MiB |
| Return to fit | 6.64 MiB | 19.56 MiB |

The original fully detailed 256 padded textures require 100 MiB base RGBA.
These figures exclude source pixels, compositor/worker scratch, driver storage
and other windows. They are payload counts, not physical VRAM measurements.
The same probe checked 24 visible 18-square edits at 100%: all remained immediate,
with median/max owner display-update time of **0.243 / 0.304 ms**, including the
navigator patches. Writes and correctness checks are outside that timing; it
is not brush rasterization or physical input latency. Reduced tiles still use
background reconstruction when edited. The full-detail partial-upload path
remains immediate, and the policy makes no hard wall-time guarantee.


Three alternating moving-display trials per frozen build passed all 24 cases,
including final pixels, immediate visibility and bounded cache accounting. This
benchmark does not set a viewport: it checks the existing full-detail dirty
path and cache, separately from adaptive residency. CPU results do **not**
establish a net speedup: 19 of 24 case medians increased, while five decreased.
The largest increase in accumulated owner time was 2.04 ms across 48 calls
(24 edits plus idle updates). For the 2048-square tiled cases, 18-square edits
on one/eight/32 layers moved from 2.36/6.63/19.40 ms to 2.61/7.04/19.64 ms;
32-layer 80-square edits moved from 31.20 to 33.15 ms. These descriptive medians
include extra display bookkeeping and OS timing variation; they do not isolate
a cause. No universal CPU or input-latency improvement is claimed for this pass.
The queue probe also verified all six edit pipelines on the final binary.

The complete per-case summaries, viewport samples, validation commands,
executable/source hashes and inspected capture hashes are recorded in
[efficiency-audit-20261023.json](benchmarks/efficiency-audit-20261023.json).
Raw local traces and images are under `artifacts/efficiency/`.


## Cached viewport plans and reduced-tile patches - 2026-10-23

The next pass implements the first two responsiveness priorities:

- **Reuse unchanged viewport plans.** A cache key includes document dimensions,
  viewport bounds, pan/origin, zoom, rotation, mirroring, document center and
  physical pixel density. Small canvases ignore irrelevant view changes. The
  desired plan remains separate from the frozen in-flight generation; a view
  change requests a successor while allowing the current frame to finish.
  Repeated checks reuse the cached vector instead of allocating and transforming
  every tile again. Its retained capacity is included in metadata accounting.
- **Patch reduced textures directly.** Small edits now composite an aligned
  source region and reduce it into the existing canvas and navigator textures.
  Box-filter coverage and padded gutters remain exact. Compact tiles expand
  directly to their retained resolution. Tiny marks can survive canvas reduction
  while averaging to the navigator's old color; a constant overview is retained
  in that case so navigator rendering stays consistent. Full-detail edits keep
  their existing path. Larger damage, stale metadata and unfinished generations
  still use background preparation and atomic publication.

This supersedes the preceding pass's restriction that reduced-tile edits always
require background reconstruction. Saved artwork, export resolution and the
supported canvas-size limit are unchanged.


Three alternating frozen-build trials, dense partial-alpha 4096-square artwork,
800 x 600 viewport, 24 sequential settled edits per case and trial:

| Zoom | Damage | Before owner ms | After owner ms | After p95 ms | Queued RGBA before / after |
| --- | --- | ---: | ---: | ---: | --- |
| Fit (13%) | 18 square | 247.318 | 0.251 | 0.353 | 6.64 MiB / 1.06 KiB |
| Fit (13%) | 80 square | 248.693 | 1.037 | 1.372 | 6.64 MiB / 4.78 KiB |
| 50% | 18 square | 280.519 | 0.255 | 0.384 | 12.50 MiB / 4.06 KiB |
| 50% | 80 square | 275.438 | 1.074 | 1.410 | 12.50 MiB / 18.28 KiB |
| 100% | 18 square | 0.279 | 0.258 | 0.380 | 5.13 KiB / 5.13 KiB |
| 100% | 80 square | 1.347 | 1.332 | 1.732 | 50.33 KiB / 50.33 KiB |

Owner medians sum elapsed Context::run time through publication. The previous
reduced path rebuilt the complete display generation in this dense flat-storage
fixture; its queued bytes equaled the retained display payload on each edit.
The new path acknowledges complete small damage and uploads aligned patches.
All **288 reduced-resolution edits** completed in their initial update, versus
zero previously. All 144 full-detail edits remained immediate in both builds.
Retained texture payload is unchanged: 6.64 MiB fit, 12.50 MiB at 50%, and
19.82 MiB at 100% in this viewport.

These results isolate display updates, not brush rasterization or physical pen
latency. Owner timing excludes worker CPU, pixel writes, setup, validation and
polling sleeps. Sequential edits wait for settlement; this is not a sustained
input trace. The report separately retains polling wall time and p99, with 72
samples per case/build. No hard deadline or statistical confidence is claimed.
The existing 24-case full-detail moving-display benchmark passed three times per
build: 15 owner-total medians fell and nine rose, so there is no universal
full-detail CPU speedup claim.

Final full verification passed **901 normal Rust tests**, **910 all-feature
tests**, Clippy in both configurations, formatting and **36 Python tests**.
Eight explicit probes/large fixtures stay outside normal test runs. The new
release probe passed on both frozen binaries in all six alternating runs.
Measurements, individual trials and executable/source hashes are in
[viewport-patches-audit-20261023.json](benchmarks/viewport-patches-audit-20261023.json).


Native OpenGL and DirectX 12 checks also completed on the final release build.
Each used four patterned 4096-square layers, two rounds, a generated 240-sample
brush stroke per round, and 16 mask-edit cycles per round. Stroke Undo/Redo,
mask oracles, complete Undo/Redo sequences, and asynchronous save/reopen all
passed. Both final canvas/navigator captures were inspected. These are automated
correctness checks; generated packets bypass physical tablet delivery. Raw native
reports and captures are retained under `artifacts/viewport-patches/`, with
hashes and checkpoint counts in the benchmark summary.

## Bucket fill scanlines — 2026-10-24

The bucket's per-pixel frontier and destination writes repeated work millions
of times on a 4096-square canvas. Connected-region traversal now groups pixels
into horizontal runs, and destination writes resolve copy-on-write storage once
per tile segment. The same connectivity routine serves advanced fills, mask
fills and the contiguous wand. It preserves four-neighbor connectivity and
checks cancellation during traversal, including inside long rows.

Edge-preserving fills write their solid component in runs and retain the
existing individual soft-edge coverage calculation. A component covering the
entire canvas has no outside boundary, so a packed-bit check skips that boundary
search. For other components, source colors are read only when an outside
neighbor needs examination. Ordinary fully opaque pixel blends also avoid
unnecessary floating-point blending. Selection coverage, alpha locks,
premultiplied tolerance and transparent hidden-RGB semantics are unchanged.

The initial UI settings enable **Fill to edges**, with tolerance **12**. These
were benchmarked explicitly, separately from ordinary fills with tolerance zero
and edge preservation disabled. Median worker completion times for the default
settings were:

| 4096 × 4096 fixture | Before | After |
| --- | ---: | ---: |
| Empty/sparse layer, fill entire layer | 1,582 ms | 655 ms |
| Dense flat layer, fill entire layer | 1,641 ms | 443 ms |
| Dense tiled layer, fill entire layer | 1,647 ms | 575 ms |
| Dense tiled layer with 24 history entries, fill entire layer | 1,681 ms | 528 ms |
| Dense flat destination, visible-layer reference, half-canvas fill | 1,018 ms | 615 ms |

With edge preservation disabled, full-canvas worker medians across these four
storage/history fixtures fell from 1.19–1.81 seconds to 0.41–0.51 seconds. The
plain baseline was measured before the overnight pause and is retained as a
separate comparison, rather than pooled with the default-UI measurements.

Each number is the median of three release samples. Worker time includes fill
preparation, publication, history settlement and 1 ms polling sleeps; it excludes
screen refresh and GPU work. Direct fill timings are recorded separately. These
are local benchmark results, not guaranteed click-to-visible latency. Large
visible-reference fills still require compositing, and gap closing/expansion
still require morphology passes.

Every benchmark checks all destination pixels, unchanged reference pixels and
worker undo; its fixed 512/2048 gap/expansion cases also check undo/redo. New
normal tests compare connectivity and blending with separate scalar oracles,
covering diagonal separation, narrow winding paths, branches, tile boundaries,
soft selections, alpha locks, snapshots, cancellation and full-canvas edge
shortcuts. Full verification passed **905 normal Rust tests**, **914 all-feature
Rust tests**, both Clippy configurations, formatting and **36 Python tests**;
eight explicit probes remain ignored in normal runs.

The original and optimized binaries were frozen before timing. The default-UI
baseline was repeated after the overnight pause to confirm the current
comparison; the earlier baseline is retained too. No compilation or other
benchmark ran alongside these measurements. Final executable changes after
timing only clarify benchmark report scope and constrain the diagnostic size
override to its valid 512–4096 fixture range. Raw samples, comparison summaries
and hashes are in
[fill-scanlines-audit-20261024.json](benchmarks/fill-scanlines-audit-20261024.json).
Local logs and frozen binaries are under `artifacts/fill-scanlines/`.

## Mirrored feature transforms — 2026-10-24

Added an optional **Mirror pair** workflow for existing paired features on the
active raster layer or mask. Select one feature, choose **Transform mirrored
pair…** from Selection properties, or use **Ctrl+T > Mirror > Mirror pair**.
The partner follows the reflected movement, scale, rotation, flip, perspective
or mesh deformation. Each side samples its own original artwork. The mirror
axis starts from the active symmetry guide and can be repositioned or tilted.
Blue handles control the selected side; an orange outline marks its partner.

Both sides are computed from one frozen source and published as one gesture.
Apply/Cancel and undo/redo use the existing asynchronous preview lifecycle.
The driving selection remains selected afterward. Source selections overlapping
their reflected partner are rejected with a one-side selection instruction;
overlapping destinations combine coverage symmetrically. The two features must
belong to the same active layer or active mask. This is a bilateral edit, not
radial duplication or a replacement of the partner's artwork with a clone.

Paired previews use the existing coalescing worker and bypass the single-region
GPU drag overlay. This preserves compositing and distinct source content on
both sides; no interactive-latency improvement is claimed. Selection bitmaps
use the existing full-canvas coverage representation, and mesh inverse maps
are bounded to each destination region.

Verification passed **913 standard Rust tests**, **922 all-feature Rust tests**,
both Clippy configurations, formatting and **36 Python tests**. New checks
cover independent per-side transform oracles in all three sampling modes,
reflected curved mesh triangles, horizontal/tilted axes, masks, source-region
crossings, off-canvas clipping, cancellation, a single undo step, legacy
settings and UI clicks with the sidebar visible or hidden. The native preview
fixture is available through `--capture-mirror-pair`; logs and captures are kept
under `artifacts/mirrored-transform/`.
