# Dudel release notes

Windows x64 is the supported platform; Linux builds are experimental. Each
release is a portable ZIP: extract it and run `dudel.exe` (see the
[README](README.md#download-and-install)).

## Unreleased

- **Gradients:** the Fill tool has a Gradient mode (tap **G** again). Drag
  across the canvas for a linear, radial, angle, reflected or diamond gradient
  from the main to the secondary color, the main color to transparent, or your
  own stops. Selections and Lock transparency apply, and on a mask a gradient
  makes a soft fade.
- **New filters:** **Filter > Gaussian blur**, **Sharpen** (unsharp mask) and
  **Add noise**, with a live preview, inside the selection, as one undo step.
- **Filter > More adjustments:** every adjustment-layer type (Curves, Gradient
  Map, Color Balance, Selective Color, Posterize, Threshold and the rest) can
  also change the layer's pixels directly, with the same editor and a live
  preview.
- **Named workspaces:** **View > Workspaces** saves the panel arrangement under
  a name and switches between saved arrangements.
- **Smudge and blur brushes:** a brush's new **Mode** setting smudges (drags the
  paint under the brush along the stroke) or blurs instead of painting, with
  Opacity as the strength, and built-in **Smudge** and **Blur** presets (in an
  existing library: **Library > Add built-in brushes**).
- **Brush graphs:** Brush Studio's new **Brush graph** section wires pen
  inputs (pressure, speed, tilt, twist, distance, random) through curves and
  math nodes into a brush's size, opacity, flow and spacing, with a live
  preview and its own undo. **Make graph from current settings** starts from
  the brush as it is and paints identically. Brush packs that contain a graph
  are written as version 3, which older Dudel refuses rather than importing
  without the graph.
- **Brush Studio in its own tab:** the Studio opens as a tab beside your
  projects and uses the whole canvas area, so the graph editor has room.
  **Float as window** brings back the floating window.
- **Sharing brushes:** **Export brush…** saves a `.dudelbrush` file with your
  name, a license and a description. **Import brush…** (or dropping a file)
  shows the brush, who made it and its license before adding it.
- **Color in brush graphs:** Hue shift, Saturation, Brightness and Color mix
  outputs change each dab's color: color jitter, pressure toward the
  secondary color, darker or paler by speed or tilt.
- **More brush graph nodes and templates:** Rotation and Scatter outputs;
  Direction, Tilt direction, Wave, Noise and Steps nodes; and templates for
  effects the plain settings cannot make: Downstroke ink, Twisting ribbon,
  Dashed line, Wobbly ink, Color jitter, Pressure color, Speed spray, Stipple
  and Tilt shading.
- **Test pad in view:** Brush Studio keeps a true-size test pad below its
  settings, with Undo, color stripes for trying smudge and blur, and a choice
  of paper.

## 0.3.0 (2026-11-10)

A polish release for the first outside testers: a friendlier start, clearer
panels and dialogs, and a report when something goes wrong. Projects and file
formats are unchanged from 0.2.0.

- A **welcome window** on startup: start a new canvas, open a file or a recent
  project, and see the few keys worth learning first. Untick *Show this window
  when Dudel starts* to skip it; **Help > Welcome screen** brings it back.
- If Dudel ever has to close because of an internal error, it now says so
  instead of vanishing, keeps a short problem report and reminds you that
  unsaved work can be restored from the recovery snapshot on the next start.
  **Help > Problem reports folder** opens the reports; nothing is sent anywhere.
- The **Help** menu links to the quick start guide, the problem report form and
  the releases page (**Check for updates…**). Dudel still never connects to the
  internet by itself.
- Empty checkboxes and radio buttons are clearly visible in dark themes; before,
  they blended into dialog backgrounds.
- **Dialogs share one layout:** a title with a close button, and the buttons
  at the bottom right with the main action highlighted and Cancel last. Esc or
  the close button cancels. Unsaved-changes questions offer **Save**, **Don't
  save** and **Cancel**.
- **A tidier layer toolbar:** new layer, folder, duplicate, merge, delete, add
  mask and the three locks stay; layer effects, adjustment layers, reference
  images and every mask command are in its **⋯** menu. The Artwork/Mask row
  appears only for layers that have a mask, leaving room for another layer.
  **Layer > Layer mask** has the mask commands too.
- **Color and Navigator are tabs** above the Layers panel, so the color wheel
  always has room and no longer scrolls out of sight on smaller screens. Dudel
  remembers the tab you chose. When the tool settings panel is open, color sits
  there as before.
- The Layers panel starts taller, showing more layers on laptop screens. The
  Navigator no longer repeats the zoom and rotation sliders of the status bar.
- **Color ideas:** **Palette > Extract from canvas** (or **from layer**) finds
  the main colors of the artwork, or of the selection, and the palette suggests
  shading ramps and related hues built from the current color. Suggestions
  become swatches only when you save them.
- Brush names in the pen quick menu are no longer cut short.
- The blend mode list uses the whole window height, so every mode stays visible
  wherever the Layers panel sits.

## 0.2.0 (2026-11-09)

Editable text, an export window that remembers its settings, and a quick menu
under the pen. Projects with text layers are saved as format v8, which 0.1.0
can't open; it refuses them instead of dropping the text. Everything else in
0.1.0 is unchanged.

### Text

- A **Text tool** (T) for typing straight onto the canvas: click to type on one
  line, or drag a box for text that wraps. The box has handles to resize it, and
  text layers move, scale and rotate with the usual transform.
- Fonts installed on Windows, with proper kerning and ligatures, Japanese and
  other scripts through the system's input methods, and vertical text.
- Formatting: font and style, size, color, bold, italic, underline,
  strikethrough, letter and line spacing, alignment and justify, and an outline.
  With part of the text selected, bold, italic, underline, strikethrough and
  color apply to the selection alone.
- **Fit text to box** (Format menu) shrinks text that would overflow its box,
  never past the size you chose, and re-fits it as you type or resize.
- Hebrew and Arabic read right to left, with Latin words and numbers among
  them kept left to right.
- Text stays editable after saving and reopening. Clip Studio Paint text opens
  as editable text where its settings are supported.
- **Done** never freezes on large text. While the final drawing finishes, the
  bar shows **Keep editing** (Esc), and large text draws about 7x faster than
  before. If the text's font is missing, Done keeps your typing and asks you
  to choose an installed font.

### Exporting images

- **File > Export > Export image…** (Ctrl+Shift+E) opens one window for PNG,
  WebP, JPEG, BMP and GIF, with JPEG quality, a background color for flattening
  and transparency where the format can keep it. A preview shows the result,
  including JPEG quality loss and GIF color limits, before you pick a file.
- **Export again** (Ctrl+Shift+R) writes the project's last export to the same
  file with the same settings, with no questions asked.
- Timelapses also export as MP4 if FFmpeg is installed.

### Pen quick menu

- Press the pen's side button, or right-click, on the canvas. A ring opens under
  the tip with your starred brushes, recent colors and up to four commands of
  your choice (**View > Settings > Drawing**). One tap picks; the side button
  again, Esc or a tap outside closes it without painting.
- On Windows the side button now works as a right-click everywhere in the app,
  for example in the Layers panel menus.

### Smaller changes

- **Add mask** adds a mask in one click: showing everything, or only the
  selection when there is one. The menu beside it still offers a mask hiding
  everything.
- One mouse-wheel notch over the blend mode moves exactly one mode; it used to
  skip a mode at times.
- Dropping or pasting a file that only looks like an image, such as a web page
  saved as `.webp`, now says so instead of reporting a decoding error.

### Limits in 0.2.0

- Each text layer has one font, size and spacing; only bold, italic, underline,
  strikethrough and color can differ within it.
- Text exports as pixels to PSD, and SVG export leaves text out. Text on a path
  isn't supported, and plugins can't create text.
- Each line takes its direction from its first letter, so a wrapped line of
  Hebrew that starts with a Latin word reads left to right. Justify doesn't
  stretch lines that mix directions.
- Switching tabs, closing a project, undo and save still wait for a large
  text's final drawing (about half a second at worst on a 100 MP canvas).
- The export preview is reduced to 256 pixels, so fine detail can differ from
  the file. Only PNG keeps the project's DPI.

## 0.1.0 (2026-11-05)

The first public pre-release.

- **Drawing:** Pencil, Pen, Brush and Airbrush with pen pressure, pressure
  curves and dynamics; a Smooth or Rope stabilizer; custom tips, paper grain,
  scatter and tapers; Brush Studio with a test pad; low-latency drawing on by
  default.
- **Fill and shapes:** fill with *Fill to edges* and gap closing, lasso fill and
  erase, mirror and radial symmetry, rulers, guides and a grid with snapping.
- **Layers:** raster and vector layers, folders, clipping, blend modes (including
  glow modes and Pass through), locks, layer masks, editable layer effects and
  adjustment layers; up to 1,000 layers.
- **Editing:** rectangle, lasso, polygon and magic wand selections; free
  transform with distort, perspective and mesh; mirror-pair edits; editable
  vector lines.
- **Workflow:** tabs, command search (Ctrl+K), custom shortcuts, a preview
  window, reference images and boards, timelapse recording with GIF and PNG
  export, and UI themes.
- **Files:** `.dudel` projects with atomic saves and crash recovery; PNG import
  and export; layered PSD/PSB import and export; Clip Studio Paint import for
  the subsets in [format support](docs/FORMAT_SUPPORT.md); SVG export of
  vector lines.
- **Extensions:** Rhai and WebAssembly filters and API 2 plugins for brush
  presets, tool presets, commands and panels.
- **Packaging:** a portable ZIP with the C runtime built in and optional
  `.dudel` thumbnails in File Explorer.

## Known limits

- Canvases are up to 16,384 pixels per side and 100 megapixels in total.
  Documents hold up to 1,000 raster layers, 2,000 layer and folder entries and
  ten folder levels, with 2 GiB of stored pixels including masks.
- Newer projects need a matching Dudel: text layers need 0.2.0, and older
  versions refuse such files instead of dropping what they can't read.
- CSP and PSD import cover the subsets in [format support](docs/FORMAT_SUPPORT.md).
  Unsupported content opens from the file's saved preview or composite and is
  listed in **Import details**.
- PSD export writes vector and text layers as pixels. SVG export holds vector
  lines only.
- There is no color management for print work.
- Builds are not code-signed yet, so Windows SmartScreen may ask before the
  first start (**More info > Run anyway**).

## Tested systems

Dudel is tested on Windows 11 x64 with a desktop RTX 3060 and an Intel UHD
laptop, and with an XP-Pen Artist 12 pen display. Minimum Windows, memory and
graphics requirements have not been established yet; Windows 10, Windows on ARM
and other tablets have not been tested. Linux builds pass automated checks but
lack hands-on tablet testing. Reports from other systems are very welcome.

## Reporting a problem

Use **Help > Report a problem…** or the
[issue form](https://github.com/swimmingyoshi/dudel/issues/new/choose). Include
the version from **Help > About Dudel**, your Windows version, graphics and
tablet drivers, display scaling and the steps that lead to the problem. If Dudel
closed unexpectedly, attach the newest file from **Help > Problem reports
folder**. Attach artwork only if you're happy for it to be public.
