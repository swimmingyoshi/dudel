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
| Photoshop `.psd` | RGB/grayscale 8/16-bit PSD v1 | No | 8-bit layers, groups, masks, vector shapes, gradient fills, layer effects and adjustment layers; saved composite for other features |
| Indexed `.didx` | Experimental opt-in | Experimental opt-in | Development format; use ordinary `.dudel` for normal work |

Native saves use v4 unless layer masks require v5. Opening v5 projects requires
a mask-aware Dudel build; do not assume an older executable can read a newer file.
The supported document envelope is 1-16,384 pixels per side, at most 100,000,000 pixels
in total (Clip Studio Paint's 10,000 x 10,000), 1,000 raster layers, 2,000 total layer/folder
entries and folder depth ten (Photoshop's nesting limit). Stored pixels (painted tiles only)
are capped at 2 GiB, raster and layer masks combined; builds before 2026-10-29 accept at
most 4096 x 4096 and 512 MiB in mask-aware projects. Builds before this change open at most 32 raster layers.
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
| Image material layers (placed pictures) | Editable raster layers: the stored original is resized and placed where CSP shows it (CSP saves only the original). Distorted (non-rectangular) placements are skipped |
| Layer pixels beyond the canvas edge | Kept in CSP; Dudel imports the part on the canvas |
| Canvases set in inches | Converted at the canvas resolution; other physical units are unsupported |
| Color proofing or canvas color adjustment that is switched off | Ignored (only switched-on settings change the picture) |
| Ordinary folders, nested folders, clipping | Native hierarchy/compositor |
| Standard blend modes, edit/alpha locks, opacity, visibility | Mapped to native properties |
| Plain paper | Rasterized color layer |
| Timelines | The frame the file was saved on; other frames and keyframed layer changes are not imported |
| Animation folders | Every cel kept as a layer; only the cel shown on the saved frame is visible |
| Editing rulers, guides and light-table overlays | Omitted, listed in Import details |
| Glow dodge, Add (glow), Through (pass-through folders) | Native Color dodge (glow), Add (glow) and Pass through |
| Layer palette color labels | Ignored (a Layers-panel label; it does not change the picture) |
| Vector layers drawn with plain round brushes | Drawn from their strokes into editable raster layers (CSP saves strokes, not pixels). Width follows each point's pen pressure; CSP's tapering and brush curves are approximated, so lines can be slightly heavier. Strokes using textured, spray or watercolor brushes skip the layer |
| Text, balloon, material, correction and effect layers; masks, monochrome | That layer (or folder and its contents) is skipped and named in Import details |
| Layer color settings | Pixels keep their stored colors; noted in Import details |
| Profiles, proofing and color adjustments | Layered conversion rejected |
| Unknown channel layouts, damaged references or more than 1,000 raster layers | Layered conversion rejected |

The Import details window opens by itself only when something was left out or may
look different (a skipped layer, an effect Dudel cannot draw). Background that does
not change the picture (the saved timeline frame, animation cels, drawing aids, how
the file was read) waits behind the **Import details** button in the status bar.

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
implements PSD v1 RGB and grayscale, raw/PackBits/ZIP, and ZIP prediction. Supported 8-bit
raster layers retain offsets, alpha, opacity, visibility, clipping, blends (including
Dissolve, and Linear/Color dodge saved with Transparency Shapes off as their glow
forms), locks, isolated and pass-through groups, and simple raster masks. Plain RGB
solid fills become raster layers. Text and smart-object layers import as the pixels
Photoshop stored for them (named in Import details); fill opacity applies except on the
dodge/burn family, where it behaves differently. Grayscale documents expand to RGB.
Vector shapes with solid fills are drawn from their paths (Photoshop marks their stored
pixels as not matching what it draws): all four path operations, even-odd and non-zero
winding, and solid strokes inside, centred or outside the combined outline. Vector
masks on ordinary layers and folders become layer masks. Strokes use round joins
(Photoshop's miter corners differ slightly) and stroke anti-aliasing is approximate.
Gradient fill layers and gradient-filled shapes are drawn too (linear, radial, angle,
reflected and diamond; angle, scale, offset, reverse, align with layer; color and
opacity stops with midpoints). Stops blend linearly, where Photoshop smooths them, so
gradients can differ by up to about 16 levels. Noise gradients, stops using the
editor's foreground/background colors, pattern fills, dashed strokes and other
unsupported descriptors use the saved composite with an explicit Import details report.
Layer and vector masks keep their density and feather (a Gaussian blur of the feather
radius); when a layer has both, the user mask Photoshop stores separately is used. They are not silently approximated as editable layers.

Layer effects (layer styles) on ordinary layers import as Dudel layer effects, so they
stay editable (fx in the layer properties). Where Dudel has no equivalent (shape and
fill layers, whose effects follow the shape outline; fill opacity; folders) they become
separate layers instead, as Photoshop's Layer > Layer Style > Create Layers does. Either
way they are drawn the same: drop shadows and outer glows under their layer, color and
gradient overlays, inner shadows, inner glows and inside strokes clipped to it, and
strokes (outside, inside or centred; color or gradient fill) where Photoshop draws
them; bevel and emboss (inner, outer, emboss and pillow; smooth or chisel) becomes a
highlight and a shadow layer lit from the effect's angle and altitude. Effects on
folders follow the folder's composited contents: their outer effects go under the
contents and inner ones clip to the folder. Separate effect layers are named after
their layer ("Layer's Drop Shadow") and no longer follow edits to it. Several effects of one kind, the light angle (global or own),
knockout and fill opacity are respected; shape and fill layers' effects follow the
shape's outline. Shadows and glows use a Gaussian blur fitted to Photoshop's output
(Photoshop does not document its blur), within about 5 levels on average in the test
files; bevels are approximate (about 3 to 18 levels). Satin, stroke emboss, pattern
overlays and pattern strokes, and noise gradients are skipped and named in Import
details; contours other than linear, bevel textures and effect noise are drawn
without and noted.
Adjustment layers import as Dudel adjustment layers, which recolor everything below them
(or only their clipping base when clipped; inside a pass-through folder they reach
below it, inside an isolated folder only its contents), faded by their opacity and
mask: invert, posterize, threshold, levels, curves, brightness/contrast (current and
legacy), exposure, hue/saturation (including colorize and hue ranges) and gradient
maps. The curve shapes follow Photoshop as measured by the MIT-licensed psd-tools
project; in its adjustment test files invert, threshold, posterize, curves, levels,
exposure and gradient maps match within a few levels, brightness/contrast within
about 13, and hue/saturation within a few levels except where several hue ranges
change saturation together. Color balance, vibrance, black & white, photo filter
(RGB, Lab or XYZ colors), channel mixer and selective color import too, but no
reference implementation measures them, so they are Dudel's own approximations: the
psd-tools single-layer files and the ag-psd adjustment stack match, while strong
settings can differ from Photoshop. Color lookup (3D LUT) layers, and the color-only
kinds on grayscale documents, are left out and named in Import details. Imported adjustment layers are ordinary Dudel adjustment layers:
their settings open in the Adjustment window (hue/saturation color ranges and
gradient-map middle stops are kept but only the master settings and end colors
are editable).
Document-wide blocks after the layer list (patterns, text engine data, linked files)
no longer block layered import; files with only a background open as one layer.

Photoshop saves a transparent document's flattened image blended with white. Dudel
detects this (no semi-transparent pixel can be darker than its white share) and
removes it, so flattened imports keep their true edge colors; plain-color images from
other writers are left as they are.

Writers disagree on layer order: Photoshop lists the bottom layer first, others the
top. Group markers only nest one way, so grouped files decide it themselves. For flat
files Dudel renders a sample grid both ways and keeps the order matching the file's
saved image, saying so in Import details when it reads a file top first. A 146-layer
4096 x 2249 rig PSD written top first imported with every pixel within one level of
its saved image.

Public Photoshop-saved test files from the MIT-licensed
[ag-psd](https://github.com/Agamnentzar/ag-psd) and
[psd-tools](https://github.com/psd-tools/psd-tools) projects were checked (not
distributed). ag-psd: 59 import with layers and 6 are background-only; psd-tools,
which concentrates on edge features: 199 of 279 with layers. Layered imports match the
saved image except anti-aliased text edges (Photoshop renders text slightly differently
for the flattened image), a writer that flattened onto white without a background layer,
and two small pass-through/fill cases. The rest fall back to the flattened image or are
unsupported color modes (CMYK, Lab, indexed, bitmap, 32-bit) and PSB.

16-bit RGB imports use the saved composite and convert to 8-bit channels. Extra
alpha channels are not mistaken for transparency; merged transparency uses the
PSD layer-count marker. Embedded profiles are reported but not converted.
PSB, CMYK, Lab, indexed and bitmap modes, and 32-bit channels are unsupported.
Files are capped at 2 GB (the PSD limit) and decoded layer payload shares the 2 GiB
document budget. A PSD without
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
file's saved composite (`saved_composite`: exact, differing and mean difference;
layer effects and adjustment layers are included, as they are in exports)
and lists the imported stack (`stack`). With `DUDEL_VERIFY_IMAGES` set, the
rendering, the saved image and each layer are also written as PNGs next to the report.
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
