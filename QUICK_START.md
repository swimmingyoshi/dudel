# Start drawing in Dudel

Dudel is a Windows drawing application in development. Save editable work as a
`.dudel` project and export PNG copies when you want to share the image.

The current Windows x64 build uses the Microsoft Visual C++ v14 runtime. If launch
reports a missing VCRUNTIME140.dll, install Microsoft's current x64 redistributable
from the [official download page](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist).

## Create and draw

1. Choose **File > New canvas** (`Ctrl+N`). Choose **Sketch**, **Square**, or **4K**,
   or enter custom width and height. **Swap W/H** changes orientation. The form
   shows pixel count and memory per fully painted layer. Then choose
   **Create canvas**. Each side can be 1-4096 pixels, up to a full 4096 x 4096 canvas, which is enough
   for 4K artwork such as 3840 x 2160. Very large canvases with many fully painted
   layers can reach the 512 MiB artwork budget; Dudel reports that clearly. The new canvas
   opens in its own tab (see [Tabs](#tabs-preview-window-and-folders)); at the tab
   limit it replaces the current project after Save, Discard or Cancel.
2. The tool rail has four paint tools: **Pencil**, **Pen**, **Brush** and
   **Airbrush**. Press **B** for the paint tools and tap **B** again to step
   through them. Each one lists its own presets in the brush selector and keeps
   its own size and settings, so switching back finds the brush as you left it.
   Adjust Size and Opacity in the options bar; click the foreground color chip to
   choose a color. **E** selects Eraser; **X** swaps foreground/background colors.
   The cursor shows the brush tip's real shape: the outline of round and square
   tips, and the tip image itself (in the current color) for textured tips. While
   you draw it follows the ink, growing, shrinking and turning with pressure,
   stroke direction or tilt; the eraser's outline is dashed.
   The Eraser either erases with the current brush's shape (**Brush shape**, the
   default) or with its own round **Eraser tip** with separate strength, hardness
   and pressure settings. In both modes the eraser has its own size: with the
   Eraser selected, the size slider and **[ ]** change the eraser, and the pen
   keeps its size. Switch modes in the Eraser's panel or the brush selector. A
   pen's eraser end uses the same choice, and its dashed outline appears as soon
   as you hover with the pen flipped.
   In **Brush shape** mode, opacity, pressure, texture and stabilizer settings
   are shared with the paint brush; editing them also changes that brush.
   Choose **Eraser tip** for independent eraser settings. Tap **E** to stay on
   Eraser, or hold it for more than half a second and release to return to your
   previous tool. Tap **B** from Eraser to return to the last paint group.
   The **Stabilizer** slider smooths hand jitter; at 0 the line follows the pen
   exactly. The **Smooth / Rope** button beside it chooses how the line follows.
   Choose **Rope** there for a slow-follow stabilizer: the line trails the pen on a
   string (**Distance**, in the same menu, sets its length up to 200 screen pixels),
   holds still until the pen pulls it taut, then follows with a slight delay. It
   gives steady curves and straight runs, a guide line shows the string while you
   draw, and the stroke ends where the line is when you lift. The menu also has
   **Less smoothing on fast strokes**. These settings are saved per brush preset.
   If the line seems to trail the pen even at stabilizer 0, see
   [Pen display and graphics](#pen-display-and-graphics).
3. Draw a few strokes. **Ctrl+Z** undoes; **Ctrl+Shift+Z** redoes. These shortcuts
   act on artwork when you are drawing; text fields retain normal text editing.
4. Hold **Space** and drag to pan; scroll to zoom. **Ctrl+0** fits the canvas and
   **Ctrl+1** shows actual pixels. **H** flips the view without changing artwork.
   The checkerboard button at the right end of the status bar (**Ctrl+Shift+T**, or
   **View > Show transparency**) draws transparent areas as a checkerboard instead
   of white paper; it is remembered, and exports are unchanged.

On a smaller window, the options bar puts additional controls under **Stroke
options**, and the sidebars may hide to give the canvas more room. Use **View >
Navigator, color and layers** or **View > Tool property panel** to show a sidebar.
**Tab** hides/shows panels. **View > Automatic panel layout** restores responsive
panel choices and shows panels again after manual changes.
In short windows, **Navigator** and **Color** open from the inspector's top row.
In the shortest layout, **Layer** holds layer actions, blend, opacity and masks,
leaving room for the selected layer. Narrow inspectors use a draggable opacity
percentage; click it to type a value.

## Build up the illustration

Press **G** for the bucket. **Fill to edges** is on by default and replaces the
background at soft pencil/pen borders to avoid pale seams. Use **Fill options**
in the top bar or the Fill panel to change it, adjust tolerance, close small gaps
or expand/shrink the region. For linework on a separate layer, choose **Visible
layers** or that specific layer as the reference and fill on a color layer below
it. Selections still limit the fill; mask fills use mask coverage boundaries.

Use **Ctrl+Shift+N** to add a layer above the current layer: for example, separate
sketch, ink and color. Select layers in the Layers panel; double-click a name to
rename it. **Ctrl+J** duplicates the selected layer. Layer opacity and blend mode
controls sit above the list. A locked layer cannot be painted on.
The blend list has 26 modes in the usual groups (darken, lighten, contrast,
comparative and color component modes). Hover it and scroll, or press Up/Down,
to step through modes and judge each one on the canvas.

The sparkle (**fx**) button in the layer action row opens Layer Effects for the
selected layer, and a sparkle badge on a layer row marks layers with effects (click it
to edit them; it is crossed out when all are switched off). Effects include drop and
inner shadows, outer and inner glows, strokes, color and gradient overlays, and bevel
and emboss. Effects follow the layer's pixels as you paint (they refresh when the
stroke ends), can be switched off, reordered or removed, and save with the project.
Merge down bakes them into the merged pixels.

**Adjustment layers:** the half-circle button in the layer action row adds levels,
curves, brightness/contrast, exposure, hue/saturation, vibrance, color balance, black &
white, photo filter, channel mixer, selective color, gradient map, invert, posterize or
threshold above the selected layer. It recolors everything below it
(or, clipped, only its base); its opacity and mask fade it. Click its thumbnail to
open the Adjustment window: in Curves, click the graph to add a point, drag to move
it and double-click to remove it.

**Masks:** a masked layer shows its mask thumbnail beside the artwork. Click the
mask thumbnail (or **Mask** under the layer properties) to paint the mask: the
targeted thumbnail gets an accent outline and the options bar shows **Mask**. Paint
black to hide, white to reveal. **Mask actions > Show mask on canvas** (or Alt+click on the
mask thumbnail) shows the mask itself on the canvas in grays while you edit it; click
**Artwork** or the artwork thumbnail to paint the layer again. **Mask actions** has
enable, link, invert, apply and remove.

Press **L** for the Lasso: draw a shape and lift to fill it with the current
color. Its panel switches between **Fill** and **Erase** (or tap **L** again) and
between Freehand, Rectangle and Ellipse outlines, with options to constrain
proportions and draw from the center.

Press **M** for Selection or **V** for Move. The tool property panel exposes their
options. **Ctrl+D** deselects. If you enter a transform preview, finish with its
Apply action or cancel with Escape before continuing other editing operations.

For paired features such as eyes, select **one** feature on the active layer,
then choose **Transform mirrored pair…** in Selection properties. You can also
press **Ctrl+T** and enable **Mirror pair** in the transform properties or the
toolbar's **Mirror** menu. Drag the blue handles: the orange partner outline
follows with reflected movement, rotation and deformation. Both regions use
their own existing artwork; the selected eye is not copied over the other eye.
Both features must be on the active layer (or active mask).

The axis starts from the active layer's symmetry guide, or the canvas center
when no guide is configured. **Mirror axis** adjusts its center and tilt;
**Use symmetry guide** copies the guide again. Select one side of the
axis—the opposite selection is included automatically. A selection that
crosses the axis, such as a circle in the middle, splits along it; the box fits
the driving half. Hold **Ctrl** while moving to snap that half back onto the
axis so both halves meet again. Apply commits both
sides as one undo step, and Escape restores both. The driving selection remains
selected afterward. Turn **Mirror pair** off for an ordinary one-sided edit.

**G** selects Fill and **I** selects Picker. **Alt+click** temporarily samples a
visible color. **Help > Keyboard shortcuts** lists the full bindings.

To draw from a picture, use **Layer > Add reference image…** (or the picture
button above the layer list). References are listed with the layers but are never
part of the artwork: painting, fill, export and undo ignore them. Use the
**Pointer (O)** to manage them: press on a reference to select and move it, drag a
corner to scale and the top handle to rotate (**Shift** snaps to 15°); pressing
empty canvas pans. Selecting a reference in the layer list switches to the
Pointer. Its panel sets opacity, above or below the artwork, flip and lock.
References are saved with the project. Drag their rows among artwork layers or onto folders to organize them.
Use **+ Add images...** in the selected reference's header to add up to 12 PNGs
to one layer. Select a thumbnail to transform that image; **Images** menu has
Arrange images and Remove image. See [reference boards](CREATOR_FEATURES.md)
for memory limits and persistence details.

The color wheel sits under the tool settings on the left; drag the bar above it to
resize. The bars between the brush library, brush settings, color, navigator and
layers all work the same way: drag to give an area more room, double-click to return it
to its automatic size. Dudel remembers the sizes. Drag the panel's edge to make it
wider, which also widens its sliders. Drag the left panel's edge all the way toward the tool rail to close the
tool settings. Clicking a tool in the rail always shows its settings, including
when a smaller window hid them automatically. Keyboard shortcuts switch tools
without opening the panel. Saving, opening and filter progress appear in the status
bar, so the canvas never shifts while they run.

From the canvas, **F6** focuses workspace controls. **Tab** / **Shift+Tab** move
focus, and **Space** or **Enter** activates a button. **Escape** leaves a field or
dismisses the current popup. With no control focused, **Tab** hides or shows panels.

For brush editing, open the brush selector, choose **Brush settings**, then
**Open Brush Studio**. Choose **Test pad** to try strokes with the brush.
The tool property's **Basic** page keeps Size, Opacity, Stabilizer and Follow
visible. Expand **Paint & pressure** for Flow, Hardness and pressure controls;
use **Tip**, **Texture** and **Advanced** for detailed tuning.
**Save as new** keeps a new
preset; **Update preset** replaces the selected preset after confirmation;
**Revert** returns to its saved settings. Brush export/import supports moving
presets between installations.

## Save, reopen and export

**Ctrl+S** saves the editable project; the first save asks for a `.dudel` path.
Wait for **Saved** in the document header. **Ctrl+Shift+S** opens Save as for a
separate project copy. **Ctrl+O** or **File > Open recent** reopens saved work.
Layers and editable document state belong in the project file.

Save as refuses a filename owned by another open tab. Choose a different name,
or close that tab after reviewing its changes, before replacing its file.

Each save stores a small preview of the artwork in the project. On Windows,
**View > Settings > File Explorer previews > Show .dudel previews** makes File
Explorer show it as the file's thumbnail, like CSP files. It needs
`dudel_thumbnails.dll` beside `dudel.exe`, no administrator rights, and applies
to projects saved from then on. Run it again if you move the Dudel folder;
**Remove .dudel previews** returns to plain icons. Use it before deleting the
Dudel folder so Windows no longer looks for the DLL at its old location.

While a save is queued or running, **Cancel save** requests cancellation. Drawing
and undo remain available. Cancellation is checked during encoding and after the
current file flush, before publication. If publication has already passed that
fence, the save may finish and report success. Canceling a save also cancels a
waiting New/Open/Close action; the current artwork stays open.

**File > Export PNG** flattens the image onto white paper. **File > Export
transparent PNG** keeps composited transparency. Export creates an image copy;
continue saving the `.dudel` project for later editing. **File > Import > PNG as
canvas** opens an image as a new document.

To bring images into the current document as layers, any of these work, with PNG,
JPEG, WebP, BMP and GIF files up to 16384 pixels per side:

- **Drag files onto the window.** Each image becomes a new layer above the current
  one, centered where you drop it. Dropped `.dudel`/`.clip` projects open in tabs;
  packages, brush packs, palettes and themes are imported.
- **Ctrl+V** (Edit > Paste image as layer) pastes a copied image, or every image
  file copied in File Explorer, as new layers.
- **File > Import > Images as layers** picks one or more files.

Images larger than the canvas are scaled down to fit; nothing is cropped. The new
layer's area is selected, so **Ctrl+T** transforms it straight away, and each
image is one undo step. **Edit > Paste as reference image** puts the clipboard
image on a new reference layer instead.

**File > Export project package (.zip)** writes a portable copy: the exact
project, every layer and mask as a full-canvas PNG in folders that mirror your
layer folders, the reference images, a flattened preview and a `manifest.json`.
**File > Import > Project package** unpacks it into your Projects folder (never
over an existing file) and opens it. A package without `project.dudel` is
rebuilt from its layer PNGs, so layers edited in another program can come back.

If Dudel offers recovery snapshots at startup, choose **Restore copy**, inspect
the recovered drawing, then save it as a project. **View > Recover artwork**
reopens that list when snapshots are available. Recovery restores the last
completed snapshot, so regular saving is still part of the workflow. Recovery
becomes eligible after 15 seconds and waits for strokes and pending edits to
settle. Continuous drawing, active adjustments or slow storage can increase the
time between completed snapshots; it is not a guarantee to preserve every stroke.

Mask-aware projects share a 512 MiB budget between raster pixels and layer-mask
coverage. Thirty-two fully populated maximum-area raster layers leave no space
for additional mask data. Smaller or sparse layers leave room for masks; the app
rejects projects beyond the supported payload budget.

## Tabs, preview window and folders

Each project opens in its own tab above the canvas. Click a tab to switch,
**Ctrl+Tab** / **Ctrl+Shift+Tab** to cycle, and **Ctrl+W**, the **×** button or a
middle-click to close one. A project that is already open is shown rather than
opened twice. Background tabs keep their artwork, undo history and view but use
no processing time; a switch waits a moment if a save, export or stroke is still
finishing. **View > Settings > Tab limit** sets how many projects can be open
(default 8); at the limit, opening replaces the current project after asking about
unsaved changes, and a limit of 1 keeps the classic single-project behavior.
Quitting asks about each project with unsaved changes by name.

**View > Preview window** opens a second window with only the artwork, to put on
another monitor, show viewers, or capture in OBS (use a Window Capture source).
Right-click it for options: fit the whole canvas or follow the main view, a
workspace, black, white or chroma-green background, a pen pointer, and always on
top. Reference layers are hidden by default. To leave out other layers, untick
**Show in preview window** in a layer's right-click menu or use **Hide layers**
in the preview's menu. Double-click or **F11** toggles full screen.

**View > Settings > Storage & folders** shows where files go. The Dudel folder
(Documents/Dudel by default) holds Projects, Exports, Packages, Themes, Brushes and
Palettes subfolders; each can point elsewhere, and file dialogs start there.
Project dialogs start beside the open project unless you turn that off. Themes
saved to the Themes folder appear in **View > Theme**. Settings, recovery
snapshots and extensions stay in the app data folder shown on the same page.

## Pen display and graphics

Enable Windows Ink in the tablet driver and choose a pressure-sensitive brush.
Open **View > Tablet input** to inspect detected pressure and native pen input.
Try light-to-heavy strokes, slow diagonals, fast curves and lifting the pen.
Mouse-like movement alone does not establish pressure support. Current-build
XP-Pen validation is tracked in [the hands-on checklist](XP_PEN_VALIDATION.md).

### If the line lags behind the pen

At stabilizer 0 Dudel places every dab exactly at the pen, in the same frame it
receives the pen input. Any remaining delay comes from how frames reach the
screen. **Low latency** (in **View > Settings > Display...**) is on by default:
frames are shown without waiting for the display's refresh (vsync) and at most
one is queued. With it off, Dudel waits for vsync and lets the graphics driver
queue up to two frames, so new ink can appear two or three refreshes after the
pen moves (roughly 35-50 ms on a 60 Hz display). On the owner's XP-Pen Artist 12
that was a visible lag, which is why low latency became the default.

If the line trails the pen, check that **Low latency** is ticked; changes apply
after restarting Dudel. If you see tearing (a brief horizontal seam while the
canvas moves) or want the GPU to work less while you draw, untick it. Dudel keeps
your choice.

The same window selects the graphics API: **OpenGL** (default, most compatible),
**Automatic** (DirectX 12 or Vulkan), **Vulkan** or **DirectX 12**. Both choices
apply after restarting; if a chosen API cannot start, Dudel falls back to OpenGL.
**View > Settings > Performance** shows CPU frame and drawing times; it does not
measure pen-to-screen latency.
Supported CSP import remains limited; unsupported content is rejected rather
than treated as a fully compatible editable document.

Build/run instructions are in [README](README.md). The current development
status and remaining release gates are in [the delivery plan](DELIVERY_PLAN.md).

See [file format support](FORMAT_SUPPORT.md) for the native-version and restricted CSP import matrix.

## Timelapse and personalization

Use View > Timelapse to record artwork and export an animated GIF or keep its PNG frames. View > Theme selects or imports a persistent UI palette. Reference rows have separate visibility/lock controls and Move up/down actions. See [creator features](CREATOR_FEATURES.md) for recording limits, reference organization and the new brush presets.

ZIP packages from this build also preserve layer masks (including enabled/link
settings), locks, guides and reference-board placements in the manifest. When
rebuilding from edited PNGs, keep their canvas dimensions and the manifest paths.
Keep project.dudel for an exact project restore, including history-independent
project data that is not represented by editable layer PNGs. Older packages
without reference placement metadata require their embedded project.dudel.

In Rope mode, **Distance** sets rope length from 0 to 200 screen pixels,
independently of follow smoothing. It appears in the toolbar's Smooth / Rope menu,
brush settings, Stroke options and Brush Studio. Save/update the brush preset to keep the setting. Older brush
presets retain their former rope length until you adjust it. A painting contact
can start in the empty canvas surround and travel off the page and back; paint
is clipped to document pixels, and the whole contact remains one undo action.

Layer rows show a symmetry badge for their own settings, a globe for inherited
global symmetry, and a ruler badge for guides belonging to that layer. Click a
badge or **Guides** to edit the layer, assign guides, add/remove guides,
or select the layer and move its handles with Pointer. An Off layer override
disables symmetry there while keeping global symmetry on other layers.
The **Global** control above the layer list edits shared symmetry independently
of layer overrides. Global guide lines, rulers and the grid have their own count.

View > Performance distinguishes active-project allocations from tracked CPU
allocations across every open project, including parked tabs and references.
Canvas/reference/thumbnail RGBA estimates are separate; these are diagnostic
estimates, not total process memory or committed graphics memory. The window
scrolls when needed. Leave it closed during normal performance measurements.

Large canvases now retain more detail near the visible area and use a smaller
representation elsewhere. Detail returns as you pan or zoom; saved artwork and
exports keep their original resolution. Properties > Studio opens the section
you were adjusting. View > Performance includes recent latency percentiles and
cache statistics for diagnosing pauses while drawing.
