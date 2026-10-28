# Dudel professional drawing application delivery plan

Established 2026-10-15. This is the current execution plan. The internal
product plan retains the product requirements and historical checkpoints.

## Outcome and scope

Deliver a dependable Windows illustration application with a quiet, canvas-focused
workspace and complete daily drawing workflows on the XP-Pen 13. Prioritize
artwork safety, predictable tools and comfortable drawing alongside speed.
Build on the Rust application and existing optimizations. Compare against established
apps for specific unresolved behavior; keep observations separate from design choices.

The first professional release covers sketching, inking, painting, organizing
layers, selecting/transforming artwork, making brushes, saving/recovering projects
and exporting images. Animation, cloud collaboration and exhaustive CSP feature
parity are outside this release. Color-managed print claims require actual profile
support and verification. Experimental formats remain opt-in until validated.

## Delivery order

| Priority | Work package | Completion evidence |
| --- | --- | --- |
| 1 | Workspace and discoverability: canvas space, brush switching, contextual options, consistent tool states, readable feedback and unclipped panels | Rendered workspace at compact/wide sizes and 100/150/200% scale; walk through New → brush/color → draw → undo → layer → save → export |
| 2 | Everyday workflow gaps: audit commands and undo, layer organization, selections, fill, transform apply/cancel, navigation and shortcut/text focus | Complete sketch → line art → color → shading workflow; regression coverage for state transitions, locks and selections; visual checks for tool feedback |
| 3 | Artwork safety: regular save/open/save-as, recovery, replacement prompts, stale workers and cancellation; indexed format kept separate | Inject failures and interruption, reopen prior/current files, preserve unsaved edits, demonstrate recovery and compatible reopening |
| 4 | Drawing quality and measured responsiveness: physical input capture/replay, brush performance, large edits and long sessions | Real XP-Pen checks and repeatable replay; frame/owner/worker measurements reported separately; no unmeasured FPS claims |
| 5 | Essential missing capabilities: audit canvas resize/crop, layer masks, common adjustments and file exchange, then implement in workflow order | Each capability has a complete visible entry point, useful defaults, apply/cancel, undo, persistence and error handling where relevant |
| 6 | Brush creation and library polish: preview, duplicate/edit/save/reset, pack portability and restart behavior | Create ink/pencil/textured brushes; restart and import into clean preferences with equivalent rendering |
| 7 | Interop and release hardening: controlled CSP folder/clipping fixtures, supported imports, packaging, shortcuts, scaling and compatibility documentation | Known supported files reproduce expected pixels; unsupported content is explained; install/start/recover/export checks and sustained user drawing session pass |

Work on measured bottlenecks when they block these workflows. Avoid raising
canvas/layer limits until supported dense documents meet memory, responsiveness
and persistence gates. A feature is complete only when its UI, engine, history,
persistence and cancellation behavior are coherent, as applicable.

## Verification and progress rules

Track evidence separately as implemented, automated-tested, visually verified,
hardware-verified and release-ready. Existing tests do not imply GUI/hardware
acceptance. Do not mark the overall goal complete until the release gates pass.

For each work package, record the problem, resulting behavior, relevant checks,
remaining limitations and build/artifact identity. Preserve established painting
and storage behavior through targeted regressions and the existing suite. Use
isolated capture modes and generated documents for visual and failure studies.
Never turn historical test totals or single-run benchmarks into current claims.

## Current work

Latest checkpoint (2026-10-17): the retained Rust growth in repeated DX12 use was
wgpu-core's deferred texture-view/bind-group destruction list, which egui-wgpu
never drains. A nonblocking per-frame `Device::poll` now releases it. Over the
twelve-round DX12 session, matching-idle Rust drift fell from +30 MiB to zero and
process-private drift from +77 to +45 MiB. The non-Rust remainder is the next
memory study. Details: `research/WGPU_DEFERRED_DESTROY.md`.

Previous checkpoint (2026-10-17): View > Automatic panel layout now restores
responsive sidebar choices and shows panels after manual overrides/Tab hiding.
Real menu-interaction tests verify visibility, preference roundtrip and preserved
artwork/history from 480x320 through 2560x1000; native menu captures were inspected
at 100/200% and a 480x320 workspace. Added a first-use `QUICK_START.md` and corrected
stale README menu/Undo wording. Full suites: 760 default/763 profiling passed,
zero failed, one existing high-memory test ignored each; default/all-feature
Clippy, formatting, whitespace and optimized build pass. Details:
`research/AUTOMATIC_PANEL_LAYOUT.md`.

The repeated DX12/RTX 3060 resource finding now reproduces with Rust and optional
GPU suballocator reports. Matching reopened-idle Rust requests grow about 30 MiB
over rounds 4-12, while all 36 matching initial/before-save/final GPU idle samples
hold 35.25 MiB suballocated in a 320 MiB reserve (148 allocations/two blocks).
The growing process commitment is still not fully assigned. Both twelve-round
profile runs passed document correctness; this is not a memory-gate pass or an
identified leak owner. Details: `research/DX12_ALLOCATION_OWNERSHIP.md`, with prior
normal-build evidence in `research/SUSTAINED_NATIVE_DX12.md`. The new GPU probe is
feature-only, fixed-size and distinguishes unavailable from measured zero.
Current full suites remain 760 normal/763 profiling passed; default/all-feature
Clippy, formatting, whitespace, optimized builds and native availability/disabled
paths pass. Next identify retained Rust allocation call stacks/owners before
choosing a fix. Production probes remain disabled by default.
Physical XP-Pen, sustained native memory/GPU interaction, clipboard/interop and
packaging gates remain open.

Baseline source: `0ae282aa543154c03233b9c140270e98ce3d8522`.
Baseline verification: 583 tests passed, one ignored; Clippy and formatting passed.
The initial source/research review and REA configuration are complete.

First implementation: compact drawing workspace and quick brush access. The
automatic Brush/Eraser workspace reserves at least 65% width for the canvas at
1280 logical pixels by keeping advanced brush properties closed. Wide layouts
show both sidebars. Explicit panel choices remain authoritative. Other tools
retain contextual properties. The preset button opens the existing searchable
library, including favourites and recent brushes, with a separate settings action.
At narrower logical widths, stabilization, snapping and symmetry move into a
Stroke options menu. The tool list scrolls independently above undo/redo so all
tools and color chips remain reachable on short, highly scaled workspaces.

Automated verification for this first pass: 584 tests passed, one existing test
ignored; warning-denied all-target Clippy, formatting and whitespace checks pass.
The layout regression checks actual canvas space at 1280/1600/1920/2560 logical
widths, contextual tool properties and explicit panel preferences. A debug build
is used for isolated capture review; this is not a release performance result.
The 100/150/200% capture variants were inspected. The compact brush toolbar fits
after grouping secondary controls; the short tool rail now scrolls while keeping
undo/redo visible. At 200% the automatic inspector is hidden and can be reopened
from View. Navigator/color/layer accessibility, the brush popup interaction and
other tools at high scale still need the broader workflow pass. Captures use the
current display's window bounds/DPI; requested window sizes can be OS-clamped.

## Ordered PNG export checkpoint — 2026-10-15

The workflow audit found that PNG export composited the immediate document on the
UI thread without settling queued strokes. Recent marks or their final taper could
be omitted, and direct destination writes could damage an existing image on error.

PNG export now closes logical gestures and requests an ordered render snapshot,
then composites and encodes that immutable document on a background worker.
Later drawing is excluded from that export and can continue. Only one export is
active at a time, independently of project saves. Progress and Cancel export are
shown; a canceled queued request retains/consumes its ticket without writing.
Cancellation is checked during compositing and after encoding before publication;
PNG encoding itself is not interruptible. A completed publication reports success
even if cancellation arrives afterward.

Writes use same-directory staging, file flush and atomic replacement. Non-PNG
destinations (including renamed project data) are rejected. Export does not change
the project path, saved revision, dirty state or history; finishing an active
gesture still records the corresponding artwork normally. Canvas replacement can
proceed while the captured export runs. Application close waits for the worker,
with its Cancel export control still available. Snapshot retention participates
in the shared allocation registry, with a conservative full RGBA output reserve;
PNG codec and allocator overhead remain separate from this estimate.

Five targeted tests pass: real queued stroke/taper inclusion and later-edit
exclusion in opaque/transparent output; unchanged project save state and captured
source across canvas replacement; early/late cancellation preserving an existing
PNG with staging cleanup; destination/write failure protection; and queued cancel
plus close waiting for a controlled live worker. These are automated behavior
checks, not a physical filesystem power-cut test or a full GUI walkthrough.

The next accessibility gap was color selection: compact layouts hide the inspector,
while the rail's overlapping color chips only swap main/secondary colors. Painting
tool options now include a Color menu, showing the current color and opening the
existing wheel/hex/recent/swatches UI in a scrollable popup. It uses the same
drawing color and palette as the inspector. This makes color selection accessible
without opening a large sidebar or changing the established X/chip swap action.

Checkpoint verification: **589 tests passed, zero failed, one existing ignored**;
warning-denied all-target Clippy, formatting and whitespace checks pass. Debug and
optimized release builds pass. The Color control and compact brush toolbar were
visually inspected in fresh 100% and 200% capture variants. Color-popup interaction
and the complete layered project workflow still require their own checks.
Current runnable candidate: `target/release/dudel.exe`.

## Popup interaction and layered workflow checkpoint — 2026-10-15

Actual egui pointer/keyboard tests exposed two problems that screenshots could not:
the default menu close policy dismissed color/brush/stroke controls on an inside
click, and canvas shortcuts consumed Escape before an open popup could close.
Control popups now remain open while editing, filtering, starring brushes or
adjusting sliders, and dismiss on outside click or Escape. Choosing a brush
explicitly dismisses its picker, including reselecting the current preset to reset
its edited settings. Open popups own keyboard input; canvas shortcuts, native paste
shortcut consumption and temporary tool keys wait until the popup is dismissed.

`src/workflow_tests.rs` exercises the actual compact workspace with both sidebars
hidden: open the brush picker, choose G-pen, search for Pencil, choose/reselect it,
open Color, focus hex, Ctrl+A and enter F34, then close with Escape. It checks exact
color, preset state, no unintended artwork selection/history and no tool switch
behind the popup. This is egui interaction verification, not an OS/native pen test.

A separate complete generated workflow uses application commands and real workers:
new canvas; queued sketch stroke; create/paint another layer before settling;
rename; polygon mask; fill; move; exact undo/redo; opacity; group; guide edits;
simultaneous native save and transparent PNG export; new canvas; asynchronous
reopen; opaque export. Native document fields and RGBA/mask values, guide state and
both PNG composites match exactly. Save path, saved/dirty state and fresh reopened
history are checked. Native file dialogs are replaced by test-chosen temporary
paths; this does not certify the full GUI walkthrough or hardware drawing feel.

Isolated visual QA now also supports `--capture-brush-picker` and
`--capture-color-picker`, with the same output/width/height/scale arguments as
`--capture-ui`. These show the real popup UI without loading user artwork,
preferences or installed extensions.

The popup captures found additional scaling defects: the brush picker's footer
extended beyond a short viewport and menu-styled swatch buttons collapsed to thin
strips. Popup height now follows viewport height, space is reserved for the brush
settings action, and swatches have explicit 32-point square targets. A third
interaction test opens the picker at 768 x 420 logical points and activates its
visible settings action. Screenshots cover native rendering separately from these
headless egui checks.

Checkpoint verification: **592 tests passed, zero failed, one existing ignored**;
warning-denied all-target Clippy, formatting and whitespace checks pass. Fresh
brush/color popup captures at 100% and 200% were inspected after the fixes; the
settings action is visible and swatches retain square targets. Debug and optimized
release builds pass; the current candidate remains `target/release/dudel.exe`.

The source audit at the popup checkpoint found no canvas resize/crop actions, persistent layer
masks or built-in adjustment UI. Selection coverage is already implemented and is
distinct from layer masks. These remain required capability work under priority 5.

## Canvas resize/crop checkpoint — 2026-10-15

Canvas > Canvas size opens a modal with pixel dimensions, nine placement anchors,
an outline preview and Apply/Cancel. It changes bounds without resampling pixels.
Added space is transparent; cropped pixels are recoverable through Undo. Canvas >
Crop to selection bounds uses the selection's rectangular bounding box, retaining
selection coverage rather than cutting the artwork to the mask's outline.
Supported canvas limits remain unchanged.

Every raster layer is remapped, including hidden/locked layers; folders, layer
identity/properties and the active entry are retained. Packed sparse row visits
avoid flattening whole layers, retain alpha-zero RGB and skip zero pixels. Masks
retain exact coverage. A selection removed entirely by resizing remains an empty
mask, with a status hint and working Select > Deselect / Ctrl+D, so the resize
does not silently enable painting over the whole canvas.

Resize/crop and quarter-turn rotation run as ordered background commands after
earlier artwork. A progress modal prevents new input from using stale dimensions,
and Cancel targets the specific operation. Publication/history are atomic; queued
save/export snapshots see the new bounds after successful publication. Resize
and crop check cancellation during row work. Rotation currently finishes its
existing raster scan before observing late cancellation; its UI stays separate
from that worker, and canceled unpublished output is rejected.

Session-local shared coordinate frames let current guides follow crop/resize/
rotation through undo/redo without restoring older independent guide edits.
Explicit symmetry centers, ruler points and guide lines are mapped; implicit
symmetry centers continue following the canvas center. Off-canvas guide positions
survive save/reopen within the supported coordinate domain. Frames are not stored
in project files: saved guides already use the current coordinates, so no format
version change is needed. Their allocations are deduplicated as metadata in the
existing memory registry. History cost planning now includes the patch header,
preserving the broad-fill gate that avoids allocating large redundant undo patches.

New tests cover nine anchors with grow/shrink/mixed dimensions, sparse and flat
source pixels across tile edges, hidden RGB, folders/locks/properties, exact mask
coverage, no-op/limits/invalid source/cancel, empty selections, metadata sharing,
guide edits across resize/crop/rotation/undo/redo/reopening, and queued painting
through resize and project/PNG output. Pointer/keyboard tests cover dialog Cancel,
dimension entry/Apply/Undo and deselecting a cropped-out selection. Isolated
`--capture-canvas-size` uses the existing capture arguments for visual QA.

The dialog scrolls its controls above fixed Apply/Cancel actions on short displays;
an interaction test checks Cancel at 480 x 320 logical points. Anchor icons use
native vector painting and accessible names. Fresh native captures at 100% and
200% were inspected; capture window dimensions can be OS-clamped, so they do not
replace the explicit logical-size layout test. **602 tests passed, zero failed,
one existing ignored**; warnings-denied all-target Clippy and formatting pass.
Debug and optimized release builds pass. Current candidate: `target/release/dudel.exe`.

Next: persistent layer masks and common adjustments, followed by image resampling
and export dimension controls (distinct from changing canvas bounds). Remaining release work still
includes full GUI/tablet workflow validation, supported dense-document memory and
responsiveness measurements, filesystem/device durability, full CSP folder/clipping
parity and packaging/accessibility. The overall professional-readiness goal remains
active.


## Native raster adjustments checkpoint - 2026-10-15

Filter now offers Brightness / contrast, Hue / saturation and Levels. Each opens
one consistent dialog with live canvas preview, a before/after Preview checkbox,
Reset, Apply and Cancel. Controls scroll above the pinned actions on short screens.
Adjustments modify the selected visible, unlocked raster layer through the current
selection; alpha and hidden RGB in transparent pixels remain intact. Levels has
input black/white points, midtone gamma and output black/white points. These are
baked raster edits, not persistent adjustment layers or color-managed print tools.

Preview workers always sample the frozen original, coalesce rapid setting changes
and reject stale results. Cancel restores the source without creating history or
changing saved state. Apply publishes the verified result with one undo step and
sliced history settlement, without computing the pixels again. No-op Apply retains
redo. Pending artwork finishes before preview acquisition. Save/export are blocked
while the unconfirmed preview is open; recovery excludes it. Replacement/close
cancels preview first. Canceled workers remain tracked until joined, and closing
waits for that cleanup. Source/result snapshot allocations join the deduplicated
memory registry; worker output and scratch remain outside its documented count.

Automated verification: 611 passed, zero failed, one existing high-memory test
ignored. This includes adjustment math and invalid settings, sparse tile-edge
pixels, partial selection coverage, alpha/hidden RGB, locks/cancellation, rapid
settings changes, stale source preservation, recovery exclusion, queued preceding
edits, no-op redo, apply/undo/redo, exact project reopening and PNG pixels. Actual
egui menu/click/keyboard tests cover Cancel/Apply and a 480 x 320 logical viewport.
Warnings-denied all-target Clippy, formatting and whitespace checks pass.

The optimized release build passes. Native Hue / saturation captures at 100% and
200% were inspected: labels, controls and fixed actions fit, and adjusted artwork
is consistent across the canvas, navigator and layer thumbnail. Captures are
`target/qa/adjustment-100.png` and `target/qa/adjustment-200.png`. Windows clamps
capture window sizes, so exact minimum-layout evidence comes from the logical
viewport interaction tests. Candidate: `target/release/dudel.exe`, SHA-256
`61F801D5EDE63EF837184CF6F97221683556529B2B3BDFEAFF200F6FFA43E7CE`.
The existing running release process was preserved; its locked previous executable
was renamed to `target/release/dudel-running-38472.exe` so the new build could be
written without closing that application session.

Persistent layer masks, image resampling and export dimensions remain required
next capabilities. Existing hardware, durability,
CSP interoperability, accessibility and packaging release gates remain open. The
overall professional-readiness goal stays active.

## Layer-mask engine checkpoint - 2026-10-15

Implemented the coverage engine in `src/layer_mask.rs` and audited the consumers
needed before enabling mask creation. The integration contract is in
`research/LAYER_MASK_INTEGRATION.md`. This stage does **not** add masks to the
document model, project/recovery formats, compositing or the Layers panel; those
remain required next work, followed by mask painting and geometry integration.
It does not establish CSP mask behavior or complete the layer-mask capability.

The engine uses single-channel 64-pixel sparse COW tiles, compact edge tiles and
constant default coverage. White and black masks allocate no pixel tiles. It
provides grayscale coverage edits, selection-coverage construction, inversion,
cancelable geometry mapping, alpha baking that retains hidden RGB, weak region
stamps and allocation descriptors usable by the memory registry. The bounded
canonical codec is a subrecord for future versioned storage, not a change to
existing native/indexed project files. Decode requires expected geometry and a
payload budget and rejects malformed/unordered/redundant/truncated records.

Eight new tests cover constants and canvas limits, no-op sharing, grayscale/alpha
rounding, clipped edge tiles, unchanged shared tiles, weak-stamp invalidation and
payload lifetime, allocation deduplication, inversion, crop/growth/rotation/flips,
pre-cancellation and cancellation during remapping, and malformed/budgeted codec
inputs. **619 passed, zero failed, one existing ignored** across the full suite.
Warnings-denied all-target Clippy, formatting, whitespace and optimized build pass.

`--benchmark-layer-masks NEW_REPORT.json` exercises the engine without launching
the application or overwriting reports. The first report,
`benchmarks/layer-mask-engine-release-1.json`, identified unnecessary per-pixel COW
work in bulk construction/remapping. Private worker results now build tiles once;
index lookup also avoids redundant tile-coordinate divisions. The follow-up
`benchmarks/layer-mask-engine-release-2.json` verifies sparse edit/codec/inversion
against every expected pixel and dense rotation against an independent byte-array
oracle. At 2048 square, observed construction changed from 60.55 to 11.01 ms and
rotation from 81.07 to 26.59 ms. These are single-run engine observations, not UI
latency or repeatability claims. Dense operations still require background workers.

The 64 x 64 sparse edit crosses four tiles. Its 2048-square case measured 0.081 ms,
8,240 tracked source bytes and 24,720 additional COW bytes. Counts include metadata,
tile directory and byte vectors, and exclude allocator/Arc overhead and scratch.
No whole-canvas coverage cache is materialized. Current optimized binary:
`target/release/dudel.exe`, SHA-256
`323594055EDEF2819B4E5E51B3F2F4316D2E892B249C6E6A118826FE54CE05DF`.

Next: add version-gated mask state and regular/recovery persistence together;
extend history/memory ownership and compositing/cache invalidation, then finish
geometry, tool routing and a coherent visible mask workflow before exposing it.
The wider professional-readiness goal and all outstanding release gates remain
active.

## Layer-mask document/storage/rendering checkpoint - 2026-10-15

Layers now carry optional coverage with enabled/linked state, guarded by document
version 2 and native container v5. Version-1 unmasked documents retain their
existing v4 encoding path. Bounded mask subrecords follow raster tiles and precede
optional selection coverage, with metadata declarations fixing their count/order.
Regular save/open, recovery and bounded JSON serialization preserve exact bytes
and flags, including masks on folders and disabled masks. Old readers reject the
new container/version. Legacy v3 and experimental indexed writes reject masked
documents; indexed rejection occurs before creating a destination or changing a
journal. Packed raster and mask bytes together must fit the existing 512 MiB
payload ceiling. Canvas/layer and input-file limits have not been raised.

History compaction and strict source guards include document version and mask
identity/state, retaining shared snapshots for mask edits rather than reducing
them to lossy RGBA patches. Live/history/worker memory descriptors register mask
allocations. Compositing scales leaf and isolated-folder contributions by enabled
coverage after existing layer opacity; clipping bases include mask coverage.
Display generations, tile and prefix signatures, and thumbnails respond to
mask-only changes. Unrelated mask tiles retain cached prefix states. Encoding
omits tiles restored to default even before reclaim, without mutating snapshots.

This is a storage/rendering stage, **not an exposed layer-mask editing feature**.
The application temporarily rejects masked documents at the load-publication
boundary, preserving the existing canvas. Mask creation remains unavailable until
geometry, destructive operations, clipboard and captured tool-target routing are
integrated; then the temporary gate must be removed. The full remaining contract
is tracked in `research/LAYER_MASK_INTEGRATION.md`.

Fifteen new tests cover mask/document versions and malformed geometry, JSON,
folder/disabled/linked flags, native/recovery round trips, legacy rejection,
truncation/trailing/mismatched records, failed-save destination preservation,
independent per-pixel alpha fixtures, clipping/folder coverage, exact undo/redo,
snapshot allocation sharing, temporary app-load preservation, indexed destination
preservation, combined payload limits, canonical default tiles, mask-only cache
invalidation/reuse and thumbnail updates. The sparse-visibility texture test now
waits on its worker with a deadline and yields instead of racing a fixed 64-poll
limit; its pixel and upload-byte requirements remain intact.

**634 tests passed, zero failed, one existing high-memory test ignored**. All-target
warnings-denied Clippy, formatting, whitespace and optimized build pass. Isolated
`--capture-layer-mask` captures at 100%/200% were inspected: feathered concealment
is rendered correctly, with matching canvas/navigator/thumbnail at 100%; the
compact 200% workspace fits its visible controls. Captures are
`target/qa/layer-mask-100.png` and `target/qa/layer-mask-200.png`. These synthetic
fixtures do not validate mask painting, real pen input or CSP mask equivalence.

Current optimized candidate: `target/release/dudel.exe`, SHA-256
`3B883C6B62165FB01F48527964BAEAF86A86ABEC7776D1006AD15C131F3C6AE0`.
The existing running app session was preserved; its previous locked executable is
`target/release/dudel-running-33304.exe`. Next work is linked geometry and
mask-tool targeting, then the visible end-to-end mask workflow. The complete
professional application goal and outstanding release gates remain active.

## Layer-mask geometry checkpoint - 2026-10-15

Canvas resize/crop and quarter-turn rotation now remap all layer masks, including
disabled/unlinked masks and masks on folders. Added canvas space reveals content
with white coverage. Whole-layer raster flips, integer Move, affine, perspective
and mesh transforms carry linked masks with the artwork, including disabled ones.
Unlinked masks remain in document coordinates. Explicit selection transforms edit
raster pixels behind a stationary layer mask; a dedicated mask-editing target is
still required. These semantics are defined by Dudel, not verified CSP behavior.

Mask sampling uses straight grayscale independently of raster alpha. Nearest,
bilinear and bicubic share the transform's inverse geometry; outside-source samples
are white. Equivalent coverage retains source identity, so uniform-mask no-ops
and zero-return previews do not create mask changes. Mask-only geometry changes
are undoable. Move/affine candidates carry mask state through publication, and all
three cancellation paths restore original coverage. Version-2 direct transforms
and worker candidates validate the combined payload limit before publication.
Whole-layer flips with masks enter the existing background command barrier.
Canvas rotation now stages its result privately and checks cancellation during
raster, layer-mask and selection-coverage rows.

Integer Move also now retains hidden RGB from transparent source pixels when
their destination is transparent, fixing its discrepancy with affine sampling.
Transparent pixels continue contributing no color over retained opaque artwork.

Masked affine dragging uses the existing coalescing full-quality worker. The raw
GPU raster overlay cannot represent linked/stationary masks or parent-folder
coverage and is used only for mask-free documents. No masked-drag frame-rate or
tablet-latency claim is made; owner/worker/display and physical checks remain.
The full mask editing feature stays gated pending tool targets, merge/apply/remove,
clipboard and visible workflow integration.

Thirteen new tests cover all nine resize anchors and grow/shrink/mixed dimensions,
folder/disabled/unlinked coverage, rotation round trips and cancellation, linked
versus explicit-selection transforms, mask-only undo/no-ops, integer move versus
affine coordinates, fractional grayscale sampling values, perspective with an
independent rational inverse and mesh translation, stale source rejection,
zero-return and queued cancel, payload-limit preservation, worker admission,
affine drag/cancel, and queued resize/rotate/move/save/export/undo. **647 passed,
zero failed, one existing high-memory test ignored** in the complete suite.
Warnings-denied all-target Clippy, formatting, whitespace and optimized build pass.

Native `--capture-layer-mask-transform` fixtures at 100% and 200% were inspected:
the feathered mask follows the scaled/rotated raster, with matching navigator and
thumbnail at 100%, and visible Apply/Cancel controls at both scales. The 200%
capture exposed a partially clipped selected Move icon. The tool rail now reveals
its selection on tool or available-height changes while preserving manual scrolling
between those changes. All seven UI workflow tests pass after that UI change,
and a final native 200% capture confirms the selected icon is fully visible.
Captures: `target/qa/layer-mask-transform-100.png`,
`target/qa/layer-mask-transform-200.png` and
`target/qa/layer-mask-transform-200-final.png`.

Current candidate: `target/release/dudel.exe`, SHA-256
`502787CBF0266517B2043996890AB175DD9B710573E9BF35AF7D7D403E359787`.
The running app session was preserved. Next: captured mask-painting/tool targets
and destructive/clipboard mask semantics, then enable and verify the visible
mask workflow. The complete professional application goal remains active.

## Layer-mask commands and consumer checkpoint - 2026-10-15

Added process-local mask identities and typed ordered commands for reveal-all,
hide-all and selection-derived mask creation, inversion, enabled/linked state,
Apply and Remove. Commands address the captured layer and expected mask slot,
reject removed/replaced targets, respect locks, and validate/cancel privately before
publication. IDs survive edits and history, are fresh on duplicate/reopen, and are
excluded from persisted mask data. The existing native v5 representation remains
unchanged. No-op metadata commands retain redo and document revision.

Apply uses 8-bit alpha baking `(alpha * coverage + 127) / 255`, retaining RGB,
ignoring the current selection and preserving layer style. Enabled-mask Apply
requires unlocked transparency. Disabled-mask Apply removes its metadata without
rewriting raster pixels. Lazy row edits avoid acquiring a pixel writer for an
all-white mask. Applying a folder mask requires Merge folder first.

Fixed merge-down's double-mask path: both enabled leaf masks contribute to the
merged raster once, then the lower mask is removed. The existing lower-layer
opacity/blend/clipping contract remains in place. Merge folder bakes child masks
and keeps its own mask on the resulting raster. Ungroup rejects a masked folder
instead of silently dropping that mask, with a remove-or-merge explanation.
Masked merges use the background edit barrier.

Active-layer clipboard capture now bakes enabled mask coverage before selection
coverage, zeroing excluded holes. Disabled masks do not affect copied pixels;
layer opacity/blend and parent-folder effects remain outside raw layer copy.
Paste makes a raster with baked alpha and no mask. Cut still edits raw selected
pixels and retains the source mask; clipboard write failure prevents publication.
Pixel-patch Undo is checked for exact pixel values and stable mask identity rather
than incorrectly requiring the original raster allocation identity.

Nine new tests cover selection coverage and constant seeds, stable/fresh/stale
mask identities, apply/disabled/locks/hidden RGB/selection independence, one-step
command undo/redo and no-op redo, cancellation, folder apply/ungroup handling,
leaf/folder mask compositing across merges, clipboard coverage and disabled masks,
cut write failure/success, paste and undo. **656 passed, zero failed, one existing
high-memory test ignored** in the full suite. Warnings-denied all-target Clippy,
formatting, whitespace and optimized release build pass. These are automated core
checks; no new mask UI or physical clipboard/pen workflow is claimed.

Current candidate: `target/release/dudel.exe`, SHA-256
`9407AE6788C8A991A145FFDC5E20DCCC4CA87E9CC4CC48369C7D945C4ECBF927`.
The running application was preserved. The editing UI and loader gate remain
until stroke settings, live dabs/opacity/taper/rollback, other mask tool targets
and live stroke payload limits are integrated. Next work is that mask painting
path, followed by the visible end-to-end workflow. The full professional drawing
application goal and its remaining release gates stay active.

## Captured mask stroke engine checkpoint - 2026-10-15

Stroke settings capture raster or a stable mask slot. A shared painting-target
adapter routes live dabs, pressure/grain/symmetry, stroke-opacity accumulation,
taper replay and sliced rollback to grayscale coverage without touching raster
RGB/alpha. Black conceals, white and eraser reveal. Layer/parent visibility and
layer locks apply; raster alpha lock does not prevent mask painting. Disabled and
folder masks are supported. Layer and mask identities guard deferred cursors and
rollback against replacement. Rollback restores the original shared coverage;
tile reclaim visits only affected regions. Existing raster settings omit the new
default target from serialization, retaining their previous byte representation.

Version-2 raster and mask contacts validate combined payload limits at settlement.
An over-budget stroke rolls back before a later queued save/export snapshot and
does not acquire an undo entry. Mask-bearing stroke recording is explicitly
rejected pending recorder format support. The loader/UI gate remains in place.

Eight new tests cover opacity/black/white/eraser/alpha lock, an independent opaque
grayscale raster reference for pressure/grain/symmetry/selection/taper, incremental
rollback, queued mask creation/stroke/save/reopen/export/undo, stale targets and
locks/visibility, disabled folder masks, raster and mask payload-limit rollback,
and replacement-layer cursor guards. Full suite: **664 passed, zero failed, one
existing high-memory test ignored**. Warnings-denied all-target Clippy, formatting,
whitespace checks and optimized release build pass. Durable command logs are in
`target/qa/mask-stroke-{tests,clippy,release}.log`.

Native `--capture-layer-mask-stroke` rendering fixtures at 100% and 200% were
inspected, including the composited canvas and navigator at 100%. Captures wait
for queued strokes to settle. Files: `target/qa/layer-mask-stroke-100.png` and
`target/qa/layer-mask-stroke-200.png`. These are synthetic rendering checks, not
interactive mask-focus or physical tablet validation.

Current candidate: `target/release/dudel.exe`, SHA-256
`2E7FAD56256CB088B4725F17210D2F9822A44D351C5E7C400B50A0CE2C1E95EB`.
The running user application was preserved. Next: coherent saved-state handling
for canceled/no-op contacts across queued edits and guides, visible mask focus and
pending-target admission, then remaining fill/clear/clipboard/transform targeting.
Do not expose mask editing or remove the loader gate before these workflows are
coherent. The professional application goal remains active.

## Queued stroke saved-state checkpoint - 2026-10-15

Live damage previously advanced the same revision used to decide whether artwork
was saved. A canceled/no-op contact restored coverage/pixels and history, but
could leave a clean painting marked unsaved. Capturing a boolean at input admission
would also lose earlier queued edits that had not rendered yet.

The queue now publishes separate start/completion boundaries. The application
captures modified state at execution, after preceding commands have reached it,
and settles that state before the next command. Redraw/source revisions continue
to advance monotonically during preview, protecting stale-worker guards and live
thumbnails. A separate committed-content token advances on confirmed strokes and
other persistence changes. Cancel/no-op settlement retains earlier queued edits,
independent guides, later edits and redo, without claiming an unconfirmed preview
as committed artwork. The recording diagnostic's synchronous path retains its
existing revision behavior; this checkpoint covers the normal queued input path.

Project saves capture the committed token alongside existing guide/source state.
A successful save can recognize the restored painting after a canceled contact
even when newer display revisions exist. Failed saves and saves preceding real
artwork/guide changes leave the canvas modified. Recovery cleanup runs when a
confirmed save matches that restored state, retiring any captured recovery worker
without canceling unrelated selection/filter input. Canvas replacement retires
the saved-content marker. Export continues to leave project save state unchanged.

The render pump processes the new gesture boundaries within its existing shared
two-millisecond/4096-unit budget. It still stops at save/export snapshots and
clipboard fences. Regression testing caught and corrected a pump that continued
into following artwork in the same UI update after starting a save snapshot.

Five added application tests cover raster/mask rollback with earlier queued edits
and independent guides, no-op redo and clean state, following queued metadata,
successful/failed save confirmation during live painting with recovery retention,
and mask snapshot confirmation after no-op/cancel versus unsaved guide changes.
Full suite: **669 passed, zero failed, one existing high-memory test ignored**.
These are automated application/controller checks; no new interactive mask UI or
hardware validation is claimed. Logs: `target/qa/stroke-state-tests.log`,
`target/qa/stroke-state-save-fences.log`, `target/qa/stroke-state-clippy.log` and
`target/qa/stroke-state-release.log`.

Warnings-denied all-target Clippy, formatting/whitespace checks and optimized build
pass. The native queued mask-stroke fixture was rendered and inspected again at
100% (`target/qa/layer-mask-stroke-state-100.png`) after the queue changes; its
canvas, navigator and layer thumbnail remain consistent. This checks settled
rendering, not pointer-driven target selection. Current candidate:
`target/release/dudel.exe`, SHA-256
`98B409AF7D25C006C8CF030E6798FC8558E3FDD508D191D82F7351F7481F038E`.
The existing user application session was preserved.

Next: visible mask focus and pending-target admission, then remaining
fill/clear/clipboard/transform target behavior and the complete visible workflow.
Mask creation/edit controls and masked-document loading remain gated. Physical
XP-Pen validation and the broader professional release gates remain outstanding;
the overall goal remains active.

## Mask focus and pending-target checkpoint - 2026-10-16

Pending layer metadata now includes mask identity, enabled and linked state.
Creation/invert/toggle/apply/remove projection matches typed command execution.
Merge-down clears the baked lower mask in the projection; folder merge retains
its root mask; masked-folder ungroup is rejected consistently. Typed duplication
reserves copied mask slots from the new copy identities, so a later contact can
capture the right mask before the copied layer exists in the document. Raster
pixels and mask coverage remain independently shared until written.

Application painting focus now captures a layer and mask slot independently of
layer selection. Focus changes close existing logical input and clear Shift
continuation; previously queued dabs keep their original settings. Creation can
follow a reserved new layer and immediately admit mask painting. Focused duplicate
followed by another mask stroke targets only the copy. Failed creation, stale
slots, layer removal and replacement cannot redirect mask dabs to raster artwork.
Undo of same-slot painting retains focus; removal/replacement reconciles focus
when settled. Focus is session UI state and does not change persisted artwork.

Diagnostic mask captures now expose separate artwork/mask thumbnails, selected
target borders, disabled-mask crosses, target buttons and a Mask actions menu
(create/reveal/hide/selection, enabled/linked, invert, apply, remove). A toolbar
target chooser remains available when the inspector is hidden. Canvas packets
capture this target, including mask brush/eraser input with raster alpha lock.
Row actions carry the displayed mask ID and validate projected state. Layer and
mask widget IDs remain stable across frames; a replacement between pointer press
and release cannot inherit the old mask's click.

Fill/lasso/clear/move/affine/filter/adjustment/clipboard actions explicitly require
the artwork target until their dedicated mask paths are integrated. Whole-canvas
geometry retains coordinated mask remapping. Recording rechecks document version
after pending mask commands are flushed, rejecting unsupported mask recording.
These guards are temporary integration work, not a substitute for the required
mask tool behavior. `mask_controller::EDITING_ENABLED` remains false: regular
creation controls and masked-document loading are still gated. Only isolated
capture mode exposes these controls while integration continues.

Seven added controller tests cover reserved layer/create/stroke/duplicate/stroke,
failed creation, switching targets with queued dabs, unsupported action safety,
projection parity across mask commands/merges/folders, recording after pending
creation, and stale thumbnail/slot admission. An additional egui workflow uses
real hit testing for both thumbnails, replaces a mask between press/release,
paints via canvas input packets with alpha lock, invokes Undo, toggles enabled
state and switches targets through the toolbar on a short inspector-free layout.
The Windows input harness now supplies egui's command flag with Ctrl, matching
eframe's platform events; this corrected an ineffective synthetic Undo chord.

An earlier parallel full-suite/release run hit the existing five-second undo
compression deadline. The full suite subsequently passed without competing
release compilation. After the widget identity and layer visibility refinements, **678 tests passed,
zero failed, one existing high-memory test ignored**. Warnings-denied all-target
Clippy and formatting/whitespace checks pass. Diagnostic screenshots now also wait for thumbnail revisions to
settle, in addition to queued painting and display work.

Native focus captures exposed a partially clipped selected mask row at 100% after
adding target controls. The mask layer area now reserves at least 240 logical
pixels when space permits, and its scroll area reveals the selected row on target
or available-size changes. Manual scrolling remains authoritative between these
changes. A second egui workflow checks full thumbnail visibility on a short
viewport, preserves a manual scroll away, and reveals the target again after a
layer-area size change. Canvas replacement resets the reveal state.

Final optimized build passes. Native captures at 100%, 150% and 200% were inspected:
the selected mask row is fully visible where the inspector is shown, and the
toolbar target chooser fits at every captured scale. Files:
`target/qa/mask-focus-{100,150,200}-final.png`. At 150%, the navigator image is
partly clipped inside its scrollable area; fitting that map and its controls into
the compact inspector remains part of the broader layout pass. These synthetic
captures and egui tests do not verify a physical tablet or the complete mask
workflow. Current candidate: `target/release/dudel.exe`, SHA-256
`3241E2630F26BBC78F4C8E3CE0AB9212592C01B3D34B81375C3038E952C9CFBF`.
Existing user artwork/session state was preserved. Verification logs:
`target/qa/mask-focus-{tests,targeted,scroll,clippy,release}.log`.

Next: dedicated mask fill/clear, clipboard and transform targeting, clear
color/coverage feedback, and complete visible save/reopen/export workflow checks.
Then remove the creation/loading gate and perform supported-scale and physical
XP-Pen validation. The professional application goal remains active.

## Mask clear and bucket fill checkpoint - 2026-10-16

Mask Clear and Fill now address the captured layer/mask identities through ordered
background commands. Clear paints toward white with selection coverage; whole-mask
clear ignores selection. Both operations edit disabled and folder masks and
ignore raster alpha lock, while enforcing layer locks and ancestor visibility.
Raster RGB/alpha, mask identity, enabled/linked flags and selection remain intact.
Typed request validation, final payload validation and source/cancellation guards
prevent stale, malformed, canceled and oversized results from being published.

Active fill reads the frozen mask as opaque grayscale. Visible fill uses the
checked pre-edit artwork composite; explicit layer references retain the existing
raw-raster semantics and never fall back when missing. Tolerance uses the existing
premultiplied color/alpha comparison; gap closing and signed expansion retain the
established region algorithm. Fill opacity multiplies selection coverage before
interpolating the coverage byte toward the drawing color's grayscale value.

The normal region path visits only the component and its boundary, with bounded
visitation/result bitsets. It avoids a whole-canvas eligibility scan, and mask
painting iterates only set region bits. Selected clear visits its bounds. Neither
operation expands the destination mask or active-mask reference to RGBA; Visible
reference still explicitly composites RGBA. Uniform whole-canvas fill/clear keeps
constant, tile-free coverage, including an explicit all-white selection mask.
These are implementation properties, not new responsiveness or RSS measurements.

Canvas Fill now captures mask focus, and its Reference control says Active mask.
Context clear is labeled Clear mask for mask focus, including folders. The normal
Delete action uses the same mask clear route. Fill is rejected while canvas bounds
work is pending so old canvas coordinates cannot target a later resized canvas.
Regular mask creation/loading remains gated pending the remaining tool workflows.

Seven new tests cover fractional clear and whole/no-op identity, active versus
raw references and constant coverage, an independent opaque grayscale raster
oracle for gap closing/expansion/selection/opacity and frozen visible references,
locks/visibility/stale IDs/invalid input/cancellation/folders, queued creation/fill/
save/clear and separate undo/redo, canceled worker redo preservation and private
rejection at the combined 512 MiB logical payload ceiling, and component/boundary
visitation. The egui workflow now performs mask fill through canvas packets and
Delete-to-clear through keyboard input with raster alpha lock still enabled.

Current automated suite: **685 passed, zero failed, one existing high-memory test
ignored**. Logs: `target/qa/mask-fill-{tests,targeted,clippy,release}.log`.
The native `--capture-layer-mask-fill` fixture renders an actual queued mask fill
with matching tool/color/opacity settings for visual inspection. Physical pen
input and the full save/reopen/export mask workflow remain unverified.

Warnings-denied all-target Clippy, formatting/whitespace checks and optimized
release build pass. Native `target/qa/mask-fill-100.png` and
`target/qa/mask-fill-200.png` were inspected. The 100% capture shows Active mask,
matching color/tolerance/opacity controls, the composited canvas and navigator,
and a fully visible selected coverage thumbnail; the 200% toolbar remains
unclipped with the sidebars automatically hidden. These are settled rendering
checks; the egui workflow supplies the interaction evidence. Current candidate:
`target/release/dudel.exe`, SHA-256
`DCD453EC2E862ADBA6DA9DB439B23906DDD43A14B50246EB6B62D770934BEF59`.
Existing user artwork/session state was preserved.

Next: mask lasso, clipboard and transform targets, clear coverage/color feedback,
and the remaining visible workflow. The compact navigator layout issue and wider
professional release gates remain tracked. The full application goal stays active.

## Mask lasso checkpoint - 2026-10-16

Lasso fill and erasing now capture artwork-versus-mask together with the layer
identity when the contact starts. Admission uses projected mask identities, so a
new layer/mask or a duplicate need not finish before a lasso contact is accepted.
The worker validates the captured slot and uses the existing antialiased polygon,
selection and opacity math. Mask fill interpolates toward grayscale; erasing
reveals white. Raster RGB/alpha, selection, enabled/linked flags and alpha lock
remain unchanged. Disabled and folder masks are editable; layer/ancestor
visibility and layer locks apply. New contacts wait for pending canvas bounds.

Shape painting shares the coverage-only helper with the mask edit engine and
does not expand destination mask data to RGBA. A fully covered, fully selected,
opaque shape retains tile-free constant mask storage. No-op operations retain
redo and source coverage identity. Cancellation, stale source/slot failures and
combined payload-limit errors cannot publish a private result. One successful
lasso remains one artwork undo step. Existing raster lasso behavior is preserved;
old serialized settings omit the new default target and decode as raster.

Five new tests cover an independent opaque grayscale raster oracle for fractional
polygon/selection coverage and opacity (fill and erase, enabled and disabled),
no-op/cancel/stale-slot/undo/redo behavior, folder masks and constant storage plus
512 MiB logical payload rejection, legacy/new settings serialization, and actual
application admission of pending layer/create-mask/lasso/duplicate/lasso with
independent slot ownership and separate undo states. The egui workflow now draws
mask lasso fill and erasing through canvas packets at low simulated pressure,
checking uniform coverage and unchanged alpha-locked raster storage.

Full automated suite: **690 passed, zero failed, one existing high-memory test
ignored**. Logs: `target/qa/mask-lasso-{tests,targeted,clippy,release}.log`.
The isolated `--capture-layer-mask-lasso` fixture admits a real lasso contact
through the application with matching tool/color/opacity state. Physical pen
input, long-session responsiveness and the complete mask workflow remain release
requirements; normal creation/loading is still gated.

Warnings-denied all-target Clippy, formatting/whitespace checks and optimized
release build pass. Native `target/qa/mask-lasso-100.png` and
`target/qa/mask-lasso-200.png` were inspected: the triangular coverage edit appears
consistently on the canvas and navigator/thumbnail where shown, matching 70%
opacity and black mask paint. Target feedback and the selected tool remain visible
at 200%. These are settled rendering fixtures; interaction is covered separately
by the egui workflow. Current candidate: `target/release/dudel.exe`, SHA-256
`CD726173D4F81350149D1E08AE1AB3164BD478C9B78AF5263D506A98D41A194F`.
Existing user artwork/session state was preserved.

Next: mask clipboard and transform targets, coverage/color feedback, the complete
save/reopen/export workflow, and the remaining inspector-layout/hardware gates.
The full professional application objective remains active.

## Mask clipboard checkpoint - 2026-10-16

Copy, Cut and Paste now capture artwork-versus-mask independently of later focus
changes. Existing raster clipboard behavior remains intact. Mask copy exports
raw grayscale coverage with selection coverage as alpha, zeroing excluded holes.
Mask/layer enabled state, opacity, blend and parent appearance are not baked into
this data. Copy can read a locked/hidden mask; writing actions require an unlocked,
shown target. Disabled and folder masks remain editable, and raster alpha lock
does not prevent mask cut/paste.

Cut constructs its selected white-reveal candidate privately, validates it, and
checks exact source/revision again before writing the clipboard. Only a successful
write can publish that candidate. Failed writes, stale targets/sources, malformed
images and combined payload-limit errors leave artwork/history unchanged. Paste
uses the existing centered/clamped placement policy, rejects oversized input and
blends image luminance through clipboard alpha multiplied by selection coverage.
It preserves selection and mask metadata, edits the existing slot and creates no
layer, including at the layer limit. Transparent/no-op pastes preserve redo,
revision and modified state; successful edits have independent undo steps.

The existing clipboard barriers and workers now support mask targets. A pending
mask creation can be followed by paste, a captured mask stroke and a snapshot;
paste/history settlement holds the later commands until atomic publication.
Changing visible focus does not redirect these captured operations. Frozen sources
remain registered in the existing memory estimator; worker scratch/output and OS
clipboard allocations are still outside that estimate. Uniform full-canvas input
can retain tile-free constant coverage rather than allocating dense mask tiles.

The Edit menu now names Copy mask, Cut mask and Paste into mask when appropriate.
Mask paste remains enabled at the raster layer limit. The egui workflow checks
these labels without reading/writing the real clipboard. A capture-only paste
entry point injects synthetic image data into the production worker/barrier path
and settles guide revisions before taking its source snapshot.

Six new tests cover raw grayscale/selection-alpha copying, cut write failure and
success, luminance/alpha/selection blending and placement, disabled/folder masks
and layer-limit paste, stale source/slot rejection, failed/transparent/no-op paste
history and barrier release, private payload rejection, and pending creation/
paste/stroke/snapshot order with separate undo and native save/reopen. A tiny dab
is compared with synchronous reference painting rather than assuming its
antialiased pixel is fully covered. Full suite: **696 passed, zero failed, one
existing high-memory test ignored**. Logs:
`target/qa/mask-clipboard-{tests,targeted,clippy,release}.log`.

These are injected-backend and application tests; physical Windows clipboard
interoperability remains a release check. Normal mask creation/loading stays
gated. Next: mask move/transform, coverage/color feedback and the complete visible
workflow, followed by the remaining layout/interop/XP-Pen gates. The full goal
remains active.

Mask clipboard candidate verification: warnings-denied all-target Clippy,
formatting/whitespace checks and optimized release build pass. Native synthetic
paste captures `target/qa/mask-clipboard-100.png` and
`target/qa/mask-clipboard-200.png` were inspected: the centered coverage edit,
canvas/navigator/thumbnail where visible, target feedback and Pasted into mask
status agree. The selected mask row remains visible at 100%; the compact toolbar
fits at 200%. The real clipboard was neither read nor overwritten. Current
candidate: `target/release/dudel.exe`, SHA-256
`9DAF3B2E0FF7D22EE1C10CCDC39AA8CA47ECD5FDEE82013A91F48D326822AD12`.
Existing user artwork/session state was preserved.

## 2026-10-16: mask target Move and Transform

Move, flips, affine, perspective and mesh transforms now capture the selected
mask slot and edit coverage without moving artwork. Disabled/unlinked and folder
masks work independently of raster alpha lock. Frozen previews coalesce requests;
pending gestures resolve their source after earlier commands. Vacated regions
reveal white. Fractional selections move with the coverage, replace shared
coverage without pale overlap edges, and retain explicit emptiness off canvas.
Private results validate the combined payload limit before publication.

Ten new tests cover coordinate/selection and interpolation oracles, extreme
offsets, source rejection, sparse constants, folder targets, payload overflow,
coalescing/reversion, application Apply/Cancel, deferred Move release and separate
undo/redo. An obsolete test expecting transforms to reject mask focus was updated;
unsupported mask adjustments/filters remain guarded. Full suite: **706 passed,
zero failed, one existing high-memory test ignored**. Warnings-denied all-target
Clippy, formatting and whitespace checks pass. Logs:
`target/qa/mask-transform-{tests,targeted,clippy,release}.log`.

Normal mask creation/loading remains gated. Next audit geometry preview saved
state against pending save confirmation and independent guide changes, finish
coverage/color feedback and selection references, then verify the complete visible
workflow before enabling masks. Masked drag performance and physical XP-Pen checks
remain outstanding; no new responsiveness claim is made from these tests.

Optimized release build passed. Native mask-target transform captures
`target/qa/mask-target-transform-100.png` and `mask-target-transform-200.png` were
inspected: mask focus, transformed coverage and handles are visible, and the
100% navigator/coverage thumbnail agree with the canvas. The compact 200% toolbar
fits. At 100%, the navigator's lower controls are clipped by the layer divider;
the inspector layout issue is broader than the previously recorded 150% case.
These synthetic captures do not establish physical pen behavior or drag latency.
Current candidate: `target/release/dudel.exe`, SHA-256
`442CF69C333A988D7C1498EAFA26BD72F18CB71B4ED2961F885300490EE5C8A3`.

## 2026-10-16: geometry preview persistence and navigator layout

Move/Transform live redraws now advance display/source invalidation separately
from committed content. A persistence baseline captured at source acquisition
survives final background settlement. Apply records one content change; Cancel
and return-to-identity reconcile against confirmed saves without losing guide
edits or following FIFO commands. Save confirmation keeps a live preview unsaved;
successful earlier snapshots become clean after rollback when appropriate. Failed
saves retain modified/recovery state. Error exits retire the baseline and preserve
published changes; document replacement clears it.

The persistence regressions exercise raster/mask targets, Cancel/identity/Apply,
successful/failed confirmation before/after settlement, independent guides, direct
Move Cancel, queued cancellation, zero return and a later ordered edit. Temporary
project writes use the production save path. Full suite for this persistence
checkpoint: **708 passed, zero failed, one existing high-memory test ignored**;
`target/qa/geometry-state-{tests,targeted,check}.log`.

Navigator layout now budgets preview height against its visible inspector area
and control rows. Short areas move rotation/fit/flip into View options; the
shortest areas also move zoom controls there. The preview still shows the whole
canvas, and stored preferred preview height survives temporary compression.
Viewport budgeting is independent of scroll offset. The interaction regression
checks fully visible normal controls and the compact menu, clicks View options
and Flip canvas, and restores the larger layout without losing preferred height.

Final combined suite: **709 passed, zero failed, one existing high-memory test
ignored**. Warnings-denied all-target Clippy, formatting, whitespace and optimized
release build pass. Logs: `target/qa/geometry-navigator-{tests,clippy,release}.log`;
the targeted UI log is `target/qa/navigator-layout-targeted.log`.

Native `geometry-navigator-{100,150,200}.png` captures under `target/qa` were
inspected. At 100%, the navigator's zoom/rotation/fit/flip rows fit above the layer
divider. At 150%, the whole-canvas preview, zoom and View options are visible;
the selected mask row remains reachable. At 200%, automatic layout hides the
inspector while preserving the canvas, mask target and Apply/Cancel controls.
These captures do not verify physical pen input, OS clipboard exchange or drag
latency. No user artwork/preferences were loaded or changed by the captures.

Current candidate: `target/release/dudel.exe`, SHA-256
`A25F79466C9368E7A71C856DA44A088607720A2E782324D6BA11D01EF72800A2`.
Next: coverage/color feedback and selection-reference semantics in mask focus,
then the complete visible creation/paint/transform/undo/save/reopen/export workflow
before enabling masks. The professional application goal remains active.

## Mask workflow enabled - 2026-10-16

Normal mask creation and native masked-project/recovery loading are now enabled;
the temporary UI and loader gate was removed. An ordinary egui workflow, without
capture mode, creates a mask from the layer controls, paints grayscale, performs
undo/redo, toggles enabled state, selects raw coverage, duplicates to an independent
slot, transforms that mask, saves, reopens through the normal loader, restores
recovery and verifies PNG export. Loading an unsaved canvas still awaits the
ordinary replacement decision. This establishes the automated visible workflow,
not overall professional release readiness.

Mask focus now has independent grayscale foreground/background session values,
default black/white. Coverage controls offer Conceal, Reveal and a reveal-percentage
slider; swapping or picking coverage preserves artwork RGB colors and recent
colors. Brush, bucket, lasso and test-pad feedback use that coverage. Eraser always
reveals. Picker/Alt reads raw mask coverage, including disabled masks, and rejects
a pending/replaced slot without falling back to artwork. Disabled-mask coverage
controls explain why painting is stored without changing the composite.

The wand's Active reference now captures the mask slot when mask-focused; Visible
and named artwork references retain their existing meanings. Tests cover pending
creation, focus changes after release, stale slots, fractional values, connectivity,
locked/hidden/disabled/folder sources and legacy settings. Select > From mask
coverage reads the raw mask; From artwork coverage uses raster alpha multiplied by
an enabled mask, excluding opacity/parent styling. Coverage selection is undoable,
allows locked/hidden sources and retains explicit emptiness.

Nine additional tests cover independent mask colors, coverage selection semantics,
wand reference/ownership/cancellation/legacy serialization, real egui coverage-popup
and picker/wand input, and the complete normal workflow. An obsolete loader-gate
test now verifies clean loads and preservation of unsaved canvases. One existing
mask-fill application fixture now selects white coverage through the separate
mask-color state. Full suite: **718 passed, zero failed, one existing high-memory
test ignored**. Warnings-denied all-target Clippy, formatting and whitespace pass.
Logs: `target/qa/mask-workflow-{tests,clippy,release}.log`.

Native visual/build verification follows. Experimental indexed storage and stroke
recording still reject masked documents explicitly; native CSP mask import and
physical OS clipboard/XP-Pen behavior remain unverified. Next measure masked drag
work and finish the remaining everyday drawing, interop and release gates.

The first native coverage-popup review exposed a 200% toolbar overflow from the
longer Coverage label. Short toolbars now use direct numeric size/opacity fields;
the narrowest move those controls into Stroke options. The rendered interaction
regression verifies full button bounds and opens the menu at effective widths of
840, 768, 640 and 480 points, including access to the moved controls. Another regression
covers a child retaining a previous wider layout: responsive choices now clamp
against the visible viewport, including after DPI changes. Runtime capture traces
showed the native 200% viewport was 768 points wide; that width now falls within
the numeric-control breakpoint and the regression. Final captures below
supersede that first visual review.

Final optimized build passed. `target/qa/mask-coverage-final-{100,150,200}.png`
captures were inspected: Coverage popup, grayscale chips/percentage, target
feedback and the full Stroke options button fit at all three scales. The native
200% capture uses direct numeric size/opacity controls. Capture logs now report
viewport width, pixel scale and toolbar mode; observed viewport widths were
1536/1024/768 points respectively. Screenshots establish those layouts, not pen
behavior, clipboard exchange or measured drag latency.

Current candidate: `target/release/dudel.exe`, SHA-256
`4FCB8F7CF6A9A741BF551CD0CFF72A356CBF2CF08897AD29386FCF99C7EBF3CB`.
The initial rebuild preserved the running user process by renaming its old binary
to `target/release/dudel-running-10064.exe`; no user session was terminated. The
new build is at the normal executable path. No user artwork was used in captures.
The full professional application goal remains active.


## Mask-transform performance checkpoint (2026-10-16)

Added a reproducible release CPU probe with separate owner, worker/publication,
Undo/Redo and resident-memory observations. Each run covers 90 measurements over
1024-square, 2048-square and 4096x1024 documents: sparse/dense whole masks,
rectangular/fractional selections and folder masks; Move, integer affine and
bilinear fractional translations; ready previews and pending final requests.
Independent pixel/selection oracles verify results, unchanged artwork/flags and
one-action Undo/Redo outside the timed region.

Selected moves now visit disjoint affected regions, integer affine translations
share the exact Move path, sparse white masks visit bounded materialized regions,
and dense whole moves copy tile row spans. Bulk sampled output preserves default
coverage and unchanged tile identities. Selection bounds are accumulated during
construction. Cancellation, explicit empty selections, extreme offsets, sampling
quality and near-limit payload behavior have regression coverage.

At 2048 square, two-run ready-preview wall ranges changed as follows:

- Dense rectangular Move: 91-108 ms to 0.69-0.76 ms.
- Dense whole-mask Move: about 75 ms to 2.68-2.94 ms.
- Sparse bilinear fractional translation: 197-199 ms to 1.28-1.31 ms.
- Dense bilinear fractional translation: 255-270 ms to 193-200 ms, still an
  interactive responsiveness issue.

The observed owner-call maximum was 0.550 ms after the changes. These CPU timings
exclude compositing, display/GPU presentation, real frame scheduling and tablet
input; they do not establish FPS or end-to-end drag latency. Resident snapshots
increased (maximum 62.2 MiB versus 38.8 MiB), requiring a session memory soak.
No memory reduction or leak conclusion is supported. Full methodology, all
results and retained report names: `research/MASK_TRANSFORM_PERFORMANCE.md`.

Verification: 725 passed, zero failed, one existing high-memory test ignored;
all-target warnings-denied Clippy, formatting, whitespace and release build pass.
Native 100/200% transformed-mask captures were inspected, with matching canvas,
navigator/thumbnail where visible, target and Apply/Cancel feedback. Logs/captures:
`target/qa/mask-performance-*`. Final release and frozen probe binary SHA-256:
`7BD99B0E24B371C9041F755B15BE0EC505EE5FC568DDA287B341C711C122898E`.
Frozen binary: `target/qa/dudel-mask-transform-final.exe`.

Next: native display/frame and sustained-session memory measurements, then address
remaining dense fractional-transform latency without changing final sampling
quality. Physical input, OS clipboard, supported interop and the broader release
gates remain open. The professional application goal stays active.


## Mask lifecycle memory checkpoint (2026-10-16)

Added `--soak-mask-transforms NEW_REPORT.json`, a bounded 64-cycle same-process
check of dense, fractional-selection and sparse 2048-square mask application
lifecycles. Each cycle verifies 18 ready/pending Move/affine results plus Undo/Redo
and releases document/history/worker ownership before sampling. The Windows
memory query now reports both working-set and private committed bytes. Existing
report files are preserved. Only scalar summaries survive between cycles.

Two release runs verified 2,304 operations in 118.9 and 128.7 seconds. Post-cycle
private committed ranges were 2.60-13.38 and 2.61-13.84 MiB; resident ranges were
35.77-45.57 and 10.68-20.61 MiB. Run 1 finished above its early warmed memory level;
run 2 returned near its early level after larger intermediate values. No
reproducible monotonic increase was shown, and leak freedom or a native-session
plateau is not established. These released-state snapshots cannot establish a
reduction from previous live result/history measurements.

Durable reports: `benchmarks/mask-lifecycle-soak-release-{1,2}.json`.
Method, medians, limitations and build identity:
`research/MASK_LIFECYCLE_MEMORY.md`. Full suite remains 725 passed, zero failed,
one existing high-memory test ignored; warnings-denied all-target Clippy,
formatting, whitespace and optimized build pass. Current candidate/frozen probe
SHA-256: `AF6B85D34BABE494E82AE213D94537E738A4AF48CC864A01717EDB741EFE8A5E`.

Next: persistent native display/history/session measurements, including comparable
idle checkpoints, rather than treating fresh-app CPU lifecycles as full session
acceptance. Dense fractional transforms and physical input/clipboard/interop/
packaging gates remain open. The professional application goal remains active.


## Persistent native mask session checkpoint (2026-10-16)

Added a visible isolated native diagnostic that keeps one app alive with a
supported 2048-square four-layer document, normal canvas/display workers,
navigator/thumbnails and accumulating history. It verifies clean Cancel, sixteen
full-quality bilinear mask translations against an independent pixel oracle,
per-edit Undo/Redo, all-sixteen Undo/Redo endpoints, and asynchronous native
save/reopen with fresh mask identity. It releases history before a settled memory
checkpoint, then captures the final saved mask-focused UI. It ignores external
input and does not load/save user preferences or artwork. Isolated renderer
fallback no longer persists a graphics setting. Missing/invalid/existing report
paths fail; a stopped diagnostic without a verified report exits with failure.

Final 100/200% application-zoom runs each completed in about 34.6 seconds, with
2,496 callback observations. UI-prefix p95 was 2.81/2.53 ms and max 5.07/4.70 ms.
Independent verification/command driving is separate and can exceed 100 ms;
callback intervals therefore do not establish ordinary FPS, GPU execution or
pen-to-display latency. Final screenshots at both scales were inspected.

Tracked undo storage reached 60.44 MiB; releasing it before saving reduced
resident memory by about 60 MiB in both runs. Entering/canceling the first preview
raised private commitment by 68-69 MiB without corresponding tracked document
ownership. Its cause remains unproven. The diagnostic's fixed baseline ownership
is included in tracked counts, including separate decoded artwork after reopen.
Neither leak freedom nor sustained-session readiness is established.

Durable reports: `benchmarks/native-mask-session-release-{100,200}.json`.
Full method, data, images/logs, hardware inventory and limitations:
`research/NATIVE_MASK_SESSION.md`. Full suite: 725 passed, zero failed, one
existing high-memory test ignored; all-target warnings-denied Clippy, formatting,
whitespace and optimized build pass. Release/frozen probe SHA-256:
`D18205F57DF818BFC52FC672C17A939DD9BB365C2C89600C3E198B11D8A1126D`.
Frozen executable: `target/qa/dudel-native-session.exe`.

Next address the dense fractional-transform worker bottleneck, improve mask-aware
Move guidance, and profile private commitment over longer comparable native
checkpoints. Physical drawing, OS clipboard, supported interop and packaging
remain release gates. The professional application goal remains active.


## Dense mask sampling and target guidance checkpoint (2026-10-16)

Bilinear coverage sampling now shares one tile lookup for interior 2x2 taps and
converts the footprint origin once, preserving the exact arithmetic/rounding and
white-outside semantics. Large whole-mask sampled output uses at most four bounded
sampling children of the existing edit worker; all launched children join before
publication/error/cancellation return. Failed thread creation computes that batch
locally. Small selections and integer Move keep their scheduling. Tile sharing,
original default coverage, compact constants and document quotas are preserved.

Two interleaved release comparisons verified all 360 application measurements
against independent mask/selection oracles and one-action Undo/Redo. At 2048 square,
dense bilinear fractional translations fall from 203.35-203.58 ms to 36.38-36.87
ms. Some small control cases vary or are slightly slower; no universal speedup or
full-rate interaction claim follows. Observed maximum owner call is 1.333 ms.
Resident process snapshots are higher in the after probes; no memory reduction
or leak conclusion is supported. Full values: `research/MASK_SAMPLER_OPTIMIZATION.md`.

Move guidance now names selected mask coverage/whole-mask editing in toolbar,
sidebar and rail. Painting, erasing, filling, picking and lasso hints describe
coverage/conceal/reveal when mask-focused. Native captures at 100/200% were
inspected. Both short persistent native workflows again verify Cancel, sixteen
mask edits, Undo/Redo and asynchronous save/reopen with fresh mask identity. UI
prefix p95 is 3.09/2.73 ms; diagnostic driving/oracles and GPU time are excluded.
History release again recovers about 60 MiB resident memory, while the initial
private-commit increase remains unexplained.

Verification: 727 passed, zero failed, one existing high-memory test ignored;
all-target warnings-denied Clippy, formatting, whitespace and optimized build pass.
Current release/frozen sampler binary SHA-256:
`AA95700D4A4A14F877E03E60CDC54905E65DF9962050BF1D350F22EEA50A0003`.
Frozen executable: `target/qa/dudel-mask-sampler-parallel.exe`.
Durable before/after/native reports are named in the sampler report.

Next broaden realistic drawing/geometry acceptance, profile unexplained native
private commitment in longer sessions, and complete remaining physical input,
clipboard, supported interop and packaging gates. The professional application
goal remains active.


## Brush creation and compact studio checkpoint (2026-10-16)

Brush Studio exposes Save as new beside Update preset and Revert. Creation and
sidebar saving share a validated command that selects a uniquely named new entry
and reveals it through library filters. Duplicate names obey the existing
200-byte limit and UTF-8 boundaries, preventing the previous valid-name-plus-
copy-suffix invalid entry. Full brush equality replaces preview-image hashing
for edited-state feedback, so stabilization-only changes enable Update/Reset.
Favourite metadata stays synchronized with its selected library entry, including
normalization of legacy stale current-brush flags on preference decode.

The studio has bounded viewport-aware dimensions, wide section navigation or a
compact section chooser, a smaller compact preview and a fixed settings viewport.
Footer actions and status/errors remain outside the scroll area. Long captions
reserve space for the star and expose their complete name on hover. Native
100/150/200% captures were inspected after correcting a clipped status line.

Five added regressions cover safe maximum-length names, stabilization/favourites,
atomic invalid/full-library rejection, creating ink/pencil/image presets through
preference and pack files with identical settings/rendered pixels, and actual
egui Save/Revert/section/resize/error interactions at compact sizes. The preference
test uses app save/shared hydration with temporary storage, not the real user
settings or a physical process restart.

Verification: 732 passed, zero failed, one existing high-memory test ignored;
all-target warnings-denied Clippy, formatting, whitespace and optimized build pass.
Details: `research/BRUSH_CREATION_WORKFLOW.md`.
Current release/frozen studio executable SHA-256:
`8FED0D4F7EC63B3A880E60A2C2604CC7C112E8C5550E5EB0F16C61155C2046A2`.
Frozen binary: `target/qa/dudel-brush-studio.exe`.

Next audit and protect failed preference restoration: default fallback must not
silently lead to replacing valuable prior settings on later autosave. Native
restart/durability, physical brush input, longer session memory, external clipboard,
supported CSP fixtures and packaging remain open. The professional application
goal remains active.


## App preference recovery checkpoint (2026-10-16)

Failed application preference decoding now blocks preference replacement and egui
memory persistence. Closing the new recovery window leaves writes paused and a
persistent Review banner available. Export original writes exact UTF-8 bytes to a
new staged/synced non-overwriting file. Replacement actions re-read the protected
value and verify its backup byte-for-byte; they validate current/candidate settings
before lifting the guard. Restoring a file closes prior input and clears stale
brush filters/recents. Project saving remains independent and tested.

Five added regression groups cover repeated guarded saves; malformed/future/
invalid data and bounded errors; exact backups, missing/changed files/storage,
changed protected values and invalid session rejection; validated atomic model
restore; asynchronous project save/reopen while guarded; and actual recovery UI
hit testing. Tests use fake storage and owned temporary files. Native 100/150/200%
fixture captures were inspected with the banner and all recovery controls visible.

Verification: 737 passed, zero failed, one existing high-memory test ignored;
all-target warnings-denied Clippy, formatting, whitespace and optimized build pass.
Details: `research/PREFERENCE_RECOVERY.md`.
Release/frozen recovery executable SHA-256:
`B4A7EA30EA640B4CFFE0A99570B9A44C18D54CBB3ED551C5BD1690E876C015B9`.
Frozen binary: `target/qa/dudel-preferences-recovery.exe`.

The eframe 0.33.3 container audit found that failed RON reads become an empty map
and container writes use direct file creation. This app-key guard cannot detect
that missing-key presentation or certify crash-safe whole-container writes.
Next harden durable settings storage, failure/retry and isolated real-store
restart/recovery, preserving existing libraries and user data. Large settings I/O
latency, physical input, long sessions, external clipboard, supported CSP fixtures
and packaging remain open. The professional application goal remains active.

## Whole-container settings durability follow-up (2026-10-16)

Closed the framework direct-write gap by isolating eframe's file in a private
session directory and publishing validated RON snapshots through synced atomic
replacement. Retains the previous distinct valid version, preserves invalid
previous data before rotation, refuses competing/stale writers, and exposes
save failure/retry. Corrupt-root startup can load the previous valid preferences
while retaining the protected original until an exact verified export permits
replacement. Diagnostic captures cannot write to the real framework store.

Verification: 745 passed, zero failed, one existing high-memory test ignored;
all-target warnings-denied Clippy, formatting, whitespace and release build pass.
Six separate-process native phases passed (save, reopen, corrupt, repair, failure,
retry). Viewed the actual recovery and failure UI at 1280x800. Reports, limitations
and reproduction: `research/SETTINGS_STORE_DURABILITY.md`.
Release/frozen binary SHA-256:
`BE0952F180A9621EB29038856E81062920B71671C00BB27C3D45A3F0995ECB8C`.
Frozen binary: `target/qa/dudel-settings-store.exe`.

Next resolve the long fixture brush name pushing the rightmost toolbar Snap
control outside the 1280-wide viewport. Hardware input, sustained memory/GPU
interaction, external clipboard, supported CSP fixtures and packaging remain open.
The full professional application goal remains active.

## Responsive brush toolbar and compact command header (2026-10-16)

Bounded/truncated the current brush picker while preserving the full name and
preset workflow. Brush/Eraser primary controls use compact fields as needed;
inline stroke controls are measured against remaining visible space. Compact
Symmetry expands within Stroke options, with scrolling through all settings.
Narrow windows put document state above a compact, directly visible command row.
Long filenames truncate without overlapping commands or save status. Main menus
are height-bounded and reopen at the top after viewport changes.

Verification: 748 passed, zero failed, one existing high-memory test ignored;
all 17 application workflow tests, all-target warnings-denied Clippy, formatting,
whitespace and optimized build pass. Viewed seven final native captures spanning
100/150/200%, 480x320 logical minimum, brush/stroke popovers and 1920-wide inline
controls. Original and frozen executable SHA-256:
`789644A48F40B20CA9F00B44C8AF02EAAE43C896C1E7FFF2DF3197DD198BF72D`.
Frozen executable: `target/qa/dudel-responsive-toolbar-settled.exe`.
Details: `research/RESPONSIVE_BRUSH_TOOLBAR.md`.

The release initially encountered an in-use executable and that app session was
left untouched; the final normal release build succeeded once it was available.
Next finish New canvas cancellation and keyboard UX, then continue native
resource/input and release gates. Physical XP-Pen, external clipboard, supported
CSP fixtures and packaging remain open. The professional app goal remains active.

## New canvas cancellation, keyboard flow and safe replacement (2026-10-16)

Replaced the unguarded floating creation window with a validated modal form.
Width/height support initial focus, Tab and Enter; Cancel/Escape preserve the
painting, and Enter on focused Cancel cancels. Canvas shortcuts and Tab panel
switching are blocked, including simultaneous input in the Ctrl+N opening frame.
Invalid sizes stay editable and cannot create a document. Scrollable fields and
pinned actions fit the minimum logical viewport.

Submission uses the shared unsaved-work guard. Cancel returns to the retained
form; Discard creates only after confirmation; Save writes the old drawing before
creating the new document. Already-running saves retain the pending form. Failed
saves and saves invalidated by newer edits cancel replacement, preserve artwork
and history, and explain the outcome inside the form.

Verification: 758 passed, zero failed, one existing high-memory test ignored;
all-target warnings-denied Clippy, formatting, whitespace and optimized build pass.
Viewed five native captures at 100/150/200%, including valid and over-limit forms
at 480x320 logical pixels. Details: `research/NEW_CANVAS_WORKFLOW.md`.
Release/frozen binary SHA-256:
`977EBAF5E2B6BA96DC7D85BDB5089F140AD437609D47BC121AE293624730AC6B`.
Frozen binary: `target/qa/dudel-new-canvas.exe`.

Next extend sustained native drawing/resource validation and release checks.
Physical XP-Pen, OS clipboard, supported CSP fixtures and packaging remain open;
no physical input or professional-release certification follows from form tests.
The full professional drawing-application goal remains active.

## Repeated native drawing/resource study (2026-10-16)

Extended the isolated native diagnostic to twelve rounds at each of 100% and
200%: generated pressure-varying stabilized canvas input, mask-transform Cancel,
sixteen byte-oracle mask edits, per-edit/full Undo/Redo, history release, save and
reopen, and reset to a comparable baseline. The two runs completed 24 strokes,
384 mask edits and 24 save/reopens over 813.54 seconds. The old short mode also
passed. Added separate instrumented app-command timing and fixed-capacity trace
accounting. A byte/metadata snapshot oracle accepts legitimate fresh storage after
compact Undo without weakening mask-transform artwork-sharing checks.

Matching idle snapshot/display allocation counts remained stable (at most 15 bytes
of snapshot-total variation across all rounds), with history
returning to zero. Process commitment still has an unexplained first-preview
increase of 67.70/69.40 MiB and reopened-idle tail drift of 12.05/12.91 MiB beyond
constant registered ownership (trace use increases about 1.62/1.64 MiB). This is
not proof of a leak or a closed memory gate. UI-prefix p95 was 2.857/2.695 ms;
maximum instrumented app-command time was 7.432/7.049 ms. Diagnostic driver/oracle
maxima reached 156.74/150.53 ms, so callback intervals cannot represent FPS or
physical input latency.

Verification: 759 passed, zero failed, one existing high-memory test ignored;
all-target warnings-denied Clippy, formatting, whitespace and optimized build pass.
Inspected both native screenshots and a standalone memory chart. Raw reports,
checked summaries, command timings, launch observation caveat and reproduction:
`research/SUSTAINED_NATIVE_DRAWING.md`.
Release/frozen binary SHA-256:
`96C645773B3D705732A94E0F5C72D9AD3BAD7C125E2D10D69FDB86DD161369F3`.
Frozen binary: `target/qa/dudel-sustained-native-v2.exe`.

Next profile first-preview/reopen allocator/renderer ownership. Physical XP-Pen,
real OS clipboard/input, GPU interaction, supported CSP fixtures and packaging
remain open. The professional drawing-application goal remains active.

## Optional Rust allocation accounting (2026-10-16)

Added an opt-in `allocation-profile` feature with nonallocating System-wrapper
hooks and native checkpoint live/peak/cumulative request counters. Normal builds
keep their usual allocator. Short OpenGL checks at 100/200% and another twelve
rounds at 100% all exited zero, including byte-oracle transforms, generated brush
input, Undo/Redo, history release and save/reopen. The first cancelled preview
adds 65-69 MiB process commitment with less than 23 KiB more live Rust requests.
Matching-idle Rust live drift over rounds 4-12 is 0.693 MiB; private reopened-idle
drift is 2.250 MiB. Registered snapshot/display counts are stable. Diagnostic
metadata ownership and heap backing/external renderer ownership remain open;
the counters do not establish a leak verdict or close the memory gate.

Verification: feature suite 762 passed; default suite 759 passed; zero failures
and one existing high-memory test ignored in each. Default/all-feature Clippy,
formatting, whitespace and both optimized builds pass. Default native mode also
passes and reports profiling disabled. Reports, inspected captures, limitations
and reproduction: `research/RUST_ALLOCATION_PROFILE.md`.
Normal release/frozen executable `target/qa/dudel-allocation-normal.exe` SHA-256:
`F93D0159533BF17A2080A9900E3A981AE207772C67609E79E2152778FB06EE9E`.
Separate profiled executable: `target/qa/dudel-allocation-profile.exe`.

Next independently attribute diagnostic checkpoint metadata, then profile heap
backing/external renderer ownership. Physical XP-Pen, OS input/clipboard,
supported CSP fixtures and packaging remain open. The full professional drawing
application goal remains active.

## Fixed diagnostic storage and renderer control (2026-10-16)

Removed growing JSON checkpoint ownership from the native memory diagnostic:
fixed-size records reserve all 23/288 samples before warmup, report capacity/used
bytes, preserve existing JSON fields and fail instead of growing past the reserve.
The analyzer validates fixed storage and still accepts historical reports. A new
plot compares live Rust requests with process commitment/residency.

Another twelve-round OpenGL drawing/save/reopen run passed, with all 288 records
in 57,600 bytes. Matching-idle Rust live drift over rounds 4-12 fell from 0.693
MiB to 6.25 KiB; history returned to zero and registered snapshot/display counts
stayed constant. The larger process-memory discrepancy remains: first preview
added 69.875 MiB private commitment. Matched short checks using the same profiled
binary passed on OpenGL and actual DirectX 12/RTX 3060; first-preview private
increases were 68.516/0.441 MiB respectively, with substantially higher initial
DX12 commitment. No allocator-stack ownership, leak verdict, sustained DX12 or
physical pen-latency claim follows. Renderer preferences were not changed.

Verification: full suites 762 profiling/759 default passed, zero failures, one
existing high-memory test ignored each; default/all-feature Clippy, formatting,
whitespace and optimized builds pass. Restored default native mode passed with
profiling disabled and all 23 records in a fixed reserve. Inspected four native
captures and the comparison chart. Evidence and reproduction:
`research/FIXED_NATIVE_CHECKPOINTS.md`.
Normal release/frozen `target/qa/dudel-fixed-checkpoints-normal.exe` SHA-256:
`F3F90C501484C88C6E7C4FB2699EE4D714E1269110FF94BF9F616C21D391E988`.

Next profile heap backing/external renderer ownership and sustained modern-renderer
behavior. Physical XP-Pen, real OS input/clipboard, supported CSP fixtures and
packaging remain open; the complete professional application goal stays active.

## Panel-layout recovery and sustained DX12 evidence (2026-10-17)

Added View > Automatic panel layout near Show all panels. It clears persistent
manual sidebar overrides and restores visible responsive panels without artwork,
revision, modified-state or history changes. Real menu tests cover 480x320,
960x700, 1280x800 and 2560x1000 plus preference encode/restore and prior Undo.
Inspected native menu captures at 100/200% and small size. Added a current first-use
guide at `QUICK_START.md`, linked from README, with source-checked UI labels and
corrected stale inspector/rename wording. Evidence:
`research/AUTOMATIC_PANEL_LAYOUT.md`.

A frozen normal executable completed the twelve-round native drawing workflow on
actual DirectX 12/RTX 3060: 12 generated strokes, 192 byte-oracle transforms,
12 save/reopens and all 288 checkpoints. Correctness passed, but the resource
gate remains open: matching reopened-idle private commitment grows 77.680 MiB
from round 4 to 12 despite stable registered document/display ownership and
released history. The short backend check did not establish sustained stability.
Documented raw counters, inspected native screenshot/chart, timing boundaries
and artifact identity in `research/SUSTAINED_NATIVE_DX12.md`.

Current full verification: 760 default/763 allocation-profile passed; zero failed,
one existing high-memory test ignored each. Default/all-feature Clippy, formatting,
whitespace and optimized build pass. Current release/frozen executable:
`target/qa/dudel-automatic-layout.exe`, SHA-256
`0DB81976F4DBCDE44579B37A3408978A2B08B73707BA2BB205F09D8DD53432D8`.
Next profile the exact repeated DX12 workload with optional Rust counters.
Physical XP-Pen, clipboard, controlled CSP interop and packaging gates remain open;
the complete professional application goal remains active.

## DX12 Rust and GPU allocation ownership (2026-10-17)

Repeated the twelve-round DX12/RTX 3060 workflow first with Rust request counters,
then with an optional wgpu GPU-suballocator summary. Both exited zero, completing
12 generated strokes, 192 byte-oracle mask edits and 12 save/reopens each. Rust
live growth reproduces at about 30 MiB across matching reopened-idle rounds 4-12.
All 36 matching GPU idle samples in the new study remain at 35.25 MiB suballocated,
320 MiB reserved, 148 allocations and two blocks. Registered document/display
ownership stays constant and history returns to zero. This narrows the next
investigation to retained Rust call stacks/owners while leaving the remaining
process commitment unassigned; it does not establish a leak cause or close the
memory gate. No forced GPU waits, renderer-default or dependency change was made.

Added feature-only GPU summaries with temporary report data dropped before Rust
sampling. Native checks cover supported DX12, unavailable OpenGL and disabled
normal-build probes; all pass complete cycles and fixed-storage validation.
Current full suites: 760 normal/763 profiling passed, zero failures, one existing
high-memory test ignored each. Default/all-feature Clippy, formatting, whitespace
and both optimized builds pass. Raw reports, inspected captures/chart, source
review boundaries and build identities: `research/DX12_ALLOCATION_OWNERSHIP.md`.
Restored normal release/frozen `target/qa/dudel-gpu-profile-normal.exe` SHA-256:
`15CA2C30D15F7983FE07CBCB964487B762507E8AEF1B1802F979A6F8D2AB9DD6`.

Next capture retained Rust allocation call stacks, with a matching executable/PDB
pair and process-local tracing if needed. Physical XP-Pen, OS input/clipboard,
controlled CSP interop and packaging remain open; the complete application goal
stays active.
