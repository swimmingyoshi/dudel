# Dudel 0.1.0 for Windows

A Windows illustration application. Release preparation is accepted using the
existing automated, performance and owner workflow evidence, within the limits
in [the release decision](RELEASE_DECISION.md). This document does not imply that
the package has been uploaded to a public store.

## Unreleased workflow and extension changes

- Selection options and the compact toolbar expose Transform selection directly.
  Overlapping mirror pairs split ownership along the mirror line.
- Guides settings use stable popup IDs and work after layer override changes
  with both mouse clicks and same-frame taps.
- New canvas has a live size preview, portrait/landscape/HD/A4 presets and rotation.
  Settings now opens a scrollable window with four categories.
- Extension API 2 contributes brushes, native tool presets, commands and panels,
  with capability validation, namespaced IDs and an SDK example. API 1 stays supported.
- CSP adds ordinary folders/clipping, standard blends, locks and Gray layers.
  Animation files open on their saved frame with every cel kept as a layer.
  Vector, text and other unsupported layers are skipped and listed instead of
  failing the import. Files that still fail offer an explicitly flattened
  saved-preview import. PSD RGB import supports bounded editable subsets and a
  reported saved-composite fallback.

- Documents hold up to 1,000 raster layers, 2,000 entries and ten folder levels
  (was 32 / 64 / six), with a 2 GiB stored-pixel budget. Such projects need this
  version to open.
- New blend modes: Dissolve, Color dodge (glow), Add (glow), and Pass through for
  folders. CSP and PSD imports map them.
- PSD import reads each file's own layer order. Some writers list layers top
  first; groups decide it, and flat files are checked against the saved image.
- Layer effects: **fx** in the layer properties adds editable drop and inner
  shadows, outer and inner glows, strokes, color and gradient overlays, and bevel
  and emboss. They follow the layer's pixels, save with the project, and appear in
  exports. Older Dudel builds open such projects without the effects.
- Adjustment layers: levels, curves, brightness/contrast, exposure,
  hue/saturation, vibrance, color balance, black & white, photo filter, channel
  mixer, selective color, gradient map, invert, posterize and threshold, from the
  half-circle button in the layer toolbar or from PSD files. They recolor what
  lies below them (or their clipping base), with a curves editor and sliders in
  the Adjustment window.
- PSD layer effects stay editable on ordinary layers, and become separate layers
  (like Photoshop's Create Layers) on shape and fill layers and folders:
  strokes, color and gradient overlays, drop and inner shadows, outer and inner
  glows, bevel and emboss, also on folders. Satin and pattern effects are listed
  in Import details instead.
- CSP import reads image material layers (placed pictures), layers that extend
  past the canvas, canvases set in inches or millimetres, layer masks on raster
  layers, and files with color proofing
  settings that are switched off. Vector layers are drawn from their strokes
  as raster layers instead of being skipped; Layers-panel color labels no
  longer skip layers. Screentone layers import with their dots drawn as CSP
  draws round dots (even shading for other dot shapes and very fine tones),
  with their CSP mask as an editable layer mask.
- Photoshop large documents (`.psb`) open like `.psd` files, and CMYK files open with
  their colors converted through the file's color profile. 16-bit files open with
  their layers instead of as one flattened layer.
- Canvases up to 16,384 px per side and 100 megapixels (10,000 x 10,000), up from
  4096 x 4096, for new canvases, resizing and CSP/PSD/PNG imports. Mask-aware
  projects share the full 2 GiB pixel budget. Import details text can be selected
  and copied, or copied whole with Copy all.
- Import details opens by itself only when a CSP or PSD import left something
  out; other notes (timeline frame, animation cels, drawing aids) are behind the
  Import details button in the status bar.

See [format limits](FORMAT_SUPPORT.md) and [API 2](plugins/API_2.md). These source
changes require rebuilt release artifacts and do not extend the historical
release approval to universal CSP/PSD compatibility or a complete plugin platform.

## Use

Extract the complete archive and run `dudel.exe`. See `QUICK_START.md` for drawing,
native project saving, recovery and PNG export. `BUILD.json` identifies the exact
executable, source inputs, toolchain and workspace state used by this package.
Dudel is **portable**: there is no installer. Extract the zip anywhere, including a
USB stick, and run `dudel.exe`. The Visual C++ runtime is built in, so nothing else
needs installing. Artwork goes to `Documents\Dudel` (inside OneDrive if Documents is
redirected). Settings and recovery snapshots stay in `%LOCALAPPDATA%\Dudel` on each PC.
To remove Dudel, delete its folder; your projects stay. Extract before running:
starting `dudel.exe` from inside the zip runs a temporary copy.

Downloads are not code-signed yet. Windows SmartScreen may show "Windows protected
your PC" on first launch: choose **More info > Run anyway**. Compare the zip's
SHA-256 with the published `.sha256` file to confirm the download is intact.

## Final 0.1.0 changes (2026-10-28)

- **Low latency drawing is on by default.** Ink no longer trails the pen by the
  two queued vsync frames the owner saw on the Artist 12. If a display tears,
  untick **View > Settings > Display > Graphics API and latency... > Low latency** and restart; a saved
  choice is kept.
- The brush toolbar keeps the same layout in Smooth and Rope mode. Follow mode,
  Rope distance and "Less smoothing on fast strokes" moved into the new
  **Smooth / Rope** button, so turning Rope on no longer pushes Snap and Symmetry
  out of view.
- The shipped binaries no longer contain the build machine's folder paths, and
  packaging refuses binaries or archives that do.

## Recent workflow fixes

- Dudel starts on PCs without Microsoft's Visual C++ runtime; it no longer fails
  with error 0xC0000135.
- Clicking a tool in the left rail always shows its settings, including on smaller
  screens where they were hidden automatically.
- Saving, opening and filter progress appear inside the status bar, so the bar and
  canvas no longer jump while they run.
- Canvases up to a full 4096 x 4096 (16.7 MP), enough for 4K artwork. This was 4.2 MP.
- The brush library, brush settings, color wheel, navigator and layers areas can be
  resized much further on small screens. Double-click a divider to reset it; sizes
  are remembered.
- Project tabs draw their close button inside the tab.
- The status bar shows "Backup saved …" while there are unsaved changes, so you can
  see that crash recovery has a copy. It is not a substitute for saving.
- Opening, replacing or closing a large project now returns the previous document's
  memory to Windows once the app settles. Idle memory no longer rises by up to
  ~450 MiB after reopening a 32-layer project. The two-hour maximum-document study
  now passes.
- Transform dragging uses the full layer compositor when blending, clipping,
  translucency, masks, upper layers or visible references make a separate mesh
  misleading. Simple topmost Normal layers retain the instant mesh preview.
- Transform properties scroll above Color in short panels, preventing help text
  from painting across the color controls. Apply/Cancel remain in the toolbar.
- View > Performance shows tracked CPU allocations across all open projects,
  including parked documents and shared reference images, plus separate RGBA
  texture estimates. The window scrolls on small screens; these figures are
  allocation estimates rather than total process memory or committed VRAM.
- [Current release status](RELEASE_STATUS.md) separates source improvements,
  packaged candidates and the remaining acceptance gates.
- Save as refuses a destination owned by another open project tab, preserving
  that project's file and both tabs' editable state.
- Global symmetry and ruler/grid snapping stay with their project when switching
  or closing tabs; navigation alone does not create unsaved guide changes.
- Brush preview cache checks hash dynamics directly, avoiding repeated JSON
  allocation. This is a component optimization, not a drawing-latency claim.
- The audience preview's Workspace background follows the customized canvas
  surround color.

Use the [candidate workflow checklist](CANDIDATE_TEST_CHECKLIST.md) for a short
Artist 12 exercise before reporting feedback.

## Stable adjustment dialogs

Brightness/Contrast, Hue/Saturation and Levels keep their controls and preview
status area in place while updates run. Slider adjustments no longer change the
centered dialog's height, and the adjustment popup opens without a fade effect.
Controls remain visible while the initial preview prepares; Apply waits for the
current preview. Short windows retain scrolling and reachable action buttons.

## Bucket edge fill

The bucket now enables **Fill to edges** by default. It replaces the background
contribution at soft pencil/pen borders rather than leaving a pale seam or
painting over the outline. It supports transparent and flattened linework,
separate reference layers, selections, opacity and Undo/Redo. Use **Fill options**
in the toolbar for reference choice, edge fill, gap closing and expansion.

Windows candidates also include `RunPerformanceStudy.ps1` for an unattended,
build-identified DX12 study. See the packaged README for prerequisites and the
one-command invocation. It is an engineering check, not release certification.

## Current boundaries

- One active document. Dimensions 1-4096 per side, up to 4096 x 4096 (16,777,216 pixels) for 4K artwork;
  up to 32 raster layers and 64 total layer/folder entries. Mask-aware projects
  share a 512 MiB raster-and-layer-mask payload budget.
- Native `.dudel` retains editable artwork; mask-aware projects need a compatible
  reader. PNG export is a flattened sharing copy. Indexed `.didx` is experimental.
- CSP import supports a restricted subset. No general CSP/PSD compatibility or
  color-managed print fidelity is claimed.
- The October 21 sustained study passed. The owner confirms working Artist 12
  use and Explorer previews on October 28. Precise physical latency, actual GPU
  commitment and some native OS workflow details remain unclaimed; see current
  release status for their scope and the evidence carried forward.
- Dudel uses GPL-3.0-only. Paid executable downloads are planned; matching source
  remains available to build yourself. See `DISTRIBUTION.md`, `SOURCE.md`,
  `LICENSE` and `THIRD_PARTY_NOTICES.md` in the complete package.
- Initial unsigned distribution is approved. Reviewed dependency notices are
  retained; a universal minimum Windows/GPU specification is not claimed.
  `DEPENDENCIES.json` is an inventory; actual collected
  terms and provenance are indexed under `third-party-licenses/`.

## Tested systems and prerequisites

The Windows candidate is an x64 portable ZIP. Extract the complete folder before
running it, and keep `dudel_thumbnails.dll` beside `dudel.exe` for optional Explorer
previews. A separate Visual C++ runtime installation is not required. Python 3.11+
is needed only for the optional engineering checks, not for drawing.

These are tested configurations, not minimum specifications or blanket acceptance
of the current candidate:

| Configuration | Evidence and remaining limits |
| --- | --- |
| Windows 11 Pro 26200; Ryzen 5 3600X; approximately 40 GiB RAM; RTX 3060, driver 32.0.15.9579 | October 21 DX12 standard and two-hour maximum resource study passed on candidate `20261021-190309-093`. Current source `0d8e429` passes local automated checks and reviewed DX12/OpenGL captures. Changed rendering/timelapse resource behavior still needs acceptance measurements. |
| Windows 11 Pro 26200; Core i5-10310U; 15.7 GB RAM; Intel UHD, driver 31.0.101.2114 | October 22 laptop round 2 (`20261022-170440-814`) passed launch, OpenGL/DX12 diagnostic sessions, save/reopen, recovery and portable use with the Visual C++ runtime uninstalled. This was a second Windows installation with other software installed, not a pristine machine. These remain historical compatibility results; the current candidate is being validated on the reference PC. No repeat laptop study is scheduled. |
| XP-Pen Artist 12 | Owner confirms on October 28 that it works and has been tested repeatedly during development. This is accepted qualitative usability feedback; exact driver/scaling metadata and measured pen latency are not inferred. No repeat tablet session is scheduled. |
| Ubuntu 24.04 hosted CI | Build/tests and X11 OpenGL startup checks pass on `0d8e429`. Linux remains experimental; physical tablet, Wayland and distribution acceptance are pending. |

Minimum Windows version, RAM and GPU requirements have not been established.
Windows 10, Windows ARM and other tablet models have no release acceptance in this
record. The 512 MiB document payload limit is not the application's total RAM
requirement; history, display resources and background file jobs use additional
memory. A successful small-project startup does not establish dense-4K performance.

Detailed evidence: [current status](RELEASE_STATUS.md),
[desktop resource study](benchmarks/heap-compaction-study-20261021.json),
[laptop results](research/CLEANPC_LAPTOP_2026-10-22.md) and
[tablet validation](XP_PEN_VALIDATION.md).

## Report a defect

Record BUILD.json's version/hash, Windows version, GPU/driver, tablet/driver,
display scaling, canvas/layer sizes, brush, steps, expected/actual behavior and
whether it reproduces on a fresh disposable canvas. Attach a minimal fixture only
when it contains no private artwork. Keep original projects and recovery copies.

See [file format support](FORMAT_SUPPORT.md) for the native-version and restricted CSP import matrix.

## Creator tools update

Adds opt-in disk-backed artwork timelapse with pause/resume, reopening recordings and cancellable animated GIF export; persistent built-in/imported UI themes; compact reference selection and within-group ordering; and four textured/paint brush presets. Adds a plugin packaging/validation SDK and package sharing from the extension manager. Theme setup is cached and offscreen brush previews are skipped. Quick development verification is now the default; hosted Windows release checks explicitly retain Full mode. See CREATOR_FEATURES.md, PLUGIN_DEVELOPMENT.md and TESTING.md. Native latency, long recording sessions and Linux runtime acceptance remain separate gates.
