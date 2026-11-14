# Roadmap

Updated 2026-11-13, with the 0.5.0 pre-release. This replaces the earlier plan
files; their history is kept in the project's archive.

## Direction

Dudel aims to be a dependable everyday drawing app for pen tablets: the tools
artists use constantly, a clear interface and artwork that is never lost. Breadth
of format support and features matters less than polish and reliability.

## 0.3: ready for outside testers

A polish release with no large new features.

- [x] Welcome window with new canvas, open, recent projects and basic keys.
- [x] Problem reports and a clear message when Dudel has to close.
- [x] Help menu links: quick start, report a problem, releases page.
- [x] Visible checkboxes and radio buttons; highlighted main dialog buttons.
- [x] More room for layers; no duplicate zoom and rotation controls.
- [x] Color and Navigator as tabs in the right panel.
- [x] Workspace reorganization: decided 2026-11-10 to keep today's panel
      arrangement; named workspaces save it (0.4). Docking stays a later idea.
- [x] Docs reorganized under `docs/`; process records archived.
- [ ] Find three to five testers on other tablets (Wacom, Huion, XP-Pen
      models besides the Artist 12) and act on their reports first.
- [x] Layers panel: rarely used toolbar actions in a menu; the mask row only
      for layers with a mask.
- [x] One dialog style: title, close button and button order everywhere.
- [x] Build and attach the Linux tarball for each release (cross-built since 0.3.0).

## 0.4: painting essentials

The everyday tools and filters artists expect from other painting apps, in
this order (agreed 2026-11-10), adjusted by tester feedback:

1. [x] **Gradient tool:** the Fill tool's Gradient mode (tap **G** again)
       paints linear, radial, angle, reflected or diamond gradients from the
       current colors or custom stops, reusing the layer-effects gradient
       model. Later: a library of named gradients.
2. [x] **Filters:** Gaussian blur, sharpen (unsharp mask) and add noise, each
       with a live preview (blur reuses the layer-effects blur).
   [x] Every adjustment-layer type (curves, posterize, threshold, color
       balance and the rest) also applied directly: **Filter > More
       adjustments**, with the Adjustment window's editors.
3. [x] **Blending brushes:** a brush Mode that smudges (drags color along the
       stroke) or blurs instead of painting, with built-in Smudge and Blur
       presets. Later: a color-mixing ("wet paint") mode that also lays down
       the current color.
4. [x] **Brush graph, milestones 1–4** (engine, editor, sharing single
       brushes as `.dudelbrush` files, per-dab color): see the
       [brush graph plan](BRUSH_GRAPH_PLAN.md). Brush Studio opens as its own
       tab with a test pad in view; Rotation and Scatter outputs, rhythm and
       noise nodes, templates, and milestone 5 (groups and the Formula node)
       followed, then Water, Pigment, Thickness, Paper texture and
       Granulation outputs (2026-11-13). Next: refine the editor from
       testers' use, then special canvases.
5. [x] Named workspace layouts: **View > Workspaces** saves which panels show,
       their sizes and the Color/Navigator tab, and switches between them.

## Canvas materials

The canvas as a surface that paint reacts to, in stages:

1. [x] **Paper surfaces:** a per-project paper (`src/paper.rs`) with a
       procedural height field (grain, weave, fibers, tooth), roughness, grain
       size, direction and absorbency. Dabs deliver less paint in valleys
       unless they press hard enough; no stored texture and no simulation.
2. [x] Pigment deposition: a brush's Granulation makes paint settle into the
       paper's valleys. Buildup by paint thickness waits for stage 4.
3. [x] Wetness (`src/wet.rs`): Wet brushes leave water in a sparse per-layer
       map. Live: a wet dab blends into still-wet paint under it. After the
       stroke: wet tiles exchange paint with wet neighbors (conserving it),
       water spreads and evaporates by the paper's absorbency, fibers carry
       paint along their direction; drying undoes with the stroke that left the
       paint wet and uses at most
       about a quarter of frame time. Rewetting dried paint is not done yet,
       and timelapse replays do not include drying (rewetting came in
       stage 5).
4. [x] Material-aware layers (`src/material.rs`): a layer's material is
       Watercolor (drying water carries pigment down its gradient between
       wet pixels, conserving it: rims darken and fresh water pushes paint
       into blooms), Dry media (the paper's tooth, deepened, applies to every
       brush; no water) or Thick paint (a one-byte relief channel saved in
       the project's `RELF` trailer, built by each dab with bristle streaks
       along the stroke heading, lowered by the eraser; a light layer
       clipped to the paint is drawn at composite time and redrawn only on
       relief tiles that changed). The relief follows moves, transforms
       and flips (of the whole layer or of a selection), canvas rotation
       and resizing. The paper shows on
       the canvas display (`src/paper_view.rs`).
5. [x] Advanced materials: pigment mixing (`src/pigment.rs`: colors as
       16-band reflectance spectra from seven overlapping primaries, mixed
       by Kubelka–Munk with luminance-weighted concentrations and a residual
       that keeps single colors exact; a per-thread table of recent mixes)
       for wet-on-wet, paint landing on wet paint, smudge and blur on
       watercolor and thick paint layers. Clear water: a Wet brush's
       Pigment setting; water re-wets dried paint and lifts a little.
       Palette knife: Smudge on thick paint carries the relief with the
       paint, Blur smooths it. Drying mixes too: paint running in from each
       wet neighbor mixes as pigment where its color differs. Not yet: paint
       thickness changing how colors mix.

## 0.5: installing, updating and painting speed

- [x] Install on this PC and updates from inside Dudel (next section).
- [x] Wet, smudge, blur and thick paint dabs across processor cores; large
      wet brushes and the palette knife keep up with the pen on the owner's
      tablet. Measurements and what is left are in the
      [engineering backlog](ENGINEERING_BACKLOG.md).
- [x] Tips that turn with the stroke start turned; no display flicker
      around a large brush; a wet stroke and its drying undo together; pure
      black mixes as a pigment.
- [x] Brush graph outputs for the material settings.
- [x] Code structure and builds: the painting function and `main.rs` broken
      up, a `fast` build profile, shorter CI.
- [ ] Update from one published release to the next, tried on an installed
      and a portable copy (possible once 0.5.0 is published).

## Installing and updating

- [x] **Install on this PC** from the portable copy: per-user programs folder,
      Start menu and desktop shortcuts, `.dudel` file type, File Explorer
      previews, an entry under Windows Settings > Apps that removes it. No
      separate installer and no administrator rights; the ZIP stays portable.
- [x] **Help > Check for updates**: only when asked. Reads the public release
      list, downloads the Windows package, checks its checksum and replaces
      the running copy file by file, undoing everything if a file cannot be
      placed. Details in [DISTRIBUTION.md](../DISTRIBUTION.md#installing-and-updating).
- [x] `dudel.exe PROJECT...` opens projects, as File Explorer starts it.
- [ ] An application icon (needs artwork): shortcuts and `.dudel` files show
      the generic program icon until then.
- [ ] Signed update records, so an update is trusted through the project's
      own key and not only through GitHub and its checksum.
- [ ] In-place updates on Linux (the check links to the download today).

## Later

These wait for tester feedback or for a reason to do them:

- Code signing.
- Color management for print work.
- Editable animation ([proposal](ANIMATION_PLAN.md)).
- Plugin manager: version ordering and updates; a stable panel widget list.
- Store distribution.

## Known gaps in imports and exports

- PSD: knockout, CMYK adjustment layers, color lookup layers, Hue/Saturation
  range blending, shape strokes under an effect stroke, mask link state and
  PSB export. Not yet checked against PSDs saved by Krita.
- CSP: full-size pixel comparison against saved previews; glow alpha over partly
  transparent backdrops.
- Vector lines: miter joins are drawn round and stroke anti-aliasing is
  approximate.
