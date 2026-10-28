# Persistent layer masks: implementation contract

2026-10-15. This is required capability work under `DELIVERY_PLAN.md`, not a
replacement for the professional application goal. The engine, tool targets and
ordinary editable workflow are now available. Physical input, measured masked
drag responsiveness and supported external interchange remain release checks.

## Current integration status

The document/persistence/rendering stage is now implemented and tested. `Layer`
carries optional enabled/linked coverage, document version 2 gates masks, and
container v5 stores ordered bounded mask subrecords after raster tiles and before
optional selection coverage. Version-1 unmasked documents retain the v4 path.
Regular project/recovery storage and bounded JSON serialization preserve coverage
and flags. Legacy v3 and experimental indexed writers reject masks; indexed
rejection precedes destination creation or journal mutation. Masked documents
share the existing 512 MiB packed raster payload ceiling with artwork, without
raising canvas/layer limits or input-file limits.

Strict snapshots and history compaction include mask identity/state and document
version. Mask changes retain shared full snapshots instead of losing coverage in
RGBA-only patches. Mask allocations enter live, history and worker descriptors.
Leaf and isolated-folder coverage is composited after layer opacity (and existing
CSP leaf byte-opacity quantization). Clipping base coverage includes its mask.
Display-tile signatures, prefix states, thumbnails and display source generations
track mask-only changes; untouched mask tiles can keep their cached states.

Canvas resize/crop and quarter-turn rotation now map every mask, including
disabled/unlinked and folder masks; added canvas space has white coverage. Linked
whole-layer raster flips, Move, affine, perspective and mesh transforms move mask
coverage with the artwork. An explicit selection transforms raster pixels behind
the stationary layer mask; unlinked masks also stay put. Selecting a mask now
routes Move, flips, affine, perspective and mesh transforms to coverage alone,
independent of enabled/linked state and raster alpha lock. Folder masks are valid
targets. Mask grayscale sampling is
independent of raster alpha, with nearest/bilinear/bicubic quality and white
outside the source canvas. Mask-only changes count as edits; equivalent results
retain source coverage identity. Direct and worker transforms reject over-budget
results before publication. Move and affine cancellation restore coverage along
with raster/selection state. Canvas rotation checks cancellation during raster,
layer-mask and selection-coverage work.

Masked affine drags use the coalescing full-quality worker because the raw-raster
GPU overlay cannot represent stationary/linked mask and folder contributions.
That overlay remains available for mask-free documents. CPU owner/worker probes
are now recorded in `MASK_TRANSFORM_PERFORMANCE.md`; native display/frame timing
and physical validation remain open. The native
`--capture-layer-mask-transform` fixture exercises a linked masked layer with
scale/rotation and the existing Apply/Cancel UI.

Masks now have process-local identities that persist through edits/history and are
fresh on replacement, duplication and reopening. Native/JSON mask payloads do not
store these IDs. Strict source guards include identity. Typed queued commands
create reveal-all/hide-all/from-selection coverage, invert, enable/link, apply and
remove masks. Existing-mask commands capture the expected mask ID and reject
replacement targets. Commands respect layer locks; enabled-mask alpha baking
requires unlocked raster transparency. Apply bakes byte alpha with nearest-byte
rounding and retains RGB; it ignores the canvas selection and leaves layer style
in place. Disabled-mask Apply removes the mask without rewriting pixels. All-white
Apply also avoids acquiring a pixel writer when no alpha byte changes.

Merge-down bakes enabled masks from both leaves once and removes the lower mask
from the merged raster, retaining the existing lower-style contract. Merge-folder
bakes child masks while retaining its own mask/state on the result. Applying a
folder mask requires merging the folder first. Ungrouping a masked folder rejects
instead of silently dropping coverage; remove the mask explicitly or merge the
folder to keep its appearance. Undo retains the pre-operation masks and pixels.
Clipboard capture combines the active layer's enabled mask and selection coverage;
opacity/blend/parent-folder effects remain outside raw active-layer copy. Pasting
creates a raster with baked clipboard alpha and no mask. Cut edits the selected
raster pixels, preserves its mask, and publishes only after the clipboard write.

Captured stroke settings now select raster or a stable mask ID. Live dabs,
stroke-opacity accumulation, pressure/grain/symmetry, taper replay and sliced
rollback use the same painting math through a grayscale target adapter. Mask
painting leaves raster RGB/alpha untouched, ignores raster alpha lock, honors
selection and layer locks, and supports disabled/folder masks. Erasing reveals
white coverage. Cursor and rollback guards reject replaced layers/mask slots.
Rollback restores the original shared coverage. Version-2 raster and mask strokes
validate combined payload size at settlement and roll back over-budget contacts
before later queued snapshots. Queue tests cover creation followed by a captured
mask stroke, native save/reopen, export, and independent undo steps.

Queued stroke saved-state bookkeeping now captures its baseline at execution,
after prior command results reach the app. Live redraw/source invalidation remains
monotonic, while committed content advances separately at stroke settlement.
Canceled/no-op contacts restore the appropriate saved state without losing prior
queued edits, independent guide changes or following commands. A confirmed save
can match the restored content despite newer redraw revisions; failed saves do
not clear modified state. Now-saved recovery cleanup avoids canceling unrelated
input. Save/export snapshots and clipboard fences retain their UI-update boundary.

Pending layer metadata now carries each mask slot's identity and enabled/linked
state. Queued creation, mask commands, duplicate, merge and ungroup project the
same metadata as execution. Typed duplication reserves independent copied mask
slots from the reserved copy identities, including nested contents. New-layer
creation followed by mask creation/painting and duplication/painting can be
admitted before raster work finishes. Failed creation cannot redirect a captured
mask contact to raster pixels.

Application focus now explicitly distinguishes artwork and a captured mask slot.
Thumbnail actions carry the displayed mask identity and compare it against pending
metadata; row and mask widgets also keep stable layer/slot IDs across frames. A
replacement between pointer press and release cannot inherit the previous mask's
click. Toolbar target controls deliberately address the current projected slot.
Changing focus closes the earlier contact without draining its pixels, clears
Shift continuation, and leaves already-captured dabs bound to their original
storage. Mask focus survives same-slot edits and stroke undo; removal/replacement
resets focus when settled. Diagnostic captures expose grayscale mask thumbnails,
separate target borders/buttons, enable/link/invert/apply/remove and creation
actions, plus a toolbar target chooser accessible with the inspector hidden.
Actual canvas packets use this focus, including mask painting with alpha lock.
Mask clear and bucket fill now edit captured coverage on the ordered background
path, including folder and disabled masks, selection coverage and opacity. Clear
reveals white coverage and whole-mask clear ignores selection. Active fill reads
frozen grayscale coverage; visible fill reads the checked artwork composite and
explicit layer references retain raw-raster semantics. Gap closing and expansion
reuse existing region behavior. Plain fills evaluate only the connected component
and its boundary, using bounded visitation/result bitsets; painting visits the
set bits without expanding mask coverage to RGBA. Uniform whole-mask results keep
constant storage, including all-white selection masks. Canceled, stale, locked,
hidden, invalid or over-budget work is rejected before publication.

Lasso fill/erasing now captures the same stable painting target at contact start.
Mask lasso combines antialiased polygon and selection coverage with opacity,
interpolating toward grayscale or white for erasing. It supports disabled/folder
masks and ignores raster alpha lock without touching RGB/alpha or selection.
Uniform full-mask shapes retain constant storage. Queued creation/duplication,
no-op redo, cancellation, stale slots and combined payload rejection are tested.
Legacy lasso settings omit the default raster target and still decode as raster.

Mask clipboard actions now capture the target layer and mask slot. Copy exports
raw coverage as opaque grayscale, using selection coverage as clipboard alpha
and zeroing excluded holes; enabled state and layer/parent styling are not baked.
Cut prepares selected white reveal privately and publishes only after a successful
clipboard write. Paste centers image luminance within the current selection or
canvas, blends through clipboard alpha and selection coverage, and preserves
selection, flags and all raster data. It edits the existing mask without adding a
layer, including folder masks and stacks at the raster layer limit. Worker source
guards, payload validation, one-step undo and no-op redo preservation are tested.
Uniform whole-canvas pastes retain constant storage where applicable. Actual OS
clipboard round trips remain unverified; automated tests use injected clipboard
backends, and native captures use synthetic input without touching the clipboard.

Unsupported mask filter/adjustment
actions reject with an artwork-target explanation. Whole-canvas geometry keeps its
existing coordinated raster/mask behavior. Recording checks the document again
after flushing pending mask creation and rejects unsupported mask captures.

Mask creation and native masked-document loading are now available in the normal
application; the temporary UI/loader gate was removed. The ordinary egui workflow
creates a mask, paints grayscale, performs undo/redo, toggles it, selects its raw
coverage, duplicates it with independent identity, transforms coverage without
moving artwork, saves/reopens through the actual loader, restores recovery and
compares PNG export with the current composite. Loading still preserves an
unsaved canvas pending the normal replacement decision. Experimental indexed
storage and stroke recording reject masks explicitly; native CSP mask import is
not claimed. Diagnostic captures remain isolated from user artwork/preferences.

Current evidence: 727 tests passed, zero failed, one existing high-memory test ignored;
warnings-denied all-target Clippy, formatting, whitespace and release build pass.
Native static, transformed-mask and queued mask-stroke captures at 100% and 200%
were inspected. Seven
UI workflow tests also pass after making the tool rail reveal its selected tool
on tool/height changes. This proves the documented storage/rendering/geometry
stages. An additional egui workflow checks thumbnail hit testing, target switches,
canvas mask input, Undo, enabled state and the toolbar chooser with the inspector
hidden. A second workflow verifies selected-mask reveal after layer-area resizing
and preservation of manual scrolling. These do not prove the full mask workflow, CSP interoperability or tablet
input.

Focus controls were also inspected in final 100%/150%/200% native captures. The
selected mask row is revealed after target/available-size changes, while manual
scrolling is preserved. The 150% navigator remains partially clipped in its
scroll area and needs the broader compact-inspector layout pass. This does not
establish overall workspace or mask-workflow readiness.

Mask-fill captures at 100%/200% were also inspected: the Active mask reference and
matching fill settings are visible at 100%, with matching canvas/navigator and
coverage thumbnail; the compact 200% toolbar fits. The automated workflow also
performs bucket fill and Delete-to-clear through canvas/keyboard input. Remaining
tool targets, interop and physical pen behavior are still release requirements.

Lasso mask captures at 100%/200% were also inspected, with matching triangular
coverage and opacity, navigator/thumbnail where visible, and selected-target/tool
feedback. The egui workflow supplies lasso fill/erase interaction evidence; these
captures do not establish physical input or full workflow readiness.

Synthetic mask-paste captures at 100%/200% were inspected through the production
paste worker/barrier path, with matching canvas/navigator/thumbnail where shown
and Pasted into mask feedback. They bypass OS clipboard access and do not verify
external clipboard formats or real application-to-application interchange.

## Mask target geometry checkpoint

Mask target geometry captures both layer and mask identities. Pending Move and
Transform intent waits for preceding commands before acquiring its frozen source.
The same coalescing worker handles raster and mask previews; canceled superseded
requests cannot publish. Mask geometry leaves artwork bytes and mask flags alone,
uses white for vacated/outside coverage, and transfers selection coverage with
the transform. Shared fractional selection coverage is replaced without pale
overlap seams. Interpolation normalizes selected values by sampled selection
coverage. Exact rectangular selections retain their compact metadata; selections
moved entirely off canvas remain explicitly empty. Integer offsets use checked
arithmetic, including extreme i64 values. Combined payload-limit failures stay
private. Mesh inverse rasterization and mask/selection row work check cancellation.
Equivalent coverage retains the original identity, and constant results retain
tile-free coverage storage.

Automated geometry coverage includes integer flips/moves, fractional overlap,
rectangular/empty selections, interpolation edges, translated perspective/mesh
coordinate oracles, replaced/locked/hidden targets, sparse constants, folder masks,
payload rejection, coalescing/reversion, one-step undo/redo, and application
Apply/Cancel and pending Move release with raster transparency locked.
`--capture-mask-target-transform` is a separate synthetic mask-target diagnostic;
the older `--capture-layer-mask-transform` still transforms linked artwork.
Native captures at 100% and 200% were inspected with this mask-target diagnostic:
coverage and handles agree with the selected target; the 100% navigator and mask
thumbnail match the canvas, and the compact 200% toolbar fits. The 100% navigator
also clips its lower controls at the layer divider, extending the known inspector
layout issue beyond 150%. These captures do not establish drag latency or physical
pen behavior. Release candidate SHA-256:
`442CF69C333A988D7C1498EAFA26BD72F18CB71B4ED2961F885300490EE5C8A3`.

## Geometry preview persistence

Move and Transform now capture modified/content state when their source is
acquired, after preceding commands settle. Live redraws invalidate displays and
source generations without advancing the committed content token. That baseline
stays live through final FIFO settlement. Apply advances content once if the
final result differs; Cancel and return-to-identity restore the appropriate saved
state without erasing guide edits or following queued commands. Save confirmation
cannot clear unsaved state while a geometry preview is live; a successful earlier
save becomes clean after rollback if there are no independent changes. Failed
saves do not clear that baseline or recovery data. Error exits retire the preview
baseline and preserve already-published changes. Document replacement resets it.

The save regression checks raster and mask targets, Cancel/identity/Apply,
successful/failed confirmations before and after settlement, and independent guide
edits using real temporary project writes. A Move regression checks direct Cancel,
queued cancellation and zero return with guide and following command revisions.
Full suite at this persistence checkpoint: 708 passed, zero failed, one existing
high-memory test ignored; `target/qa/geometry-state-{tests,targeted,check}.log`.

The navigator now budgets preview height using the visible inspector viewport,
without changing the stored preferred height or depending on scroll offset.
Short panels expose rotation, fit and flip through View options; the shortest
also move zoom there. A whole-canvas thumbnail remains visible. An egui workflow
checks full visibility of the normal controls and compact menu, clicks View
options and Flip canvas, and restores the expanded layout/preferred height.
Combined suite: 709 passed, zero failed, one existing high-memory test ignored;
warnings-denied all-target Clippy, formatting and whitespace pass. Logs:
`target/qa/geometry-navigator-{tests,clippy,release}.log`.
Optimized build passes. Native `geometry-navigator-{100,150,200}.png` captures
were inspected: the full control rows fit at 100%; 150% exposes zoom and View
options below a whole-canvas preview; 200% uses the automatic inspector-hidden
layout. This resolves the observed navigator clipping in those fixtures without
establishing physical pen or drag-latency behavior. Current release SHA-256:
`A25F79466C9368E7A71C856DA44A088607720A2E782324D6BA11D01EF72800A2`.

## Coverage feedback and selection references

Mask focus has separate session-local grayscale foreground/background values,
defaulting to black and white. Coverage popup/inspector controls offer conceal,
reveal and a percentage slider; mask edits and swaps preserve artwork RGB colors
and do not add grayscale entries to artwork recent colors. Brush, fill, lasso and
the brush test pad use the chosen coverage. Eraser always reveals. Picker/Alt
samples raw mask coverage, including disabled masks, with a guarded slot lookup.
The palette explains disabled-mask editing; grayscale feedback names the target.

The wand's Active reference captures the mask slot when mask-focused, including
pending creation. Visible and explicitly named layer references still read their
usual artwork sources. Raw coverage is treated as opaque grayscale for tolerance
and connectivity; locks, visibility and enabled state do not alter reference
data. Replaced slots fail rather than falling back. Legacy wand settings decode
with no captured mask and omit the optional field when serialized.

Select > From mask coverage uses raw 8-bit coverage even when disabled. From
artwork coverage combines raw alpha with its enabled mask, excluding layer opacity
and parent/folder styling. Both are selection-only operations, allow locked/hidden
sources, retain explicitly empty selections and have normal undo/worker guards.

Final coverage UI captures `target/qa/mask-coverage-final-{100,150,200}.png` were
inspected with an optimized build. Grayscale controls and target feedback fit at
each scale. Numeric size/opacity controls keep Stroke options fully visible at
200%; narrower toolbars move those controls into that menu. Rendered checks cover
840/768/640/480-point toolbars and an oversized parent layout. Native capture
traces record the actual viewport/pixel scale instead of assuming logical size
from requested window dimensions. Coverage-workflow checkpoint release SHA-256 (historical):
`4FCB8F7CF6A9A741BF551CD0CFF72A356CBF2CF08897AD29386FCF99C7EBF3CB`.

## Remaining release checks

The two-run CPU probe verifies actual mask preview/final-request publication and
Undo/Redo against independent byte oracles at supported sizes. Sparse moves,
integer translations and small selections are much faster; dense 2048-square
fractional transforms still take 193-200 ms. See
`MASK_TRANSFORM_PERFORMANCE.md` for scope, executable identity and all results.
Resident snapshots rose to 62.2 MiB versus 38.8 MiB in the baseline; no memory
reduction or leak conclusion follows. `MASK_LIFECYCLE_MEMORY.md` records a later
two-run released-ownership lifecycle check (2,304 verified operations), checkpoint
build identity and limitations. Short persistent native display/UI and history-
release checks are recorded in `NATIVE_MASK_SESSION.md`; they do not measure GPU
execution or physical latency. Measure sustained-session memory and profile the
initial private commitment, verify physical XP-Pen input and Windows clipboard
round trips, and complete remaining interop/package gates in the delivery plan.
Stroke recording currently
rejects version-2 documents explicitly until its mask replay format is supported.

## Engine delivered in this stage

`src/layer_mask.rs` stores independent 8-bit coverage in 64-pixel sparse tiles.
One shared metadata allocation owns a tile directory; byte payloads share between
snapshots. Missing tiles read the mask's default coverage. Constant white/black
masks allocate no coverage tiles. Edge tiles contain only their actual canvas
extent. Writes detach metadata and touched tiles; no-op writes retain identity.
Reclaim removes tiles restored to default. No whole-canvas compatibility cache is
created. Grayscale painting interpolates toward a target value; live painting now
supplies selection coverage and stroke-level opacity through the target adapter.

The engine includes selection-coverage construction, inversion, cancelable
destination-to-source geometry mapping, alpha baking with hidden RGB preserved,
weak region stamps and deduplicatable storage/tile allocation identities. A bounded
canonical `LMAS` subrecord carries dimensions, default coverage and ordered packed
tile records. Decode requires expected dimensions and a coverage-byte budget;
malformed dimensions, over-budget payloads, duplicate/unordered/out-of-range tile
indices, redundant tiles and truncation fail privately. This subrecord is now used
by container v5 in `.dudel` and recovery. Indexed containers reject mask-bearing
documents. The outer decoder enforces metadata declarations, checksums, combined
payload budgets and end-of-payload rules.

`--benchmark-layer-masks NEW_REPORT.json` runs engine verification and writes a
new report without replacing existing files. Measurements exclude application
input, document/controller ownership, undo, display, project storage and hardware.

## Required integration sequence

1. **Model and format together.** Add an optional mask record and enabled/linked
   state to each eligible layer; initialize legacy layers without a mask. Masked
   documents must have an explicit version gate that older readers reject rather
   than silently discarding coverage. Preserve byte compatibility for unmasked
   projects. Integrate regular save/load and recovery, including malformed-file
   and interrupted-write tests. Indexed saving must either support masks with a
   new manifest version/object namespace or reject before any transaction; it
   must never silently omit masks. Keep replay fixture budgets explicit.
2. **History and memory.** Include mask state in strict source comparisons,
   compaction eligibility and all history snapshot/worker allocation descriptors.
   Mask edits need bounded undo settlement; adding/enabling/removing a mask must
   undo independently of artwork painting. No-op/net-zero strokes retain redo.
3. **Compositing and invalidation.** Apply coverage to leaf and isolated-folder
   contributions with defined opacity/rounding order, including clipping bases.
   Update `canvas_textures`, `prefix_cache`, navigator and thumbnail source
   signatures. A mask-only edit must invalidate every affected cached result;
   unrelated mask tiles should retain cache reuse. Visible-reference fill/wand,
   flattening, PNG output and saved thumbnails must agree with the canvas.
4. **Geometry and destructive operations.** `canvas_size`, quarter-turn rotation,
   move, selected-layer flips and affine preview must preserve mask coordinates.
   Linked whole-layer moves move artwork and masks together; selections and an
   unlinked mask require explicit target semantics. Duplicate keeps equivalent
   coverage with independent identities. Merge/flatten/apply-mask bake the correct
   contribution exactly once. Clipboard visible-copy/cut and selection-from-alpha
   need explicit behavior for disabled masks and fractional coverage.
5. **Painting and tools.** Capture raster-versus-mask target in queued stroke
   settings and pending-layer projection. Route pressure, grain, symmetry,
   stabilization, stroke opacity, taper/finalized-prefix replay, eraser and undo
   to mask coverage without overwriting RGB or layer alpha. Honor selection and
   locks. Fill/gradient/shape and unsupported raster filters need visible,
   consistent target behavior. Black conceals and white reveals; changing layers,
   deleting/duplicating masks and stale workers cannot redirect a captured stroke.
6. **Visible workflow.** Add mask-from-selection/reveal-all, separate mask
   thumbnail/target focus, enabled/linked state, invert, apply and remove commands.
   Clearly show when drawing edits a mask. Keep keyboard focus, context actions,
   active target and canvas feedback coherent. Verify a real workflow from creating
   a mask through paint/undo/toggle/duplicate/transform/save/reopen/export at compact
   sizes and supported scales, followed by physical pen input.

## Evidence required before exposing creation commands

Tests must compare independent per-pixel mask/compositing fixtures, exact alpha
rounding, disabled-mask identity, clipping/folder combinations, all geometry paths,
source ownership/cancellation, undo/redo and file reopening. Malformed/new-version
files must fail without replacing the current canvas or destination. Test sparse
and dense supported documents; report UI/owner time separately from worker time
and actual memory separately from tracked allocation estimates. Native screenshots
alone do not verify tablet input or long-session stability. Keep mask UI unavailable
until its displayed result, history and persistence are coherent.


## Follow-up sampled-mask performance and guidance

`MASK_SAMPLER_OPTIMIZATION.md` records the newer candidate: 727 tests pass,
interior bilinear footprints share tile lookups, and large full-mask sampling uses
bounded children of the edit worker with joined cancellation/failure handling.
The two-run 2048-square dense fractional case improves from about 203 ms to
36-37 ms. Target-aware mask tool guidance is visible at 100/200%; both native
workflows pass again. Earlier measurements and frozen binaries above remain
checkpoint evidence. Full-rate interaction, sustained native memory and physical/
external interoperability gates are not closed by that improvement.
