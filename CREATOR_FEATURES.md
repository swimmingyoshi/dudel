# Timelapse, themes and reference organization

## Timelapse

Tick **Record a timelapse** when creating a canvas (the choice is remembered),
or press **Record this canvas** in **View > Timelapse** for one already open.
Recording starts with the first stroke and saves a frame of the settled artwork
at most every few seconds (2-60, under **Recording options**). Frames go to the
**Timelapses** folder (**View > Settings > Storage & folders**), one folder per
canvas, so nothing is asked while you paint. An untouched canvas leaves no
folder behind.

Each tab keeps its own recording: switching tabs never stops it, and the tab
shows a red dot while it records. The status bar shows **REC** (or **Paused**)
beside the usual messages; click it for the controls. Unchanged artwork is never
captured twice, so breaks and edits that leave the picture as it was add no
frames. Pause leaves out work you plan to undo. Stop captures the latest state;
**Continue recording** later adds to the same recording.

A saved project's recording continues automatically when the project is opened
again, as long as it was not stopped and the canvas size is the same. Resizing
the canvas stops the recording and keeps its frames.

The **Playback** section plays the recording in the window, while it is still
recording too: Play/Pause, drag the timeline to scrub, and set frames per second
(1-30); the length shown includes a 2-second hold on the finished artwork.
**Export GIF…** writes that playback at Large (1280 px), Medium (960 px), Small
(720 px) or Tiny (480 px) size. Each GIF frame stores only the pixels that
changed, so export is fast and files stay small; repeated frames just lengthen
the previous one. Export shows progress, can be canceled, and never replaces a
file that is not a GIF. GIF has a limited palette; **Export MP4** writes
full-color H.264 video at the selected size and speed, including the final hold.
MP4 requires FFmpeg with libx264 on PATH. Video export supports progress and
cancellation, and odd pixel dimensions are padded with white to even dimensions.
**Show frames** opens the numbered PNG frames for a video editor.
**Open an earlier recording…** plays and exports any recording folder.

See [export options](docs/EXPORT_OPTIONS.md) for image formats and encoder setup,
and the [animation plan](docs/ANIMATION_PLAN.md) for future editable timelines.

Recordings contain artwork, including the paper background, up to 1280 pixels
on the longest side. Panels, the cursor and reference images are excluded.
Original projects, edit history and PNG exports are unaffected. Limits are 7,200
frames and approximately 1 GiB of frames per recording (one final frame can
cross the byte threshold). Only one frame is written at a time; a busy disk
delays the next frame instead of queuing more. This is interval capture, not an
exact replay of every edit. Closing the application can omit the last frame
being written.

## Window controls

Dudel uses a borderless window with theme-colored minimize, maximize/restore and
close buttons in the top bar. Drag the document title or empty space between
the menus and window buttons to move it; double-click that area to maximize or
restore. Drag an edge or corner to resize. Window movement and resizing use the
operating system or Linux compositor, including its tiling rules.

**File > Minimize window**, **Maximize window / Restore window** and **Exit**
provide menu alternatives. Exit, the close button and Alt+F4 use the same
Save / Discard / Cancel flow for unsaved artwork. Closing waits for existing
save and export cleanup. Fullscreen disables resizing and maximize controls.

## Themes

Choose **View > Theme**: Dudel Dark, muted warm-gray Paper or Graphite. The choice persists in
preferences. Import/export uses small `.dudeltheme` JSON files. Start from
`examples/themes/plum.dudeltheme` or export a preset and edit its RGB triplets.
Imported palettes validate their name and minimum text contrast. Themes change
the interface only; artwork colors and exported images retain their values.

Use **View > Theme > Edit theme** to change panel, menu, text and selection
colors with live preview. **Outside canvas** sets the empty workspace around
the artwork, independently of the document's paper and transparency display.
Start from a preset or name your own palette, then **Apply theme** to retain it.
**Reset** returns to the palette from when the editor opened; Cancel or
closing the editor discards the preview. Unconfirmed previews are not saved.
Unreadable text combinations show a warning and keep the last valid preview.
The editor keeps its draft sample and Apply / Save theme / Cancel controls visible
while the color form scrolls. The draft sample always updates, including temporary
low-contrast combinations; the workspace keeps the last readable colors until you
fix them. Editing the name does not pause color preview. **Save theme...** writes
the current draft to a `.dudeltheme` file without closing the editor; **Apply theme**
keeps it as your workspace choice. Invalid drafts explain what needs fixing.
The editor keeps the controls and pickers in their opening palette while the
workspace previews your draft. Contrast feedback reserves a fixed area so
picker anchors stay in place. Picker RGB/linear input preferences survive
live theme changes; draft edits repaint the sample even while workspace
preview is paused.
If text contrast disables Apply / Save, **Restore readable colors** restores the
last readable panel, window, text and selection colors while retaining your
name and outside-canvas color. You can also adjust the swatches manually.
The drawing wheel releases its ring/square mode after each contact, so clicking
the square and then the hue ring works independently.

Your outside-canvas color persists in preferences and exported `.dudeltheme`
files. Older files without that color retain the original dark surround.

## Reference images

Reference layers sit in the normal layer list. Drag a row above/below artwork
or into a folder; the row context menu also has **Move into folder** and
**Move up / Move down**. Folder visibility, opacity, masks and collapse apply
to the board. New reference layers start above the selected artwork entry.
Older above/below references keep their placement until reordered. The quick
Above/Below artwork choices remain in **Reference settings**.

A reference layer can hold **12 PNG images**, with up to eight reference layers
per project. Name boards Hair references, Boots references, or any other category.
Select the layer and choose **+ Add images...** (multiple files can be selected).
Batch PNG decoding runs in one background worker with **Cancel import**. The
batch is added together only when every image passes validation; failure,
cancellation or opening another project leaves existing boards unchanged. The
target board's image transforms are held while importing; other boards and
artwork stay usable. A thumbnail strip selects the individual image to move/scale/rotate with
**Pointer (O)**. **Images menu > Arrange images** lays out the board as a grid;
**Remove image** removes only the selected image. Adding images arranges the
board automatically. Visibility, opacity and position lock are shared by the
board. **Done** or Esc returns to drawing. Images can extend outside the canvas.

New images are reduced to a 1536-pixel longest edge and share a **128 MiB RGBA
import budget per project**. Image allocations are shared with display workers;
unchanged textures are reused. In-stack boards use bounded display tiles and
small thumbnail textures; images outside the canvas retain a view texture.
These are payload limits, not a measured total process/VRAM or frame-rate promise.

Boards, image placements and layer positions persist in project saves and
recovery. References stay out of PNG export, fill sampling and timelapse, and
reference transforms remain independent of artwork undo. Folder moves carry
boards by folder identity. Deleting a folder preserves its reference boards at
the document root; undoing that deletion restores the association in the session.
Older files with single-image references continue to load. Older builds reject projects using multi-image boards or explicit stack positions
rather than silently dropping those settings.

## Brushes

Dry gouache, Charcoal, Round wash and Flat bristle join the six existing presets.
They use distinct flow, pressure, grain and tip settings; embedded textures travel
with brush packs. Existing libraries are preserved: use the library's
**Add built-in brushes** action to add missing presets. Customized entries
with an existing preset name are retained.

Presets belong to one of four paint tools: Pencil, Pen, Brush or Airbrush. The
library shows the active tool's presets (searching covers all of them), and
**Brush settings > Tags > Group** moves a preset to another tool; update the
preset to keep the change. Presets saved before groups existed are sorted by their
name and tags (pencil, sketch and charcoal go to Pencil; ink, liner and marker to
Pen; airbrush, spray and very soft large brushes to Airbrush; everything else to
Brush). Mechanical pencil, Soft 6B, Maru pen, Brush pen, Hard airbrush, Spray and
Blending wash join the built-in set; **Add built-in brushes** adds them to an
existing library.

## Extensions and development checks

See [plugin authoring and sharing](PLUGIN_DEVELOPMENT.md) and
[quick versus full verification](TESTING.md). Routine verification skips release
builds, diagnostic feature builds and large-memory fixtures. UI themes are
applied only when changed; offscreen brush rows avoid preview rendering, hashing
and texture uploads. End-to-end frame/pen latency still requires native measurement.

## Compact tool settings

The brush sidebar uses fixed **Brush / Tip / Texture / More** pages instead of
stacked collapsible groups. Brush keeps size, opacity, flow, hardness,
stabilization and pressure toggles together. Tip controls shape/rotation; Texture
controls grain; More selects the full pressure, dynamics, spacing/taper, scatter,
name/tags and test-pad controls. **Studio** opens the larger editor.

The preview and preset row are smaller, with modified preset names highlighted
instead of a separate warning banner. The library still has search, favourites
and its resizable divider. Color uses a complete adaptive-size wheel and a fixed
hex/current-color row; **Swatches** opens recent colors, the full palette and its
save/import/export actions. A few palette colors stay one click away beside it.
Short windows scroll only the active settings page, keeping neighboring regions
in place. Drag the dividers to rebalance library and color space.

**Palette > Extract from canvas** or **Extract from layer** finds the main colors
of the visible artwork or the active layer (3–12, set by the slider), most-used
first. With a selection active, only the selection is read. Transparent pixels
and the white paper are ignored, and the same artwork always gives the same
colors. Results appear under **Color ideas** and become swatches only through
**Save all** or a color's right-click menu. **Color ideas** also suggests colors
built from the current color: shading ramps with cooler shadows and warmer
highlights, analogous, complementary (through neutral), split-complementary and
triadic hues, and steps toward the secondary color. Clicking a suggestion paints
with it and keeps the row in place; picking a color anywhere else rebuilds the
row. Everything runs in Oklab inside Dudel, with no model or download.
