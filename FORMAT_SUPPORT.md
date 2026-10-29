# File format support

Use `.dudel` for editable projects and PNG for sharing flattened artwork.
This is the Windows 0.1 support scope; see [the release decision](RELEASE_DECISION.md).

| Format | Open/import | Save/export | What is retained |
| --- | --- | --- | --- |
| Dudel `.dudel` | Yes, including supported older native readers | Yes | Editable layers, folders, masks and document state supported by this build |
| PNG | Image import | Flattened opaque or transparent export | Pixels; no editable layer hierarchy |
| Clip Studio Paint `.clip` | Editable raster subset; optional saved-preview fallback | No | Raster and Gray layers, folders, clipping, standard blends, locks, saved animation frame; unsupported layers skipped and listed |
| JPEG, WebP, BMP, GIF images | Drop or paste copied files as raster layers | No source-format export | Static decoded image, scaled down to fit the current canvas; no animation editing |
| Project package `.zip` | Yes | Yes | Exact embedded project plus editable layer/mask PNGs, folder paths, references, preview and manifest |
| Theme `.dudeltheme` | Yes | Yes | Workspace theme colors; separate from artwork |
| Photoshop `.psd` | RGB 8/16-bit PSD v1 | No | Supported 8-bit layers/groups/masks; saved composite for other features |
| Indexed `.didx` | Experimental opt-in | Experimental opt-in | Development format; use ordinary `.dudel` for normal work |

Native saves use v4 unless layer masks require v5. Opening v5 projects requires
a mask-aware Dudel build; do not assume an older executable can read a newer file.
The supported document envelope is 1-4096 pixels per side, at most 16,777,216 pixels
in total (a full 4096 x 4096), 32 raster layers, 64 total layer/folder entries and folder
depth six. Mask-aware projects have a combined 512 MiB raster-and-layer-mask payload budget.
These limits also apply to PNG, CSP and PSD document imports. Builds before
2026-10-22 accept at most 4,194,304 pixels and cannot open larger native projects.

## Clip Studio Paint import

Open or drop a `.clip`, or choose File > Import > CSP raster canvas. Successful
imports create an **unsaved copy**. Save as `.dudel`; native saving protects CSP
and PSD source files, including renamed files recognized by their signatures.

| Source feature | Current behavior |
| --- | --- |
| Supported 8-bit RGBA tiles, partial alpha, names and offsets | Editable raster layers |
| Gray expression-color layers | Editable raster layers (gray expanded to RGB) |
| Ordinary folders, nested folders, clipping | Native hierarchy/compositor |
| Standard blend modes, edit/alpha locks, opacity, visibility | Mapped to native properties |
| Plain paper | Rasterized color layer |
| Timelines | The frame the file was saved on; other frames and keyframed layer changes are not imported |
| Animation folders | Every cel kept as a layer; only the cel shown on the saved frame is visible |
| Editing rulers, guides and light-table overlays | Omitted, listed in Import details |
| Vector, text, balloon, material, correction and effect layers; masks, palette colors, monochrome, glow/pass-through blends | That layer (or folder and its contents) is skipped and named in Import details |
| Layer color settings | Pixels keep their stored colors; noted in Import details |
| Profiles, proofing and color adjustments | Layered conversion rejected |
| Unknown channel layouts, damaged references or more than 32 raster layers | Layered conversion rejected |

A skipped layer never has its stored pixels misread as ordinary raster art; it
is left out and listed by name. If CSP's selected layer was skipped, the first
imported raster layer is selected instead.

The cel shown on the saved frame comes from the animation folder's track
keyframes (constant interpolation over the track's time base). If they cannot be
read, Dudel uses the cel name the track stores and says so, since that value can
lag behind the saved frame. A cel name that is not inside the folder shows
nothing, as in CSP.

Folders without a mask no longer fail solely because of their folder masking
flag. Standard blend identifiers and edit/alpha-lock flags are mapped using the
observed file representation and the independent
[clip_to_psd reader](https://github.com/dobrokot/clip_to_psd). Group and blend
rendering uses Dudel's compositor; this is not a promise of CSP rendering parity.

If layered import fails, **CSP import options** offers **Open saved preview**.
This explicitly imports one flattened saved image at its actual pixel dimensions;
it never silently upscales it to claim original resolution. Many CSP previews
are only 1000 × 1000 even for larger canvases. Animation previews represent one
saved frame. To retain full resolution when layered conversion is unavailable,
export PNG from CSP and keep the original `.clip` for editing.

## Photoshop import

Open/drop `.psd` or use File > Import > Photoshop document. The bounded reader
implements PSD v1 RGB, raw/PackBits/ZIP, and 8-bit ZIP prediction. Supported 8-bit
raster layers retain offsets, alpha, opacity, visibility, clipping, common blends,
locks, ordinary isolated groups, and simple raster masks. Plain RGB solid fills
become raster layers. Pass-through groups, complex masks, effects, smart objects,
text and other unsupported descriptors use the saved composite with an explicit
Import details report. They are not silently approximated as editable layers.

16-bit RGB imports use the saved composite and convert to 8-bit channels. Extra
alpha channels are not mistaken for transparency; merged transparency uses the
PSD layer-count marker. Embedded profiles are reported but not converted.
PSB, non-RGB modes, 32-bit channels and 16-bit ZIP prediction are unsupported.
Files are capped at 512 MiB and decoded layer payload is bounded. A PSD without
a usable compatibility composite cannot fall back when layer conversion fails;
save it with Maximize Compatibility enabled in the source editor.

The reader follows [Adobe's PSD specification](https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/).
Neither format has an export writer. Failed, canceled and stale background loads
preserve current artwork.

## Verification scope

Regression tests cover container/tile validation, PSD raw/RLE/ZIP decoding,
layer offsets/order/alpha/masks, truncated streams, cancellation, source protection
and background publication. `--verify-import INPUT [NEW_REPORT.json]` exercises
the same editable import path without opening the editor;
`--verify-clip-preview INPUT [NEW_REPORT.json]` validates the explicit preview path.
Reports include dimensions, layer counts, notes and per-layer pixel hashes.
For PSD input the report also compares Dudel's rendering of the import with the
file's saved composite (`saved_composite`: exact, differing and mean difference).
A PSD converted from a 2000 × 2000 CSP sample with two folders and Subtract
layers imported with all 15 entries; 89% of pixels matched exactly, with a mean
channel difference of 1.2, and the differences lie along anti-aliased line art.

A local 30-file CSP corpus was checked during this change: 29 layered imports
succeeded (up from 9) and all 30 saved previews decoded. The remaining file has
more raster layers than Dudel allows. Nine layers were skipped across the corpus
(4 vector, 1 text, 4 unsupported layer types or blend modes). Downscaled renders
of animation, Gray-layer and skipped-layer samples were compared by eye with
CSP's saved previews and showed the same frame. Source hashes stayed unchanged.
These private samples are not distributed. Unknown formats and full application
rendering parity remain outside this verification claim.

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
