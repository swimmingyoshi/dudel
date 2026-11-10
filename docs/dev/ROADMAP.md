# Roadmap

Updated 2026-11-10, after the 0.3.0 pre-release. This replaces the earlier plan
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
- [ ] Workspace reorganization: decide which panels can be docked, tabbed or
      hidden (brush library, color, navigator, layers, references) and whether
      layouts can be saved (see *Named workspace layouts* below).
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
4. [x] **Brush graph, milestone 1** (engine only): see the
       [brush graph plan](BRUSH_GRAPH_PLAN.md). Next: milestone 2, the
       editor.
5. [ ] Named workspace layouts, after the workspace reorganization above.

## Later

These wait for tester feedback or for a reason to do them:

- Code signing and an optional installer with file associations.
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
