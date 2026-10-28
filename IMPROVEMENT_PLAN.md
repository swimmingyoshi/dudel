> Historical assessment from 2026-10-11. Feature gaps and code/test counts below describe that prototype. Follow [DELIVERY_PLAN.md](DELIVERY_PLAN.md) for current priorities and gates, and [the current assessment](research/PROJECT_ASSESSMENT_2026-10-17.md) for the reviewed implementation.

# Dudel: improvement and feature plan (2026-10-11)

This plan follows a review of the code (≈11.5k lines of Rust across 43 modules) and
of the internal product plan, which recorded product goals, release gates and
progress at the time; this document decides **what to build next and in
what order**. Goal (unchanged): a production-ready Windows illustration app with the
everyday features artists expect from Clip Studio Paint, ibis Paint and Procreate,
comfortable on the user's XP-Pen Artist 12.

Verified on 2026-10-11: `cargo test` gives 97 passed, 1 ignored (the maximum-size
stress test).

---

## 1. What we have

| Area | Implemented | Notable limits |
|---|---|---|
| Canvas & view | Zoom 5–3200% anchored at the cursor, pan, view rotation with snapping, flip canvas, navigator, fit/100%, nearest-neighbour above 200% and mipmaps | One texture for the whole canvas; canvas limited to 4096 × 4096 |
| Input | Pen pressure through egui touch events, duplicate mouse events suppressed, ordered packets, cancel when focus is lost | No timestamps, tilt, rotation, eraser end or barrel buttons; no hover pressure |
| Strokes | Spacing carried between samples, pressure interpolation, five-point pressure curve, distance stabilizer, scatter, symmetry (mirror/4-way/radial) | Straight segments between samples; stabilizer has one mode; no taper; no stroke-level opacity |
| Brush tips | Round, square, image (PNG, up to 128²), aspect, angle, paper grain, tags, search, `.dudelbrushes` packs, previews, test pad | Hard edges alias; small sizes break into dots; no smudge/blur/mixing; no dynamics from tilt or speed |
| Layers | Up to 32 raster layers, folders (depth 6), 12 separable blend modes, clipping, locks, alpha lock, merge, duplicate, thumbnails, drag reorder | Caps of 32 layers and 64 entries; no layer masks; no adjustment layers; renaming is not undoable |
| Tools | Brush, eraser, fill (tolerance, active layer only), picker, rectangle selection, move | No lasso/wand, transform, gradient, shapes, text or rulers |
| History | Copy-on-write layers, changed-region patches, 100 steps / 512 MiB | Pen-down copies the whole layer; pen-up scans the whole layer |
| Files | v3 binary gzip container with atomic writes, background save/load, recovery snapshots, PNG import/export (opaque and transparent), palettes | No JPEG/WebP/PSD; no clipboard; no canvas resize or crop; single document |
| Extensions | Rhai and sandboxed WASM filters, manager, SDK, background jobs with cancel and one undo step | Filters only; no built-in adjustment filters |
| Workspace | CSP-style layout, tool rail, tool property panel, color wheel with recents, palette, references window, shortcut list, capture modes for screenshots | Shortcuts hard-coded; no command palette; no custom shortcuts or pen-button mapping |

## 2. Findings from the code review

These are specific, verifiable problems. Each is solved by a work package in §4.

### 2.1 Pixel storage scales with the whole canvas, not with the changes

- Each layer is one flat `Arc<Vec<u8>>` ([pixels.rs](src/pixels.rs)). The first dab of
  every stroke triggers `Arc::make_mut`, which **copies the whole layer** (16 MB at
  2048², 64 MB at 4096²): this causes a hitch at pen-down.
- At pen-up, `changed_rect` ([document.rs:712](src/document.rs#L712)) **compares the
  whole layer** against the snapshot, which causes a second hitch. A diagonal stroke
  stores its full bounding box in history.
- Empty layers cost full memory: 32 layers at 4096² use about 2 GiB. This is why the
  limits are 32 layers and a 4096 canvas ([document.rs:30](src/document.rs#L30),
  [document.rs:190](src/document.rs#L190)). Pro work (A4 at 600 dpi is 4961 × 7016,
  and CSP files often have 100+ layers) needs both limits raised.
- A single GPU texture for the canvas ([canvas.rs:619](src/canvas.rs#L619)) also
  caps the size, at the GPU's maximum texture side (often 8192 or 16384).

**Conclusion:** tiled, sparse layer storage is the foundation. Selection masks,
transform, smudge, layer masks and PSD all touch pixel storage. If they are built
before tiles, they will have to be written twice.

### 2.2 Brush rendering quality

- **Aliasing:** coverage is 1.0 inside `hardness × radius`
  ([document.rs:613](src/document.rs#L613)), so the soft edge is a fraction of the
  radius. Small hard brushes, such as the built-in chisel at hardness 1.0, get stair-
  stepped edges, and large ones blur. A fixed ~1 px edge falloff, independent of
  brush size, is needed.
- **Thin-line dropouts:** a pixel is skipped unless its centre lies inside the tip
  ([document.rs:599](src/document.rs#L599)), so sub-pixel and 1–2 px brushes break up.
  These dabs need area-weighted coverage.
- **No stroke opacity ceiling:** each dab blends straight onto the layer, so a 50%
  stroke that crosses itself goes darker, and opacity and flow cannot be set
  separately (needed for markers and glazing).
- **8-bit accumulation:** each dab rounds to u8, so the faint tails of soft, low-flow
  brushes never build up. This causes banding.
- **Path shape:** samples are joined by straight segments
  ([stroke.rs:111](src/stroke.rs#L111)), so fast curves become polygons. There is no
  start/end taper, which inkers use constantly.

### 2.3 Compositing

- Each displayed pixel walks every layer in f32 ([document.rs:281](src/document.rs#L281)).
  A full composite takes ~33 ms at 2048² on all cores. That is acceptable at 2048,
  but undo, layer toggles and opacity drags at 4096+ with many layers will lag.
- Composites of the layers below and above the active layer are not cached, so every
  dirty region recomposites all layers.

### 2.4 Input

- egui/winit gives position and force only. Tilt, rotation, the eraser end, barrel
  buttons and event timestamps need a Windows pointer backend (`WM_POINTER*`, with
  WinTab as a fallback). Without them, velocity dynamics, tilt shading and "flip pen
  to erase" are impossible.

### 2.5 Application structure

- `Dudel` ([main.rs:51](src/main.rs#L51)) is one struct with about 70 fields, mixing
  document, view, tool, UI and job state. This blocks multiple documents and makes
  each feature touch everything.
- Shortcuts are matched by hand in [canvas.rs:137](src/canvas.rs#L137), separately
  from the menus. There is no action registry, so shortcuts cannot be customised and
  there can be no command palette or pen-button mapping.
- Tools are enum branches inside `canvas_ui`. Adding lasso, transform and gradient
  this way will make that function unmanageable.
- `Selection` is a rectangle stored in `Document` and in the file format
  ([selection.rs](src/selection.rs)), so mask selections need a schema migration.

### 2.6 Housekeeping

- **The project is not under version control.** With refactors this size, this is the
  biggest risk to the codebase. Run `git init` and make a first commit before Phase 1.
- README still says "1024 × 768 raster canvas"; ROADMAP.md is superseded. Both are
  stale.

## 3. Principles for ordering

1. **Artwork safety first:** no change may weaken atomic saves, recovery or undo.
2. **Foundations before features:** build tiles, the stroke buffer and the command
   registry before the features that depend on them.
3. **Everyday features over breadth:** look at what an artist touches every minute
   (brush feel, undo, selection, transform, fill, colour) before rarer features.
4. **Measure, don't assume:** each performance package extends `--benchmark` with
   before/after numbers in `artifacts/`. Each UI package gets `--capture-ui` checks at
   100% and at 1920 × 1080 @ 150%, plus a hands-on XP-Pen check.
5. **Keep files readable:** every format change adds a version, and older projects
   keep opening.

## 4. Phased plan

Effort: **S** ≈ one working session, **M** ≈ 2–4, **L** ≈ 5+. No dates, per the
product plan's policy.

### Phase 0 — Baseline and safety (S)

- `git init`, `.gitignore` for `target/` and generated artifacts, first commit.
- Developer overlay (View > Performance): frame time, dabs per frame, composite and
  upload milliseconds, history memory.
- Extend `--benchmark` with: pen-down cost, pen-up/history cost, a 4096² canvas with
  20 layers, undo latency and resident memory. Record the baseline.
- Hands-on XP-Pen session covering the untested items in the product plan (navigator drag,
  symmetry centre, folder drag, panel divider, fast curves). Log the results.
- Bring README up to date; mark ROADMAP.md as historical.

**Gate:** the repository is committed, the baseline numbers are recorded, and the
hardware findings are logged.

### Phase 1 — Brush feel (M) — the most visible gains

1. **Analytic anti-aliasing:** coverage = smoothstep over about 1 px at the tip edge,
   independent of size, with hardness controlling an *additional* inner falloff.
   Area-weighted coverage below about 2 px diameter, so thin lines stay continuous.
2. **Stroke buffer:** dabs accumulate into a per-stroke coverage buffer (u16 or f32,
   using max or additive mode depending on the brush). The buffer is composited onto
   the layer at display time and committed on pen-up. This gives:
   - separate **opacity** (stroke ceiling) and **flow** (per-dab) settings;
   - no darkening where a stroke overlaps itself, and smooth low-flow build-up;
   - a natural place for a live eraser preview, and for cancelling a stroke cleanly.
3. **Taper:** start/end taper length for size and opacity, using pressure taper when
   pressure is available. End taper is applied on pen-up using the buffered tail.
4. **Curve smoothing:** a centripetal Catmull-Rom curve between input samples, so
   fast curves stay round.
5. **Stabilizer modes:** keep "string pull" and add time-weighted smoothing
   (Procreate StreamLine / CSP "stabilization") with catch-up on lift.
6. **Speed dynamics:** size and opacity respond to stroke speed. Timestamps are
   approximated from frame time until Phase 3.

**Gate:** replay-fixture tests for AA edges, continuity of a 1 px line, the opacity
ceiling across overlaps, and taper symmetry. The new benchmark shows no regression.
The user confirms the inking feel on the XP-Pen. Existing presets keep their look,
with the new defaults applied as a one-time migration.

### Phase 2 — Tiled engine and rendering (L) — the foundation

1. **Sparse tiles:** layers become maps of 256² RGBA tiles, each `Arc`-shared and
   `None` when empty. Copy-on-write happens per tile, so pen-down copies only the
   tiles it touches.
2. **Dirty-tile tracking** replaces `changed_rect`. A history entry stores only the
   tiles that changed. Pen-up cost becomes proportional to the stroke, not the canvas.
3. **File format v4:** tile records, with empty tiles omitted. v1–v3 stay readable.
   Recovery uses the same records.
4. **Tiled display textures:** the canvas is drawn as a grid of textures, so canvas
   size no longer depends on the GPU's texture limit. Only dirty tiles are uploaded.
5. **Composite cache:** cache the composite below the active layer, so painting
   recomposites only the active layer and the layers above it. Then evaluate **GPU
   compositing** (custom wgpu callback with shaders for the 12 blend modes, clipping
   and folders) against the cache, using the benchmark. Choose by measurement.
6. **Raise limits:** canvas up to 10000², 256 raster layers (subject to memory),
   with a visible memory estimate in New Canvas.
7. Thumbnails and the navigator read from tiles, with a level-of-detail pyramid
   instead of full subsampling.

**Gate:** at 4096² with 20 layers, pen-down and pen-up each take under 2 ms, undo
takes under 50 ms, and empty layers cost almost nothing. A 60 FPS stroke on the
user's hardware is measured. Every existing compositing, history and storage test
passes against tiles, and a v3 project opens with identical pixels.

### Phase 3 — Pen input backend (M)

- A Windows `WM_POINTER` handler, attached to the eframe window through its raw
  handle, provides: timestamps, pressure, tilt X/Y, twist, the eraser end, barrel
  buttons and hover position. Keep the egui path as a fallback. Add WinTab only if
  the XP-Pen driver needs it.
- **Eraser end** switches to the eraser tool while it is in use.
- **Barrel buttons** are configurable: picker, pan, zoom, undo, a command, or right
  click.
- Pressure calibration in Preferences > Input: a global curve, a minimum-pressure
  threshold, and a live readout including tilt.
- Input-to-display latency measured by an on-screen marker test, and recorded.

**Gate:** the user verifies on the XP-Pen that the eraser end, both barrel buttons and
tilt work. Fallback works when the backend is unavailable. Phase-1 speed dynamics
switch to real timestamps.

### Phase 4 — Command architecture, shortcuts and documents (M)

- **Action registry:** every command has an ID, label, icon, default shortcut and
  enabled state. Menus, the tool rail, shortcuts, pen buttons and plugins all call
  it through the registry.
- **Shortcut editor**, with conflict detection and import/export. Optional CSP and
  Photoshop presets.
- **Command palette** (Ctrl+K) and a customizable **quick-access bar** (CSP Quick
  Access, ibis-style) for tablet use without a keyboard.
- **Tool trait:** each tool owns its input handling, overlay drawing and options UI.
  Move the existing six tools to it.
- Split `Dudel` into `App` (preferences, library, extensions) and `DocumentSession`
  (document, history, view, jobs, path). Add **document tabs**.
- Make layer rename and other metadata changes undoable.

**Gate:** all existing shortcuts still work through the registry, and a remapped
shortcut persists across restarts. Two documents can be open with independent
history and save state. `canvas_ui` no longer branches per tool.

### Phase 5 — Selection and transform (L)

- **Mask selections:** a tiled 8-bit mask replaces the rectangle, with a schema
  migration that turns old rectangles into masks. Painting, fill, filters and move
  use mask coverage, so edges are soft.
- Tools: rectangle, ellipse, **lasso**, polygon lasso and **magic wand** (on the
  layer or all layers). Each has new/add/subtract/intersect modes (Shift and Alt).
- Commands: invert, select layer opacity (Ctrl+click the thumbnail), grow/shrink,
  feather, and border.
- Marching-ants overlay, plus an optional tinted mask preview.
- **Free transform** of a selection or layer: scale, rotate and flip, then distort,
  perspective and **mesh warp**. Interpolation choices: nearest, bilinear, bicubic.
  Live preview, Enter to commit, Esc to cancel, one undo step.
- **Copy, cut, paste and paste-in-place**, including the system clipboard
  (Windows CF_DIB / PNG), so images move to and from other apps.
- **Lasso fill** and **lasso eraser** (CSP and Procreate inking staples).

**Gate:** a sketch can be selected, transformed, pasted between documents and pasted
from a browser; every step undoes once and respects locks; replay tests cover the
selection operations.

### Phase 6 — Line art and colouring (M–L)

- **Fill:** reference "all layers" or a chosen reference layer (CSP), gap closing,
  expand/contract by N px so fill tucks under anti-aliased lines, and an antialias
  option. Fill runs in the background, with progress, on large canvases.
- **Layer masks**: paint to hide or reveal, link/unlink, apply, and a thumbnail on
  the layer row. Ctrl+click loads the mask as a selection.
- **Adjustments**, both as destructive filters with live preview and later as
  adjustment layers: hue/saturation/lightness, brightness/contrast, levels, curves,
  colour balance, Gaussian blur, sharpen, add noise, and posterize/threshold.
- **Gradient tool:** linear and radial, foreground to transparent, saved gradients.
- **Shape tools:** line, rectangle, ellipse and polyline, stroked with the current
  brush. **QuickShape:** hold the pen at the end of a stroke to snap it to a line or
  ellipse.
- **Rulers:** straight line, ellipse and concentric guides, then 1–3 point
  perspective rulers (a CSP signature feature).
- Colour panel: HSL and RGB sliders, colour harmony, and a colour-history strip in
  the quick-access bar.
- Blend modes: add the non-separable Hue, Saturation, Color and Luminosity modes,
  plus Add (linear dodge) and Linear burn.

**Gate:** the full sketch → line art → flat fill (references the line layer, no gaps,
no halo) → masked shading → adjustments → export workflow completes on the tablet
without workarounds.

### Phase 7a — Brush creation and live preview (M) — planned 2026-10-15

Goal: making and tuning a brush feels like CSP's sub-tool detail and Procreate's
Brush Studio, with every change visible immediately.

- **Live preview at the top of brush settings:** a pressure-ramp stroke and a
  short S-curve redrawn by the real engine whenever a setting changes, on a
  worker with the newest settings winning (no UI stall). Beside it, a **tip
  preview** of one dab (shape, hardness, aspect, angle, image tip, grain) and a
  short dab chain showing spacing. The existing test pad stays for free drawing.
- **One property registry for brush settings**: each setting declared
  once with id, range, default, unit and whether it accepts dynamics. Panel rows,
  presets, reset-to-default, "changed from preset" markers and a future dual
  brush all come from it, replacing hand-written rows.
- **Effector model for dynamics**: every eligible setting is
  base x curve(input) for pressure, tilt, speed, direction and random, edited in
  one curve widget, replacing the separate pressure flags.
- **Brush Studio window** for deeper editing: Shape (round/square/image, import,
  make tip from selection), Grain, Stroke (spacing, taper, jitter), Dynamics,
  Colour dynamics (HSV jitter), Stabilizer, Rendering (flow by spacing, later
  wet mixing), with undo inside the editor and Save / Save as / Revert.
- **Preset management:** duplicate, groups, favourites, recently used, per-row
  reset, and a versioned brush file format that round-trips every setting.

**Gate:** each setting change updates the preview within one frame on the UI
thread and finishes the redraw in under 30ms for 18-300px tips; presets
round-trip exactly; replay benchmarks per built-in preset stay within budget.

### Phase 7 — Painting brushes and the brush ecosystem (M–L)

- **Smudge/blend** brush (samples under the tip with carried colour), **blur**
  brush, and **colour mixing** (wet paint with dilution).
- Dynamics: tilt → size, opacity and angle; direction-following angle; hue,
  saturation and value jitter per dab and per stroke; random seed per stroke.
- **Dual brush** (a second tip masks the first) and textured eraser.
- Larger tips (up to 512²) with mipmapped tip sampling for small sizes, and grain
  with bilinear sampling and an optional per-stroke offset.
- **Import:** Procreate `.brushset` / `.brush` (shape and grain PNGs, mapped
  parameters) and Photoshop `.abr` sampled tips. CSP `.sut` is a stretch goal.
- Brush library: groups/sets, drag reorder, favourites, and a recently-used list.

**Gate:** distinct ink, pencil, watercolour-style and blending presets ship built in.
A Procreate brushset imports with recognizable behaviour. Replay benchmarks per
preset stay within the frame budget.

### Phase 8 — Files and interop (M)

- JPEG and WebP import/export with quality settings; export at a chosen scale.
- **PSD export** with layers, folders, opacity, blend modes and clipping, and **PSD
  import** for layered work coming from other apps.
- Canvas size, image size (resampling), crop to selection, and canvas flip
  (pixels).
- **Timelapse recording:** record strokes or tile snapshots, then export MP4 or GIF
  (ibis and Procreate users expect this).
- Thumbnails embedded in `.dudel` files, plus a start screen showing them.
- Persistent references stored in the project, with picking colour from a reference.

**Gate:** a layered PSD round-trips through Krita, CSP and Photoshop with layers,
blend modes and clipping intact. Large exports run in the background.

### Phase 9 — Release quality (M)

- Installer (MSIX or Inno Setup), file association for `.dudel`, an update check,
  and a crash handler that writes a report alongside the recovery snapshot.
- Colour management: sRGB working space by default, display profile, and embedded
  ICC on export. A soft-proof preview comes later.
- Accessibility pass (AccessKit labels, keyboard focus), localization scaffolding,
  and UI scale settings in the app.
- A long-session soak test (hours of replayed input with periodic saves); a
  compatibility matrix (tablets, drivers, GPUs); a published performance table.

**Gate:** the product plan's Milestone G gate: no known artwork-loss defects, critical
workflows complete, and user validation on the XP-Pen.

## 5. Optimization backlog (measured, not guessed)

| # | Optimization | Where | Expected effect | Phase |
|---|---|---|---|---|
| O1 | Per-tile copy-on-write | pixels, document | Pen-down copies the touched tiles only, not 16–64 MB | 2 |
| O2 | Dirty-tile history instead of whole-layer diff | document History | Pen-up cost proportional to the stroke | 2 |
| O3 | Sparse empty tiles | pixels, storage | Empty layer ≈ 0 bytes; smaller files and recovery snapshots | 2 |
| O4 | Composite cache below the active layer | document, canvas | Paint cost depends on layers above the active one, not all | 2 |
| O5 | Tiled texture upload | canvas | Removes the texture-size cap; uploads are per tile | 2 |
| O6 | GPU compositing (if O4 is not enough) | new render module | Layer toggles and opacity drags in real time at 4096²+ | 2 |
| O7 | Dab row loops: hoist `selection`/grain checks, compute coverage spans per row, SIMD-friendly layout | document `dab_mapped` | Large soft brushes (currently the slowest preset) | 1–2 |
| O8 | Integer/fixed-point premultiplied layer cache for display | document | Removes per-pixel division in compositing | 2 |
| O9 | Reuse worker threads (rayon or a pool) instead of `thread::scope` per composite | document | Lower overhead for medium regions | 2 |
| O10 | Background fill and wand on large canvases | document, jobs | No UI freeze on 4096+ fills | 6 |
| O11 | Thumbnail/navigator level-of-detail pyramid updated from dirty tiles | thumbnails, navigator | Stops repeated full subsampling | 2 |

Each item is done when the benchmark JSON in `artifacts/` shows before/after numbers
for the same workload.

## 6. Deliberately deferred

Animation (onion skin, timeline), vector layers, comic panels and text-on-path, 3D
models and posing, cloud sync and collaboration, and macOS/Linux builds. These are
revisited after Phase 9. Text tool: a basic horizontal text layer can join Phase 8 if
the user needs it for their work.

## 7. Immediate next steps

1. Phase 0: `git init` and first commit, performance overlay, extended benchmark,
   hands-on XP-Pen log.
2. Phase 1, items 1–2 (anti-aliasing and the stroke buffer). They are self-contained
   in the dab path and the user will feel them immediately.
3. Write the tile-storage design (tile size, the `Pixels` API to replace, the history
   entry format, the v4 file layout), then start Phase 2.
