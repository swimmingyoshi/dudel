# Dudel 0.2 upgrade plan and handoff

Updated 2026-10-28. This file is the working checklist for the post-0.1.0 work.
Any agent picking this up: read this first, update the checkboxes and the
"Last session" notes as you go, and keep [DELIVERY_PLAN.md](DELIVERY_PLAN.md) for
0.1 history only.

## Base state

- Released source: commit `67716b6` "Dudel 0.1.0", tag `v0.1.0` (master and linux).
- All work below is **uncommitted** in the working tree on top of `67716b6`.
- Local gates (see [TESTING.md](TESTING.md)): `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`,
  `python scripts/check_public_tree.py`, then `scripts/Verify.ps1` for the full set.
- Hosted CI minutes are scarce. Verify locally, batch pushes, run hosted CI once per
  final candidate. Docs-only commits use `[skip ci]`.

## CI status for 0.1.0 (checked 2026-10-28)

Both hosted runs on `67716b6` failed, for reasons outside the app code:

| Run | Result | Cause | Status |
| --- | --- | --- | --- |
| Windows verification | 918 tests, Clippy and Python tests passed; failed at `check_public_tree.py` | `benchmarks/linux-ci-introduction-windows-20261018.json` held the CI runner's checkout path, which the public-tree rule flags only on the runner | Fixed in working tree (`<repo>` placeholder, like the other benchmark files) |
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
- [ ] Layer effects: see goal 7.
- [ ] Known vector differences: miter joins drawn round; stroke AA approximate;
      Photoshop strokes under intersect/subtract path ops differ (psd-tools
      path-operations/* files); color burn / hard mix / vivid light layer
      tests differ (~1500 px) - check fill-opacity/special-8 handling
- [ ] Remaining PSD gaps by frequency in those sets: vector shape layers (~70,
      need vector rendering; Photoshop marks their pixels irrelevant), adjustment
      and fill layers (~40), layer effects lfx2/lfxs (~30: owner decision whether
      to import without effects + note, or keep falling back), grayscale/CMYK/
      indexed/32-bit modes (~60; grayscale is cheap and also covers the Live2D
      tool's _depth.psd files), mask feather/density (~13), 16-bit layers (Lr16)
- [ ] Owner's Live2D recovery outputs: all open; earlier "10% exact" was the
      comparison counting invisible colors, not an import problem
- [ ] Test against PSDs saved by Krita
- [ ] Performance: adding a layer costs ~2 ms at 100 layers, ~28 ms at 500,
      ~53 ms near 1,000 (release). Find the per-layer work done on every edit.
- [ ] PSB, CMYK/grayscale, 16-bit layered, pass-through groups, adjustment layers
- [ ] PSD export (none yet)

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

- [ ] Data model: an ordered effect list per layer, saved in the project
      format (new version, old files unchanged)
- [ ] Compositor draws effects live from the same renderers, cached per layer
      and invalidated by edits, so painting stays fast
- [ ] Layer panel UI: add/remove/reorder/toggle, settings popups, undo
- [ ] PSD import keeps effects as effects (stage A layers become an option)
- [ ] PSD export writes them back (once PSD export exists)

## Before committing this batch

- [x] Local gates pass (fmt, Clippy, 927 tests, public-tree check) - 2026-10-28,
      after fixing formatting and 5 Clippy errors in `psd_import.rs`/`extension_api.rs`
- [x] `scripts/Verify.ps1 -Mode Full` passed (2026-10-28)
- [x] Manual tablet pass for items 1-3 (owner); settings window not yet checked
- [x] Reviewed `src/psd_import.rs` (found the layer-order bug) and `src/extension_api.rs`
- [x] Committed locally: CI evidence fix, then the feature batch (shared files
      like `main.rs` made per-feature commits unbuildable). Not pushed.

## Last session

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
