# Brush graph

Status: proposal for review, 2026-11-10. Nothing here is implemented yet. It
answers the "first deliverable" of the brush graph brief: how a node-based brush
editor fits the brush engine Dudel already has, and what to build first.

The short version: Dudel already has a small typed curve system for brush
dynamics. The brush graph should grow out of it, not sit beside it. A graph is
compiled once into a flat list of operations, evaluated once per dab where the
existing dynamics are evaluated today, and its outputs feed the same dab fields
that size, opacity, flow, spacing and rotation already use. Brushes without a
graph keep painting exactly as they do now.

## 1. The brush engine today

A stroke travels through four stages. Each has one owner:

| Stage | Where | What it decides |
|---|---|---|
| Pen input | `input.rs`, `native_pen.rs`, `wayland_pen.rs` | `stroke::Sample`: position, pressure, tilt `[x, y]` in degrees, twist, barrel button, eraser end, native timestamps |
| Dab placement | `stroke.rs` (`Stroke::sample`, `finish`, `catch_up`) | Stabilizer and rope, spacing, taper, per-dab random value, rotation (fixed, direction or tilt, plus jitter), scatter offset, arc length |
| Per-dab values | `brush.rs` (`size_factor`, `opacity_factor`), `document.rs` (`DabRaster::new`) | Radius, opacity × flow, interval-compensated flow, angle, pixel bounds |
| Pixels | `document.rs` (`paint_dab_rows`), `stroke_buffer.rs`, `dab_paint.rs`, `dab_bands.rs`, `brush_edge.rs` | Tip coverage (round, square, image tip, softness profile), grain, accumulation into the stroke buffer, opacity ceiling, masks, selections and locks |

Strokes are painted on a render worker (`render_queue.rs`), which receives the
whole `Brush` in `stroke_buffer::Settings` when the stroke begins. Undo, masks,
symmetry and cancellation all happen around the stroke buffer, so anything that
only changes per-dab values inherits them for free.

### What a brush is

`brush::Brush` is one flat, serde-serialized struct: size, opacity, flow,
hardness, spacing, pressure curve, stabilizer, softness profile, taper in/out,
tip shape and image, aspect and angle, grain image, amount and scale, scatter
and its seed, angle source and jitter, tags and tool group. `Brush::validate`
bounds every field.

`dynamics::BrushDynamics` is the closest thing to a graph today: size, opacity,
flow and spacing can each take up to four inputs (pressure, speed, tilt,
random), each through a curve of 2–16 points; the curve results multiply, then
a minimum is applied by clamping or remapping. `dynamics::curve_ui` is the curve
editor.

### Where brushes are stored

- The brush library and each tool group's live brush live in preferences
  (`preferences.rs`, key `dudel.preferences.v1`).
- Brush packs (`brush_pack.rs`) are JSON: format `dudel-brush-pack`, version 1
  or 2, up to 512 presets, tip and grain images embedded.
- **Projects never store brushes.** A brush graph cannot make a project
  unreadable.
- Replay fixtures (`replay.rs`) serialize stroke settings, brush included, for
  tests and benchmarks.

### Preview and Studio

`brush_studio.rs` renders a live preview on a worker (`LivePreview`, newest
settings win, half resolution after a slow render) from a fixed tip dab plus a
sine stroke with rising and falling pressure. The Studio has nine sections
(size and opacity, pressure, dynamics, tip, stabilizer, scatter, grain, name and
tags, test pad). `brush_previews.rs` draws the small library thumbnails.

### Extensions

Rhai filters (`plugins.rs`) run under operation, depth and size limits; WASM
filters (`wasm_filters.rs`) run under fuel. Scripted tools (`scripted_tool.rs`)
turn pen samples into lines that the native brush draws, never touching the
document, with an operation limit per call. Extensions declare capabilities
(`extension_api.rs`). None of these run per dab today.

## 2. What can be reused

- **Inputs:** every input the brief asks for already exists on `Sample` or in
  `Stroke`: pressure, tilt (altitude and, from `[x, y]`, azimuth), twist,
  normalized speed, arc length (`distance`), stroke heading, a deterministic
  per-dab random value (`random_value(dab_index, scatter_seed)`) and timestamps.
- **Curves:** `dynamics::Curve` (validation, evaluation) and `curve_ui` become
  the Curve node and its editor unchanged.
- **Outputs:** `DabRaster::new` and `Stroke::sample` already consume per-dab
  size, opacity, flow, spacing and rotation. The graph only has to supply them.
- **Determinism:** dab placement is already reproducible from the input
  samples and `scatter_seed`; undo, replay and timelapse depend on that today.
- **Preview:** `LivePreview` re-renders on any change; a graph is just another
  setting to hash.
- **Sharing:** brush packs already embed tips and grain, so a graph brush in a
  pack is self-contained.

## 3. What is missing

- **Per-dab color.** The stroke buffer accumulates coverage and composites it
  with one color per stroke (`commit_coverage_row`). Color outputs (hue jitter,
  color by pressure) need the buffer to carry color per dab. That is a real
  engine change, so color is left out of the first graph release.
- **Per-pixel textures.** Grain and tips are evaluated per pixel in the hottest
  loop. Procedural texture nodes would add work there. The first release keeps
  tip and grain as ordinary brush settings; texture nodes come later, if at all.
- **Stroke-end information.** During live drawing the stroke's total length is
  unknown, so "position along stroke" can only be distance from the start.
  Taper-out keeps working as it does now.
- **Several random streams.** There is one random value per dab. The graph needs
  one per Random node: hash `(dab_index, scatter_seed, node id)`.
- **Graph editor UI.** Nothing in Dudel draws nodes and wires yet.

## 4. Graph model

New module `brush_graph.rs`, independent of the editor:

```rust
pub struct BrushGraph {
    pub version: u32,          // 1
    pub nodes: Vec<Node>,      // at most 64
    pub links: Vec<Link>,      // at most 128
}
pub struct Node { pub id: NodeId, pub kind: NodeKind }   // NodeId(u32), stable
pub struct Link { pub from: (NodeId, u8), pub to: (NodeId, u8) } // output port → input port
pub enum NodeKind { Pressure, Tilt, ..., Curve(Curve), Remap { .. }, Multiply, ..., SizeOutput, ... }
```

- Parameters live inside `NodeKind` variants, so each node type validates its
  own values. Unconnected inputs use a per-port default stored on the node.
- **Editor layout is stored separately**: `Brush::graph_layout` (node
  positions only). It is saved with the brush but never compiled, and moving a
  node does not change how the brush paints.
- `Brush` gains `graph: Option<BrushGraph>` with `#[serde(default)]`; brushes
  without a graph serialize exactly as today.
- **Validation** (`BrushGraph::validate`, called from `Brush::validate`) refuses
  unknown node versions, duplicate ids, links to missing nodes or ports,
  incompatible port types, two links into one input, cycles, more than one
  output node of a kind, and out-of-range parameters. An invalid graph is
  refused when it is loaded or imported, and the brush keeps its last valid
  graph. It can never reach the drawing path.
- **Migration**: `version` lets later releases convert old graphs; unknown
  newer versions are refused with a clear message.

## 5. Node and port types

Two port types in the first release:

- **Amount**: a number from 0 to 1 (pressure, curves, size and opacity factors).
- **Angle**: degrees (tilt direction, stroke direction, rotation).

Converting between them takes an explicit node (Angle → Amount and back), so a
pressure value cannot be wired into rotation by accident.

First node set, chosen because each one maps onto something the engine can
already do:

| Group | Nodes |
|---|---|
| Inputs | Pressure, Tilt (altitude), Tilt direction, Twist, Speed, Stroke direction, Distance (with a length setting), Random, Constant |
| Shaping | Curve, Remap (in range → out range), Invert, Smooth (eases changes across dabs) |
| Combine | Multiply, Add, Minimum, Maximum, Mix |
| Outputs | Size, Opacity, Flow, Spacing, Rotation, Scatter |

Deferred: Color outputs (need per-dab color), texture nodes (per-pixel cost),
subgraphs and extension nodes (section 9).

**Interplay with existing settings:** a connected output owns its parameter
completely. When Size is connected, the brush's pressure-size toggle and size
dynamics are ignored; when it is not, they apply as now. The Studio greys out
the settings a graph has taken over, so pressure is never applied twice. Taper
in and out still multiply the final size and opacity, as they do for every
brush.

## 6. Runtime

1. **Compile** when the graph changes (in the Studio, or when the brush is
   loaded): validate, sort topologically, and lower to `CompiledGraph`, a flat
   `Vec<Op>` over a fixed register array (`[f32; 64]`) with each output's
   register index. Compiling is cheap and happens on the UI thread.
2. **Ship** the compiled graph with the stroke: `stroke_buffer::Settings`
   carries an `Arc<CompiledGraph>`, so the render worker never sees the editable
   graph.
3. **Evaluate once per dab** in `Stroke::sample`, where the dab's random value,
   heading and spacing are already known. Results go into `Sample` fields
   (`size`, `opacity`, `flow` beside the existing `spacing` and `rotation`),
   which `DabRaster::new` uses instead of recomputing dynamics.
4. No allocation, no hashing of the graph and no traversal in the drawing path:
   one pass over the op list per dab. Smooth nodes keep their state in a small
   per-stroke array, reset when the stroke starts.

Random nodes hash `(dab_index, scatter_seed, node id)`, so the same input
always paints the same pixels: undo, recovery replay, timelapse and the preview
stay exact.

## 7. Editor

A **Graph** section in the Brush Studio, beside Dynamics:

- A node canvas drawn with egui's painter: nodes as rounded panels with named
  ports, wires as curves, pan with Space or the middle button, zoom with the
  wheel, grid snapping off by default.
- Add nodes from a searchable, grouped list (right-click or a button); delete,
  duplicate and multi-select with the usual keys.
- The selected node's settings appear in a side strip, using existing widgets
  (`curve_ui` for curves, sliders elsewhere).
- Problems are shown on the node and port they concern, in plain words ("Size
  has two inputs").
- Its own undo stack of graph snapshots (graphs are tiny), separate from
  document history.
- The live preview sits above the canvas and updates as wires change.

The node canvas is custom, about one module, rather than a new dependency: the
existing node-editor crates either lag egui releases or bring their own visual
style. This can be revisited if the canvas grows past what it needs.

"Make graph from current settings" converts a brush's present pressure,
dynamics, rotation and scatter into an equivalent graph, so artists start from
the brush they know.

## 8. Persistence and sharing

- Graph brushes save in preferences like any brush.
- Brush packs that contain a graph are written as **version 3**. Older Dudel
  refuses version 3 packs instead of importing them without their graphs.
  Packs without graphs stay version 2.
- Older Dudel reading newer *preferences* ignores the `graph` field and paints
  the brush with its plain settings; this is noted in the release notes.
- A single brush exports as a one-brush pack (`.dudelbrush`), with its preview
  image, author and license fields added to the pack metadata.
- No hosted marketplace.

## 9. Later

Subgraphs and groups, color outputs once the stroke buffer carries per-dab
color, multiple tip layers, procedural textures, time-based parameters.
Extension-provided nodes would run as compiled Rhai expressions under a fixed
operation budget per dab, with no access to the document or files; WASM's fuel
model is too costly to enter once per dab. That needs its own review.

## 10. Performance

Graph evaluation is a few dozen float operations per dab, against hundreds to
thousands of pixel operations for the dab itself, so the risk is not the math
but accidental allocation or locking in the dab path.

- Add a `brush-graph` case to `benchmark.rs`: the existing `dynamic_ink` brush
  against its converted graph, same replay. Gate: at most 2% more stroke time,
  zero extra allocations per dab (checked with `allocation_trace.rs`).
- A micro-benchmark of `CompiledGraph::evaluate` on a 30-node graph.
- Keep the existing round-tip fast path in `paint_dab_rows` for graph brushes;
  it only depends on per-dab radius and opacity, which the graph supplies.

## 11. Phases

| Phase | Delivers | Gate |
|---|---|---|
| 1. Engine | Model, validation, compiler, evaluator, Size/Opacity/Flow/Spacing/Rotation/Scatter outputs, conversion from current settings | Converted brushes paint identical pixels; malformed graphs refused; benchmark gate |
| 2. Editor | Graph section in the Studio: canvas, wiring, node list, settings strip, errors, undo, live preview | Workflow tests for add, wire, delete, undo; usable at 1366 × 768 |
| 3. Sharing | Pack version 3, `.dudelbrush` export and import, metadata | Round-trip; older versions refuse; malformed files refused |
| 4. Color | Per-dab color in the stroke buffer, color nodes | Buffer memory and speed unchanged for brushes without color nodes |
| 5. Extensions | Subgraphs, extension nodes | Separate security review |

## 12. First milestone: the engine

Phase 1, with no editor yet. Acceptance criteria:

1. `brush_graph.rs` with the model, validation and migration hook; unit tests
   for every refusal in section 4 (cycles, type mismatches, missing nodes,
   duplicate inputs, bad parameters, unknown versions).
2. `CompiledGraph` compiled once per stroke, carried in `Settings`, evaluated
   once per dab with no allocation.
3. "Make graph from current settings" for every built-in brush, and a test that
   each converted brush paints **pixel-identical** strokes to the original
   across a recorded replay with pressure, tilt and speed.
4. Brushes without a graph are unchanged: existing tests, previews and
   benchmarks pass untouched.
5. Benchmark gate from section 10.
6. Preferences round-trip a graph brush; a malformed graph in preferences is
   dropped with a status message instead of failing settings load.

Until the editor lands, graphs are reachable only through conversion and tests,
so milestone 1 changes nothing artists can see. That keeps it safe to merge
before the editor design is settled.
