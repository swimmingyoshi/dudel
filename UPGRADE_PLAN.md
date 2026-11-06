# Dudel 0.2 upgrade plan and handoff

Updated 2026-11-03. This file is the working checklist for the post-0.1.0 work.
The current review follow-ups and their verification are tracked in
[STABILIZATION_PLAN.md](STABILIZATION_PLAN.md). Dated results below remain
historical evidence for the builds they identify.
Any agent picking this up: read this first, update the checkboxes and the
"Last session" notes as you go, and keep [DELIVERY_PLAN.md](DELIVERY_PLAN.md) for
0.1 history only.

## Base state

- 0.1.0 source: tag `v0.1.0`, on the release records of the rebuild from `bffcbea`,
  published as a GitHub pre-release on 2026-11-05 (see Last session).
- Determine unpublished work with `git log --oneline origin/master..HEAD` against
  the locally known remote ref; the dated handoffs below are not a live push status.
  Pushing starts hosted CI; ask the owner first.
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
- [x] One hosted Windows run on the 2026-11-04 rebuild of 0.1.0 passed (`e20c76b`,
      run 41948291941). Hosted Linux did not run: the `linux` branch was not updated

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
- [x] Audit other `MenuButton`s whose preceding layout can change (same bug class).
      Found one: the toolbar Symmetry button named the mode, so choosing a mode
      widened the bar, collapsed it to compact and closed the popup (940-1030 and
      1420-1470 px windows). The inline label is now fixed; the mode is in its
      tooltip (`toolbar_symmetry_popup_stays_open_while_its_label_grows`). Fill
      options, Smooth/Rope and Color stay open across 640-1700 px. Other
      `menu_button`s close on any click or are hover submenus.
- [x] Manual check with mouse and pen (owner, 2026-11-03)

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

- [x] Plugin manager UI: list, enable/disable, uninstall, show capabilities and errors
      (Filter > Manage extensions; capabilities are described in words, raw IDs on hover)
- [x] Permission prompt on install showing declared capabilities; replacements
      mark capabilities the installed release lacked
      (`installing_asks_first_and_lists_what_the_package_can_do`)
- [x] Scripted tools (2026-11-02): tool kind `script` (capability `tools.script`),
      Rhai `begin`/`point`/`end` return points on tracks drawn by native samplers
      with the current brush into one stroke (`src/scripted_tool.rs`); example
      `plugins/example-scripted-tools.dudelplugin` (source `plugins/sdk/scripted-tools/`:
      parallel lines, hatching, coil, hatching eraser); `plugin_tool.py` inlines
      `source_file`. Owner pen check passed 2026-11-03
- [x] Brush asset contributions (2026-11-03): brushes name `image_file`/`grain_file`
      PNGs; `plugin_tool.py` converts them with `dudel.exe --brush-texture tip|grain`
      (same conversion as the brush settings import) and times out on an older exe;
      example `plugins/example-textured-brushes.dudelplugin` (leaf scatter, grainy
      pencil, chalk; source and generated PNGs in `plugins/sdk/textured-brushes/`)
- [ ] More panel widgets; decide the stable widget list before 0.2
- [x] Load/unload without restart: install, enable, disable and uninstall apply at once
- [x] Owner test (2026-11-01): the example's "Invert selection" was a color filter.
      Added native Select > Invert selection (Ctrl+Shift+I, soft edges kept, undoable)
      and host command `invert_selection`; example renamed to "Invert colors", 1.1.0.
- [x] Installed filters (API 1 packages, API 2 filter commands) are Filter menu items;
      loose .rhai/.wasm files moved to Filter > Run filter from file
- [ ] Update/version conflict handling (releases are free text, so no ordering yet)
- [x] Limit tests for malformed API 2 manifests (size, depth, counts): all held
      (`malformed_manifests_are_refused_at_their_limits_without_panicking`)

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
- [x] **Vector layers** (2026-10-29, `src/clip_vector.rs`): CSP stores strokes, not
      pixels; Dudel reads them and draws them as raster pixels (radius = width x
      pressure, edge by anti-alias level). Rechecked 2026-11-03: all 69 owner CSP
      files import; REnderPurple (the only full-size preview) 0.69 mean levels off.
      The only skipped layer left is Animation.clip's "Story Information" text.
      Follow-up 2026-11-03: native vector recordings and editable CSP strokes
      landed through `7a30db3`, using project v7. CSP's SizeEffector end tapering
      remains outside the documented subset.
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
- [x] Performance with many layers (2026-10-31, measured in the release
      build at 880-990 painted layers on 2048 x 2048): adding a layer grew to
      ~67 ms as the undo history filled, because every checkpoint and settle
      re-measured history memory by hashing every tile of every layer of
      every snapshot (SipHash). `snapshot_memory::Registry` now skips a layer
      storage it has already counted (storage is copy-on-write) and hashes
      addresses with a multiplicative hasher: ~6 ms with 90 snapshots. The
      Layers panel laid out all rows every frame (~10 ms of a 12-18 ms
      frame); rows off screen now only reserve their height (~1.3 ms, frames
      3-6 ms). Thumbnails are keyed by layer id, so inserting below many
      layers no longer shifts and redraws all of them. The strip compositor
      skips empty layers whose zero source changes nothing (clipped, or
      right after another empty unclipped layer at the same depth): a full
      2048 x 2048 recomposite of 990 small layers 10.2 s -> 0.95 s. Hide,
      undo and select stay under ~13 ms per frame. Open: `Document::validate`
      and the document clone in `Edit::prepare` (~1.5-2 ms at 1,000 layers);
      blend math for dense layers is still scalar.
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
- [x] PSD export: adjustment layers (2026-10-31, `adjustment::to_psd`, the
      inverse of `from_psd`, layouts per psd-tools and blocks dumped from the
      public set): all 16 kinds; `CgEd` + zeroed `brit`, `Crv ` tail, `Lvls`
      omitted, gradient-map tail, 4-row `mixr`, `phfl` v2 RGB. Hue ranges
      keep their order (overlaps apply in order). CSP tone curves: 9 samples
      (within 1-2 levels); CSP HSL as hue2 master (up to 20 levels on the
      +60/-40/+20 sample). Measured: hue exact, darkening within 1, but CSP's
      HSV saturation (+-40: up to 62) and lightening (+20: up to 39) have no
      Photoshop form; two layers or fitted values gain nothing (mean 3.8 ->
      3.5, max 26 either way), so it stays one editable layer. Public
      set: every adjustment file reopens exact except grayscale exposure
      (gamma 1.75 vs RGB 2.2).
- [x] PSD export: editable effects (2026-10-31, `layer_effects::to_psd`):
      `lfx2` in Photoshop's compact form (Scl, masterFXSwitch, present
      effects; several of a kind as `*Multi`, first on top), key sets copied
      from Photoshop files. Layers Photoshop cannot show (glow blend modes,
      gradient glows/shadows, two glows or bevels of a kind) fall back to
      effect layers with a note. File > Export PSD for Live2D keeps the
      effect-layer form (Cubism ignores styles). Public set: every effect file
      reopens with the same effects and an exact render; psd-tools reads them.
- [x] Disabled effects (2026-11-02): switched-off effects marked `present`,
      and all effects under a master switch that is off, import switched off
      (`Effects::disabled`) on layers and folders whose effects stay editable.
      Public sets: no pixel changes in 360 files; ag-psd effects/src.psd and
      psd-tools effects-enabled.psd now keep theirs
- [ ] PSD export: mask link state (not stored; masks reopen linked), PSB for
      files over 4 GB of layer data, 16-bit export
- [x] PSD export opens in Live2D Cubism Editor 5.2 (owner, 2026-10-31: Frieren
      export and a v2 re-export). Cubism needs ResolutionInfo, cannot read
      ZIP layer channels, and reads records bottom first; see FORMAT_SUPPORT.
- [x] PSD export opens in Affinity and Photopea (owner, 2026-10-31)
- [ ] PSD export: owner check in Krita (on Linux) and Photoshop if available
- [x] Export speed (2026-11-02): Frieren 460 -> ~215 ms (byte-identical
      file). Layers and the merged image are RLE-encoded in parallel, and
      full composites split into several row bands a core, so the busy
      middle of a character file no longer holds up the rest (343 -> 174 ms)

Docs updated: [FORMAT_SUPPORT.md](FORMAT_SUPPORT.md), [RELEASE_NOTES.md](RELEASE_NOTES.md).

### 6. UI polish

- [x] Settings: one scrollable window with Workspace, Drawing, Display, Storage
      pages (`src/settings_ui.rs`), replacing the Settings menu
- [x] New canvas: live size/proportion preview, portrait/landscape/HD/A4 presets,
      swap orientation (`src/new_canvas.rs`)
- [x] Symmetry/guides popups: layout, grouping, spacing (2026-11-01): Symmetry and
      Guides sections, modes in an even 3-column grid, labels left of sliders, framed
      buttons in the toolbar menu too, popup up to 560 px tall. Captures:
      `--capture-symmetry`, `--capture-layer-guides` now open the popups. Icons not done.
- [x] Other popups and dialogs (2026-11-01): hints use a readable Small style (12.5 pt,
      was egui's 9 pt); adjustment and Layer Effects sliders put labels first in a
      fixed column (`workspace::labeled_slider`). Button order was already action
      first, Cancel last. Left alone: some tool windows can collapse, Settings cannot.
- [x] New canvas: custom preset save, DPI/print size, recent sizes (2026-11-01).
      "Save as preset" names the size and DPI (12 max, right-click removes); the last
      5 created sizes show under Recent; both persist in preferences (`new_canvas`).
      Print size sits beside DPI (cm, inches on hover). `--capture-new-canvas-presets`.
- [x] Settings: search, reset-to-default per page (2026-11-01). Search matches labels
      and keywords across pages ("wacom", "vsync", "plugins"); each page has a confirmed
      "Reset this page" that leaves other pages alone.
- [x] Screenshot pass at 100% / 150% / 200% scale (2026-11-01): ten dialogs captured
      at each scale; all fit or scroll, no clipping beyond the fixes above.

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
- [x] PSD import keeps supported effects native on raster layers (no separate
      shape source), including reduced fill, and on folders with fill 255.
      Separate shape/fill outlines and reduced-fill folder effects still bake.
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
- [x] Draw effects in the background (2026-11-01): the canvas uses
      `layer_effects::expand_for_display`; an out-of-date layer shows its previous
      drawing while `background::spawn` redraws it (one per layer state; newer states
      cancel older), then repaints. The cache no longer holds its lock while drawing.
      Status bar shows "Drawing layer effects". Stroke end on a 4096^2 drop-shadow
      layer: 649 ms blocking -> 0.1 ms (`probe_stroke_end_time_with_a_drop_shadow`).
      Exports and thumbnails still draw synchronously.
- [x] Redraw only around what changed (2026-11-01, owner feedback: slow to appear,
      undo slow, canvas twitch). Effects are cached per effect (`Part`), so opacity,
      blend, visibility and name only re-dress cached layers; unchanged effects are
      reused when one changes; changed pixels redraw each effect only within its
      `influence` (`redraw_windows`, whole when that covers half the layer). Three
      recent states per layer stay cached, so undo is instant. One background
      drawing per layer, newest state next, so effects follow the stroke. 4096^2
      drop shadow after a small edit: 649 ms -> 0.36 ms. The status spinner was
      touch-sized and grew the bar (the twitch); now 16 px
- [x] Large brushes: painting gets 8 ms a frame instead of 2 while 3+ dabs are
      queued (`PAINT_CATCH_UP_DABS`), 4x the catch-up rate. A 300 px dab costs
      ~3 ms (`probe_large_brush_stroke`): blend ~1.4, accumulate ~0.7, coverage
      ~0.5 (the full per-pixel coverage, with grain and flow, ~1.5).
      Large dabs now paint in 32-row bands across cores (`dab_bands`), each
      band owning its tile rows and stroke blocks; the queue paints such a dab
      as one unit. The pool wakes helpers together and lingers 50 us. 300 px:
      ~3 -> ~1 ms per dab; 100 px: 0.58 -> 0.18 ms
- [x] Effects on folders natively (2026-11-02): they follow the folder's
      composited contents (children with their own effects), kept per
      folder and recomposited only where children's pixels changed, so the
      effect cache redraws around changes as for layers. Under-effects go
      beneath the contents, inner ones clip to the folder; hidden folders
      skip drawing. Canvas draws them in the background; PSD import keeps
      them native (fill 255), export writes them as styles; Merge folder
      keeps them on the merged layer
- [x] Fill opacity (2026-11-02): `Layer.fill` fades a layer's own pixels,
      not its effects. Folded into opacity when the document is expanded
      (`needs_expansion`), so the compositor is unchanged; effects draw with
      it (shadow knockout, inner effects carry its alpha). Fill slider in
      the Layer Effects window (`Property::Fill`, same gesture as effects).
      Saved in projects and packages; PSD `iOpa` read and written natively,
      except the eight modes where Photoshop draws fill differently
- [x] Fill below 100% with inside effects (2026-11-02 stabilization): combine
      the layer and inside effects in layer space, then apply source alpha and
      mask once. The cached combined raster replaces the displayed owner;
      local pixel edits update affected regions and count toward effect memory.
      Baked PSD export keeps fill baked rather than applying it twice. Pixel,
      cache, save/load and PSD round-trip regressions pass; final verification
      is recorded in [STABILIZATION_PLAN.md](STABILIZATION_PLAN.md).
- [x] Satin (2026-11-02): |blur(shape moved +offset) - blur(shape moved
      -offset)|, inverted by default, clipped inside; in the Add effect menu,
      read from and written to PSD (`ChFX`); contours other than linear are
      noted as approximated
- [x] Pattern Overlay and pattern strokes (2026-11-02): `Paint::Pattern`
      (tile, scale, phase from the canvas origin), bilinear and premultiplied;
      `pattern.rs` keeps tiles up to 1024 px, saved in projects as base64 PNG.
      PSD: the global pattern table (`Patt`/`Pat2`/`Pat3`, RGB/gray, raw/RLE)
      is read and effects find patterns by id; export writes `Patt` and
      `patternFill`. ag-psd's pattern case decodes exactly and renders within
      3.2 levels mean of Photoshop's composite. New overlays start with gray
      dots; Load image… picks a picture. Bevel textures are still skipped
- [x] Gradient editor (2026-11-02): preview bar over a checkerboard, style
      (linear, radial, angle, reflected, diamond), reverse, align with layer,
      angle, scale, and color and opacity stops (add at the widest gap,
      move, recolor, remove); edits survive PSD as before
- [x] Effect-layer memory counts in each project's tracked memory: the
      snapshot registry visits cached drawings' tiles (shared states once);
      the Performance window also shows the global effect cache total
- [x] Manual check in the app with pen (owner, 2026-11-03)
- [x] PSD export writes them back as editable effects (see goal 5)

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
- [x] Adjustment UI follow-ups (2026-11-02): hue/saturation edits Master or
      one of the six color ranges (changed ones marked; PSD ranges written
      in the slot named for their hues), gradient map stop editing with the
      effects window's bar and stop rows, Layer > New adjustment layer and
      Layer effects, a histogram in levels of what shows below the layer
      (sampled in the background, `Document::histogram_below`)
- [x] Masks (2026-10-29): user and vector mask density and feather, and the
      separate "real" user mask (channel -3) when a vector mask exists; PSD
      notes grouped by kind with layer names. psd-tools 181 -> 193/279
- [x] PSD fallbacks (2026-11-02): 32-bit files (Lr32, float prediction,
      linear samples and setting colors to sRGB, Mt32 merged transparency),
      Lab, indexed, bitmap, duotone and multichannel modes, CMYK and Lab
      setting colors, artboards (folder mask + background layer), pattern
      fill layers; gradient ramps now span edge to edge along their angle
      (20 public files closer to Photoshop, 2 within 0.1). Public sets: ag-psd
      83/89, psd-tools 273/316 open without falling back (PSD and PSB)
- [ ] Remaining PSD fallbacks: knockout `knko` (9), CMYK adjustment layers
      (8; would need adjustments applied in CMYK), noise gradients (4), mask
      flags (3), stroke widths (4), dashed strokes (3), `lmgm`/`clbl`/`brst`/
      Blend If; 32-bit layers blend in sRGB where Photoshop uses linear light;
      a reflected gradient on the 4x4 psd-tools files still differs; pattern
      fill phase is off by a pixel on some tiles of psd-tools patterns.psd

### 9. Vector layers (owner goal, 2026-11-03)

- [x] Strokes kept as recordings and redrawn exactly; transforms move strokes;
      partial eraser, restyle; CSP vector layers import as editable strokes
- [x] Eraser "Up to crossings"; Pointer picks and lassoes single lines, with move,
      reshape (Reach), size, taper, smooth, recolor and delete, each one undo step
- [x] F resizes the brush or Reach (`src/quick_resize.rs`); new layers in the
      Layers panel right-click menu (`src/layer_menu.rs`). Owner confirmed F resize
- [x] Line actions in command search; layer filter and multi-selection are per tab
- [x] Owner's pen pass of the vector workflow (2026-11-04), before the items below
- [x] Fill stops at line centres; selections limit vector lines and the vector
      eraser by cutting lines to what fits; handles scale and rotate picked lines;
      SVG export (`src/svg_export.rs`), with exports moved to File > Export
- Known limits: scattered or mirrored lines refuse selections. SVG leaves out
  raster layers, masks, effects and brush texture. PSD export writes rendered
  pixels, because the format has no editable brush strokes.

### 10. Text tool (0.2 candidate, 2026-11-05)

Design, stages and the owner's open decisions are in
[TEXT_TOOL_PLAN.md](TEXT_TOOL_PLAN.md).

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
- [x] Owner's manual pen pass (2026-11-03): Layer Effects window and badges, mask view,
      PSDs with effects and adjustment layers, settings window
- [x] Owner approved the push (2026-11-03, b7492d8). Its hosted run failed only on
      a racy test (background drawing finishing before the first display call);
      the test now accepts either order

Earlier batch (2026-10-28): gates and `Verify.ps1 -Mode Full` passed; owner's
tablet pass for items 1-3; `psd_import.rs` and `extension_api.rs` reviewed.

## Last session

- 2026-11-05 (Claude, launch prep): 0.1.0 is a public pre-release, but nobody
  outside the project has tried it. Added TESTER_GUIDE.md and a feedback form, and
  updated the GitHub description, topics and Discussions. Text tool designed in
  TEXT_TOOL_PLAN.md; no code yet. `origin/linux` has two packaging commits not on
  master, including the LINUX_PORT.md update.

- 2026-11-05 (Claude, 0.1.0 rebuild): pushed `e20c76b`, the owner moved `v0.1.0`,
  and hosted Windows CI passed. The owner's pen pass of selection limits, fill to
  line centres, line handles and SVG export passed on 2026-11-05. The draft GitHub
  release has the rebuilt ZIPs; only publishing is left.

- 2026-10-31 (Claude, many-layer speed): see the Performance item in goal 5.
  Owner chose 0.1.0 (rebuilt from current code) for the first public release,
  after this speed work. Hosted CI was refused for billing (account payment
  or spending limit), so local `scripts/Verify.ps1 -Mode Full` is the evidence.

- 2026-10-31 (Claude, editable effects): layer styles exported, Live2D
  menu entry for effect layers, `--roundtrip-psd-effect-layers`. CSP HSL
  options measured (two layers or fitted values gain nothing). Owner asked
  to push when ready and then recreate the releases (never went public).

- 2026-10-31 (Claude, PSD adjustment export): adjustment layers written back
  (see goal 5); `psd_vector::encode` now ships (was test-only). The round-trip
  report compares adjustments by effect on 216 sample colors. Owner confirmed
  Cubism, Affinity and Photopea open exports.

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
