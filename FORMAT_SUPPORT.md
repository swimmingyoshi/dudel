# File format support

Use `.dudel` for editable projects and PNG for sharing flattened artwork.
This is the Windows 0.1 support scope; see [the release decision](RELEASE_DECISION.md).

| Format | Open/import | Save/export | What is retained |
| --- | --- | --- | --- |
| Dudel `.dudel` | Yes, including supported older native readers | Yes | Editable layers, folders, masks and document state supported by this build |
| PNG | Image import | Flattened opaque or transparent export | Pixels; no editable layer hierarchy |
| Clip Studio Paint `.clip` | Restricted raster subset below | No | Only features accepted by strict preflight |
| JPEG, WebP, BMP, GIF images | Drop or paste copied files as raster layers | No source-format export | Static decoded image, scaled down to fit the current canvas; no animation editing |
| Project package `.zip` | Yes | Yes | Exact embedded project plus editable layer/mask PNGs, folder paths, references, preview and manifest |
| Theme `.dudeltheme` | Yes | Yes | Workspace theme colors; separate from artwork |
| Photoshop `.psd` | No | No | Not supported |
| Indexed `.didx` | Experimental opt-in | Experimental opt-in | Development format; use ordinary `.dudel` for normal work |

Native saves use v4 unless layer masks require v5. Opening v5 projects requires
a mask-aware Dudel build; do not assume an older executable can read a newer file.
The supported document envelope is 1-4096 pixels per side, at most 16,777,216 pixels
in total (a full 4096 x 4096), 32 raster layers, 64 total layer/folder entries and folder
depth six. Mask-aware projects have a combined 512 MiB raster-and-layer-mask payload budget.
These limits also apply to PNG and clipboard imports. Restricted CSP import keeps its
tested 4,194,304-pixel limit. Builds before 2026-10-22 accept at most 4,194,304 pixels and
cannot open larger projects.

## Clip Studio Paint import

Choose the CSP raster import action. A successful import opens an **unsaved copy**;
save it as a new `.dudel` project. Dudel does not write `.clip` files and protects
CSP sources from being overwritten by native saves.

The reader accepts only the observed static, pixel-unit raster representation:
resolved supported raster tiles, Normal blending, visibility, layer opacity,
names and positions, with supported plain paper metadata where present. Acceptance
depends on the file's complete metadata and tile layout, not just its visible image.
There is no general promise of compatibility with a CSP version.

| Source feature | Current import behavior |
| --- | --- |
| Supported raster channels, partial alpha and offsets | Converted into editable raster layers |
| Ordinary folders, nested folders or clipping layers | Rejected pending controlled CSP rendering comparisons |
| Non-Normal blending, layer masks, lock or palette metadata | Rejected |
| Text, vector, material, gradient or effect payloads | Rejected |
| Animation/timelines | Rejected |
| Source profiles, proofing or color adjustments | Rejected; no color-managed print fidelity claim |
| Unsupported rulers/guides/light-table metadata | Rejected |
| Missing raster references, damaged blocks or out-of-limit documents | Rejected; missing content is not assumed blank |

Unsupported content produces an error identifying the category or source field.
A failed, canceled or stale background import preserves the current document.
For unsupported artwork, export a flattened PNG from CSP and import that PNG;
retain the original `.clip` for editing in CSP.

Developer read-only comparison tools can study additional features with explicit
omissions. Their results do not make those features eligible for editor import.

## Verification scope

The current source has generated container, tile, preflight and background-load
regressions. The latest focused audit passed 42 format tests and five load-job tests;
`benchmarks/format-support-audit.json` identifies the test executable and logs.
These tests establish the checked conversions and rejections, not independent CSP
rendering parity. Controlled matching CSP/PNG fixtures for folders and clipping
remain follow-up work before those currently rejected features can be supported.
The bounded first-release decision is recorded in `RELEASE_DECISION.md`.

## Image layers and reference boards

File > Import > PNG as canvas creates a document from PNG pixels. Dropping a PNG,
JPEG, WebP, BMP or GIF, or pasting image files copied from a file manager, instead
adds raster layers to the current project. Incoming layer images are bounded to
16,384 pixels per side and 67,108,864 pixels total; larger-than-canvas images are scaled
down to fit. This exchange path is distinct from the supported document envelope.
GIF is decoded as a static image here. Timelapse export writes a separate GIF;
Dudel does not reopen that file as an editable animation timeline.

Reference boards accept added PNG files, up to twelve images per board and eight
boards per project. New imports are reduced to a 1536-pixel longest edge and share
a 128 MiB RGBA import budget. Placements, folder membership and order are saved
with the project. REFS1 preserves legacy single-image above/below placements;
REFS2 marks multi-image or explicitly ordered boards. Older readers reject REFS2
instead of silently dropping its placement data. References do not enter normal
artwork exports or timelapse.

## Portable project packages

File > Export project package writes a ZIP with project.dudel, manifest.json, a
flattened preview, reference images, and layer/mask PNGs in directories matching
the layer folders. Import validates and stages a new project in the configured
Projects folder and never replaces an existing project.

Keep project.dudel for an exact native restore. Removing that entry selects
reconstruction from edited layer PNGs and manifest metadata. Version 2 manifests
retain masks (including enabled/link state), layer locks, folder collapse, guides
and reference-board metadata/placements. Keep PNG canvas dimensions and paths.
This route does not recreate native data absent from that manifest, such as the
active selection. Older manifests without reference placement metadata require
their embedded project to preserve references. This is an interchange ZIP,
not a general archive extractor.
