# Brush graph

Status: accepted 2026-11-10; milestones 1 (the engine, [section
13](#13-milestone-1-as-built)), 2 (the editor, [section
14](#14-milestone-2-as-built)), 3 (sharing, [section
15](#15-milestone-3-as-built)) and 4 (color, [section
16](#16-milestone-4-as-built)) are implemented, followed by the nodes and
templates in [section 17](#17-unique-nodes-and-templates). Milestone 5
(groups and formulas) was [proposed](#18-proposal-milestone-5-groups-and-formulas),
accepted and [built](#19-milestone-5-as-built). It answers the "first deliverable" of
the brush graph brief: how a node-based brush editor fits the brush engine
Dudel already has, and what to build first.

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

## 13. Milestone 1 as built

`src/brush_graph.rs`, 2026-11-10. Where it differs from the plan above:

- **Outputs:** Size, Opacity, Flow and Spacing. Rotation and Scatter stay
  plain brush settings for now: the stroke draws their random values from one
  shared sequence per dab, and keeping that order exact while a graph takes
  them over needs its own design. With them goes the Angle port type, so every
  port carries an amount from 0 to 1 and there is nothing to mismatch yet.
- **Nodes:** Pressure (raw, or shaped by the brush's pressure response),
  Speed, Tilt, Twist, Distance, Random (seed 0 is the dab's own random value;
  other seeds are independent streams), Constant, Curve, Remap, Invert,
  Multiply, Add, Minimum, Maximum, Mix, and the four outputs. Smooth and Tilt
  direction wait for the editor.
- **Missing sensors:** a value the pen does not report travels as *missing*.
  Curve, Remap, Invert and Mix pass it on, Multiply and Add leave it out,
  Minimum and Maximum take the other input, and an output that receives only
  a missing value stays at full for that dab. This is what the existing
  dynamics already do, which is why conversion can be exact.
- **Evaluation:** the compiled graph is cached on the graph itself (rebuilt
  after an edit) rather than shipped separately in `stroke_buffer::Settings`;
  it travels with the brush the stroke already carries. `Brush::factors`
  evaluates it once for size, opacity, flow and spacing together, at the two
  places each dab needs them (spacing in `Stroke`, the dab in `DabRaster`).
  Registers are a stack array, so a dab allocates nothing.
- **Layout:** `graph_layout` arrives with the editor; nothing reads it yet.
- **Conversion:** `BrushGraph::from_brush` builds the graph for a brush's
  pressure toggles and size, opacity, flow and spacing dynamics. A test paints
  every built-in brush, plus brushes using every dynamics input, both minimum
  modes and a custom pressure curve, along a stroke with pressure, tilt and
  uneven timing, before and after conversion: the pixels are identical.
- **Benchmark:** `--benchmark` now includes "Speed dynamics ink (graph)", the
  dynamics ink driven by its converted graph. Release build, 41 runs: median
  stroke time 8.31 ms plain, 8.49 ms with the graph (+2.1%), against run-to-run
  noise of about ±5%. That is at the 2% gate rather than clearly under it;
  reaching it took evaluating once per dab instead of once per setting.
- **Persistence:** brush packs holding a graph are version 3; a version 1 or 2
  pack claiming a graph is refused. A damaged graph in preferences is removed
  on load, with a status message, and the brush keeps its plain settings.

## 14. Milestone 2 as built

`src/graph_editor.rs`, 2026-11-10: the **Brush graph** section of Brush
Studio. Where it differs from section 7:

- **Canvas:** nodes with named input dots on the left, an output dot on the
  right and a footer that shows the node's setting or a problem in plain words
  ("Not connected", "Needs an input"). Wires are curves. Drag a node to move
  it, drag from an output to an input to wire, drag an input's wire away to
  move or remove it; drag empty space (or the middle button) to pan; the wheel
  zooms about the pointer. **Fit** frames every node.
- **Always valid:** every edit is checked on a copy first. A wire into a used
  input replaces it; a wire that would make a loop is refused with a message;
  outputs already present are greyed out in the node list. The graph a brush
  paints with is therefore never invalid.
- **Adding:** **Add node** in the toolbar or a right-click on the canvas, as a
  grouped list (Inputs, Shaping, Combine, Outputs) with a description on
  hover. Right-click a node to delete it. Single selection only; multi-select
  and duplicate are left for later.
- **Settings strip:** beside the canvas when the section is at least 540
  points wide, below it otherwise. Curve nodes use the dynamics curve editor.
- **Undo:** up to 100 graph-and-layout snapshots per preset, separate from
  document history; a setting dragged on a slider or curve is one step. No
  keyboard shortcuts, so Delete and Ctrl+Z keep their document meanings.
- **Taken-over settings:** the pressure checkboxes and the Dynamics section
  show "set by the brush graph" for outputs the graph drives.
- **Layout:** node positions are `Brush::graph_layout`, saved with the brush
  and excluded from the preview cache key. "Make graph from current settings"
  lays nodes out in columns by depth.
- **Tests:** model edits (loops refused, replaced and removed wires), the
  layout direction, and a workflow test at 1366 × 768 that makes a graph,
  adds a node from the menu, drags it, wires it with the mouse, undoes and
  redoes, and deletes it. `--capture-brush-graph` takes a screenshot.

## 15. Milestone 3 as built

`src/brush_share.rs` and `src/brush_pack.rs`, 2026-11-10.

- **`.dudelbrush`** is a one-brush pack: the same JSON as `.dudelbrushes`
  (version 3 with a graph, 2 without), plus an optional `info` object with
  author, license and description (100, 100 and 1000 characters, no control
  characters except newlines in the description). Older Dudel ignores `info`.
- **Export:** **Preset > Export brush…** opens a form (author and license are
  remembered in preferences; **Common licenses** offers CC0 1.0, CC BY 4.0,
  CC BY-SA 4.0 and All rights reserved), then the save dialog with a file name
  made from the brush name.
- **Import:** **Import brush…**, **Import pack…** and dropped `.dudelbrush` or
  `.dudelbrushes` files all open one review dialog before anything joins the
  library: a sample stroke rendered from the brush itself, the name (and how
  many more for packs), author, license, description, and notes when a brush
  uses a graph or smudges or blurs. Plain `.json` brushes from older Dudel
  still import. Files are read up to 16 MB.
- **Refused:** malformed JSON, unknown versions ("made by a newer Dudel"),
  version 1–2 packs claiming a graph, invalid brushes or graphs, and
  over-long or control-character details; nothing reaches the dialog.
- **Deviation:** no stored preview image. Dudel renders the brush itself in
  the dialog, which cannot disagree with how it paints and keeps files small.
- **Brush Studio as a tab** (asked for alongside this milestone): the Studio
  opens as a tab beside the project tabs and fills the canvas area; the canvas
  keeps its background work (render queue, previews) while hidden. **Float as
  window** restores the floating window; the choice is a preference.

## 16. Milestone 4 as built

2026-11-10.

- **Color outputs:** Hue shift (a range setting, 0 to 180 degrees; 0.5 is no
  change), Saturation and Brightness (multipliers, 1 is no change) and Color
  mix (0 main color, 1 secondary). A missing value leaves that part of the
  color alone. `ColorShift::apply` blends toward the secondary color first,
  then turns hue and scales saturation and value in HSV.
- **Secondary color:** `stroke_buffer::Settings` gained `secondary`; canvas
  strokes pass the real secondary color (mask strokes its gray), the Studio
  preview and library thumbnails a fixed contrasting one.
- **Stroke buffer:** for a brush whose graph sets color, each pixel also keeps
  the stroke's mixed color, allocated per 32 px block only when such a dab
  reaches it. A dab of color `c` delivering `d` over coverage `C` and color
  `K` gives coverage `C + d(1 − C)` and color `(c·d + K·C·(1 − d)) / C'`, so
  overlapping dabs blend instead of the last one winning. The result is
  composited over the original exactly as before.
- **Unchanged for other brushes:** brushes without color outputs never take
  the color path: the pixel-identity test still passes, the buffer adds one
  empty slot per block, and an interleaved release benchmark against the
  previous build shows every brush within noise (−5.6% to +1.0%).
- **Limits:** color brushes skip the multi-core path for very large dabs, and
  Taper out does not apply to them (re-tapering would have to replay the
  colors). Smudge and blur ignore color outputs.
- **Test pad** (asked for alongside): Brush Studio keeps a true-scale test
  pad below the settings when it is at least 460 points tall, with Clear,
  Undo (12 strokes), Stripes in the main and secondary colors, and white,
  gray or dark paper; it can be hidden. A pad shown in two places handles
  each frame's pen input once. `--capture-color-brush` shows a color brush
  on the pad.

## 17. Unique nodes and templates

2026-11-10, after the question "is the graph just a second editor for the
same settings?". Milestone 1 deliberately made every pressure and dynamics
setting convertible to a graph, so on its own the graph overlapped the
Dynamics section. What it could do that the panels cannot was color, Add /
Minimum / Maximum / Mix (dynamics only multiply), Twist and Distance, and
several independent random values. This round adds what no panel expresses:

- **Rotation** (range up to 180°; 0.5 keeps the brush angle) and **Scatter**
  (0 to two brush sizes) outputs. A graph that wires one owns it: the Turns
  with, Jitter and Scatter controls grey out. Brushes without them take the
  old path in `Stroke::scatter`, including its random draw order, so they
  paint identically.
- **Direction** and **Tilt direction** inputs, centered so 0.5 is rightward:
  with a 180° Rotation range the tip turns exactly with the stroke. The
  stroke's heading travels on `Sample::heading`, set only when a graph reads
  it.
- **Wave** (smooth, triangle, dashes, ramp; period in pixels along the
  stroke), **Noise** (smooth value noise along the stroke, seeded) and
  **Steps** (2–32 levels). All are pure functions of the dab, so replay and
  undo stay exact.
- **Templates** (Brush Studio > Brush graph > Templates, or Start from a
  template), each replacing the graph as one undo step: Downstroke ink,
  Twisting ribbon, Dashed line, Wobbly ink, Color jitter, Pressure color,
  Speed spray, Stipple, Tilt shading. A test paints each and checks it
  differs from the plain brush; an opt-in test
  (`DUDEL_TEMPLATE_SHEET=out.png cargo test template_sheet -- --ignored`)
  draws them all on one sheet.
- A first "Calligraphy" template (tip turns with the stroke) was dropped: the
  plain **Turns with: Stroke** setting already does it. Templates should only
  show what the graph adds.

### Material outputs (2026-11-13)

Asked for by the owner so node brushes can do what the canvas materials do:
**Water**, **Pigment**, **Thickness**, **Paper texture** and **Granulation**
outputs. Like Size and Flow, each gives 0 to 1 per dab and scales the brush's
own setting (`Factors::material`, carried on `DabRaster` to the painter), so
the panel slider stays the most a dab gets and brushes without them paint as
before. Mode (Paint, Wet, Smudge, Blur) stays a panel setting: switching what
a dab does mid-stroke is not a value to blend. Test:
`nodes_drive_water_pigment_thickness_paper_and_granulation_per_dab`.

A graph that turns the tip with the stroke (Direction into Rotation over
180°) is found by trying it once over many pen inputs
(`Compiled::turn_with_stroke`), so it gets the same along-the-stroke dab
spacing as a brush set to Turns with: Stroke.

## 18. Proposal: milestone 5, groups and formulas

For review; nothing here is built. The brief asked for sub-graphs and
extension-provided nodes under a controlled capability model.

**Groups (sub-graphs).** Select nodes and choose **Group**: they become one
node whose unwired inputs and outputs are its ports, with a name the artist
picks. Groups can be opened to edit, saved to a personal node library and
shared inside brush files. They are plain data inside the graph, compiled by
inlining into the same flat operation list, so they cost nothing per dab and
add no security surface. Limits: groups nest four deep, and a graph stays at
most 64 operations after inlining (raising the cap to 128 is possible; the
register file is the only cost).

**Formula node instead of script nodes.** A node with up to three inputs
(a, b, c) and an expression such as `clamp(a * 2 - b, 0, 1)` or
`mix(a, b, smoothstep(0.2, 0.8, c))`. The expression language is Dudel's own:
numbers, `+ - * /`, parentheses, and a fixed list of functions (min, max,
clamp, abs, floor, fract, sqrt, pow, sin, cos, mix, smoothstep, step). No
variables, loops, strings, calls out or state, at most 64 tokens. It is
parsed once and lowered into the same operations as other nodes. There is
nothing to sandbox, because the language can only compute a number;
non-finite results count as missing, like an absent sensor.

**Why not Rhai or WASM per dab.** A stroke paints thousands of dabs at about
3 µs each in the benchmark. Entering a script engine twice per dab would
roughly double stroke time, and each engine brings its own sandbox to
maintain. If extensions ever provide nodes, the proposal is "bake once": the
extension runs when the node is added or its settings change (under the
existing filter limits), produces a curve or a 256-entry table, and the graph
stores only that result. Shared brushes then carry data, never code.

**Questions for review.**
1. Groups and the Formula node first, extension "bake" nodes later or never?
2. Raise the 64-node cap to 128 for grouped graphs?
3. Should groups also be shareable on their own (a node-library file), or
   only inside brushes?

## 19. Milestone 5 as built

2026-11-10. Answers to section 18: groups and the Formula node, yes; a
128-node cap, yes; groups shared inside brushes, with no separate file. No
extension code runs per dab.

- **Formula** (`src/brush_formula.rs`): numbers, `+ - * /`, brackets, `a b c
  pi`, and min, max, clamp, abs, floor, fract, sqrt, pow, sin, cos, mix,
  smoothstep, step; at most 200 characters and 64 tokens. Parsed once into a
  stack program (stack of 32, no allocation per dab). Unwired inputs read 0;
  a wired input whose sensor is missing stays missing, and so does any
  non-finite result. Mistakes are explained ("min needs 2 values", "“x” is
  not a known name"); the editor keeps the last valid formula on the node
  while the text is being fixed and notes inputs the formula does not read.
- **Groups** (`NodeKind::Group`): a name, up to four inputs, inner nodes and
  links with their own ids, and an inner layout. Inside are Group input
  nodes (one per input) and exactly one Group result. Brush outputs stay in
  the main graph. Groups nest four deep. Compiling inlines groups under fresh
  ids before lowering, so they cost nothing per dab; the 128-node cap counts
  the inlined graph; the main graph holds at most 256 links.
- **Making groups:** select nodes and choose Group. Values wired in from
  outside become inputs (distinct sources, at most four); the selection must
  give exactly one value outward (or, if nothing outside reads it yet, end in
  exactly one node). Ungroup reverses it, rewiring inputs and the result;
  grouping and ungrouping keep the brush's output identical (tested).
- **Editing:** every edit runs on a copy of the current level (main graph or
  open group) and is kept only if the whole graph validates. Double-click or
  Open group enters a group; a breadcrumb (Back, Main graph › …) leaves it.
  Multi-select with Ctrl- or Shift-click and Shift-drag boxes; dragging moves
  the selection; Delete and Group act on all of it. Group inputs and results
  cannot be deleted on their own.
- **Reuse:** Add node › Groups lists every group in the brush library's
  graphs once (by name and contents). Sharing a group means exporting a brush
  that holds it.
- **Tests:** formula grammar and refusals; group/ungroup equivalence; refusals
  (outputs, several results); nested groups with a formula evaluating
  correctly and surviving save and load; the inlined-node cap; library
  de-duplication; and a Studio workflow test that selects two nodes with
  click and Ctrl-click, groups them, opens the group by double-click, goes
  back, ungroups and undoes. `--capture-brush-group` shows an open group.

## 20. Limits lifted

2026-11-10, after milestone 5:

- **Groups give several values.** A wire now records which output it leaves
  from (`Link::from_port`, omitted when 0, so brushes saved before still
  load). Groups have up to eight outputs ("Out 1…") and eight inputs, one
  Group result node per output inside. Grouping turns every distinct value
  read outside into an output; ungrouping rewires each consumer to the value
  behind its output. Tested: a two-output group paints the same as the nodes
  it replaced, through save, load and ungroup.
- **Editor keys.** While the pointer is over the graph canvas (on this or
  the previous frame): Delete/Backspace, Ctrl+Z, Ctrl+Y and Ctrl+Shift+Z,
  Ctrl+A, Ctrl+G, Ctrl+Shift+G and Esc. They are taken at the start of the
  app's shortcut handling, before any document shortcut can see them; a
  workflow test checks that Delete and Ctrl+Z change the graph and leave the
  document and its history untouched.

## 21. Seeing what a graph does

2026-11-11. The editor could build any graph but could not show what one
computes, so a graph was learned by painting with it. Now:

- **Values on the nodes.** Compiling records, for every main-graph node
  output (a group's outputs included, and for brush outputs the value wired
  into them), the register that holds it (`Compiled::probes`).
  `Compiled::trace` runs the same operations as painting and returns those
  values; painting never calls it, so the per-dab cost is unchanged. The
  editor draws each value at its output dot and as a bar under the node's
  title. Inlining records, for every level, which flat node stands behind
  each node output (a group input shows what feeds it from outside, a group
  result what is wired into it), so values show inside groups too;
  `trace` takes the path of open groups.
- **Try inputs.** The values are shown for a pen input set with sliders
  (pressure, speed, tilt, direction, distance along the stroke, random), or,
  for three seconds after each dab, for the latest dab drawn on the test pad
  (`Scratchpad::recent_dab`).
- **Search and drop-to-add.** **Add node** starts with a search over node
  names and descriptions; Enter takes the first match. Letting a wire drawn
  from an output go over empty canvas opens the same search there, and the
  node chosen is added and wired into its first input as one undo step. A
  wire picked up off an input and let go is still just removed.
- **Hover help** names what a node, input or output does and how to wire it.
- While any text field has the keyboard, the editor's keys stay out of the
  way, so Backspace in a search or formula never deletes nodes.
