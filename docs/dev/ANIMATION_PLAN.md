# Animation timelines and frames

Status: deferred beyond 0.2.0. This proposal is retained for future planning;
it is not part of the current release scope in the [roadmap](ROADMAP.md).
Editable animation is separate from timelapse recording.

## Scope

Build a frame-by-frame raster animation MVP with a timeline dock, exposure durations, playback, onion skinning and animation export. Ordinary illustrations remain single-frame projects. Timelapse remains a recording of the drawing process, not the editable animation model.

Start with 12 fps, integer exposure durations, loop playback, and a shared canvas and layer stack. Support 1–60 fps in the model with explicit export format limits. Defer audio, tweening, camera tracks and animated vector properties. Initially non-raster layers remain static across frames; confirm this policy during the prototype.

## 1. Model and persistence

- Add an optional animation model to `Document` with stable frame, track and cel IDs. Map layer tracks to cels and frame slots to cel references plus duration. Distinguish blank cels from held exposures. Duplicate creates independent editable content using existing copy-on-write pixel storage; explicit linked cels can follow later.
- Keep playback cursor, zoom and onion-skin preferences in per-tab UI state. Persist the selected editing frame as workspace metadata, outside artwork history.
- Extend validation, change detection, snapshot registration and memory accounting to all reachable cels, deduplicating shared allocations. Set explicit frame, duration and decoded-byte limits.
- Version project/tile storage and recovery deliberately. Old projects load as one frame; older app versions must reject animated files rather than silently discard frames. Audit autosave, recovery, thumbnails and project packages before enabling animation saving.
- Gate: round-trip animated projects, interrupted-save recovery, shared-tile accounting, malformed-ID/oversized-input rejection and unchanged old-project behavior.

## 2. Ordered editing and undo

- Add frame/cel targets to render tickets and document edits. Capture the target when a stroke starts; switching frames finishes the gesture and never redirects queued dabs.
- Add insert blank, duplicate, delete, reorder and exposure commands to undo. Forbid deleting the last frame; restore a valid editing selection after undo/reorder/deletion.
- For the MVP, masks and effects are shared layer properties while raster cel pixels vary over time. Clearly show static non-raster layer behavior.
- Gate: pending edits during frame/tab switches, undo after deletion, cancellation and concurrent exports all affect the intended frame and preserve save state.

## 3. Timeline and playback

- Add a resizable dock with virtualized frame cells, layer rows, playhead, frame numbers and exposure controls. Deliver explicit add/duplicate/delete controls before drag reorder.
- Cache thumbnails by cel revision and composite settings. Generate only visible thumbnails in bounded background jobs and invalidate changed entries.
- Use elapsed-time playback with exposure accumulation and loop ranges. Skip display frames when necessary without changing authored timing. Pause for edits and restore the editing frame after preview.
- Gate: variable exposures at different fps, long-timeline responsiveness, keyboard navigation, stable selection and per-tab playback isolation.

## 4. Onion skin

- Composite previous/next exposures with adjustable count, opacity and tint; optionally omit repeated held cels.
- Keep overlays outside document composites, recording, thumbnails and exports. Cache by cel revision and display settings under the existing memory budget.
- Gate: correct alpha/tint, no leakage into export, invalidation on undo and bounded allocation on large canvases.

## 5. Animation exports

- Extract a reusable frame-source interface from timelapse: dimensions, count, rational timing, alpha policy and cancellable frame decoding. Implement recorded-PNG and immutable animation-snapshot sources.
- Deliver PNG sequences, then GIF and MP4 through background jobs and atomic publication. Preserve variable exposures and document GIF timing/palette limits. Add encoder availability detection and an optional executable picker for FFmpeg.
- Evaluate animated WebP/APNG separately; still-image WebP support does not imply animation support.
- Export one captured timeline revision while editing continues. Stage sequences in a new folder and publish after success, never mixing partial output into an existing sequence.
- Gate: decoded frame order/duration/alpha, cancellation during decoding and encoding, missing encoder, disk-full handling, destination protection and unchanged project state.

## Delivery order

Ship each numbered milestone as a reviewable change, in dependency order. Do not release editable animation until persistence and recovery cover every cel. Before estimating dates, measure a 100-frame 1080p project, a large sparse canvas and repeated held exposures. Establish numerical memory and latency budgets from those baselines and make them release gates.
