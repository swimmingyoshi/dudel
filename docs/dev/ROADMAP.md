# Roadmap

Updated 2026-11-09, after the 0.2.0 release. This replaces the earlier plan
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
- [x] Docs reorganized under `docs/`; process records archived.
- [ ] Find three to five testers on other tablets (Wacom, Huion, XP-Pen
      models besides the Artist 12) and act on their reports first.
- [ ] Layers panel: move rarely used toolbar actions into a menu, and show the
      mask row only when it is useful.
- [ ] One dialog style: title, close button and button order everywhere.
- [ ] Build and attach the Linux tarball for each release.

## Next tools

In the order agreed after 0.1.0, adjusted by tester feedback:

1. Gradient tool.
2. Smudge and blur brushes.
3. Named workspace layouts.

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
