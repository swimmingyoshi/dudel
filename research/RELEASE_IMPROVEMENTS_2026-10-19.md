# Release improvements: preview context and open-project accounting

Base revision: `2227f4689f5cf4136179840213519ec4f8f1d48e`.
The preceding comparison and release review (kept privately)
identified transform preview fidelity, aggregate memory visibility and stale
status documents as bounded improvements before release acceptance.

## Transform preview behavior

The separate GPU mesh is now limited to a visible, top-level Normal raster layer
at full opacity, without byte-opacity quantization, clipping, masks, visible
upper layers or visible reference boards. Other contexts use the existing
coalescing worker and actual layer-stack compositor during dragging. Masked
documents already used that fallback. Simple top-layer transforms retain their
instant mesh preview; its existing 1024-pixel side cap is unchanged.

The change prevents blend/clip/opacity/occlusion mismatches without introducing
another compositor. In complex contexts the handle can lead the most recently
completed preview while the worker renders; this is not a promise of zero latency
or an end-to-end speed improvement. Final settlement, ordered commands and undo
continue through the existing engine.

A new regression exercises Multiply, clipping, partial opacity, upper-layer
occlusion, a translucent containing folder, imported byte opacity and references.
It begins an actual handle drag, checks that no raw overlay is uploaded, compares
the preview composite and pixels with an independently applied affine operation,
then exercises Cancel and Apply/one-step Undo. Existing coverage retains the
simple mesh path, masks, stale workers and coalescing behavior. Compact undo
restores pixel/metadata equality without requiring the original Arc identities.

Native capture review also found transform help painting across the Color panel.
Transform properties now use a bounded vertical scroll area above Color, with
their own scroll identity. A workflow regression checks normal/short windows
before and after scrolling and verifies Escape cancellation. The toolbar keeps
Apply/Cancel available while properties scroll. This closes an observed layout
defect rather than changing transform behavior.

## Memory visibility

The Performance window now shows tracked CPU allocations for **all open projects**
and separate canvas/reference/thumbnail RGBA payload estimates. Active and parked
sessions share one accounting definition and one identity registry, so shared
pixel buffers are counted once across documents, history, references and frozen
display sources. Reference scene descriptors participate without retaining their
image payloads. Texture estimates deduplicate shared auxiliary handles. The
window scrolls, with aggregate figures at the top.

These are diagnostic allocation estimates, not total process memory, committed
VRAM or a new enforced application memory budget. Worker output/scratch, some
bookkeeping, allocator/driver overhead, auxiliary mipmaps and preview-window
textures remain outside their stated scope. Diagnostics themselves add work
while the window is open; use the normal closed-window path for frame acceptance.

The tab lifecycle already discards older parked canvas caches, retaining only
the most recently left tab's canvas cache. That policy was preserved. New
regressions check shared versus independently allocated reference buffers,
switching, tab closure/release and frozen reference descriptors. No additional
eviction policy or larger project limits were introduced without measurements.

The isolated native fixture is:

```
dudel.exe --capture-project-memory NEW_IMAGE.png WIDTH HEIGHT SCALE
```

It creates three disposable tabs, with a reference in a parked project, and opens
Performance. It does not load user projects/preferences or launch the long study.

## Current status and the idle study

[RELEASE_STATUS.md](../RELEASE_STATUS.md) is the concise current index, linked
from the README, delivery plan and release checklist. It separates working
source, the workflow handoff and the older frozen DX12 study binary. The old
comparison is labelled historical; contradictory pending-CI language in the
delivery plan is corrected from the existing build-specific proof.

The packaged study instructions now explain that watching a movie or doing
other foreground work makes a run exploratory. Release performance evidence
requires the idle workload. The synthetic two-hour study and physical two-hour
Artist 12 session remain separate checks. Neither frozen candidate is modified.

## Validation

Final local `scripts/Verify.ps1 -Mode Full` passes: **885 default tests, 894
all-feature tests, zero failures, three intentionally ignored in each suite**;
34 Python checks; formatting; both warnings-denied all-target Clippy configurations;
and optimized compilation. Log: `target/qa/csp-improvements-final-20261019.log`.
The separate maximum-memory fixtures were not repeated for these changes.

Engineering executable SHA-256:
`FEBE26870D90DC5470EA0A73689CE77B85FBF17745ACBAAE6380A081C2A5B30D`.
Four native DX12 captures exit successfully and were visually reviewed under
`target/qa/release-improvements-20261019`: `projects-100.png`, `projects-200.png`,
`affine-100.png`, `affine-200.png`. They cover 1280x800 at application scale 1
and 960x640 at scale 2; they are not actual OS DPI/tablet acceptance. The normal
transform capture now separates settings/help from Color; the compact layout
retains toolbar Apply/Cancel. The pre-fix `affine-before-layout-100.png` preserves
the observed overlap. Memory captures show three open projects, about 4.01 MiB
tracked CPU versus about 1.51 MiB for the active project's live/additional
allocations, and about 4.69/0.01 MiB canvas/auxiliary RGBA. These small-fixture
estimates are not an RSS, peak-memory or performance acceptance measurement.

The older frozen study executable still hashes to `8D2C768E...9279C`. No long
study, hosted CI, independent source-archive rebuild, clean-machine or physical
artist session was performed for the new changes. Packaging outcomes are
recorded separately after archive checks. No release gate is accepted by these
implementation changes alone.

Packaged handoff: `dudel-0.1.0-candidate-20261019-233855-419`, built from the
recorded base plus identified working changes. [Archive verification](../benchmarks/release-improvements-20261019.json)
passes both checksums/CRCs, all 160 compiled source inputs, 881 matching notice
files, and status/checklist inclusion in both archives. Collection covers 458
dependencies with no missing notice-collection entries; this does not independently
certify whole-program licensing. The packaged EXE also starts on DX12 and its
Performance capture was visually reviewed. Study CheckOnly passes and starts
no workload. The packaged artifacts remain unchanged after this evidence update.
