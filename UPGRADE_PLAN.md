# Dudel 0.2 upgrade plan and handoff

Updated 2026-10-29. This file is the working checklist for the post-0.1.0 work.
Any agent picking this up: read this first, update the checkboxes and the
"Last session" notes as you go, and keep [DELIVERY_PLAN.md](DELIVERY_PLAN.md) for
0.1 history only.

## Base state

- Released source: commit `67716b6` "Dudel 0.1.0", tag `v0.1.0` (master and linux).
- Work through `0e0ea0f` (CSP text layers) is pushed to `origin/master`; later
  commits are local (`git log --oneline origin/master..HEAD`). Pushing starts
  hosted CI; ask the owner first.
- Local gates (see [TESTING.md](TESTING.md)): `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`,
  `python scripts/check_public_tree.py`, then `scripts/Verify.ps1` for the full set.
- Hosted CI minutes are scarce. Verify locally, batch pushes, run hosted CI once per
  final candidate. Docs-only commits use `[skip ci]`.

## CI status for 0.1.0 (checked 2026-10-28)

Both hosted runs on `67716b6` failed, for reasons outside the app code:

| Run | Result | Cause | Status |
| --- | --- | --- | --- |
| Windows verification | 918 tests, Clippy and Python tests passed; failed at `check_public_tree.py` | `benchmarks/linux-ci-introduction-windows-20261018.json` held the CI runner's checkout path, which the public-tree rule flags only on the runner | Fixed in local commit `130f16f` (`<repo>` placeholder, like the other benchmark files) |
| Linux verification | Verify and X11 OpenGL startup passed; failed at "Upload Linux startup evidence" | Account artifact storage quota hit; no repo currently stores artifacts, GitHub recalculates every 6-12 h | Wait for quota reset; consider `continue-on-error: true` on evidence uploads |

Steps after the public-tree check (maximum project round-trip, masked noisy
fixture, packaging, candidate upload) did not run on hosted Windows for 0.1.0.
The candidate upload will hit the same artifact quota until it resets.

- [x] Remove runner path from benchmark JSON
- [ ] Decide whether evidence/candidate uploads should be `continue-on-error`
- [ ] One hosted run (both workflows) on the next candidate, after the quota resets

## Goals (owner's list, 2026-10-28)

### 1. Select -> transform flow

- [x] "Transform selection" button in selection options and the compact toolbar,
      next to "Transform mirror pair" (`src/selection_ui.rs`, `src/workspace.rs`)
- [x] Workflow test `selection_transform_button`
- [x] Manual check with pen on the tablet (owner, 2026-10-28)

### 2. Mirror-pair transform with intersecting selections

- [x] Overlapping pairs split ownership along the mirror line instead of failing
      (`src/paired_transform.rs`)
- [x] Tests for vertical, horizontal and diagonal mirror lines (`affine::paired`)
- [x] Manual check: split works (owner, 2026-10-28)
- [x] Owner follow-up: the transform box fits the driving half, not the whole
      selection. `Settings::pair_frame` (session-only, `#[serde(skip)]`) is set by
      `driver_frame()` when the session starts or the axis changes (axis change
      mid-transform resets the transform). Handles, pivot, corners and meshes use
      `Settings::frame()`; `apply_pair` takes a mesh's frame from the mapping.
      Selection and pixel ownership are unchanged.
- [x] Owner follow-up: hold **Ctrl** while moving to snap the half's axis-facing
      edge onto the mirror line (Blender-style), restoring an original sub-pixel
      gap exactly; works on tilted axes (`mirror_snap` in `src/affine_preview.rs`)
- [x] Tests: `crossing_selection_frames_its_driving_half_and_meshes_use_that_frame`,
      `mirror_pair_box_fits_the_driving_half_of_a_crossing_selection`,
      `ctrl_snaps_a_moving_mirror_half_back_onto_its_axis`
- [x] Manual check: split, fitted box and Ctrl snap (owner, 2026-10-28)
- Decided: the box stays fitted to the half's selection bounds; no fit-to-pixels.

### 3. Guides popup stops opening after "layer symmetry override"

Cause: egui `MenuButton` stored its open state under an automatic widget ID. That
ID depends on what is laid out before the button, so toggling the override shifted
it, and the popup's open state was looked up under the wrong ID. Pen taps deliver
press and release in one frame, which took a different path than a mouse click.

- [x] Reproduced in a workflow test (`guides_reopen_after_override`)
- [x] Fix: `egui::Popup::from_toggle_button_response` with fixed IDs for the
      Global and Guides buttons (`src/layer_guides.rs`)
- [x] Test passes for mouse clicks and same-frame taps
- [ ] Audit other `MenuButton`s whose preceding layout can change (same bug class)
- [ ] Manual check with mouse and pen

### 4. Plugin / add-on system

Done (API 2, `src/extension_api.rs`, `src/extensions.rs`, [plugins/API_2.md](plugins/API_2.md)):

- [x] Versioned manifest (`api: 2`), declared capabilities checked against
      contributions, namespaced IDs, command reference validation
- [x] Contributions: brushes, native tool presets (shape, selection, ...),
      commands (brush/tool/filter/host actions), panels with a fixed widget set
- [x] API 1 Rhai/WASM filters still load
- [x] Example package `plugins/example-studio-kit.dudelplugin`, source in
      `plugins/sdk/studio-kit/`; `scripts/plugin_tool.py` builds API 2 packages
- [x] Tests `extension_*`

Left:

- [ ] Plugin manager UI: list, enable/disable, uninstall, show capabilities and errors
- [ ] Permission prompt on install showing declared capabilities
- [ ] Scripted tools (custom input handling), beyond presets of native tools
- [ ] Brush asset contributions (tip/grain textures), not only parameters
- [ ] More panel widgets; decide the stable widget list before 0.2
- [ ] Load/unload without restart; update/version conflict handling
- [ ] Fuzz/limit tests for malformed API 2 manifests (size, depth, counts)

### 5. CSP and PSD import

CSP (`src/clip_*.rs`):

- [x] Import no longer rejects a whole file for folder masking flags, standard
      blend modes, edit/alpha locks, rulers/guides/light-table metadata
- [x] Ordinary/nested folders and clipping import as native hierarchy
- [x] Unsupported files offer an explicit "Open saved preview" (flattened, real
      preview size, never upscaled) with an Import details report (`src/import_report.rs`)
- [x] `--verify-import` / `--verify-clip-preview` CLI checks
- [x] Unsupported layers are skipped and named in Import details instead of
      rejecting the file (`layer_issues`, import mode only; strict inspection
      still rejects). A skipped layer's payload is never read as raster pixels.
- [x] Timelines: import the saved frame. Animation folders keep every cel as a
      layer and show the cel from the track keyframes (`src/clip_timeline.rs`:
      byte-packed "binc" mixer, `Frame`/`Tag` arrays in 1/`Rate` s units);
      fallback to the stored `ImageCelName` with a caveat note
- [x] Gray expression-color layers decode (1 color byte + alpha per pixel)
- [x] Decoder budgets sized for Dudel's own limits (32 full-size layers)
- [x] Owner's 30-file corpus (private; path in agent memory or ask the owner):
      **29 layered imports** (was 9), 30/30 previews, sources unchanged. Renders
      of animation, Gray and skipped-layer samples checked by eye against CSP
      previews (anim/rabbit/cauldron confirmed the keyframe path).
- [x] Layer limits raised (owner, 2026-10-28): 1,000 raster layers, 2,000
      entries, depth 10 (Photoshop's), with a 2 GiB stored-pixel budget equal to
      the old worst case so files cannot demand more memory than before.
      Metadata cap 16 MiB, project open cap 4 GiB, PSD input cap 2 GB.
      goblincatgirl now imports. **Corpus: 30 / 30 layered.**
- [x] Blend modes added: Dissolve, Color dodge (glow), Add (glow), Pass through
      (folders only; contents composite onto the backdrop, faded back by folder
      opacity/mask/visibility). CSP 10/12/30 and PSD pass/diss/tsly-off map to
      them; Purplelady and rude Glow layers checked against CSP previews.
- [ ] **Vector layers:** CSP does NOT store their rendered pixels (the render
      mipmap's external chunks are absent in all 5 corpus files); it redraws
      from `VectorObjectList` on open. Owner wants vector layers eventually and
      pixels for now, so the next step is a CSP vector stroke reader + renderer
      (points, pressure, width, color, brush). 7 vector layers and 1 text layer
      are skipped across the corpus (rude's line art is one).
- [ ] Glow alpha over partly transparent backdrops is our reading of the
      definition (verified by eye on opaque art only)
- [ ] Pixel comparison against CSP previews at full size (most previews are
      smaller than the canvas, so only by-eye checks so far)

PSD (`src/psd_import.rs`, new):

- [x] PSD v1 RGB, raw/PackBits/ZIP, 8-bit layers with offsets, alpha, opacity,
      visibility, clipping, common blends, locks, groups, simple masks
- [x] Saved-composite fallback with report for unsupported features; 16-bit via composite
- [x] Unit tests (decoders, truncation, offsets, masks, nested groups)
- [x] Fixed 2026-10-28: records were read top-first; PSD stores them bottom-first
      with each group's end divider below its contents. Groups always fell back
      to the composite and flat files imported upside down.
- [x] `--verify-import` on a PSD now compares Dudel's render with the saved
      composite. A CSP-converted PSD (2 folders, Subtract layers): 15 entries
      editable, 89% exact pixels, mean diff 1.2, differences on anti-aliased edges.
- [x] Layer order detected per file (2026-10-28): grouped files by marker
      nesting, flat files by sampling the saved image both ways. Owner's
      Frieren VTuber PSD (146 layers, 4096 x 2249, written top first) imports
      editable and matches its saved image to within one level everywhere.
- [x] Tested against public Photoshop-saved files (2026-10-28): ag-psd test/read
      (75 PSDs, MIT) and psd-tools tests/psd_files (279, MIT), fetched to the
      session scratchpad, not the repo. Fixed: document-wide blocks rejected every
      real Photoshop file; group records' "pixels irrelevant" flag rejected every
      file with groups; white-matted flattened images (lightened edges on
      fallback); verifier compared invisible colors and failed on files without a
      flattened image. Text and smart objects now import as pixels; fill opacity.
      ag-psd: 9 → 36/75 layered (+5 background-only). psd-tools: 28/279 layered.
- [x] 2026-10-29: grayscale documents, 16-bit ZIP prediction, folder masks (the
      cut-off Codex session, reviewed and committed); vector shapes with solid fill
      and stroke, vector masks, path operations, non-zero winding (`psd_vector.rs`:
      descriptor reader, path parser, rasterizer); clipping onto pass-through
      folders fixed in the compositor. psd-tools: 31 -> 106/279 layered; ag-psd 42/75.
- [x] 2026-10-29: gradient fill layers and gradient shapes (`Gradient` in
      psd_vector.rs; geometry follows psd-tools). psd-tools 106 -> 114 layered.
- [x] Layer effects: see goal 7. Adjustment layers: see goal 8.
- [ ] Known vector differences: miter joins drawn round; stroke AA approximate;
      Photoshop strokes under intersect/subtract path ops differ (psd-tools
      path-operations/* files); color burn / hard mix / vivid light layer
      tests differ (~1500 px) - check fill-opacity/special-8 handling
- [ ] Remaining PSD fallbacks in the public sets (2026-10-29; psd-tools 181/279
      and ag-psd 53/75 layered): CMYK (15), unsupported mask flags such as
      feather/density (14), canvases over 4096 (10), knockout `knko` (9), files
      left with no layer after dropping unsupported adjustments (7), artboards
      (6), other color modes (8); 16-bit layered (Lr16) files use the composite
- [x] Owner's Live2D recovery outputs: all 51 open layered; earlier "10% exact"
      was the comparison counting invisible colors, not an import problem
- [ ] Test against PSDs saved by Krita
- [ ] Performance: adding a layer costs ~2 ms at 100 layers, ~28 ms at 500,
      ~53 ms near 1,000 (release). Find the per-layer work done on every edit.
- [x] PSB, CMYK, 16-bit layered (2026-10-30: Lr16 layer lists, converted to
      8 bits; psd-tools 215 -> 223 and ag-psd 69 -> 70 layered, no regressions)
- [x] PSD export, first version (2026-10-31, `src/psd_export.rs`): File >
      Export Photoshop document (.psd), 8-bit RGB, background job through the
      export queue. Layers and folders bottom first with dividers, Unicode
      names (`luni`), blends, opacity, clipping, visibility, locks (`lspf`),
      open/closed and pass-through folders (`lsct`), masks (canvas-relative
      rect over the painted tiles, default color, disabled flag), glow modes as
      dodge + `tsly` 0, effects via `layer_effects::expand` (Create Layers),
      flattened image matted on white, resolution resource. Check with
      `--roundtrip-psd INPUT OUT.psd [REPORT.json]`: Frieren (146 layers) and
      all 30 private CSP files exact (apart from opacity steps); 366/366
      importable public PSDs with no layer mismatches and exact flattened
      images. psd-tools (independent) opens every file.
- [ ] PSD export: adjustment layers (write `levl`/`curv`/`hue2`/… back; the
      inverse of `adjustment::from_psd`; CSP-only HueSaturationLuminosity and
      bezier curves need a mapping or a baked fallback)
- [ ] PSD export: editable effects (`lfx2` descriptors) instead of effect layers
- [ ] PSD export: mask link state (not stored; masks reopen linked), PSB for
      files over 4 GB of layer data, 16-bit export
- [x] PSD export opens in Live2D Cubism Editor 5.2 (owner, 2026-10-31: Frieren
      export and a v2 re-export). Cubism needs ResolutionInfo, cannot read
      ZIP layer channels, and reads records bottom first; see FORMAT_SUPPORT.
- [ ] PSD export: owner check in Photoshop/Photopea and Krita
- [ ] Export speed: Frieren takes ~2.3 s (exact pixel bounds visit every
      painted pixel; composite on the export thread)

Docs updated: [FORMAT_SUPPORT.md](FORMAT_SUPPORT.md), [RELEASE_NOTES.md](RELEASE_NOTES.md).

### 6. UI polish

- [x] Settings: one scrollable window with Workspace, Drawing, Display, Storage
      pages (`src/settings_ui.rs`), replacing the Settings menu
- [x] New canvas: live size/proportion preview, portrait/landscape/HD/A4 presets,
      swap orientation (`src/new_canvas.rs`)
- [ ] Symmetry/guides popups: layout, grouping, spacing, icons
- [ ] Other popups and dialogs: consistent padding, headers, button order
- [ ] New canvas: custom preset save, DPI/print size, recent sizes
- [ ] Settings: search, reset-to-default per page
- [ ] Screenshot pass at 100% / 150% / 200% scale before and after

### 7. Layer effects (owner goal, 2026-10-29)

**Goal:** PSD files with layer effects (Photoshop "layer styles") open with
their layers and look like they do in Photoshop, and Dudel later gets its own
editable layer effects built on the same code.

Why: effects are the most common reason a public test PSD still falls back to
the flattened image (~45 of ~360 files). Measured use in those files (layers
with the effect enabled / files): stroke 74/23, color overlay 71/9, drop
shadow 38/8, gradient overlay 16/9, bevel & emboss 10/6, pattern overlay 10/5,
outer glow 4/4, inner glow 3/3, inner shadow 2/2, satin 2/2. Scanner:
`fx_scan.py` (agent scratchpad; parses `lfx2`/`lfxs`/`lmfx` descriptors).

Decisions (agreed in chat):

- One set of effect renderers (`src/layer_effects.rs`), shared by import now
  and native effects later. Built on two primitives: a signed distance field
  from layer alpha (strokes, spread/choke) and a Gaussian-like blur (shadows,
  glows).
- Stage A turns each effect into its own layer, like Photoshop's Layer > Layer
  Style > Create Layers: shadows and outer glows below the layer, overlays and
  inner effects clipped above it, named "Layer's Drop Shadow" and so on.
  Picture matches, everything stays editable; the effect no longer follows
  edits to the art (Import details says so).
- An effect we cannot draw yet is skipped and named in Import details instead
  of flattening the whole file (same policy as CSP skipped layers).
- Photoshop's blur is undocumented: fit its size-to-blur relation against the
  saved images in the test files and record the result here.

Stage A - import effects as layers:

- [x] Parse `lfx2`/`lfxs`/`lmfx` (master switch, `*Multi` lists, first drawn
      on top); ignore legacy `lrFX` when a newer block exists; global light
      angle from image resource 1037 (default 120). `Scl ` is NOT applied: a
      208% file draws its 1 px strokes at 1 px
- [x] Distance field (feature transform: Photoshop counts any coverage as
      inside and puts a partly covered pixel's edge at depth alpha) and blur
      (three box passes, sigma = 0.4 x size, fitted) with tests
- [x] Stroke: outside goes under the layer and fills what the layer leaves
      uncovered (exactly red*a + green*(1-a) in Photoshop's image); inside is
      clipped to the layer; centre goes on top. Color and gradient fill
- [x] Color overlay, gradient overlay. Fixed in passing: gradients at 90° span
      the box height (the reach formula used angle mod 90, so 90° used width)
- [x] Drop shadow (angle, distance, spread, size), outer glow (color, range:
      falloff / range, capped). Knockout = the same "adds up under the layer"
      rule, so it only matters where fill opacity makes the layer see-through
- [x] Inner shadow, inner glow (edge/centre, range)
- [x] Clipped layers (effects stay in the clip group, inner ones carry the
      alpha), fill opacity, masks; shape/fill layers' effects follow the
      path's fill, not the shape stroke or gradient transparency
- [x] Effects on folders: computed from the folder's composited contents;
      outer effects inserted under the contents, inner ones clipped to the
      folder header. ag-psd group-drop-shadows 163,223 -> 6,330 px off (max 16)
- [x] Bevel and emboss (inner/outer/emboss/pillow; smooth, chisel hard/soft;
      depth, size, soften, up/down, angle + altitude): height from the distance
      field, Lambert lighting, flat parts unchanged; highlight and shadow
      layers. Mean error 3-18 levels in ag-psd effects and psd-tools
      effect-enums. Stroke emboss skipped
- [x] Fixed: a folder starting above a layer ends its clip group, so effects
      drawn over that layer no longer land inside the folder (layer_comps.psd
      failed validation)
- [x] Corpus 2026-10-29: ag-psd 42 -> 51/75 layered, psd-tools 114 -> 138/279;
      no file that imported before got worse; owner's Frieren and Live2D files
      unchanged. Per-effect mean error in ag-psd effects: drop shadow 4.8,
      outer glow 5.0, inner glow 2.9, inner shadow 2.0, gradient overlay 10.9
      (gradient smoothing); psd-tools stroke files mostly exact
- [x] Docs: FORMAT_SUPPORT.md, RELEASE_NOTES.md
- [ ] Shape strokes (`vstk`) under an effect stroke: Photoshop shows no trace
      of the shape stroke's outer half (double-stroke-effects); not modelled

Later in stage A (rarer): pattern overlay and pattern strokes (need the file's
pattern resources), satin, stroke emboss, non-linear contours (gloss and
falloff), noise, glow gradients, "layer mask hides effects".

Stage B - native layer effects:

- [x] Data model: `Layer.effects: Vec<LayerEffect>` (serde, validated on load,
      max 32, raster layers only), saved in v4 metadata as `effects`; no
      version bump - older builds ignore the field (documented)
- [x] Compositing: `layer_effects::expand` draws the effects as layers around
      their layer (same placement as import) inside
      `Document::composite_bounds_checked`, so export, fill "visible" and the
      eyedropper see them; the canvas expands its display document. Per-layer
      cache keyed by pixels/mask identity + settings; while painting, moving
      or transforming the previous drawing is reused, and a redraw forces a
      full (signature-checked) canvas update. Merge down bakes effects once.
      History compares effects (`same_snapshot`, `compact_layer`)
- [x] UI: Layer Effects window (`src/effects_panel.rs`): add preset, enable,
      reorder, remove, per-kind settings; drags are one undo step (gesture
      checkpoint, like opacity); while the render queue is busy the gesture
      is held and sent as one edit when the pointer lifts, and later edits
      build on it; bevel halves synchronize by pair number.
      `Property::Effects` for queued edits. Workflow test covers add, toggle,
      undo and the window growing (button and badge placement: see the owner
      feedback item below)
- [x] PSD import keeps effects native on raster layers (no shape source,
      fill 255, valid settings); shapes, fill opacity and folders still bake
- [x] Performance (4096² nearly full layer, release): fresh draw drop shadow
      0.26 s, stroke 0.30 s, outer glow 0.27 s, bevel 0.44 s (was 1.1-1.5 s);
      cached 0.01 ms. Parallel row bands on the shared pool, transposes for
      column passes, distance only on the needed side of the edge
- [x] Owner feedback 2026-10-29: fx moved into the layer action row (sparkle
      icon); Layer Effects window sizes to its contents (scrolls past 75% of
      the screen); layer rows show a sparkle badge (crossed out when all are
      off; click opens the window), ordered locks, effects, symmetry, guides.
      Masks: accent outline on the targeted thumbnail, Mask actions > Show
      mask on canvas / Alt+click the mask thumbnail shows the mask in grays
      (black layer under a white layer carrying the mask, so only touched
      tiles redraw). `--capture-layer-effects` screenshots all of it
- [ ] Draw effects in the background (show the previous drawing until ready)
      so huge layers never pause after a stroke; or redraw only the region a
      stroke touched plus the effect's reach
- [ ] Effects on folders natively; fill opacity; satin; pattern effects;
      gradient editor beyond the two end colors and angle
- [ ] Effect-layer memory is not yet counted in the memory panel
- [ ] Manual check in the app with pen (owner)
- [ ] PSD export writes them back as editable effects (they are written as
      effect layers for now; see goal 5)

### 8. PSD adjustment layers (owner, 2026-10-29)

- [x] Native adjustment layers (`src/adjustment.rs`, `Layer.adjustment`): the
      compositor recolors the backdrop (or the clip group when clipped) by
      opacity x mask; layers clipped to an adjustment act through its mask
      (`Level::adjust_base`). Count as present in every empty-layer shortcut;
      settings feed the tile/prefix display key. Saved in metadata; history
      compares them; painting and merging into them refused; merge down bakes
      them into the layer below
- [x] PSD: nvrt, post, thrs, levl, curv (+`Crv ` extra), brit/CgEd (legacy
      and current), expA (gray docs use gamma 1.75), hue2 (master, colorize,
      ranges with HSV range lightness), grdm (v1 and v3). Curve shapes follow
      psd-tools' measurements (MIT, credited in the module doc)
- [x] Fixed: a pass-through folder whose own coverage is zero (only
      adjustments inside) dropped its color change (commit skipped alpha 0)
- [x] Corpus: psd-tools 138 -> 181/279 layered, ag-psd 51 -> 53/75, no file
      worse; owner files unchanged
- [x] More kinds (2026-10-29): color balance (blnc, tonal weights by HSL
      lightness), vibrance (vibA), black & white (blwh, per-hue-sector weights,
      tint at the gray's luminosity), photo filter (phfl v2 RGB/Lab/gray and v3
      D50 XYZ; multiply by density), channel mixer (mixr), selective color
      (selc, relative/absolute ink changes by color-group weight). No reference
      implementation measures these, so they are approximations; psd-tools'
      six single-layer files and ag-psd's adjustment stack match the saved
      image. All in the Adjustment window and the toolbar list (15 kinds).
      Color-only kinds on grayscale documents are left out with a note
- [x] Corpus: psd-tools 193 -> 199/279 layered, ag-psd 54 -> 59, no file
      worse (fill_adjustments still differs: color lookup, pattern overlay)
- [ ] Left out with a note: color lookup (clrL, embedded 3D LUT)
- [ ] Hue/saturation: combine range saturations like Photoshop (psd-tools
      uses a measured 21x21 table); brightness/contrast within ~13 levels
- [x] UI (2026-10-29): half-circle toolbar button lists the nine kinds and
      creates "Curves 1"-style layers above the selection (`Edit::NewAdjustment`,
      works with a busy render queue); the row thumbnail shows the adjustment
      icon and opens the Adjustment window (`src/adjustment_panel.rs`):
      channel picker, levels sliders, curves editor (click adds, drag moves,
      double-click removes), brightness/contrast, exposure, hue/saturation
      (master, colorize; PSD ranges kept, can be cleared), gradient map end
      colors and reverse, posterize, threshold. Edits as one undo step per
      gesture (`Property::Adjustment`), queued like effects when busy.
      `--capture-adjustment-layer` screenshots it
- [ ] Adjustment UI follow-ups: hue/saturation range editing, gradient map
      stop editing, a Layer menu entry, histogram in levels
- [x] Masks (2026-10-29): user and vector mask density and feather, and the
      separate "real" user mask (channel -3) when a vector mask exists; PSD
      notes grouped by kind with layer names. psd-tools 181 -> 193/279
- [ ] Remaining PSD fallbacks by count: CMYK (15), 32-bit channels (10;
      earlier notes called these canvases over 4096), knockout `knko` (9),
      artboards (6), unsupported mask flags (3), stroke widths (3); `.psb`
      (large document format) is not read at all

## Before pushing (next candidate)

- [x] Review follow-ups (2026-10-29, Claude), each with a regression test:
      effect drawings share one 1 GiB budget across all caches with LRU
      eviction, release when effects are switched off, count 4 bytes per pixel
      when drawing, and show in the Performance window; projects with effects
      or adjustments save under a new `DUDL\x06` header older builds refuse;
      the canvas tracks which effect drawings it has shown (a redraw by an
      export, preview or color pick still forces the full tile check); busy
      queue effects edits are one edit per gesture and build on the last sent
      list; bevel halves pair by a stored pair number; `LICENSE-psd-tools`
      ships in source and binary packages. See [REVIEW_2026-10-29.md](REVIEW_2026-10-29.md).

- [x] Local gates pass at `da51809` (2026-10-29): `cargo fmt --check`, Clippy
      with `-D warnings`, `cargo test` (974 passed, 8 ignored),
      `python scripts/check_public_tree.py`
- [x] Review working-tree gates pass (2026-10-29): formatting, Clippy default
      and all features, 980 default / 989 all-feature tests (8 ignored each),
      3 thumbnail tests, 42 Python tests, public-tree check and release builds.
- [x] Independent review of the 2026-10-29 work (effects, adjustments,
      compositor and display caching changes): [findings and fixes](REVIEW_2026-10-29.md).
      Five bugs fixed after six failing regressions: adjustment decode validation,
      effects-window limit, dropped PSD payloads, native effects in layer-order
      detection, and both project thumbnail paths. Changes remain uncommitted.
- [x] `scripts/Verify.ps1 -Mode Full` passed on the review working tree,
      2026-10-29. Initial release replacement hit access denied; the final run
      passed. Thumbnail DLL emitted LNK4104 export warnings. See the review report.
- [ ] Owner's manual pen pass: Layer Effects window and badges, mask view,
      PSDs with effects and adjustment layers, settings window
- [ ] Owner approves the push; then one hosted run of both workflows (see CI
      status above; artifact quota may still block uploads)

Earlier batch (2026-10-28): gates and `Verify.ps1 -Mode Full` passed; owner's
tablet pass for items 1-3; `psd_import.rs` and `extension_api.rs` reviewed.

## Last session

- 2026-10-31 (Claude, PSD export): first version, see goal 5. The importer's
  per-layer limit was 64 MP and is now the canvas limit, so full-canvas layers
  of 100 MP exports reopen. Findings: mask flag bit 0 is "position relative
  to layer", not linking; Photoshop's flattened image is matted on white;
  remaining corpus render differences are opacity in 1/255 steps (CSP layers
  at e.g. 66%), which flips low-opacity Hard Mix pixels (one CSP file, up to
  56 levels). Public sets and the psd-tools venv were in the session
  scratchpad (re-clone; see FORMAT_SUPPORT verification). Gates pass (1,025
  default / 1,034 all-feature tests). Next: write adjustment layers back.

- 2026-10-29 (Claude, PSD masks + adjustment kinds): mask density/feather and
  grouped notes (5fdb500); six more adjustment kinds with PSD parsing and
  Adjustment window controls; blend-box wheel no longer scrolls the effects
  window. Import messages split into notes (something left out) and details
  (`Imported.details`, per-tab `ImportReport`): the Import details window opens
  by itself only for notes; a status-bar button reopens it. Private CSP corpus:
  the window now opens for 6 of 30 files (skipped vector/text layers), down
  from every file. Gates pass (993 tests). Committed locally, not pushed. Full
  Verify and the owner's pen pass still open. Next CSP gap: vector layers.

- 2026-10-29 (Claude, CSP gaps from the owner's Documents\CSP files): image
  material layers (original resampled to the stored corners), oversized
  layer bitmaps, inch canvases, switched-off proofing, palette color labels,
  and vector layers (`src/clip_vector.rs`: stroke layout read from the
  owner's files, radius = width x pressure fitted against CSP previews:
  Nightcrawler 5.26 -> 0.43 mean levels off, REnderPurple 5.18 -> 3.65 where
  CSP tapers low-pressure ends more). Open: CSP's size curves
  (`SizeEffector` with BrushEffectorGraphData), screentone layers (LayerType
  2), CSP layer masks, canvases over 4096 px (frame.clip, Girl_horns.clip).

- 2026-10-29 (Claude, canvas limit): 16,384 px per side, 100 MP area
  (`document::MAX_SIDE`, `MAX_PIXELS`, `SIZE_LIMIT`); every importer and the
  editor use the constants. Mask-aware payload budget raised to 2 GiB.
  `DUDEL_CAPTURE_SIDE=10000 --capture-large-canvas` checked fit and detail
  views. frame.clip and Girl_horns.clip (3000 x 4200) import and match their
  previews; ag-psd gradient-overlay (3508 x 4960) imports layered.

- 2026-10-29 (Claude, CSP tones + copy): Import details text is selectable
  (canvas Copy no longer steals Ctrl+C) with a Copy all button. Tone layers
  (LayerType 2 with EffectTone; line-and-tone conversion) import as black at
  the dot density through their CSP mask (1-byte mask packing now decoded,
  `clip_raster::decode_mask`). All 61 owner CSP files import; skipped: two
  text layers and CardBase's hidden LayerMasking=1 layer. Open: CSP masks on
  ordinary layers (no sample yet), screentone dots, text layers, `.psb`.

- 2026-10-29 (Claude, import round 3): `.psb` (psd-tools 199 -> 223, ag-psd
  60 -> 68 layered); vector edges follow the brush anti-alias level (all CSP
  vector files under 1 level off their previews); CMYK PSD/PSB through the
  embedded profile with moxcms (psd-tools 225, ag-psd 69; CMYK adjustments
  use the composite). Screentone dots not done: the only sample (2d.clip) is
  72 dpi with ~1 px tone cells, where flat shading is equivalent; the stored
  tone fields (72.0, 60.0, density) need a print-resolution sample to map
  lines/angle.

- 2026-10-30 (Claude, screentone dots): owner's SCREENTONE.clip (300 dpi, 85
  lines, 45 deg, 40%) mapped the EffectTone record (resolution, dot shape,
  density source, density, lines, render flag, angle, noise size, offsets).
  `src/screentone.rs` snaps cells to whole pixels and grows each cell's round
  dot to the density's share of its pixels; the import matches CSP's preview
  exactly (1,000,000 of 1,000,000 pixels, was a skipped layer). Only round
  dots at a set density with the observed flag and no offset are drawn;
  cells under 2 px (2d.clip) stay even shading, unchanged. Masks created
  showing everything now decode, and a tone's LayerMasking=1 is accepted
  only when its mask shows everything and was never painted (meaning of the
  bit still unknown). Open: more samples for densities over 50%, other dot
  shapes, offset grids, and 600 dpi cells.
- 2026-10-30 (Claude, hide/unhide speed): on 2d.clip (3840 x 3840, 26
  layers) hiding a full-canvas layer took ~5 s to appear: the display
  recomposited every tile at full resolution and showed nothing until all
  were done. Now (1) masks count when deciding a layer shows nothing in a
  tile, for display skipping and the compositor (Tone 6 toggle 4.4 s -> 10
  ms; Tone 4 touches 95 tiles, not 225); (2) tiles shown reduced (zoomed
  out on canvases of 2048 px and up) are first composited one pixel per
  texel and shown, then refined exactly in the background; a brush edit on
  the fast path drops the refinement (full-canvas toggle shown in ~0.55 s,
  exact after ~4.5 s); (3) the compositor walks 64-pixel strips layer by
  layer, checked equal to the per-pixel path on random documents. Prepare +
  history for a toggle is ~1.5 ms. Measure with the ignored
  `toggle_latency` test (`DUDEL_TOGGLE_FILE`). Open: full-resolution views
  (zoomed in) still recomposite at ~40 ns per layer per pixel; the blend
  math would need vectorizing.
- 2026-10-30 (Claude, CSP-TEST.clip from the owner): masks on raster layers
  (type 3 = raster with mask; the visibility 2 bit and LayerMasking 0/1
  accompany a mask, which CSP applies either way) import as Dudel masks;
  black and paper areas match the preview exactly. Millimetre canvases
  (unit 2, rounded to the nearest pixel) open the eight comic pages. Text
  layers: CSP does not save their pixels (the render block is missing), so
  they need Dudel's own text layout; still skipped. Correction layers: the
  tone curve (0.5 -> 0.667 on the master curve) is not Dudel's sRGB spline
  (155 vs CSP's 135 on the paper); a natural spline after a ~2.2 gamma
  gets within 1-2 levels, too few levels to confirm. Needs a gradient
  sample. Brightness/contrast and hue/saturation were at defaults.

- 2026-10-30 (Claude, owner's correction and tone samples): correction
  layers (type 4098, SpecialRenderType 13, FilterLayerInfo) import as Dudel
  adjustment layers. Tone curve: 32 curves x (count + 32 u16 pairs), the
  first the master; CSP's curve is a quadratic B-spline of its points (a
  Bezier for one middle point): Curves gained `bezier`. ToneCurve-TEST 0.46
  mean levels off. Brightness/contrast: out = (in + b - 128)/(1 - c) + 128,
  mapped exactly onto the legacy formula; 0.22 levels off. Hue/saturation/
  luminosity: whole-number HSV (hue 0..0x30000, saturation 0..0x10000,
  settings x 0x8000 / 100 truncated), as the new HueSaturationLuminosity
  adjustment, 0.25 levels off. Line (shape
  4) and square (shape 2) tones: square = two bands along the cell's sides,
  line = one band (the other side past 90 deg, scanned by columns); lines
  shrink up to three more times instead of the row-order cut, so 70% ends
  at 77 of 98 pixels (squares 66). Both match the owner's 1000 px samples
  pixel for pixel. Half-size previews are no use for tones: CSP re-renders
  them anti-aliased at the preview's resolution.
- 2026-10-30 (Claude, text layers): CSP does not save text pixels;
  `src/clip_text.rs` reads TextLayerAttributes' little-endian records (0x1F
  font full name, 0x20 size 1/100 pt, 0x21 direction, 0x22 color 3 x u32
  (top byte), 0x23 alignment 0/1/2 = left/right/center, 0x2A box, 0x3A
  rotation 1800 upright / 0 half turn, 0x40 laid-out quad in 1/100 px from
  the box's top-left (horizontal) or top-right (vertical), opposite corner
  when turned) and draws with ab_glyph and the named system font: no
  kerning, words wrap to the box, lines share the quad's thickness;
  horizontal baseline one descent above the line bottom, vertical baseline
  0.88 em from the line top, vertical Latin turned clockwise. FONT TEST
  (wrapped left-aligned, right-aligned, vertical; green) and CSP-TEST
  (centered, vertical turned) land within a pixel. Open: Card_Guisw's
  right-aligned line in a box shorter than the line is 3-4 px off; several
  styles in one layer still skip.

- 2026-10-31 (Claude, CardBase): its hidden "Layer 1 Copy" carries
  LayerMasking 1 with no mask; the bit does not turn masks off (CSP-TEST),
  so without a mask it is now accepted. CardBase imports with every layer,
  within one level of CSP's preview (256 pixels off by 1).

- 2026-10-29 (Claude, review follow-ups): fixed the six open review findings
  (see "Before pushing"), committed together with the review's own fixes.
  Corpora and pen testing not rerun.

- 2026-10-29 (Codex, independent review): inspected the 14 local commits with
  emphasis on the last five feature commits. Reproduced and fixed five bugs;
  open findings and verification scope are in REVIEW_2026-10-29.md. Default
  suite now has 980 passing tests, 8 ignored; all features has 989 passing,
  8 ignored. Final Full verification passed, including both release builds;
  the initial attempt hit access denied replacing the executable. Six open
  findings remain, including cache memory and old-build data loss. No files
  moved/deleted, no commit or push.

- 2026-10-29 (Claude, owner feedback + handoff): fx button moved into the layer
  action row, sparkle badge on layer rows, effects window auto-sizes, mask
  view (Mask actions > Show mask on canvas, Alt+click). Docs brought up to
  date; a review prompt was written for a new agent. Gates pass (974 tests).
  Committed locally, not pushed. Open: review findings, full Verify, owner
  pen pass, push approval.

- 2026-10-29 (Claude, adjustment layers): goal 8. Gates pass (973 tests).
  Committed locally, not pushed.

- 2026-10-29 (Claude, layer effects stage B): native editable effects (see goal
  7 stage B). Corpus numbers unchanged by the switch from baked to native.
  Gates pass (968 tests). Committed locally, not pushed. Next: PSD adjustment
  layers (owner's item 2).

- 2026-10-29 (Claude, layer effects stage A): `src/layer_effects.rs`, importer
  placement in `effect_layers` (psd_import.rs). Tools in the agent scratchpad:
  `fx_scan.py`/`fx_list.py` (which files use which effects), `fx_score.py`
  (corpus totals), `fx_region.py` (error only where each effect layer draws),
  `fx.sh` (stack, notes and side-by-side PNGs; needs `DUDEL_VERIFY_IMAGES`).
  Gates pass (961 tests). Committed locally, not pushed.

- 2026-10-28 (Claude, limits/blends/PSD): raised layer limits with matching
  memory budgets, added four blend modes incl. pass-through folders, PSD layer
  order detection (Frieren model), CSP corpus 30/30. Vector render-cache
  approach ruled out (CSP does not save it). Committed locally, not pushed.

- 2026-10-28 (Claude, CSP): skip-and-report for unsupported layers, saved
  animation frame from track keyframes, Gray layers, larger decode budgets.
  Corpus 9 → 29 of 30. Gates pass (936 tests). Committed locally, not pushed.

- 2026-10-28 (Claude, checkpoint): full Verify passed; fixed PSD layer order and
  group nesting; added saved-composite comparison; committed locally. Next: CSP
  import blockers (goal 5): skip-and-report unsupported layers instead of
  rejecting the file, then layer masks.

- 2026-10-28 (Claude, later): owner confirmed items 1-3 work by hand. Added the
  fitted half box and Ctrl snap for mirror pairs, the shortcut-list entry and
  QUICK_START text. Gates pass: fmt, Clippy, 930 tests, public-tree check.
- 2026-10-28 (Claude): diagnosed both 0.1.0 CI failures, fixed the benchmark path,
  fixed fmt/Clippy failures in the new code so local gates pass, and
  wrote this plan from the previous session's log, which ended mid-way through the
  docs update after hitting a usage limit.
