# Ordered rendering integration

The resumable tail cursor is implemented in `src/tail_replay.rs`, as a child of
`stroke_buffer.rs`. `document::DabRaster` caches geometry/delivery once and the
shared `paint_dab_rows` accepts row slices. Tail slices preserve the original dab
order and coverage arithmetic. `TailStep.changed_region` reports actual writeback
damage separately from conservative raster bounds. Canvas finalization still
drains the cursor synchronously; this is an engine checkpoint, not a completed
UI responsiveness gate.

The queue core is implemented in `src/render_queue.rs`: captured-settings strokes,
live/lift cursors, reserved end markers, bounded input, undo/redo, completed snapshots
and acknowledged document fences. Engine tests verify overlapping strokes, partial
cancel, streaming pauses and retention of earlier completed taps on queue overflow.
The canvas controller is still synchronous. Checkpoint/settlement, restore/reclaim,
undo/redo and snapshots remain synchronous queue control units. The release queue
fixture includes their CPU cost, but does not establish larger-document or UI bounds.

`advance_until` now checks a deadline between units. Version-2 release fixtures
also cover dense 2048-square single-layer documents: maximum observed calls reach
7.151 ms with a 2 ms deadline. Individual rows and synchronous controls still
overrun it; profile and slice these costs before claiming a bounded UI update.
Native overflow/focus-loss retention of previously completed taps is fixed and
tested in the input layer as well as the renderer. The current synchronous save
path settles end taper before taking a snapshot; migrate that invariant to ordered
snapshot tickets when connecting the controller.

Version-3 phase profiling now distinguishes flat compatibility and tiled dense
sources. It identifies whole-layer flat first-write copies and synchronous
history settlement as separate costs. Precise flat/mixed tile diffs reduce broad
undo patches and avoid materializing tiled flat caches, but final profiled calls
still reach 6.025 ms on dense flat and 4.087 ms on dense tiled soft fixtures.
Make settlement/rollback/reclamation cooperative without exposing incomplete
history or permitting the following queued stroke to mutate its source.

Queued rollback now owns a resumable original-pixel cursor: one 32-pixel row per
raster unit, explicit reclamation units and incremental scratch release. The
checkpoint/suspended redo remain retained until restoration completes; snapshots
and following strokes cannot overtake it. Version-4 rollback fixtures verify full
original pixels and observe maximum rollback calls of 0.418 ms across the supported
single-layer cases. Final accounting, tile scans and interrupted-plan disposal
remain synchronous unit costs, so this does not prove a universal frame bound.
Canvas cancellation remains synchronous until controller integration. History
settlement is the next cooperative control operation to implement.

History patch construction now uses `History::begin_settle` and a row-sliced
`Settlement` cursor. It retains the complete checkpoint/suspended redo until atomic
patch publication, checks source/document identity, and serializes following queue
commands. Version-5 headless fixtures verify full pixels and observe maximum queue
calls of 2.690 ms (settlement units 1.852 ms); larger/general UI bounds remain open.
Prepared patch bytes temporarily coexist with the checkpoint. Include their live
payload in controller budget accounting. Final trimming and scratch/snapshot release,
reclamation/change checks, other controls and flat first-write COW remain synchronous.
Use a generous UI unit cap plus the deadline, rather than waiting a frame after each
32-unit benchmark chunk. The next step is canvas/document/save controller integration.

The synchronous application mutation paths now settle painting before transform/
clear snapshots, selection/active-layer changes and filter source capture. Whole
layer clear retains selection in its undo source. Rename drafts, folder state and
replacement/save revision handling have application-level regressions. These
semantics must be preserved in ordered dispatch; they do not themselves connect
the canvas to the queue. Opacity now uses a discardable transaction: unchanged or
return-to-original gestures preserve redo, changed previews commit once, and
subsequent commands finalize the gesture first. Save/recovery and filter regressions
cover that boundary. Idle maintenance accepts finished jobs without evicting
suspended history. Preserve these contracts in the queue controller.

The controller implementation must cover these ordering constraints:

1. Own each pending stroke's buffer/settings in the renderer until it settles or
   cancels. Following strokes enqueue behind it; a stale result must not overwrite
   their pixels. Capture brush/color/eraser/symmetry and Shift anchors at accepted
   input, not when the queue eventually runs.
2. Map accepted points through the view at input delivery and retain their zoom
   for screen dynamics/stabilization. Subsequent pan/rotation/DPI changes must not
   move already queued marks. Replay fixtures provide the exact raster oracle.
3. Queue document changes, undo/redo, save/replacement and filters in order with
   painting. A pending tail cannot simply join synchronously on the next command;
   that would move the stall. Save snapshots must represent the completed commands
   preceding that save. Existing save completion must still respect revision/dirty
   changes from final writeback. Polling asynchronous jobs must not replace the
   document beneath a rendering operation.
4. Bound queued sample/settings payloads and retain explicit end/cancel commands.
   Overflow cancels the affected logical stroke without losing previously completed
   queued taps or their history. Focus/capture loss and Escape restore the correct
   stroke's original pixels, suspended redo and previous Shift-line anchor.
5. Spend a small time budget per UI update, checking the deadline between bounded
   units, and request repaint while work remains. Upload only writeback damage.
   Preserve input handling and view navigation while the renderer catches up.
6. Use the new `StrokeBuffer::begin_dab` cursor to slice live large dabs, prefix
   retirement and arc-reset seals too, with a shared settings Arc per stroke.
   Normal canvas painting now submits dabs to the queue, which owns and drains
   cursors. Replay capture still uses synchronous `StrokeBuffer::paint` until
   source/result capture can follow queued stroke lifetimes. Original-state restoration and history
   settlement/reclamation also need measured scheduling. Sampling/generation of
   long paths must fit the UI time budget or move into the ordered renderer.

Verification must include consecutive overlapping strokes while an earlier tail
is pending, brush/view changes between queued packets, cancel during partial
writeback, undo/redo and save/recovery ordering, overflow/focus loss, and no lost
single-frame taps. Compare final RGBA against the existing synchronous/replay
oracle. Measure actual UI update time and completion latency; CPU chunk timing
alone cannot establish tablet or display responsiveness.
